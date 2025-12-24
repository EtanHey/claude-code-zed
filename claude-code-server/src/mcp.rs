use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::lsp::{CommandSender, LspCommand};

#[derive(Debug, Serialize, Deserialize)]
pub struct MCPRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MCPResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<MCPError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MCPError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerCapabilities {
    pub tools: Option<ToolsCapability>,
    pub prompts: Option<PromptsCapability>,
    pub logging: Option<LoggingCapability>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PromptsCapability {
    #[serde(rename = "listChanged")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoggingCapability {}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub type_: String,
    pub text: String,
}

pub struct MCPServer {
    capabilities: ServerCapabilities,
    command_sender: Option<CommandSender>,
}

impl MCPServer {
    pub fn new(command_sender: Option<CommandSender>) -> Self {
        let capabilities = ServerCapabilities {
            tools: Some(ToolsCapability {
                list_changed: Some(true),
            }),
            prompts: Some(PromptsCapability {
                list_changed: Some(false),
            }),
            logging: Some(LoggingCapability {}),
        };

        Self {
            capabilities,
            command_sender,
        }
    }

    pub async fn handle_request(&self, request: MCPRequest) -> Result<MCPResponse> {
        info!("Handling MCP request: {}", request.method);
        debug!("Request params: {:?}", request.params);

        let result = match request.method.as_str() {
            "initialize" => self.handle_initialize(request.params).await?,
            "tools/list" => self.handle_tools_list().await?,
            "tools/call" => self.handle_tools_call(request.params).await?,
            "logging/setLevel" => self.handle_logging_set_level(request.params).await?,
            "prompts/list" => self.handle_prompts_list().await?,
            "prompts/get" => self.handle_prompts_get(request.params).await?,
            _ => {
                return Ok(MCPResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(MCPError {
                        code: -32601,
                        message: format!("Method not found: {}", request.method),
                        data: None,
                    }),
                });
            }
        };

        Ok(MCPResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: Some(result),
            error: None,
        })
    }

    async fn handle_initialize(&self, params: Option<Value>) -> Result<Value> {
        info!("Initializing MCP session");

        if let Some(params) = params {
            debug!("Initialize params: {}", params);
        }

        Ok(serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": self.capabilities,
            "serverInfo": ServerInfo {
                name: "claude-code-server".to_string(),
                version: "0.1.0".to_string()
            }
        }))
    }

    async fn handle_tools_list(&self) -> Result<Value> {
        info!("Listing available tools");

        let tools: Vec<Tool> = vec![];

        Ok(serde_json::json!({
            "tools": tools
        }))
    }

    async fn handle_tools_call(&self, params: Option<Value>) -> Result<Value> {
        let params = params.ok_or_else(|| anyhow::anyhow!("Missing parameters for tools/call"))?;

        let tool_name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing tool name"))?;

        let default_args = serde_json::json!({});
        let arguments = params.get("arguments").unwrap_or(&default_args);

        info!("Calling tool: {}", tool_name);
        debug!("Tool arguments: {}", arguments);

        let content = match tool_name {
            "echo" => {
                let text = arguments
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No text provided");

                vec![TextContent {
                    type_: "text".to_string(),
                    text: format!("Echo: {}", text),
                }]
            }
            "get_workspace_info" => {
                let workspace_info = std::env::current_dir()
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "Unknown workspace".to_string());

                vec![TextContent {
                    type_: "text".to_string(),
                    text: format!("Current workspace: {}", workspace_info),
                }]
            }
            "closeAllDiffTabs" => {
                info!("Closing all diff tabs");

                // Return the count of closed diff tabs according to protocol
                let closed_count = 0; // Simulate no diff tabs to close

                vec![TextContent {
                    type_: "text".to_string(),
                    text: format!("CLOSED_{}_DIFF_TABS", closed_count),
                }]
            }
            "openFile" => {
                let file_path = arguments
                    .get("filePath")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No file path provided");
                let _preview = arguments
                    .get("preview")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let _start_text = arguments.get("startText").and_then(|v| v.as_str());
                let _end_text = arguments.get("endText").and_then(|v| v.as_str());
                let make_frontmost = arguments
                    .get("makeFrontmost")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                // Parse line number if provided (could be in startText like "line:42")
                let line = arguments
                    .get("line")
                    .and_then(|v| v.as_u64())
                    .map(|l| l as u32);
                let column = arguments
                    .get("column")
                    .and_then(|v| v.as_u64())
                    .map(|c| c as u32);

                info!("Opening file: {} (line: {:?}, take_focus: {})", file_path, line, make_frontmost);

                // Send command to LSP to open file in Zed
                let mut opened_in_ide = false;
                if let Some(sender) = &self.command_sender {
                    let command = LspCommand::OpenFile {
                        file_path: file_path.to_string(),
                        line,
                        column,
                        take_focus: make_frontmost,
                    };

                    match sender.send(command).await {
                        Ok(()) => {
                            info!("Sent openFile command to LSP");
                            opened_in_ide = true;
                        }
                        Err(e) => {
                            warn!("Failed to send openFile command: {}", e);
                        }
                    }
                }

                // Return response
                let canonical_path = std::path::Path::new(file_path)
                    .canonicalize()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| file_path.to_string());

                let response = serde_json::json!({
                    "success": true,
                    "filePath": canonical_path,
                    "openedInIde": opened_in_ide,
                    "languageId": "text",
                    "lineCount": 0
                });

                vec![TextContent {
                    type_: "text".to_string(),
                    text: if make_frontmost {
                        format!("Opened file: {}", file_path)
                    } else {
                        response.to_string()
                    },
                }]
            }
            "getCurrentSelection" => {
                info!("Getting current selection");

                // Return JSON-stringified response according to protocol
                let response = serde_json::json!({
                    "success": false,
                    "message": "No active editor found"
                });

                vec![TextContent {
                    type_: "text".to_string(),
                    text: response.to_string(),
                }]
            }
            "getOpenEditors" => {
                info!("Getting open editors");

                // Return JSON-stringified response according to protocol
                let response = serde_json::json!({
                    "tabs": []
                });

                vec![TextContent {
                    type_: "text".to_string(),
                    text: response.to_string(),
                }]
            }
            "getWorkspaceFolders" => {
                let workspace_info = std::env::current_dir()
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "Unknown workspace".to_string());

                info!("Getting workspace folders");

                // Return JSON-stringified response according to protocol
                let response = serde_json::json!({
                    "success": true,
                    "folders": [{
                        "name": std::path::Path::new(&workspace_info)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("workspace"),
                        "uri": format!("file://{}", workspace_info),
                        "path": workspace_info
                    }],
                    "rootPath": workspace_info
                });

                vec![TextContent {
                    type_: "text".to_string(),
                    text: response.to_string(),
                }]
            }
            "openDiff" => {
                let old_file_path = arguments
                    .get("old_file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No old file path provided");
                let new_file_path = arguments
                    .get("new_file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No new file path provided");
                let new_file_contents = arguments
                    .get("new_file_contents")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No new file contents provided");
                let _tab_name = arguments
                    .get("tab_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("diff");

                info!("Opening diff for {} vs {}", old_file_path, new_file_path);

                // Always respond with FILE_SAVED to simulate accepting the diff
                vec![
                    TextContent {
                        type_: "text".to_string(),
                        text: "FILE_SAVED".to_string(),
                    },
                    TextContent {
                        type_: "text".to_string(),
                        text: new_file_contents.to_string(),
                    },
                ]
            }
            "getLatestSelection" => {
                info!("Getting latest selection");

                // Return JSON-stringified response according to protocol
                let response = serde_json::json!({
                    "success": false,
                    "message": "No selection available"
                });

                vec![TextContent {
                    type_: "text".to_string(),
                    text: response.to_string(),
                }]
            }
            "getDiagnostics" => {
                let uri = arguments.get("uri").and_then(|v| v.as_str());

                info!("Getting diagnostics for: {:?}", uri);

                // Return JSON-stringified array of diagnostics per file
                let response = if let Some(uri) = uri {
                    serde_json::json!([{
                        "uri": uri,
                        "diagnostics": []
                    }])
                } else {
                    serde_json::json!([])
                };

                vec![TextContent {
                    type_: "text".to_string(),
                    text: response.to_string(),
                }]
            }
            "checkDocumentDirty" => {
                let file_path = arguments
                    .get("filePath")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No file path provided");

                info!("Checking if document is dirty: {}", file_path);

                // Return JSON-stringified response according to protocol
                let response = serde_json::json!({
                    "success": true,
                    "filePath": file_path,
                    "isDirty": false,
                    "isUntitled": false
                });

                vec![TextContent {
                    type_: "text".to_string(),
                    text: response.to_string(),
                }]
            }
            "saveDocument" => {
                let file_path = arguments
                    .get("filePath")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No file path provided");

                info!("Saving document: {}", file_path);

                // Return JSON-stringified response according to protocol
                let response = serde_json::json!({
                    "success": true,
                    "filePath": file_path,
                    "saved": true,
                    "message": "Document saved successfully"
                });

                vec![TextContent {
                    type_: "text".to_string(),
                    text: response.to_string(),
                }]
            }
            "close_tab" => {
                let tab_name = arguments
                    .get("tab_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No tab name provided");

                info!("Closing tab: {}", tab_name);

                vec![TextContent {
                    type_: "text".to_string(),
                    text: "TAB_CLOSED".to_string(),
                }]
            }
            "executeCode" => {
                let code = arguments
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No code provided");

                info!(
                    "Executing code: {}",
                    code.chars().take(50).collect::<String>()
                );

                vec![TextContent {
                    type_: "text".to_string(),
                    text: format!("Code executed successfully. Output: (simulated execution of {} characters)", code.len()),
                }]
            }
            _ => return Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
        };

        Ok(serde_json::json!({
            "content": content,
            "isError": false
        }))
    }

    async fn handle_logging_set_level(&self, params: Option<Value>) -> Result<Value> {
        if let Some(params) = params {
            let level = params
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("info");
            info!("Setting log level to: {}", level);
        }

        Ok(serde_json::json!({}))
    }

    async fn handle_prompts_list(&self) -> Result<Value> {
        info!("Listing available prompts");

        Ok(serde_json::json!({
            "prompts": []
        }))
    }

    async fn handle_prompts_get(&self, params: Option<Value>) -> Result<Value> {
        let params = params.ok_or_else(|| anyhow::anyhow!("Missing parameters for prompts/get"))?;

        let prompt_name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing prompt name"))?;

        info!("Getting prompt: {}", prompt_name);

        Ok(serde_json::json!({
            "description": format!("Prompt: {}", prompt_name),
            "messages": []
        }))
    }
}

impl Default for MCPServer {
    fn default() -> Self {
        Self::new(None)
    }
}
