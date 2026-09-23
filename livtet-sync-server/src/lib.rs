//! `livtet-sync-server` — a sync daemon for the Livtet ecosystem.
//!
//! The library exposes [`run`], which connects to a SQLite database
//! (running business + client migrations), hosts the poem sync HTTP server
//! so remote devices can sync, and speaks JSON-RPC 2.0 as newline-delimited
//! JSON over stdio with its parent process (the desktop app).
//!
//! The `[[bin]]` target's `main` is a thin wrapper over [`run`].

pub mod config;
pub mod daemon;
pub mod error;
pub mod requests;
pub mod rpc;

pub use config::ServerConfig;
pub use daemon::DaemonState;
pub use error::ServerError;
pub use requests::{
    Notifier, RequestLog, RequestRecord, new_request_log, recent as recent_requests,
    record as record_request,
};
pub use rpc::{
    DeviceParams, JSONRPC, PairingBeginParams, RecentParams, ResolveParams, RpcErrorObject,
    RpcMessage, RpcNotification, RpcRequest, RpcResponse, StdioClient, TokenParams,
    encode_notification, encode_request, encode_response, error_code, method, parse_message,
};

/// Build a multi-threaded tokio runtime and run the daemon to completion.
pub fn run(config: ServerConfig) -> Result<(), ServerError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| ServerError::Config(format!("failed to build tokio runtime: {e}")))?;
    runtime.block_on(daemon::run_async(config))
}
