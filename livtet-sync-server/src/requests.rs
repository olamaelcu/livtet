//! The served-request log plus the outgoing notification sink.
//!
//! Every framed request the daemon answers is timed, recorded in a bounded ring
//! buffer, and announced to the parent process through a [`Notifier`]. The log
//! is what `requests.recent` answers when the app wants to show what a peer
//! asked and how long it took; it holds no payloads, only the operation, the
//! peer, the outcome, and the timing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// Default ring-buffer capacity for the request log.
pub const REQUEST_LOG_CAP: usize = 200;

/// One answered sync request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestRecord {
    /// The request's operation tag (`changes`, `file`, `pair_status`, …).
    pub op: String,
    /// The connection's node id, which is the caller's authenticated identity.
    pub node_id: String,
    /// The device id the request authenticated as, when it did.
    pub device_id: Option<String>,
    /// `ok`, `refused`, or `error:<code>`.
    pub outcome: String,
    /// When the request finished, as a formatted timestamp.
    pub at: String,
    /// How long the handler took.
    pub elapsed_ms: u64,
}

/// A bounded, shared ring buffer of recent requests.
pub type RequestLog = Arc<Mutex<VecDeque<RequestRecord>>>;

/// Sink used to emit JSON-RPC notifications to the parent process.
///
/// In the daemon this wraps the stdio writer channel; in tests it can be any
/// closure that records the notification.
pub type Notifier = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Create an empty request log.
pub fn new_request_log() -> RequestLog {
    Arc::new(Mutex::new(VecDeque::with_capacity(REQUEST_LOG_CAP)))
}

/// Append a record, evicting the oldest entries once the cap is reached.
pub fn record(log: &RequestLog, record: RequestRecord) {
    let mut queue = log.lock().unwrap_or_else(|poison| poison.into_inner());
    while queue.len() >= REQUEST_LOG_CAP {
        queue.pop_front();
    }
    queue.push_back(record);
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn record_at(op: &str) -> RequestRecord {
        RequestRecord {
            op: op.to_string(),
            node_id: "node".to_string(),
            device_id: None,
            outcome: "ok".to_string(),
            at: "2026-11-03 10:00:00".to_string(),
            elapsed_ms: 1,
        }
    }

    #[test]
    fn recent_returns_newest_first() {
        let log = new_request_log();
        for op in ["identity", "status", "changes"] {
            record(&log, record_at(op));
        }

        let newest = recent(&log, 10);
        assert_eq!(
            newest
                .iter()
                .map(|entry| entry.op.as_str())
                .collect::<Vec<_>>(),
            ["changes", "status", "identity"]
        );
        assert_eq!(recent(&log, 2).len(), 2, "the limit is honoured");
    }

    /// The buffer is a ring: the cap evicts the oldest record, not the newest.
    #[test]
    fn the_log_is_bounded_by_its_cap() {
        let log = new_request_log();
        for index in 0..(REQUEST_LOG_CAP + 5) {
            record(&log, record_at(&format!("op-{index}")));
        }

        let all = recent(&log, REQUEST_LOG_CAP * 2);
        assert_eq!(all.len(), REQUEST_LOG_CAP);
        assert_eq!(all.first().map(|entry| entry.op.as_str()), Some("op-204"));
        assert_eq!(all.last().map(|entry| entry.op.as_str()), Some("op-5"));
    }
}
