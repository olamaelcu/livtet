//! Incoming HTTP request log plus the outgoing notification sink.
//!
//! The request-logging middleware attached in [`crate::daemon::start_server`]
//! records every HTTP request the embedded sync server serves and pushes a
//! `request.received` notification to the parent process through a
//! [`Notifier`].

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// Default ring-buffer capacity for the request log.
pub const REQUEST_LOG_CAP: usize = 256;

/// One served HTTP request.
#[derive(Debug, Clone, Serialize)]
pub struct RequestRecord {
    /// HTTP method (e.g. `GET`).
    pub method: String,
    /// Request path (e.g. `/sync/status`).
    pub path: String,
    /// HTTP status code returned.
    pub status: u16,
    /// When the request completed (formatted timestamp).
    pub at: String,
    /// The device id advertised by the caller, when supplied.
    pub device_id: Option<String>,
}

/// A bounded, shared ring buffer of recent requests.
pub type RequestLog = Arc<Mutex<VecDeque<RequestRecord>>>;

/// Sink used to emit JSON-RPC notifications to the parent process.
///
/// In the daemon this wraps the stdio writer channel; in tests it can be any
/// closure that records the notification.
pub type Notifier = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Create an empty request log with the given capacity hint.
pub fn new_request_log(cap: usize) -> RequestLog {
    Arc::new(Mutex::new(VecDeque::with_capacity(cap)))
}

/// Append a record, evicting the oldest entries once `cap` is reached.
pub fn record(log: &RequestLog, cap: usize, record: RequestRecord) {
    if let Ok(mut queue) = log.lock() {
        while queue.len() >= cap && !queue.is_empty() {
            queue.pop_front();
        }
        queue.push_back(record);
    }
}

/// Return up to `limit` of the most recent records, newest first.
pub fn recent(log: &RequestLog, limit: usize) -> Vec<RequestRecord> {
    match log.lock() {
        Ok(queue) => queue.iter().rev().take(limit).cloned().collect(),
        Err(_) => Vec::new(),
    }
}

/// Fire a notification through the sink.
pub fn notify(notifier: &Notifier, method: &str, params: serde_json::Value) {
    let emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync) = &**notifier;
    emit(method, params);
}
