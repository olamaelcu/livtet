//! Everything the sync wire needs, without the daemon around it.
//!
//! [`crate::wire`] answers a peer's frames from six pieces of state: the engine,
//! the pool for the columns the entities do not model, this device's id, the
//! pairing maps, and the offer sink. The daemon adds a lot the wire never
//! touches — a JSON-RPC pipe, a message queue, endpoint lifecycle locks — which
//! is why the wire takes [`WireState`] rather than [`DaemonState`]: an embedder
//! such as the mobile FFI serves the same protocol from the same code with only
//! the state it actually has, instead of a second implementation that would
//! drift on authorization and range handling.
//!
//! The reporting seams are optional. A daemon reports every answered request to
//! its parent process; an embedded endpoint has no parent to tell, so it leaves
//! them `None` and the wire skips the report rather than pretending to send it.
//!
//! [`DaemonState`]: crate::daemon::DaemonState
//! [`DaemonState::wire_state`]: crate::daemon::DaemonState::wire_state

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use livtet_data::sql::SqlitePool;
use livtet_sync::SyncEngine;
use tokio::sync::Mutex;

use crate::daemon::HostBridge;
use crate::pairing::{
    OfferDecisionSink, PairDecisions, PairWaiters, new_pair_decisions, new_pair_waiters,
    no_offer_decision_sink,
};
use crate::requests::{Notifier, RequestLog, new_request_log};

/// How many requests this endpoint has answered, and when it last answered one.
#[derive(Clone, Default)]
pub struct RequestCounters {
    /// Total frames answered.
    pub served: Arc<AtomicU64>,
    /// Timestamp of the most recent answer.
    pub last_at: Arc<Mutex<Option<String>>>,
}

impl RequestCounters {
    /// A fresh pair of counters at zero.
    pub fn new() -> Self {
        Self::default()
    }
}

/// The state one sync endpoint answers from.
///
/// Build it with [`WireState::new`], then add the reporting seams the host can
/// serve. The defaults answer `Identity`, pairing, and file requests, and say
/// nothing to anyone.
#[derive(Clone)]
pub struct WireState {
    /// The local change log and entity tables.
    pub engine: SyncEngine,
    /// Raw access for the columns the entities do not model.
    pub pool: SqlitePool,
    /// This endpoint's device id.
    pub device_id: String,
    /// This endpoint's iroh node id, as peers dial it.
    ///
    /// `Identity` reports it so a peer can confirm who it reached. An endpoint
    /// that has not bound yet leaves it `None` and the device id stands in, the
    /// same fallback the daemon uses when discovery is unavailable.
    pub node_id: Option<String>,
    /// Outstanding pairing claims, keyed by the claimant's node id.
    pub pair_waiters: PairWaiters,
    /// Answered claims, kept resumable for a while.
    pub pair_decisions: PairDecisions,
    /// Where an answered offer goes.
    pub offer_sink: OfferDecisionSink,
    /// JSON-RPC notifications, when a parent process is listening.
    pub notifier: Option<Notifier>,
    /// Reverse channel for OPDS pass-through, when the host owns a route table.
    pub host: Option<HostBridge>,
    /// The most recent answered requests. Nothing is kept when `None`.
    pub request_log: Option<RequestLog>,
    /// Answer counters. Not counted when `None`.
    pub counters: Option<RequestCounters>,
}

impl WireState {
    /// The state every endpoint needs: the engine, the pool, and the device id.
    ///
    /// Pairing maps are fresh, the offer sink swallows decisions, and nothing is
    /// reported anywhere.
    pub fn new(engine: SyncEngine, pool: SqlitePool, device_id: impl Into<String>) -> Self {
        Self {
            engine,
            pool,
            device_id: device_id.into(),
            node_id: None,
            pair_waiters: new_pair_waiters(),
            pair_decisions: new_pair_decisions(),
            offer_sink: no_offer_decision_sink(),
            notifier: None,
            host: None,
            request_log: None,
            counters: None,
        }
    }

    /// Report answered requests as JSON-RPC notifications.
    pub fn with_notifier(mut self, notifier: Notifier) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// Forward `Opds` requests to a host that owns the route table.
    pub fn with_host(mut self, host: HostBridge) -> Self {
        self.host = Some(host);
        self
    }

    /// Keep the most recent answered requests for inspection.
    pub fn with_request_log(mut self, log: RequestLog) -> Self {
        self.request_log = Some(log);
        self
    }

    /// Keep an empty request log of this endpoint's own.
    pub fn with_a_request_log(self) -> Self {
        self.with_request_log(new_request_log())
    }

    /// Count the requests this endpoint answers.
    pub fn with_counters(mut self, counters: RequestCounters) -> Self {
        self.counters = Some(counters);
        self
    }

    /// Start counting requests, from zero.
    pub fn counting(self) -> Self {
        self.with_counters(RequestCounters::new())
    }

    /// Hand offer decisions to `sink` instead of dropping them.
    pub fn with_offer_sink(mut self, sink: OfferDecisionSink) -> Self {
        self.offer_sink = sink;
        self
    }

    /// Share pairing maps with another endpoint, e.g. to resume claims filed
    /// before a rebind.
    pub fn with_pairing(mut self, waiters: PairWaiters, decisions: PairDecisions) -> Self {
        self.pair_waiters = waiters;
        self.pair_decisions = decisions;
        self
    }
    /// The node id this endpoint answers as.
    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    /// The node id peers should be told this endpoint has.
    ///
    /// Falls back to the device id when no endpoint is bound, which is what the
    /// daemon reports in the same situation.
    pub fn self_node(&self) -> String {
        self.node_id
            .clone()
            .unwrap_or_else(|| self.device_id.clone())
    }
}
