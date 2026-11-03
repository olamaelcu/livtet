//! Pairing decision fan-out and offer-decision delivery.
//!
//! A pairing claim arrives on the framed iroh transport and holds its stream
//! open while the reader decides. The daemon owns the map of node id → waiter;
//! approving or rejecting wakes the claimant through the corresponding
//! broadcast channel. There is no process-global registry: [`DaemonState`] is
//! the only holder.
//!
//! [`DaemonState`]: crate::daemon::DaemonState

use std::{collections::HashMap, sync::Arc};

use tokio::sync::{Mutex, broadcast};

/// The decision sent on a waiter's broadcast channel when the reader approves
/// or rejects a claim.
///
/// `session_token` is the 26-char Crockford ULID minted at approval; it is
/// forwarded to the claimant so it can authenticate every later request. A
/// rejection carries an empty token.
#[derive(Clone, Debug)]
pub struct PairingDecision {
    /// `approved` or `rejected`, as the claimant reads it.
    pub event: String,
    /// Token issued on approval, empty on rejection.
    pub session_token: String,
}

/// One registered pairing request.
#[derive(Clone, Debug)]
pub struct PairingWaiter {
    /// Secret generated when the claim was filed. It stays with the waiter so a
    /// future protocol version can prove the claimant is the same device.
    pub claim: String,
    /// Decision fan-out the claimant is waiting on.
    pub sender: broadcast::Sender<PairingDecision>,
}

/// Node id → outstanding waiter, owned by the daemon.
pub type PairWaiters = Arc<Mutex<HashMap<String, PairingWaiter>>>;

/// Fires a [`PairingDecision`] for `node_id`, then removes the waiter.
///
/// Returns `true` when a waiter was found and signalled. A sender with no
/// receivers is not an error: the claimant may have closed its stream before
/// the reader decided, and the pairing itself still completed.
pub async fn apply_pairing_decision(
    waiters: &PairWaiters,
    node_id: &str,
    decision: &str,
    session_token: &str,
) -> bool {
    let payload = PairingDecision {
        event: decision.to_string(),
        session_token: session_token.to_string(),
    };

    let mut map = waiters.lock().await;
    let signaled = if let Some(waiter) = map.get(node_id) {
        let _ = waiter.sender.send(payload);
        true
    } else {
        false
    };
    map.remove(node_id);
    signaled
}

/// What a paired device decided about an offer this daemon sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferDecision {
    /// The sender's offer id, as recorded in its `change_log` row.
    pub offer_id: String,
    /// The paired device that answered, as a `DbId` ULID string.
    pub device_id: String,
    /// Whether it took the book.
    pub accepted: bool,
}

/// Receives [`OfferDecision`]s so the daemon can tell its parent process.
///
/// The decision arrives on the iroh stream, but the reader it concerns is on
/// the other side of the stdio control channel; this is the seam between them.
pub type OfferDecisionSink = Arc<dyn Fn(OfferDecision) + Send + Sync>;

/// A sink for callers that do not report decisions.
pub fn no_offer_decision_sink() -> OfferDecisionSink {
    Arc::new(|_decision: OfferDecision| {})
}
