//! `livtet-sync-server` — a sync daemon for the Livtet ecosystem.
//!
//! The library exposes [`run`], which connects to a SQLite database
//! (running business + client migrations), answers sync peers over iroh
//! streams, and speaks JSON-RPC 2.0 as newline-delimited JSON over stdio with
//! its parent process (the desktop app).
//!
//! The `[[bin]]` target's `main` is a thin wrapper over [`run`].

pub mod config;
pub mod daemon;
pub mod error;
pub mod mdns;
pub mod pairing;
pub mod peer;
pub mod requests;
pub mod rpc;
pub mod wire;

pub use config::ServerConfig;
pub use daemon::DaemonState;
pub use error::ServerError;
pub use requests::{Notifier, notify};
pub use rpc::{
    DeviceParams, JSONRPC, PairingBeginParams, ResolveParams, RpcErrorObject, RpcMessage,
    RpcNotification, RpcRequest, RpcResponse, StdioClient, TokenParams, encode_notification,
    encode_request, encode_response, error_code, method, parse_message,
};

/// Build a multi-threaded tokio runtime and run the daemon to completion.
pub fn run(config: ServerConfig) -> Result<(), ServerError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| ServerError::Config(format!("failed to build tokio runtime: {e}")))?;
    runtime.block_on(daemon::run_async(config))
}
