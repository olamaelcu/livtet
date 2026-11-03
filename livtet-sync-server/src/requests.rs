//! The outgoing notification sink.
//!
//! The daemon answers peers on iroh streams and reports the events its parent
//! process cares about (a pairing claim, a pull that applied rows, an answered
//! offer) as JSON-RPC notifications through a [`Notifier`].

use std::sync::Arc;

/// Sink used to emit JSON-RPC notifications to the parent process.
///
/// In the daemon this wraps the stdio writer channel; in tests it can be any
/// closure that records the notification.
pub type Notifier = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Fire a notification through the sink.
pub fn notify(notifier: &Notifier, method: &str, params: serde_json::Value) {
    let emit: &(dyn Fn(&str, serde_json::Value) + Send + Sync) = &**notifier;
    emit(method, params);
}
