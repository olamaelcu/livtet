//! The transport layer's non-HTTP pieces: error mapping, the pairing
//! broadcast registry, and `SyncSession`'s delegation to its client.
//!
//! The route-level behaviour is covered by `routes.rs` and
//! `client_contract.rs`; these cover the parts those cannot reach.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use livtet_data::orm::SqlxSqliteConnector;
use livtet_sync::{
    EntityDump, FullDump, PullResponse, PushResponse, SyncChange, SyncError, SyncStatus,
};
use livtet_sync_http::client::SyncHttpClient;
use livtet_sync_http::error::SyncHttpError;
use livtet_sync_http::server_error::ApiError;
use livtet_sync_http::session::SyncSession;
use livtet_test_utils::sync::Device;
use poem::error::ResponseError;
use poem::http::StatusCode;

// ─── server_error: SyncError → HTTP status ──────────────────────────────────

fn sync_errors() -> Vec<(SyncError, StatusCode)> {
    vec![
        (
            SyncError::UnknownEntityType {
                type_name: "widget".to_string(),
            },
            StatusCode::BAD_REQUEST,
        ),
        (
            SyncError::Conflict {
                entity_type: "work".to_string(),
                entity_id: livtet_types::DbId::new(),
                message: "diverged".to_string(),
            },
            StatusCode::CONFLICT,
        ),
        (
            SyncError::Db {
                message: "disk full".to_string(),
                violation: None,
            },
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            SyncError::Serialization {
                entity_type: "work".to_string(),
                message: "bad".to_string(),
            },
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            SyncError::Deserialization {
                entity_type: "work".to_string(),
                message: "bad".to_string(),
            },
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ]
}

/// The mapping is the documented contract at the top of `server_error.rs`:
/// an unknown entity type is the caller's fault, a conflict is its own
/// status, and everything else is ours.
#[test]
fn api_error_maps_each_sync_error_to_its_status() {
    for (err, expected) in sync_errors() {
        let rendered = err.to_string();
        let api = ApiError::from(err);
        assert_eq!(api.status(), expected, "{rendered}");
        // `#[error(transparent)]`: the wrapper must not add its own prefix.
        assert_eq!(api.to_string(), rendered);
    }
}

#[test]
fn api_error_only_blames_the_caller_for_an_unknown_entity_type() {
    let client_faults: Vec<StatusCode> = sync_errors()
        .into_iter()
        .map(|(err, _)| ApiError::from(err).status())
        .filter(StatusCode::is_client_error)
        .collect();
    assert_eq!(
        client_faults,
        vec![StatusCode::BAD_REQUEST, StatusCode::CONFLICT],
        "only an unknown entity type and a conflict are 4xx"
    );
}

// ─── error: SyncHttpError ───────────────────────────────────────────────────

#[test]
fn sync_http_error_messages_name_what_went_wrong() {
    assert_eq!(
        SyncHttpError::NotConnected.to_string(),
        "not connected: call connect first"
    );
    assert_eq!(
        SyncHttpError::Status {
            code: 503,
            body: "down".to_string()
        }
        .to_string(),
        "server returned HTTP 503: down"
    );
    assert_eq!(
        SyncHttpError::Transport("timed out".to_string()).to_string(),
        "transport error: timed out"
    );
    assert_eq!(
        SyncHttpError::Deserialize("eof".to_string()).to_string(),
        "failed to deserialize response: eof"
    );
    assert_eq!(
        SyncHttpError::Url("nope".to_string()).to_string(),
        "invalid base url: nope"
    );
}

#[test]
fn a_sync_error_passes_through_transparently() {
    let inner = SyncError::UnknownEntityType {
        type_name: "widget".to_string(),
    };
    let rendered = inner.to_string();
    assert_eq!(SyncHttpError::from(inner).to_string(), rendered);
}

#[test]
fn a_json_error_becomes_a_deserialize_error() {
    let json_err = serde_json::from_str::<SyncStatus>("{").expect_err("malformed JSON");
    let err = SyncHttpError::from(json_err);
    assert!(matches!(err, SyncHttpError::Deserialize(_)), "got {err:?}");
    assert!(err.to_string().starts_with("failed to deserialize"));
}

// ─── pairing: the process-wide broadcast registry ───────────────────────────

/// One test owns the whole lifecycle because the registry is a process-wide
/// static -- splitting this across tests would make them order-dependent.
#[tokio::test]
async fn pairing_registry_signals_then_forgets_a_waiter() {
    use livtet_sync_http::pairing::{
        PairWaiters, apply_pairing_decision, get_pair_waiters, set_pair_waiters,
        set_pair_waiters_if_empty,
    };
    use std::collections::HashMap;

    // Firing with no registry at all is a no-op, not a panic.
    assert!(
        !apply_pairing_decision("absent", "approved", "tok").await,
        "no registry means nothing to signal"
    );

    let waiters: PairWaiters = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let (tx, mut rx) = tokio::sync::broadcast::channel(4);
    waiters.lock().await.insert("pair-token".to_string(), tx);

    assert!(
        set_pair_waiters_if_empty(waiters.clone()),
        "the first registration wins"
    );
    assert!(
        !set_pair_waiters_if_empty(Arc::new(tokio::sync::Mutex::new(HashMap::new()))),
        "a second conditional registration must decline"
    );
    assert!(get_pair_waiters().is_some());

    // An unknown token finds no waiter.
    assert!(!apply_pairing_decision("other-token", "approved", "tok").await);

    // The registered token is signalled, and the payload carries the
    // session token the mobile client needs.
    assert!(apply_pairing_decision("pair-token", "approved", "session-123").await);
    let decision = rx.try_recv().expect("the waiter must receive the decision");
    assert_eq!(decision.event, "approved");
    assert_eq!(decision.session_token, "session-123");
    assert_eq!(
        decision.device_id, None,
        "the fan-out path does not know the device id"
    );

    // The entry is removed, so a replay finds nothing.
    assert!(
        !apply_pairing_decision("pair-token", "approved", "session-123").await,
        "a signalled waiter must be forgotten"
    );
    assert!(waiters.lock().await.is_empty());

    // An unconditional set replaces the registry.
    let replacement: PairWaiters = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    set_pair_waiters(replacement);
    assert!(get_pair_waiters().is_some());
}

// ─── session: delegation to the transport client ─────────────────────────────

fn empty_dump() -> FullDump {
    FullDump {
        version: 7,
        device_id: "remote".to_string(),
        entities: EntityDump {
            works: Vec::new(),
            editions: Vec::new(),
            edition_groups: Vec::new(),
            series_entries: Vec::new(),
            digital_inventory: Vec::new(),
            owned_editions: Vec::new(),
            editions_loans: Vec::new(),
            annotations: Vec::new(),
            reading_lists: Vec::new(),
            reading_list_book: Vec::new(),
            reading_progress: Vec::new(),
        },
    }
}

/// Records every call so the delegation can be asserted on, rather than
/// inferred from a return value the session could have invented.
#[derive(Default)]
struct RecordingClient {
    calls: Arc<Mutex<Vec<String>>>,
    base: Option<String>,
}

#[async_trait]
impl SyncHttpClient for RecordingClient {
    async fn connect(&mut self, base_url: &str) -> Result<(), SyncHttpError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("connect({base_url})"));
        self.base = Some(base_url.to_string());
        Ok(())
    }
    async fn status(&self) -> Result<SyncStatus, SyncHttpError> {
        self.calls.lock().unwrap().push("status".to_string());
        Ok(SyncStatus {
            latest_version: 7,
            device_id: "remote".to_string(),
        })
    }
    async fn pull_since(&self, since: i64, limit: i64) -> Result<PullResponse, SyncHttpError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("pull_since({since},{limit})"));
        Ok(PullResponse {
            changes: Vec::new(),
            has_more: false,
            latest_version: 7,
        })
    }
    async fn pull_full(&self) -> Result<FullDump, SyncHttpError> {
        self.calls.lock().unwrap().push("pull_full".to_string());
        Ok(empty_dump())
    }
    async fn push(&self, changes: Vec<SyncChange>) -> Result<PushResponse, SyncHttpError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("push({})", changes.len()));
        Ok(PushResponse {
            accepted: true,
            conflicts: Vec::new(),
            latest_version: 8,
        })
    }
    async fn resolve_conflict(
        &self,
        conflict_id: i64,
        resolution: &str,
        merged: Option<&str>,
    ) -> Result<bool, SyncHttpError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("resolve({conflict_id},{resolution},{merged:?})"));
        Ok(true)
    }
    fn base_url(&self) -> Option<&str> {
        self.base.as_deref()
    }
}

#[tokio::test]
async fn session_forwards_every_remote_call_to_its_client() {
    let device = Device::new("session").await;
    let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(device.pool().clone());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let client = RecordingClient {
        calls: calls.clone(),
        base: None,
    };

    let mut session = SyncSession::new(&conn, "local", client);
    assert_eq!(session.base_url(), None, "not connected yet");

    session
        .connect("http://127.0.0.1:9/")
        .await
        .expect("connect");
    assert_eq!(session.base_url(), Some("http://127.0.0.1:9/"));

    assert_eq!(session.status().await.expect("status").latest_version, 7);
    assert!(
        !session
            .pull_since(3, 50)
            .await
            .expect("pull_since")
            .has_more
    );
    assert_eq!(session.pull_full().await.expect("pull_full").version, 7);
    assert!(session.push(Vec::new()).await.expect("push").accepted);
    assert!(
        session
            .resolve_conflict(4, "local", Some("{}"))
            .await
            .expect("resolve_conflict")
    );

    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [
            "connect(http://127.0.0.1:9/)",
            "status",
            "pull_since(3,50)",
            "pull_full",
            "push(0)",
            "resolve(4,local,Some(\"{}\"))",
        ],
        "each session method must forward exactly once, with its arguments"
    );
}

/// The local methods run against the database, not the client -- so the
/// recording client must see nothing.
#[tokio::test]
async fn session_local_methods_bypass_the_client() {
    let device = Device::new("session-local").await;
    let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(device.pool().clone());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let session = SyncSession::new(
        &conn,
        "local",
        RecordingClient {
            calls: calls.clone(),
            base: None,
        },
    );

    assert_eq!(
        session.local_latest_version().await.expect("version"),
        0,
        "a fresh database has no change_log rows"
    );
    assert!(
        !session
            .local_resolve_conflict(1, "local", None)
            .await
            .expect("resolve"),
        "there is no conflict 1 to resolve"
    );
    let _ = session.engine();

    assert!(
        calls.lock().unwrap().is_empty(),
        "local operations must not touch the transport"
    );
}
