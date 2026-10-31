//! HTTP route handlers and `start_sync_server` for the sync protocol.
//!
//! The 9 routes under `/sync/*` are thin wrappers around the local
//! [`SyncEngine`] (which lives in `livtet-sync`).  Pairing flow uses an
//! in-process broadcast (`PairWaiters`) shared with the Tauri command layer
//! via `set_pair_waiters` / `apply_pairing_decision`.
//!
//! Unlike the source crate, the server does not install the `change_log` /
//! `conflicts` tables or the sync triggers: those are owned by
//! `livtet-data`'s client migrations, which callers must run before starting
//! the server.

use std::sync::Arc;

use futures_util::StreamExt;
use livtet_data::orm::EntityTrait;
use livtet_iroh::NodeId;
use livtet_sync::{
    Conflict, FullDump, PullResponse, PushResponse, SyncChange, SyncEngine, SyncStatus,
};
use poem::{
    EndpointExt, Error, IntoResponse, Route, Server, get, handler,
    http::StatusCode,
    listener::{Listener, TcpListener},
    middleware::{AddData, Cors},
    post,
    web::{Data, Json, Path, Query, sse},
};
use tokio::sync::{Mutex, RwLock, watch};

use crate::{
    pairing::{PairWaiters, PairingDecision},
    routes::{
        CHANGES_PATH, CONFLICTS_PATH, FILE_PATH, IDENTITY_PATH, PAIR_PATH, PAIR_STATUS_PATH,
        PULL_FULL_PATH, PUSH_PATH, PairRequest, PullQuery, RESOLVE_CONFLICT_PATH, ResolveRequest,
        STATUS_PATH, SyncIdentity,
    },
};

type DynError = Box<dyn std::error::Error + Send + Sync>;

/// How long a `/sync/pair` request stays approvable, in seconds.
const PAIR_TICKET_TTL_SECS: i64 = 300;

#[handler]
async fn get_status(engine: Data<&Arc<RwLock<SyncEngine>>>) -> Result<Json<SyncStatus>, Error> {
    let inner = engine.read().await;
    let latest_version = inner
        .get_latest_version()
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
    Ok(Json(SyncStatus {
        latest_version,
        device_id: inner.device_id().to_string(),
    }))
}

/// The peer identity, so a device that found this server over mDNS can confirm
/// the node id it saw in the TXT record before asking to pair.
#[handler]
async fn get_identity(identity: Data<&SyncIdentity>) -> Json<SyncIdentity> {
    Json(identity.clone())
}

#[handler]
async fn get_changes(
    engine: Data<&Arc<RwLock<SyncEngine>>>,
    query: Query<PullQuery>,
) -> Result<Json<PullResponse>, Error> {
    let limit = query.limit.unwrap_or(100);
    let inner = engine.read().await;
    let response = inner
        .pull_changes(query.since_version, limit)
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
    Ok(Json(response))
}

#[handler]
async fn get_pull_full(engine: Data<&Arc<RwLock<SyncEngine>>>) -> Result<Json<FullDump>, Error> {
    let inner = engine.read().await;
    let dump = inner
        .pull_full()
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
    Ok(Json(dump))
}

#[handler]
async fn post_pair(
    engine: Data<&Arc<RwLock<SyncEngine>>>,
    pair_waiters: Data<&PairWaiters>,
    remote: &poem::web::RemoteAddr,
    local: &poem::web::LocalAddr,
    Json(body): Json<PairRequest>,
) -> Result<Json<serde_json::Value>, Error> {
    let inner = engine.read().await;

    use livtet_data::client_entities::pending_pairings;
    use livtet_data::orm::{ActiveModelTrait, EntityTrait, Set};
    use livtet_types::{Address, DbId, DeviceType, PairingStatus};

    let pending_id = DbId::from(PairingStatus::Pending);
    // The pairing key is the peer's iroh node identity; reject anything that
    // is not a plausible node id before it reaches the pending table.
    let node_id: NodeId = body
        .node_id
        .parse()
        .map_err(|_e| Error::from_status(StatusCode::BAD_REQUEST))?;
    let now = time::OffsetDateTime::now_utc();
    let now_pdt = time::PrimitiveDateTime::new(now.date(), now.time());
    let expires = now + time::Duration::seconds(PAIR_TICKET_TTL_SECS);
    let expires_pdt = time::PrimitiveDateTime::new(expires.date(), expires.time());

    let device_type_id = Some(DbId::from(DeviceType::from_tag(&body.device_type)));

    // Persist the remote device's ULID so the approval step can create the
    // matching `paired_devices` row. Accept either the ULID string form or the
    // 32-char hex form, and fall back to `None` when it is not a valid id.
    let device_id = body
        .device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(&body.device_id).ok());

    // Record where the request came from so the daemon can tell this remote
    // request apart from one it minted itself (which has no origin). The
    // peer's source port is ephemeral, so relate its IP to the address this
    // server was reached on: a request from the server's own IP therefore
    // normalizes to the server's exact `ip:port` and reads back as self.
    let origin_addr = remote
        .0
        .as_socket_addr()
        .zip(local.0.as_socket_addr())
        .map(|(peer, server)| Address::new(std::net::SocketAddr::new(peer.ip(), server.port())));

    let desktop_id = inner
        .device_id()
        .parse::<DbId>()
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
    let listen_on = local.0.as_socket_addr().map(|addr| Address::new(*addr));

    // The remote's node identity is the row key. A peer that discovery has
    // just introduced is not yet in the table, so the first request inserts
    // it; a peer retrying while its ticket is open refreshes the row instead.
    let existing = pending_pairings::Entity::find_by_id(node_id.as_str())
        .one(inner.db())
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;

    match existing {
        // A decided ticket must not be reopened by replaying `/sync/pair`.
        Some(row) if row.status_id != Some(pending_id) => {
            return Err(Error::from_status(StatusCode::CONFLICT));
        }
        Some(row) => {
            let mut model: pending_pairings::ActiveModel = row.into();
            model.desktop_id = Set(desktop_id);
            model.listen_on = Set(listen_on);
            model.device_name = Set(Some(body.name.clone()));
            model.device_type_id = Set(device_type_id);
            model.device_id = Set(device_id);
            model.origin_addr = Set(origin_addr);
            model.expires_at = Set(expires_pdt);
            model
                .update(inner.db())
                .await
                .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
        }
        None => {
            let model = pending_pairings::ActiveModel {
                token: Set(node_id.0.clone()),
                desktop_id: Set(desktop_id),
                listen_on: Set(listen_on),
                status_id: Set(Some(pending_id)),
                device_name: Set(Some(body.name.clone())),
                device_type_id: Set(device_type_id),
                device_id: Set(device_id),
                origin_addr: Set(origin_addr),
                created_at: Set(now_pdt),
                expires_at: Set(expires_pdt),
            };
            model
                .insert(inner.db())
                .await
                .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
        }
    }

    let (tx, _rx) = tokio::sync::broadcast::channel::<PairingDecision>(1);
    {
        let mut waiters = pair_waiters.lock().await;
        waiters.insert(node_id.0.clone(), tx);
    }

    tracing::info!(
        "pairing request from device {} ({}) with node_id {}",
        body.device_id,
        body.name,
        node_id.as_str(),
    );
    Ok(Json(serde_json::json!({ "status": "pending" })))
}

#[handler]
async fn post_push(
    engine: Data<&Arc<RwLock<SyncEngine>>>,
    Json(changes): Json<Vec<SyncChange>>,
) -> Result<Json<PushResponse>, Error> {
    let inner = engine.read().await;
    let response = inner
        .push_changes(changes)
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
    Ok(Json(response))
}

#[handler]
async fn post_resolve_conflict(
    engine: Data<&Arc<RwLock<SyncEngine>>>,
    Path(conflict_id): Path<i64>,
    Json(body): Json<ResolveRequest>,
) -> Result<Json<serde_json::Value>, Error> {
    if !matches!(body.resolution.as_str(), "local" | "remote" | "merged") {
        return Err(Error::from_status(StatusCode::BAD_REQUEST));
    }
    let inner = engine.read().await;
    let resolved = inner
        .resolve_conflict(
            conflict_id,
            &body.resolution,
            body.merged_payload.as_deref(),
        )
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;

    if resolved {
        Ok(Json(serde_json::json!({ "ok": true })))
    } else {
        Err(Error::from_status(StatusCode::NOT_FOUND))
    }
}

#[handler]
async fn get_pair_status(
    pair_waiters: Data<&PairWaiters>,
    // `token` here is the node identity (node_id) used for identity-based pairing.
    Path(token): Path<String>,
) -> Result<impl IntoResponse, Error> {
    let waiter = {
        let waiters = pair_waiters.lock().await;
        waiters.get(&token).cloned()
    };

    let mut rx = match waiter {
        Some(tx) => tx.subscribe(),
        None => {
            return Err(Error::from_status(StatusCode::NOT_FOUND));
        }
    };

    let stream = async_stream::stream! {
        let timeout_duration = std::time::Duration::from_secs(300);

        loop {
            let delay = tokio::time::sleep(timeout_duration);
            tokio::pin!(delay);

            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(decision) => {
                            let s = serde_json::to_string(&decision).unwrap_or_default();
                            yield s;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
                _ = delay => {
                    break;
                }
            }
        }
    };

    Ok(sse::SSE::new(stream.map(sse::Event::message)).with_content_type("text/event-stream"))
}

#[handler]
async fn get_conflicts(
    engine: Data<&Arc<RwLock<SyncEngine>>>,
) -> Result<Json<Vec<Conflict>>, Error> {
    let inner = engine.read().await;
    let conflicts = inner
        .list_conflicts()
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
    Ok(Json(conflicts))
}

#[allow(clippy::disallowed_types, clippy::disallowed_methods)]
#[handler]
async fn get_file(
    Path(inventory_id): Path<String>,
    Data(engine): Data<&Arc<RwLock<SyncEngine>>>,
    req: &poem::Request,
) -> Result<poem::Response, Error> {
    use livtet_types::DbId;
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let inv_id = match hex::decode(&inventory_id) {
        Ok(b) if b.len() == 16 => {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(&b);
            DbId::from_bytes(arr)
        }
        _ => return Err(Error::from_status(StatusCode::BAD_REQUEST)),
    };

    let engine = engine.read().await;

    let row = match livtet_data::entities::digital_inventory::Entity::find_by_id(inv_id)
        .one(engine.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return Err(Error::from_status(StatusCode::NOT_FOUND)),
        Err(_) => return Err(Error::from_status(StatusCode::INTERNAL_SERVER_ERROR)),
    };

    let Some(file_path) = row.file_path else {
        return Err(Error::from_status(StatusCode::NOT_FOUND));
    };

    let path = std::path::Path::new(file_path.as_str());
    if !path.exists() {
        return Err(Error::from_status(StatusCode::NOT_FOUND));
    }

    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(_) => return Err(Error::from_status(StatusCode::INTERNAL_SERVER_ERROR)),
    };
    let metadata = match file.metadata().await {
        Ok(m) => m,
        Err(_) => return Err(Error::from_status(StatusCode::INTERNAL_SERVER_ERROR)),
    };
    let total_len = metadata.len();
    let content_type = if file_path.ends_with(".pdf") {
        "application/pdf"
    } else {
        "application/epub+zip"
    };

    if let Some(if_range) = req.headers().get("if-range")
        && let Some(cached_hash) = row.file_hash.as_deref()
    {
        let provided = if_range.to_str().unwrap_or("");
        if provided != cached_hash {
            let body = poem::Body::from_async_read(file);
            return Ok(poem::Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", content_type)
                .header("Content-Length", total_len)
                .header("Accept-Ranges", "bytes")
                .body(body));
        }
    }

    if let Some(range_header) = req.headers().get("range") {
        let range_str = range_header.to_str().unwrap_or("");
        let ranges = match http_range::HttpRange::parse(range_str, total_len) {
            Ok(r) => r,
            Err(_) => return Err(Error::from_status(StatusCode::RANGE_NOT_SATISFIABLE)),
        };
        let range = &ranges[0];

        // Use saturating arithmetic for Content-Range end position.
        // http_range validates the range against total_len, but we still guard
        // against overflow in the end-byte calculation.
        let end = range.start.saturating_add(range.length).saturating_sub(1);

        if file
            .seek(std::io::SeekFrom::Start(range.start))
            .await
            .is_err()
        {
            return Err(Error::from_status(StatusCode::INTERNAL_SERVER_ERROR));
        }

        let mut buf = vec![0u8; range.length as usize];
        if file.read_exact(&mut buf).await.is_err() {
            return Err(Error::from_status(StatusCode::INTERNAL_SERVER_ERROR));
        }

        return Ok(poem::Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header("Content-Type", content_type)
            .header("Content-Length", range.length)
            .header(
                "Content-Range",
                format!("bytes {}-{}/{}", range.start, end, total_len),
            )
            .header("Accept-Ranges", "bytes")
            .body(buf));
    }

    let body = poem::Body::from_async_read(file);
    Ok(poem::Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", content_type)
        .header("Content-Length", total_len)
        .header("Accept-Ranges", "bytes")
        .body(body))
}

pub fn make_sync_routes(
    engine: Arc<RwLock<SyncEngine>>,
    pair_waiters: PairWaiters,
    identity: SyncIdentity,
) -> impl poem::Endpoint<Output = poem::Response> {
    Route::new()
        .at(STATUS_PATH, get(get_status))
        .at(IDENTITY_PATH, get(get_identity))
        .at(CHANGES_PATH, get(get_changes))
        .at(PULL_FULL_PATH, get(get_pull_full))
        .at(PUSH_PATH, post(post_push))
        .at(PAIR_PATH, post(post_pair))
        .at(PAIR_STATUS_PATH, get(get_pair_status))
        .at(CONFLICTS_PATH, get(get_conflicts))
        .at(RESOLVE_CONFLICT_PATH, post(post_resolve_conflict))
        .at(FILE_PATH, get(get_file))
        .with(AddData::new(engine))
        .with(AddData::new(pair_waiters))
        .with(AddData::new(identity))
        .with(Cors::new())
}

/// A managed sync server instance with graceful shutdown.
///
/// Follows the same pattern as `livtet_opds_server::PoemOpdsServer`:
/// - `start()` binds the port, spawns the poem server, returns a handle
/// - `stop()` signals shutdown via watch channel, awaits the join handle
/// - `is_running()` checks if the server is currently active
///
/// On restart, callers must call `stop()` before calling `start()` again.
pub struct SyncServerInstance {
    shutdown_tx: Option<watch::Sender<bool>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl SyncServerInstance {
    pub fn new() -> Self {
        Self {
            shutdown_tx: None,
            join: None,
        }
    }

    /// Start listening on every interface at `port`.
    ///
    /// Equivalent to [`Self::start_on`] with `0.0.0.0:{port}`, discarding
    /// the bound address.
    pub async fn start(
        &mut self,
        db: livtet_data::orm::DatabaseConnection,
        device_id: String,
        port: u16,
    ) -> Result<(), DynError> {
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        self.start_on(db, device_id, addr).await.map(|_| ())
    }

    /// Start listening on `addr` and return the address actually bound,
    /// which differs from `addr` when it asks for an ephemeral port (`:0`).
    pub async fn start_on(
        &mut self,
        db: livtet_data::orm::DatabaseConnection,
        device_id: String,
        addr: std::net::SocketAddr,
    ) -> Result<std::net::SocketAddr, DynError> {
        use std::collections::HashMap;

        use poem::listener::Acceptor;
        use tokio::time::Duration;

        if self.is_running() {
            return Err("sync server already running".into());
        }

        let engine = Arc::new(RwLock::new(SyncEngine::new(db, device_id.clone())));
        let pair_waiters: PairWaiters = Arc::new(Mutex::new(HashMap::new()));
        crate::pairing::set_pair_waiters(pair_waiters.clone());
        // Without a bound endpoint the node id falls back to the device id; the
        // daemon passes the real iroh id through `make_sync_routes` itself.
        let identity = SyncIdentity {
            node_id: device_id.clone(),
            device_id,
        };
        let app = make_sync_routes(engine, pair_waiters, identity);
        let listener = TcpListener::bind(addr);

        let acceptor = listener
            .into_acceptor()
            .await
            .map_err(|e| Box::new(e) as DynError)?;
        let bound = acceptor
            .local_addr()
            .first()
            .and_then(|local| local.as_socket_addr().copied())
            .ok_or("sync server listener has no socket address")?;
        tracing::info!("sync server starting on {bound}");

        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        let join = tokio::spawn(async move {
            let signal = async move {
                while shutdown_rx.changed().await.is_ok() {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            };
            if let Err(e) = Server::new_with_acceptor(acceptor)
                .name("livtet-sync")
                .run_with_graceful_shutdown(app, signal, Some(Duration::from_secs(5)))
                .await
            {
                tracing::error!(error = %e, "sync server error");
            }
        });

        self.shutdown_tx = Some(shutdown_tx);
        self.join = Some(join);

        Ok(bound)
    }

    pub async fn stop(&mut self) -> Result<(), DynError> {
        if !self.is_running() {
            return Ok(());
        }

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }

        if let Some(join) = self.join.take() {
            let _ = join.await;
        }

        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.join.is_some()
    }
}

impl Default for SyncServerInstance {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn start_sync_server(
    db: livtet_data::orm::DatabaseConnection,
    device_id: String,
    port: u16,
) -> Result<(), DynError> {
    use std::collections::HashMap;

    let engine = Arc::new(RwLock::new(SyncEngine::new(db, device_id.clone())));
    let pair_waiters: PairWaiters = Arc::new(Mutex::new(HashMap::new()));
    // Register the broadcast registry so the Tauri app can fan out pairing
    // decisions via apply_pairing_decision() without going through HTTP.
    crate::pairing::set_pair_waiters(pair_waiters.clone());
    let identity = SyncIdentity {
        node_id: device_id.clone(),
        device_id,
    };
    let app = make_sync_routes(engine, pair_waiters, identity);
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port));
    tracing::info!("sync server started on 0.0.0.0:{}", port);
    Server::new(listener).run(app).await?;
    Ok(())
}
