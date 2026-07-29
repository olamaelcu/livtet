//! In-process helpers for pairing decision fan-out.
//!
//! Replaces the previous in-process HTTP loopback (`/internal/pairing-decision`)
//! called by the Tauri app against the sync server it itself spawned.
//! The HTTP call only existed to fire `PairingDecision` on the broadcast channel
//! so the mobile SSE stream wakes up; we now expose that fire as a direct function
//! the Tauri command calls.
//!
//! The broadcast registry is stored in a process-wide `Mutex<Option<...>>` so the
//! server task and the Tauri commands share the same map, and so it can be
//! replaced on restart (unlike `OnceLock`).

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

use serde::Serialize;
use tokio::sync::broadcast;

/// The decision payload fired on the broadcast channel when the user
/// approves or rejects a pairing in the desktop UI.  Serialised to
/// JSON in the SSE stream consumed by the mobile `/sync/pair/status/:token`
/// subscriber.
///
/// `session_token` is the 26-char Crockford ULID minted by the desktop
/// approval flow (`approve_pairing` in `livtet-tauri`). It is forwarded
/// to the mobile client so it can authenticate subsequent sync requests.
/// The mobile client already parsed `livtet://sync?...&token=` from the
/// QR code, but the broadcast field carries the *session* token rather
/// than the pairing token (they are different: the pairing token is
/// single-use and expires, the session token persists for the lifetime
/// of the pairing).
#[derive(Clone, Serialize)]
pub struct PairingDecision {
    pub event: String,
    pub device_id: Option<String>,
    pub session_token: String,
}

/// Process-wide map of pairing token → broadcast sender.  Created in
/// `start_sync_server` and registered via `set_pair_waiters`; consumed
/// by `apply_pairing_decision` (called from Tauri commands).
pub type PairWaiters = Arc<tokio::sync::Mutex<HashMap<String, broadcast::Sender<PairingDecision>>>>;

/// Registry stored in a `std::sync::Mutex<Option<...>>` so it can be
/// replaced on restart.
static REGISTRY: LazyLock<Mutex<Option<PairWaiters>>> = LazyLock::new(|| Mutex::new(None));

/// Register a new broadcast registry, replacing any previous one.
pub fn set_pair_waiters(waiters: PairWaiters) {
    let mut guard = REGISTRY.lock().unwrap();
    *guard = Some(waiters);
}

/// Register only if none is set. Returns true if the value was set,
/// false if one was already present.
pub fn set_pair_waiters_if_empty(waiters: PairWaiters) -> bool {
    let mut guard = REGISTRY.lock().unwrap();
    if guard.is_some() {
        false
    } else {
        *guard = Some(waiters);
        true
    }
}

/// Returns the registered registry, if any. Returns `None` if `start_sync_server`
/// has not yet been called (e.g., during early startup or in tests).
pub fn get_pair_waiters() -> Option<PairWaiters> {
    let guard = REGISTRY.lock().unwrap();
    guard.clone()
}

/// Fires a `PairingDecision` on the broadcast channel for the given token, then
/// removes the entry from the registry. This is what wakes the mobile SSE
/// stream subscribed via `get_pair_status`.
///
/// The decision value (e.g. "approved" or "rejected") is the same string the
/// mobile client already handles. `device_id` is unknown to the fan-out path
/// (the server's `/sync/pair` handler doesn't pass it), so it's always `None`.
///
/// `session_token` is the 26-char Crockford ULID minted by `approve_pairing`
/// in the Tauri crate. For rejections the caller passes an empty string —
/// the mobile client only uses session tokens on approval.
///
/// Returns `true` if a waiter was found and signaled, `false` otherwise.
pub async fn apply_pairing_decision(token: &str, decision: &str, session_token: &str) -> bool {
    let Some(waiters) = get_pair_waiters() else {
        return false;
    };

    let payload = PairingDecision {
        event: decision.to_string(),
        device_id: None,
        session_token: session_token.to_string(),
    };

    let mut map = waiters.lock().await;
    let signaled = if let Some(tx) = map.get(token) {
        // `send` only fails if there are no active receivers, which can happen
        // if the mobile disconnected between `post_pair` and our fan-out.
        // We don't treat that as an error — the pairing still completed.
        let _ = tx.send(payload);
        true
    } else {
        false
    };
    map.remove(token);
    signaled
}
