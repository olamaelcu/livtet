//! Poem-based HTTP server for the livtet sync protocol.
//!
//! Hosts the 9 `/sync/*` route handlers, the in-process pairing
//! decision fan-out (`set_pair_waiters`/`apply_pairing_decision`),
//! and the `impl ResponseError for SyncError` that maps the engine
//! error type to HTTP status codes.  Depends on the `client` module
//! for the `SyncEngine` and on `livtet-sync-types` for the wire DTOs
//! and `SyncError`.

pub mod error;
pub mod pairing;
pub mod server;

pub use error::ApiError;
pub use pairing::{
    PairWaiters, PairingDecision, apply_pairing_decision, get_pair_waiters, set_pair_waiters,
    set_pair_waiters_if_empty,
};
pub use server::{PairRequest, PullQuery, SyncServerInstance, make_sync_routes, start_sync_server};
