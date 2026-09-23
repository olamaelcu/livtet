//! Transport-agnostic JSON-RPC 2.0 codec, message types, and a stdio test client.
//!
//! The daemon speaks newline-delimited JSON (NDJSON) over stdio with its
//! parent process. This module owns the codec and DTOs but performs **no**
//! stdio I/O itself (the loop lives in [`crate::daemon`]); the only exception
//! is [`StdioClient`], a ready-made client for integration tests and the CLI.

use std::process::Stdio;
use std::time::Duration;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

use crate::error::ServerError;

/// The JSON-RPC protocol version string.
pub const JSONRPC: &str = "2.0";

/// Request method names understood by the daemon.
pub mod method {
    /// Liveness probe.
    pub const HEALTH: &str = "health";
    /// Aggregate daemon/server status.
    pub const STATUS: &str = "status";
    /// Recently served HTTP requests.
    pub const REQUESTS_RECENT: &str = "requests.recent";
    /// Start a new pairing session (mint a token + `livtet://` URI).
    pub const PAIRING_BEGIN: &str = "pairing.begin";
    /// List outstanding pending pairings.
    pub const PAIRING_LIST: &str = "pairing.list";
    /// Approve a pending pairing.
    pub const PAIRING_APPROVE: &str = "pairing.approve";
    /// Reject a pending pairing.
    pub const PAIRING_REJECT: &str = "pairing.reject";
    /// List paired devices.
    pub const DEVICES_LIST: &str = "devices.list";
    /// Revoke a paired device.
    pub const DEVICES_REVOKE: &str = "devices.revoke";
    /// List unresolved sync conflicts.
    pub const CONFLICTS_LIST: &str = "conflicts.list";
    /// Resolve a sync conflict.
    pub const CONFLICTS_RESOLVE: &str = "conflicts.resolve";
    /// Start the embedded HTTP server.
    pub const SERVER_START: &str = "server.start";
    /// Stop the embedded HTTP server.
    pub const SERVER_STOP: &str = "server.stop";
    /// Stop the server and exit the daemon loop.
    pub const SHUTDOWN: &str = "shutdown";

    /// Notification: a remote device requested pairing (`POST /sync/pair`).
    pub const NOTIFY_PAIRING_REQUESTED: &str = "pairing.requested";
    /// Notification: the server served an HTTP request.
    pub const NOTIFY_REQUEST_RECEIVED: &str = "request.received";
    /// Notification: a sync push/pull completed.
    pub const NOTIFY_SYNC_COMPLETED: &str = "sync.completed";
    /// Notification: the embedded HTTP server started.
    pub const NOTIFY_SERVER_STARTED: &str = "server.started";
    /// Notification: the embedded HTTP server stopped.
    pub const NOTIFY_SERVER_STOPPED: &str = "server.stopped";
}

/// JSON-RPC 2.0 error codes.
pub mod error_code {
    /// Invalid JSON was received.
    pub const PARSE_ERROR: i64 = -32700;
    /// The requested method does not exist.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The method parameters were invalid.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal daemon error.
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// A JSON-RPC request (always carries an `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// A JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorObject>,
}

impl RpcResponse {
    /// Build a success response.
    pub fn ok(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: JSONRPC.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Build an error response.
    pub fn error(id: Option<serde_json::Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JSONRPC.to_string(),
            id,
            result: None,
            error: Some(RpcErrorObject {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// The `error` member of a JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// A JSON-RPC notification (no `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Any decoded NDJSON message.
#[derive(Debug, Clone)]
pub enum RpcMessage {
    /// A request awaiting a response.
    Request(RpcRequest),
    /// A response to an earlier request.
    Response(RpcResponse),
    /// A one-way notification.
    Notification(RpcNotification),
}

/// Parameters for [`method::PAIRING_BEGIN`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PairingBeginParams {
    /// Device-type hint (e.g. `"mobile"`), accepted for forward compatibility.
    #[serde(default)]
    pub device_type: Option<String>,
    /// Token lifetime in seconds (defaults to 300).
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// Parameters carrying a pairing token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenParams {
    pub token: String,
}

/// Parameters carrying a device id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceParams {
    pub device_id: String,
}

/// Parameters for [`method::CONFLICTS_RESOLVE`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveParams {
    pub id: i64,
    pub resolution: String,
    #[serde(default)]
    pub merged_payload: Option<String>,
}

/// Parameters for [`method::REQUESTS_RECENT`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentParams {
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Decode a single NDJSON line into a [`RpcMessage`].
pub fn parse_message(line: &str) -> Result<RpcMessage, ServerError> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|e| ServerError::Rpc(format!("parse error: {e}")))?;

    let has_method = value.get("method").is_some();
    let has_id = value.get("id").is_some();

    if has_method && !has_id {
        let notification: RpcNotification = serde_json::from_value(value)
            .map_err(|e| ServerError::Rpc(format!("invalid notification: {e}")))?;
        Ok(RpcMessage::Notification(notification))
    } else if has_method {
        let request: RpcRequest = serde_json::from_value(value)
            .map_err(|e| ServerError::Rpc(format!("invalid request: {e}")))?;
        Ok(RpcMessage::Request(request))
    } else if has_id {
        let response: RpcResponse = serde_json::from_value(value)
            .map_err(|e| ServerError::Rpc(format!("invalid response: {e}")))?;
        Ok(RpcMessage::Response(response))
    } else {
        Err(ServerError::Rpc(
            "not a JSON-RPC message (no method or id)".to_string(),
        ))
    }
}

/// Encode a request line (no trailing newline).
pub fn encode_request(id: u64, method: &str, params: serde_json::Value) -> String {
    let value = serde_json::json!({
        "jsonrpc": JSONRPC,
        "id": id,
        "method": method,
        "params": params,
    });
    serde_json::to_string(&value)
        .unwrap_or_else(|_| format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}"}}"#))
}

/// Encode a response line (no trailing newline).
pub fn encode_response(response: &RpcResponse) -> String {
    serde_json::to_string(response).unwrap_or_else(|_| {
        format!(
            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":{},"message":"encode error"}}}}"#,
            error_code::INTERNAL_ERROR
        )
    })
}

/// Encode a notification line (no trailing newline).
pub fn encode_notification(method: &str, params: serde_json::Value) -> String {
    let value = serde_json::json!({
        "jsonrpc": JSONRPC,
        "method": method,
        "params": params,
    });
    serde_json::to_string(&value)
        .unwrap_or_else(|_| format!(r#"{{"jsonrpc":"2.0","method":"{method}","params":null}}"#))
}

/// A minimal stdio client for spawning and talking to the built daemon.
///
/// A background task reads stdout, routing responses and notifications to
/// separate channels; [`StdioClient::request`] correlates responses by id.
pub struct StdioClient {
    stdin: ChildStdin,
    responses: mpsc::UnboundedReceiver<RpcResponse>,
    notifications: mpsc::UnboundedReceiver<RpcNotification>,
    child: Child,
    next_id: u64,
}

impl StdioClient {
    /// Spawn `program` with `args`, wiring up its stdin/stdout.
    pub async fn spawn<P: AsRef<Utf8Path>>(
        program: P,
        args: &[String],
    ) -> Result<Self, ServerError> {
        let mut command = Command::new(program.as_ref().as_std_path());
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ServerError::Io("child stdin is not piped".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ServerError::Io("child stdout is not piped".to_string()))?;

        let (response_tx, response_rx) = mpsc::unbounded_channel();
        let (notification_tx, notification_rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match parse_message(&line) {
                    Ok(RpcMessage::Response(response)) => {
                        if response_tx.send(response).is_err() {
                            break;
                        }
                    }
                    Ok(RpcMessage::Notification(notification)) => {
                        if notification_tx.send(notification).is_err() {
                            break;
                        }
                    }
                    Ok(RpcMessage::Request(_)) => {}
                    Err(_) => {}
                }
            }
        });

        Ok(Self {
            stdin,
            responses: response_rx,
            notifications: notification_rx,
            child,
            next_id: 1,
        })
    }

    /// Send a request and await its correlated response.
    pub async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcResponse, ServerError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let line = encode_request(id, method, params);
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;

        let expected = serde_json::Value::Number(id.into());
        while let Some(response) = self.responses.recv().await {
            if response.id.as_ref() == Some(&expected) {
                return Ok(response);
            }
        }
        Err(ServerError::Rpc(
            "server closed stdout before responding".to_string(),
        ))
    }

    /// Send a request with a serializable params value.
    pub async fn request_params<T: Serialize>(
        &mut self,
        method: &str,
        params: &T,
    ) -> Result<RpcResponse, ServerError> {
        let value = serde_json::to_value(params)?;
        self.request(method, value).await
    }

    /// Await the next notification, or `None` on timeout.
    pub async fn next_notification(&mut self, timeout: Duration) -> Option<RpcNotification> {
        tokio::time::timeout(timeout, self.notifications.recv())
            .await
            .ok()
            .flatten()
    }

    /// Wait for the child process to exit.
    pub async fn wait(&mut self) -> Result<(), ServerError> {
        let _ = self.child.wait().await?;
        Ok(())
    }

    /// Terminate the child process.
    pub fn kill(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
