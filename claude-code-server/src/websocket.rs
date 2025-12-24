use anyhow::{anyhow, Result};
use dirs::home_dir;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::handshake::server::{Request, Response},
    tungstenite::Message,
    WebSocketStream,
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::lsp::{CommandSender, NotificationReceiver};
use crate::mcp::{MCPRequest, MCPResponse, MCPServer};

#[derive(Debug, Serialize, Deserialize)]
pub struct LockFile {
    pub pid: u32,
    #[serde(rename = "workspaceFolders")]
    pub workspace_folders: Vec<String>,
    #[serde(rename = "ideName")]
    pub ide_name: String,
    pub transport: String,
    #[serde(rename = "authToken")]
    pub auth_token: String,
}

pub async fn run_websocket_server(port: Option<u16>) -> Result<()> {
    run_websocket_server_with_worktree(port, None).await
}

pub async fn run_websocket_server_with_worktree(
    port: Option<u16>,
    worktree: Option<PathBuf>,
) -> Result<()> {
    run_websocket_server_with_notifications(port, worktree, None, None).await
}

pub async fn run_websocket_server_with_notifications(
    port: Option<u16>,
    worktree: Option<PathBuf>,
    mut notification_receiver: Option<NotificationReceiver>,
    command_sender: Option<CommandSender>,
) -> Result<()> {
    info!("Starting WebSocket server...");

    // Use fixed port or provided port, default to 59792
    let port = port.unwrap_or(59792);

    // Clean up any existing lock files for this port
    cleanup_existing_lock_file(port).await?;

    // Create new lock file
    let auth_token = Uuid::new_v4().to_string();
    create_lock_file(port, worktree.clone(), &auth_token).await?;

    // Start WebSocket server with proper error handling
    let addr = format!("127.0.0.1:{}", port);

    // Try to bind to the port, with retry logic
    let listener = match TcpListener::bind(&addr).await {
        Ok(listener) => {
            info!("WebSocket server listening on {}", addr);
            listener
        }
        Err(e) => {
            error!("Failed to bind to port {}: {}", port, e);
            info!("Attempting to force cleanup and retry...");

            // Try to cleanup and retry once
            cleanup_existing_lock_file(port).await?;

            // Wait a moment for the port to be released
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            match TcpListener::bind(&addr).await {
                Ok(listener) => {
                    info!("Successfully bound to port {} after cleanup", port);
                    listener
                }
                Err(e2) => {
                    error!("Failed to bind to port {} even after cleanup: {}", port, e2);
                    return Err(anyhow!("Port {} is unavailable: {}", port, e2));
                }
            }
        }
    };

    // Setup graceful shutdown handler
    let port_for_cleanup = port;
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutdown signal received, cleaning up...");
        if let Err(e) = cleanup_existing_lock_file(port_for_cleanup).await {
            error!("Error during cleanup: {}", e);
        }
        std::process::exit(0);
    });

    while let Ok((stream, peer_addr)) = listener.accept().await {
        info!("New connection from {}", peer_addr);
        let auth_token_clone = auth_token.clone();
        let notification_receiver_clone = if let Some(ref mut receiver) = notification_receiver {
            Some(receiver.resubscribe())
        } else {
            None
        };
        let command_sender_clone = command_sender.clone();
        tokio::spawn(handle_connection(
            stream,
            peer_addr,
            auth_token_clone,
            notification_receiver_clone,
            command_sender_clone,
        ));
    }

    Ok(())
}

async fn cleanup_existing_lock_file(port: u16) -> Result<()> {
    let home = home_dir().ok_or_else(|| anyhow!("Could not find home directory"))?;
    let claude_dir = home.join(".claude").join("ide");

    if !claude_dir.exists() {
        // Directory doesn't exist, nothing to clean up
        return Ok(());
    }

    let lock_file_path = claude_dir.join(format!("{}.lock", port));

    if lock_file_path.exists() {
        info!("Removing existing lock file: {}", lock_file_path.display());
        fs::remove_file(&lock_file_path)?;
    }

    Ok(())
}

async fn create_lock_file(port: u16, worktree: Option<PathBuf>, auth_token: &str) -> Result<()> {
    let home = home_dir().ok_or_else(|| anyhow!("Could not find home directory"))?;
    let claude_dir = home.join(".claude").join("ide");

    // Create directories if they don't exist
    if !claude_dir.exists() {
        fs::create_dir_all(&claude_dir)?;
        info!("Created directory: {}", claude_dir.display());
    }

    // Get current working directory or use provided worktree
    let workspace_folder = if let Some(wt) = worktree {
        wt.to_string_lossy().to_string()
    } else {
        env::current_dir()?.to_string_lossy().to_string()
    };

    let lock_file_data = LockFile {
        pid: process::id(),
        workspace_folders: vec![workspace_folder],
        ide_name: "claude-code-server".to_string(),
        transport: "ws".to_string(),
        auth_token: auth_token.to_string(),
    };

    let lock_file_path = claude_dir.join(format!("{}.lock", port));
    let json_data = serde_json::to_string_pretty(&lock_file_data)?;

    fs::write(&lock_file_path, json_data)?;
    info!("Created lock file: {}", lock_file_path.display());

    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    peer_addr: SocketAddr,
    auth_token: String,
    notification_receiver: Option<NotificationReceiver>,
    command_sender: Option<CommandSender>,
) -> Result<()> {
    info!("Handling connection from {}", peer_addr);

    let ws_stream = match accept_hdr_async(stream, |req: &Request, mut response: Response| {
        // Check if client requested MCP protocol
        if let Some(protocols) = req.headers().get("Sec-WebSocket-Protocol") {
            if let Ok(protocols_str) = protocols.to_str() {
                if protocols_str.contains("mcp") {
                    // Add MCP protocol to response
                    response
                        .headers_mut()
                        .insert("Sec-WebSocket-Protocol", "mcp".parse().unwrap());
                    info!("MCP protocol negotiated for {}", peer_addr);
                }
            }
        }
        Ok(response)
    })
    .await
    {
        Ok(ws) => {
            info!("WebSocket handshake completed for {}", peer_addr);
            ws
        }
        Err(e) => {
            error!("WebSocket handshake failed for {}: {}", peer_addr, e);
            return Err(e.into());
        }
    };

    handle_websocket_connection(ws_stream, peer_addr, auth_token, notification_receiver, command_sender).await
}

async fn handle_websocket_connection(
    ws_stream: WebSocketStream<TcpStream>,
    peer_addr: SocketAddr,
    _auth_token: String,
    mut notification_receiver: Option<NotificationReceiver>,
    command_sender: Option<CommandSender>,
) -> Result<()> {
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let mcp_handler = MCPServer::new(command_sender);

    info!("WebSocket connection established with {}", peer_addr);

    // Main message loop handling both WebSocket messages and IDE notifications
    loop {
        tokio::select! {
            // Handle incoming WebSocket messages
            msg = ws_receiver.next() => {
                match msg {
                    Some(msg) => {
                        if let Err(e) = handle_websocket_message(msg, &mcp_handler, &mut ws_sender, peer_addr).await {
                            error!("Error handling WebSocket message: {}", e);
                            break;
                        }
                    }
                    None => {
                        info!("WebSocket connection with {} ended", peer_addr);
                        break;
                    }
                }
            },
            // Handle IDE notifications
            notification = async {
                if let Some(ref mut receiver) = notification_receiver {
                    receiver.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                match notification {
                    Ok(notification) => {
                        debug!("Received IDE notification: {:?}", notification);

                        // Forward the notification to the MCP client
                        let notification_json = serde_json::to_string(&notification)?;
                        if let Err(e) = ws_sender.send(Message::Text(notification_json)).await {
                            error!("Failed to send IDE notification to {}: {}", peer_addr, e);
                            break;
                        }
                    }
                    Err(e) => {
                        debug!("Notification channel error: {}", e);
                        // Channel closed or lagged, continue without notifications
                        notification_receiver = None;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn handle_websocket_message(
    msg: Result<Message, tokio_tungstenite::tungstenite::Error>,
    mcp_handler: &MCPServer,
    ws_sender: &mut futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>,
    peer_addr: SocketAddr,
) -> Result<()> {
    match msg {
        Ok(msg) => {
            if msg.is_text() {
                let text = msg.to_text().unwrap();
                debug!("Received message from {}: {}", peer_addr, text);

                // Try to parse as MCP request
                match serde_json::from_str::<MCPRequest>(text) {
                    Ok(mcp_request) => {
                        info!("Processing MCP request: {}", mcp_request.method);

                        // Handle notifications (requests without ID) separately
                        if mcp_request.id.is_none()
                            && mcp_request.method.starts_with("notifications/")
                        {
                            info!("Processing notification: {}", mcp_request.method);
                            // Notifications don't get responses, just return
                            return Ok(());
                        }

                        match mcp_handler.handle_request(mcp_request).await {
                            Ok(response) => {
                                let response_json = serde_json::to_string(&response)?;
                                debug!("Sending MCP response: {}", response_json);

                                if let Err(e) = ws_sender.send(Message::Text(response_json)).await {
                                    error!("Failed to send MCP response to {}: {}", peer_addr, e);
                                    return Err(e.into());
                                }
                            }
                            Err(e) => {
                                error!("Error handling MCP request: {}", e);
                                let error_response = MCPResponse {
                                    jsonrpc: "2.0".to_string(),
                                    id: None,
                                    result: None,
                                    error: Some(crate::mcp::MCPError {
                                        code: -32603,
                                        message: "Internal error".to_string(),
                                        data: Some(serde_json::json!({"details": e.to_string()})),
                                    }),
                                };

                                let error_json = serde_json::to_string(&error_response)?;
                                if let Err(e) = ws_sender.send(Message::Text(error_json)).await {
                                    error!("Failed to send error response to {}: {}", peer_addr, e);
                                    return Err(e.into());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse MCP request from {}: {}", peer_addr, e);
                        debug!("Invalid message content: {}", text);

                        // Send back a JSON-RPC error response
                        let error_response = MCPResponse {
                            jsonrpc: "2.0".to_string(),
                            id: None,
                            result: None,
                            error: Some(crate::mcp::MCPError {
                                code: -32700,
                                message: "Parse error".to_string(),
                                data: None,
                            }),
                        };

                        let error_json = serde_json::to_string(&error_response)?;
                        if let Err(e) = ws_sender.send(Message::Text(error_json)).await {
                            error!(
                                "Failed to send parse error response to {}: {}",
                                peer_addr, e
                            );
                            return Err(e.into());
                        }
                    }
                }
            } else if msg.is_close() {
                info!("Connection closed by {}", peer_addr);
                return Ok(());
            }
        }
        Err(e) => {
            error!("WebSocket error for {}: {}", peer_addr, e);
            return Err(e.into());
        }
    }

    Ok(())
}
