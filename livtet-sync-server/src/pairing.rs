//! Pairing decision fan-out and claim resume.
//!
//! A pairing claim arrives on the framed iroh transport and is answered
//! immediately with a claim secret; the claimant then polls `PairStatus` on any
//! connection until the reader decides. Two pieces of state make that work:
//!
//! - [`PairWaiters`], keyed by node id, holds the claim a node currently has
//!   outstanding. Approving or rejecting wakes the broadcast channel the old
//!   single-stream protocol used to wait on.
//! - [`PairDecisions`], keyed by claim, remembers the answer for
//!   [`PAIR_DECISION_TTL`] so a claimant that was disconnected, or that comes
//!   back on a new connection, learns the outcome instead of losing it.
//!
//! There is no process-global registry: [`DaemonState`] is the only holder.
//!
//! [`DaemonState`]: crate::daemon::DaemonState

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use livtet_sync_iroh::Response;
use tokio::sync::{Mutex, broadcast};
use tokio::time::Instant;

/// How long an answered claim stays resumable.
pub const PAIR_DECISION_TTL: Duration = Duration::from_secs(300);

/// How many answered claims are remembered at once.
///
/// A pairing attempt is human-paced, so this is generous; the bound stops a
/// script from growing the map by filing and abandoning claims.
pub const PAIR_DECISION_CAP: usize = 256;

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
    /// Secret generated when the claim was filed. The claimant presents it on
    /// `PairStatus` to resume after a dropped connection.
    pub claim: String,
    /// Decision fan-out, kept for callers that still listen on it.
    pub sender: broadcast::Sender<PairingDecision>,
}

/// Node id → outstanding waiter, owned by the daemon.
pub type PairWaiters = Arc<Mutex<HashMap<String, PairingWaiter>>>;

/// Create an empty waiter map.
pub fn new_pair_waiters() -> PairWaiters {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Claim → decided outcome, owned by the daemon.
pub type PairDecisions = Arc<Mutex<HashMap<String, DecidedPairing>>>;

/// One claim the reader has answered, kept resumable for [`PAIR_DECISION_TTL`].
#[derive(Clone, Debug)]
pub struct DecidedPairing {
    /// The node id that filed the claim. A different node presenting the same
    /// claim secret gets `NotFound`, not someone else's pairing.
    pub node_id: String,
    /// Whether the reader approved.
    pub approved: bool,
    /// The device id the pairing was filed under.
    pub device_id: Option<String>,
    /// Token issued on approval, empty on rejection.
    pub session_token: String,
    /// When the decision landed, for expiry.
    pub decided_at: Instant,
}

impl DecidedPairing {
    /// The frame a claimant polling `PairStatus` receives.
    pub fn response(&self) -> Response {
        if self.approved {
            Response::Paired {
                device_id: self.device_id.clone().unwrap_or_default(),
                session_token: self.session_token.clone(),
            }
        } else {
            Response::Refused {
                reason: "the pairing was rejected".to_string(),
            }
        }
    }
}

/// Create an empty decision map.
pub fn new_pair_decisions() -> PairDecisions {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Answers `node_id`'s outstanding claim.
///
/// The waiter's claim is recorded in `decisions` before the waiter is dropped,
/// so a claimant that missed the broadcast — or never listened — can still
/// resume by secret, and a poll never observes the gap between the two.
///
/// Returns `true` when a waiter was found and signalled. A sender with no
/// receivers is not an error: the claimant may not be listening on the old
/// channel, and the pairing itself still completed.
pub async fn apply_pairing_decision(
    waiters: &PairWaiters,
    decisions: &PairDecisions,
    node_id: &str,
    decision: &str,
    session_token: &str,
    device_id: Option<&str>,
) -> bool {
    let payload = PairingDecision {
        event: decision.to_string(),
        session_token: session_token.to_string(),
    };

    let mut map = waiters.lock().await;
    let Some(waiter) = map.remove(node_id) else {
        return false;
    };

    // The decision is recorded before the waiter lock is released: a claimant
    // polling `PairStatus` never observes the instant where the waiter is gone
    // and the decision is not yet there.
    record_decision(
        decisions,
        waiter.claim,
        node_id,
        decision == "approved",
        session_token,
        device_id,
    )
    .await;
    let _ = waiter.sender.send(payload);
    true
}

/// Remember one decided claim, evicting the oldest once the cap is reached.
async fn record_decision(
    decisions: &PairDecisions,
    claim: String,
    node_id: &str,
    approved: bool,
    session_token: &str,
    device_id: Option<&str>,
) {
    let mut map = decisions.lock().await;
    prune(&mut map);
    while map.len() >= PAIR_DECISION_CAP {
        let oldest = map
            .iter()
            .min_by_key(|(_, decided)| decided.decided_at)
            .map(|(claim, _)| claim.clone());
        match oldest {
            Some(claim) => map.remove(&claim),
            None => break,
        };
    }
    map.insert(
        claim,
        DecidedPairing {
            node_id: node_id.to_string(),
            approved,
            device_id: device_id.map(str::to_string),
            session_token: session_token.to_string(),
            decided_at: Instant::now(),
        },
    );
}

/// What became of `claim`, if it was decided for `node_id` and has not expired.
pub async fn decided(
    decisions: &PairDecisions,
    claim: &str,
    node_id: &str,
) -> Option<DecidedPairing> {
    let mut map = decisions.lock().await;
    prune(&mut map);
    map.get(claim)
        .filter(|decided| decided.node_id == node_id)
        .cloned()
}

/// Drop decisions whose window has closed. The caller holds the lock.
fn prune(map: &mut HashMap<String, DecidedPairing>) {
    let now = Instant::now();
    map.retain(|_, decided| now.duration_since(decided.decided_at) < PAIR_DECISION_TTL);
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
