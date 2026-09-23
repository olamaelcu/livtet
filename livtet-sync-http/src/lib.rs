//! HTTP transport for the livtet sync protocol.
//!
//! Splits the transport half out of `livtet-sync`: the domain engine and
//! wire DTOs live in `livtet-sync`, while this crate owns the borrowed
//! HTTP surface — a transport-agnostic [`SyncHttpClient`] trait, the
//! [`SyncSession`] that pairs an engine with a client, and the shared
//! route paths / request DTOs.
//!
//! Two optional backends sit behind feature flags, both off by default:
//!
//! * `reqwest` — the client implementation ([`ReqwestHttpClient`]).
//! * `poem` — the server routes and pairing fan-out.
//!
//! With `--no-default-features` the crate compiles with zero HTTP
//! dependencies.

pub mod client;
pub mod error;
pub mod routes;
pub mod session;

#[cfg(feature = "poem")]
pub mod pairing;
#[cfg(feature = "reqwest")]
pub mod reqwest_client;
#[cfg(feature = "poem")]
pub mod server;
#[cfg(feature = "poem")]
pub mod server_error;

pub use client::SyncHttpClient;
pub use error::SyncHttpError;
pub use routes::{
    CHANGES_PATH, CONFLICTS_PATH, FILE_PATH, PAIR_PATH, PAIR_STATUS_PATH, PULL_FULL_PATH,
    PUSH_PATH, PairRequest, PullQuery, RESOLVE_CONFLICT_PATH, ResolveRequest, STATUS_PATH,
    join_url,
};
pub use session::SyncSession;

#[cfg(feature = "poem")]
pub use pairing::{
    PairWaiters, PairingDecision, apply_pairing_decision, get_pair_waiters, set_pair_waiters,
    set_pair_waiters_if_empty,
};
#[cfg(feature = "reqwest")]
pub use reqwest_client::{ReqwestHttpClient, ReqwestSyncClient};
#[cfg(feature = "poem")]
pub use server::{SyncServerInstance, make_sync_routes, start_sync_server};
#[cfg(feature = "poem")]
pub use server_error::ApiError;
