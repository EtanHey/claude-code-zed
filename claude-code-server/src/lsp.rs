use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{debug, error, info, warn};

// Notification structures for IDE to Claude communication
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SelectionChangedNotification {
    pub text: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(rename = "fileUrl")]
    pub file_url: String,
    pub selection: SelectionInfo,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SelectionInfo {
    pub start: Position,
    pub end: Position,
    #[serde(rename = "isEmpty")]
    pub is_empty: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AtMentionedNotification {
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(rename = "lineStart")]
    pub line_start: u32,
    #[serde(rename = "lineEnd")]
    pub line_end: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

// Channel for sending notifications from LSP to MCP
pub type NotificationSender = broadcast::Sender<JsonRpcNotification>;
pub type NotificationReceiver = broadcast::Receiver<JsonRpcNotification>;

// Commands from WebSocket/MCP to LSP (for bidirectional communication)
#[derive(Debug, Clone)]
pub enum LspCommand {
    OpenFile {
        file_path: String,
        line: Option<u32>,
        column: Option<u32>,
        take_focus: bool,
    },
}

// Channel types for commands
pub type CommandSender = mpsc::Sender<LspCommand>;
pub type CommandReceiver = mpsc::Receiver<LspCommand>;

// Debounce duration for selection events (ms)
const SELECTION_DEBOUNCE_MS: u64 = 150;

#[derive(Debug)]
pub struct ClaudeCodeLanguageServer {
    client: Client,
    worktree: Option<PathBuf>,
    notification_sender: Option<Arc<NotificationSender>>,
    /// Debounced selection sender - selection events go here first
    selection_debouncer: Option<watch::Sender<Option<SelectionChangedNotification>>>,
}

impl ClaudeCodeLanguageServer {
    pub fn new(client: Client, worktree: Option<PathBuf>) -> Self {
        Self {
            client,
            worktree,
            notification_sender: None,
            selection_debouncer: None,
        }
    }

    pub fn with_notification_sender(mut self, sender: Arc<NotificationSender>) -> Self {
        // Create debouncer channel
        let (debounce_tx, mut debounce_rx) = watch::channel::<Option<SelectionChangedNotification>>(None);
        self.selection_debouncer = Some(debounce_tx);

        // Clone sender for the debounce task
        let notification_sender = sender.clone();

        // Spawn debounce task
        tokio::spawn(async move {
            let mut last_sent: Option<SelectionChangedNotification> = None;

            loop {
                // Wait for a change
                if debounce_rx.changed().await.is_err() {
                    break; // Channel closed
                }

                // Got a new selection, start debounce timer
                loop {
                    tokio::select! {
                        // Wait for debounce period
                        _ = tokio::time::sleep(Duration::from_millis(SELECTION_DEBOUNCE_MS)) => {
                            // Debounce period passed, send the notification
                            let current = debounce_rx.borrow().clone();
                            if let Some(selection) = current {
                                // Only send if different from last sent
                                let should_send = match &last_sent {
                                    None => true,
                                    Some(last) => {
                                        last.file_path != selection.file_path
                                            || last.selection.start != selection.selection.start
                                            || last.selection.end != selection.selection.end
                                    }
                                };

                                if should_send {
                                    let notification = JsonRpcNotification {
                                        jsonrpc: "2.0".to_string(),
                                        method: "selection_changed".to_string(),
                                        params: serde_json::to_value(&selection).unwrap_or_default(),
                                    };

                                    if notification_sender.send(notification).is_ok() {
                                        debug!("Sent debounced selection_changed notification");
                                        last_sent = Some(selection);
                                    }
                                }
                            }
                            break; // Exit inner loop, wait for next change
                        }
                        // New selection arrived, restart debounce timer
                        result = debounce_rx.changed() => {
                            if result.is_err() {
                                return; // Channel closed
                            }
                            // Continue loop to restart timer
                        }
                    }
                }
            }
        });

        self.notification_sender = Some(sender);
        self
    }

    async fn send_notification(&self, method: &str, params: serde_json::Value) {
        if let Some(sender) = &self.notification_sender {
            let notification = JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: method.to_string(),
                params,
            };

            if let Err(e) = sender.send(notification) {
                debug!("Failed to send notification: {}", e);
            }
        }
    }

    /// Send a selection notification through the debouncer
    fn send_selection_debounced(&self, selection: SelectionChangedNotification) {
        if let Some(debouncer) = &self.selection_debouncer {
            let _ = debouncer.send(Some(selection));
        }
    }

    // Convert LSP UTF-16 code unit position to Rust UTF-8 byte position
    // LSP uses UTF-16 code units for character positions per the specification
    fn char_pos_to_byte_pos(line: &str, utf16_pos: usize) -> Option<usize> {
        let mut current_utf16_pos = 0;
        
        for (byte_pos, ch) in line.char_indices() {
            if current_utf16_pos == utf16_pos {
                return Some(byte_pos);
            }
            
            let char_utf16_len = ch.len_utf16();
            
            // If utf16_pos falls within this character's UTF-16 span, return this char's byte position
            if utf16_pos < current_utf16_pos + char_utf16_len {
                return Some(byte_pos);
            }
            
            current_utf16_pos += char_utf16_len;
        }
        
        // If utf16_pos is at the end of the string
        if current_utf16_pos == utf16_pos {
            return Some(line.len());
        }
        
        None
    }

    fn read_text_from_range(&self, file_path: &str, range: Range) -> String {
        let file_path = if file_path.starts_with("file://") {
            &file_path[7..] // Remove "file://" prefix
        } else {
            file_path
        };

        match fs::read_to_string(file_path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();

                // Handle single line selection
                if range.start.line == range.end.line {
                    if let Some(line) = lines.get(range.start.line as usize) {
                        let start_char = range.start.character as usize;
                        let end_char = range.end.character as usize;

                        if let (Some(start_byte), Some(end_byte)) = 
                            (Self::char_pos_to_byte_pos(line, start_char),
                             Self::char_pos_to_byte_pos(line, end_char)) {
                            if start_byte <= end_byte {
                                return line[start_byte..end_byte].to_string();
                            }
                        }
                    }
                } else {
                    // Handle multi-line selection
                    let mut selected_text = String::new();

                    for (i, line_index) in (range.start.line..=range.end.line).enumerate() {
                        if let Some(line) = lines.get(line_index as usize) {
                            if i == 0 {
                                // First line - from start character to end
                                let start_char = range.start.character as usize;
                                if let Some(start_byte) = Self::char_pos_to_byte_pos(line, start_char) {
                                    selected_text.push_str(&line[start_byte..]);
                                }
                            } else if line_index == range.end.line {
                                // Last line - from start to end character
                                let end_char = range.end.character as usize;
                                if let Some(end_byte) = Self::char_pos_to_byte_pos(line, end_char) {
                                    selected_text.push_str(&line[..end_byte]);
                                }
                            } else {
                                // Middle lines - entire line
                                selected_text.push_str(line);
                            }

                            // Add newline except for the last line
                            if line_index < range.end.line {
                                selected_text.push('\n');
                            }
                        }
                    }

                    return selected_text;
                }
            }
            Err(e) => {
                warn!("Failed to read file {}: {}", file_path, e);
            }
        }

        String::new()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for ClaudeCodeLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        info!("LSP Server initializing...");
        if let Some(workspace_folders) = &params.workspace_folders {
            for folder in workspace_folders {
                info!("Workspace folder: {}", folder.uri);
            }
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec!["@".to_string()]),
                    work_done_progress_options: Default::default(),
                    all_commit_characters: None,
                    completion_item: None,
                }),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        "claude-code.explain".to_string(),
                        "claude-code.improve".to_string(),
                        "claude-code.fix".to_string(),
                        "claude-code.at-mention".to_string(),
                    ],
                    work_done_progress_options: Default::default(),
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "Claude Code Language Server".to_string(),
                version: Some("0.1.0".to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("Claude Code LSP server initialized!");

        self.client
            .log_message(MessageType::INFO, "Claude Code Language Server is ready!")
            .await;
    }

    async fn shutdown(&self) -> LspResult<()> {
        info!("LSP Server shutting down...");
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        info!("Document opened: {}", params.text_document.uri);

        self.client
            .log_message(
                MessageType::INFO,
                format!("Opened document: {}", params.text_document.uri),
            )
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        info!("Document changed: {}", params.text_document.uri);
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        info!("Document saved: {}", params.text_document.uri);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        info!("Document closed: {}", params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let position = params.text_document_position_params.position;
        info!(
            "Hover requested at {}:{}",
            position.line, position.character
        );

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let position = params.text_document_position.position;
        info!(
            "Completion requested at {}:{}",
            position.line, position.character
        );

        let completions = vec![
            CompletionItem {
                label: "@claude explain".to_string(),
                kind: Some(CompletionItemKind::TEXT),
                detail: Some("Explain this code with Claude".to_string()),
                documentation: Some(Documentation::String(
                    "Ask Claude to explain the selected code or current context".to_string(),
                )),
                insert_text: Some("@claude explain".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "@claude improve".to_string(),
                kind: Some(CompletionItemKind::TEXT),
                detail: Some("Improve this code with Claude".to_string()),
                documentation: Some(Documentation::String(
                    "Ask Claude to suggest improvements for the selected code".to_string(),
                )),
                insert_text: Some("@claude improve".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "@claude fix".to_string(),
                kind: Some(CompletionItemKind::TEXT),
                detail: Some("Fix issues in this code with Claude".to_string()),
                documentation: Some(Documentation::String(
                    "Ask Claude to identify and fix issues in the selected code".to_string(),
                )),
                insert_text: Some("@claude fix".to_string()),
                ..Default::default()
            },
        ];

        Ok(Some(CompletionResponse::Array(completions)))
    }

    async fn code_action(&self, params: CodeActionParams) -> LspResult<Option<CodeActionResponse>> {
        info!("Code action requested for range: {:?}", params.range);

        // Send selection_changed notification when code action is requested
        let selected_text =
            self.read_text_from_range(params.text_document.uri.path(), params.range);
        let selection_notification = SelectionChangedNotification {
            text: selected_text,
            file_path: params.text_document.uri.path().to_string(),
            file_url: params.text_document.uri.to_string(),
            selection: SelectionInfo {
                start: params.range.start,
                end: params.range.end,
                is_empty: params.range.start == params.range.end,
            },
        };

        debug!(
            "Queueing debounced selection_changed for range: {:?}",
            params.range
        );
        self.send_selection_debounced(selection_notification);

        let actions = vec![CodeActionOrCommand::CodeAction(CodeAction {
            title: "Explain with Claude".to_string(),
            kind: Some(CodeActionKind::REFACTOR),
            diagnostics: None,
            edit: None,
            command: None,
            is_preferred: Some(false),
            disabled: None,
            data: Some(serde_json::json!({
                "action": "explain",
                "uri": params.text_document.uri,
                "range": params.range
            })),
        })];

        Ok(Some(actions))
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> LspResult<Option<Value>> {
        info!("Execute command: {}", params.command);

        match params.command.as_str() {
            "claude-code.explain" => {
                self.client
                    .show_message(
                        MessageType::INFO,
                        "Claude Code: Explain command executed (not yet implemented)",
                    )
                    .await;
            }
            "claude-code.improve" => {
                self.client
                    .show_message(
                        MessageType::INFO,
                        "Claude Code: Improve command executed (not yet implemented)",
                    )
                    .await;
            }
            "claude-code.fix" => {
                self.client
                    .show_message(
                        MessageType::INFO,
                        "Claude Code: Fix command executed (not yet implemented)",
                    )
                    .await;
            }
            "claude-code.at-mention" => {
                info!(
                    "At-mention command executed with args: {:?}",
                    params.arguments
                );

                // Parse arguments to extract file path and line range
                if let Some(args) = params.arguments.first() {
                    if let Ok(mention_data) =
                        serde_json::from_value::<serde_json::Value>(args.clone())
                    {
                        let file_path = mention_data
                            .get("filePath")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let line_start = mention_data
                            .get("lineStart")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                        let line_end = mention_data
                            .get("lineEnd")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;

                        let at_mention_notification = AtMentionedNotification {
                            file_path: file_path.to_string(),
                            line_start,
                            line_end,
                        };

                        self.send_notification(
                            "at_mentioned",
                            serde_json::to_value(at_mention_notification).unwrap(),
                        )
                        .await;

                        self.client
                            .show_message(
                                MessageType::INFO,
                                format!(
                                    "At-mention sent for {}:{}-{}",
                                    file_path, line_start, line_end
                                ),
                            )
                            .await;
                    }
                }
            }
            _ => {
                self.client
                    .show_message(
                        MessageType::WARNING,
                        format!("Unknown command: {}", params.command),
                    )
                    .await;
            }
        }

        Ok(None)
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> LspResult<Option<Vec<SelectionRange>>> {
        info!(
            "Selection range requested for {} positions",
            params.positions.len()
        );

        // For each position, create a selection range and notify about the selection
        let mut ranges = Vec::new();

        for position in &params.positions {
            info!("Selection at {}:{}", position.line, position.character);

            // Create a basic selection range (this would normally be more sophisticated)
            let range = Range {
                start: *position,
                end: Position {
                    line: position.line,
                    character: position.character + 1,
                },
            };

            ranges.push(SelectionRange {
                range,
                parent: None,
            });

            // Send selection_changed notification
            let selection_range = Range {
                start: *position,
                end: Position {
                    line: position.line,
                    character: position.character + 1,
                },
            };
            let selected_text =
                self.read_text_from_range(params.text_document.uri.path(), selection_range);
            let selection_notification = SelectionChangedNotification {
                text: selected_text,
                file_path: params.text_document.uri.path().to_string(),
                file_url: params.text_document.uri.to_string(),
                selection: SelectionInfo {
                    start: *position,
                    end: Position {
                        line: position.line,
                        character: position.character + 1,
                    },
                    is_empty: true,
                },
            };

            self.send_selection_debounced(selection_notification);
        }

        Ok(Some(ranges))
    }
}

pub async fn run_lsp_server(worktree: Option<PathBuf>) -> Result<()> {
    run_lsp_server_with_notifications(worktree, None, None).await
}

pub async fn run_lsp_server_with_notifications(
    worktree: Option<PathBuf>,
    notification_sender: Option<Arc<NotificationSender>>,
    command_receiver: Option<CommandReceiver>,
) -> Result<()> {
    info!("Starting LSP server mode");
    if let Some(path) = &worktree {
        info!("Worktree path: {}", path.display());
    }

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| {
        let mut server = ClaudeCodeLanguageServer::new(client, worktree.clone());
        if let Some(sender) = notification_sender.clone() {
            server = server.with_notification_sender(sender);
        }
        server
    });

    // Spawn command handler if we have a receiver
    // Note: This runs independently of LSP - uses zed CLI directly
    if let Some(mut receiver) = command_receiver {
        tokio::spawn(async move {
            info!("Command handler ready, waiting for commands...");

            while let Some(command) = receiver.recv().await {
                match command {
                    LspCommand::OpenFile { file_path, line, column, take_focus: _ } => {
                        info!("Handling OpenFile command: {}", file_path);

                        // Build the zed CLI argument with optional line:column
                        let zed_arg = match (line, column) {
                            (Some(l), Some(c)) => format!("{}:{}:{}", file_path, l, c),
                            (Some(l), None) => format!("{}:{}", file_path, l),
                            _ => file_path.clone(),
                        };

                        // Use zed CLI to open the file (Zed doesn't support window/showDocument)
                        match tokio::process::Command::new("zed")
                            .arg(&zed_arg)
                            .spawn()
                        {
                            Ok(_) => {
                                info!("Opened file via zed CLI: {}", zed_arg);
                            }
                            Err(e) => {
                                error!("Failed to open file via zed CLI: {}", e);
                            }
                        }
                    }
                }
            }

            info!("Command handler shutting down");
        });
    }

    Server::new(stdin, stdout, socket).serve(service).await;

    Ok(())
}
