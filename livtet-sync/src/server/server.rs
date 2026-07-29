//! HTTP route handlers and `start_sync_server` for the sync protocol.
//!
//! The 9 routes under `/sync/*` are thin wrappers around the local
//! `SyncEngine` (which now lives in `livtet-sync`'s `client` module).
//! Pairing flow uses an in-process broadcast (`PairWaiters`) shared with the
//! Tauri command layer via `set_pair_waiters` / `apply_pairing_decision`.

use std::sync::Arc;

use futures_util::StreamExt;
use poem::{
    EndpointExt, Error, IntoResponse, Route, Server, get, handler,
    http::StatusCode,
    listener::{Listener, TcpListener},
    middleware::{AddData, Cors},
    post,
    web::{Data, Json, Path, Query, sse},
};
use livtet_data::orm::EntityTrait;
use tokio::sync::{Mutex, RwLock, watch};

type DynError = Box<dyn std::error::Error + Send + Sync>;

use crate::{
    client::engine::SyncEngine,
    server::pairing::{PairWaiters, PairingDecision},
    types::{FullDump, PullResponse, PushResponse, SyncChange, SyncStatus},
};

#[derive(Debug, serde::Deserialize)]
pub struct PullQuery {
    pub since_version: i64,
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct PairRequest {
    pub device_id: String,
    pub name: String,
    pub device_type: String,
    pub token: String,
}

#[handler]
async fn get_status(engine: Data<&Arc<RwLock<SyncEngine>>>) -> Result<Json<SyncStatus>, Error> {
    let inner = engine.read().await;
    let latest_version = inner
        .get_latest_version()
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;
    Ok(Json(SyncStatus {
        latest_version,
        device_id: "desktop".to_string(),
    }))
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

#[derive(Debug, serde::Deserialize)]
struct ResolveRequest {
    resolution: String,
    merged_payload: Option<String>,
}

#[handler]
async fn post_pair(
    engine: Data<&Arc<RwLock<SyncEngine>>>,
    pair_waiters: Data<&PairWaiters>,
    Json(body): Json<PairRequest>,
) -> Result<Json<serde_json::Value>, Error> {
    let inner = engine.read().await;

    use livtet_data::client_entities::pending_pairings;
    use livtet_types::{DbId, DeviceType, PairingStatus};
    use livtet_data::orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

    let pending_id = DbId::from(PairingStatus::Pending);
    let now = time::OffsetDateTime::now_utc();
    let now_pdt = time::PrimitiveDateTime::new(now.date(), now.time());

    let pending = pending_pairings::Entity::find_by_id(&body.token)
        .filter(pending_pairings::Column::StatusId.eq(pending_id))
        .filter(pending_pairings::Column::ExpiresAt.gt(now_pdt))
        .one(inner.db())
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?
        .ok_or(Error::from_status(StatusCode::UNAUTHORIZED))?;

    let device_type_id = Some(DbId::from(DeviceType::from_tag(&body.device_type)));

    let mut model: pending_pairings::ActiveModel = pending.into();
    model.device_name = Set(Some(body.name.clone()));
    model.device_type_id = Set(device_type_id);
    model
        .update(inner.db())
        .await
        .map_err(|_e| Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;

    let (tx, _rx) = tokio::sync::broadcast::channel::<PairingDecision>(1);
    {
        let mut waiters = pair_waiters.lock().await;
        waiters.insert(body.token.clone(), tx);
    }

    tracing::info!(
        "pairing request from device {} ({}) with token {}",
        body.device_id,
        body.name,
        body.token,
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
) -> Result<Json<Vec<crate::types::Conflict>>, Error> {
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
) -> impl poem::Endpoint<Output = poem::Response> {
    Route::new()
        .at("/sync/status", get(get_status))
        .at("/sync/changes", get(get_changes))
        .at("/sync/pull-full", get(get_pull_full))
        .at("/sync/push", post(post_push))
        .at("/sync/pair", post(post_pair))
        .at("/sync/pair/status/:token", get(get_pair_status))
        .at("/sync/conflicts", get(get_conflicts))
        .at("/sync/conflicts/:id/resolve", post(post_resolve_conflict))
        .at("/sync/files/:inventory_id", get(get_file))
        .with(AddData::new(engine))
        .with(AddData::new(pair_waiters))
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

    pub async fn start(
        &mut self,
        db: livtet_data::orm::DatabaseConnection,
        device_id: String,
        port: u16,
    ) -> Result<(), DynError> {
        use std::collections::HashMap;

        use tokio::time::Duration;

        use crate::types::change_log;

        if self.is_running() {
            return Err("sync server already running".into());
        }

        change_log::setup_change_log(&db)
            .await
            .map_err(|e| Box::new(e) as DynError)?;
        let engine = Arc::new(RwLock::new(SyncEngine::new(db, device_id)));
        let pair_waiters: PairWaiters = Arc::new(Mutex::new(HashMap::new()));
        crate::server::pairing::set_pair_waiters(pair_waiters.clone());
        let app = make_sync_routes(engine, pair_waiters);
        let listener = TcpListener::bind(format!("0.0.0.0:{}", port));
        tracing::info!("sync server starting on 0.0.0.0:{}", port);

        let acceptor = listener
            .into_acceptor()
            .await
            .map_err(|e| Box::new(e) as DynError)?;

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

        Ok(())
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

    use crate::types::change_log;

    change_log::setup_change_log(&db)
        .await
        .map_err(|e| Box::new(e) as DynError)?;
    let engine = Arc::new(RwLock::new(SyncEngine::new(db, device_id)));
    let pair_waiters: PairWaiters = Arc::new(Mutex::new(HashMap::new()));
    // Register the broadcast registry so the Tauri app can fan out pairing
    // decisions via apply_pairing_decision() without going through HTTP.
    crate::server::pairing::set_pair_waiters(pair_waiters.clone());
    let app = make_sync_routes(engine, pair_waiters);
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port));
    tracing::info!("sync server started on 0.0.0.0:{}", port);
    Server::new(listener).run(app).await?;
    Ok(())
}
