//! The sync server daemon: SQLite + embedded sync HTTP server + stdio RPC loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use livtet_data::orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};
use livtet_sync::SyncEngine;
use livtet_sync_http::routes::{CHANGES_PATH, PAIR_PATH, PULL_FULL_PATH, PUSH_PATH};
use livtet_sync_http::{PairWaiters, make_sync_routes, set_pair_waiters};
use livtet_types::{Address, DbId, PairingStatus};
use poem::{Endpoint, EndpointExt, Server};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, RwLock, mpsc, watch};

use crate::config::ServerConfig;
use crate::error::ServerError;
use crate::requests::{
    self, Notifier, REQUEST_LOG_CAP, RequestLog, RequestRecord, new_request_log,
};
use crate::rpc::{
    self, DeviceParams, PairingBeginParams, RecentParams, ResolveParams, TokenParams,
};

/// Shared, long-lived daemon state.
pub struct DaemonState {
    /// The resolved configuration.
    pub config: ServerConfig,
    /// The SQLite connection pool (used for raw statements the entity
    /// models don't cover, e.g. `paired_devices.session_token`).
    pub pool: livtet_data::sql::SqlitePool,
    /// The local sync engine.
    pub engine: SyncEngine,
    /// This daemon's device id.
    pub device_id: String,
    /// Ring buffer of recently served requests.
    pub request_log: RequestLog,
    /// Sink for outgoing JSON-RPC notifications.
    pub notifier: Notifier,
    /// When the daemon started.
    pub started_at: time::OffsetDateTime,
    /// Count of HTTP requests served since startup.
    pub requests_served: Arc<AtomicU64>,
    /// Timestamp of the most recent HTTP request.
    pub last_request_at: Arc<std::sync::Mutex<Option<String>>>,
    /// Broadcast registry shared with `livtet-sync-http`'s pairing routes.
    pub pair_waiters: PairWaiters,
    /// The running HTTP server handle, if any.
    pub server: Mutex<Option<ServerHandle>>,
    /// The address the HTTP server is actually bound to.
    pub bound: Mutex<Option<std::net::SocketAddr>>,
}

/// A managed embedded HTTP server with graceful shutdown.
pub struct ServerHandle {
    shutdown_tx: Option<watch::Sender<bool>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

/// Connect + migrate the database, start the HTTP server, and run the stdio
/// RPC loop until the parent sends `shutdown` or closes stdin.
pub async fn run_async(config: ServerConfig) -> Result<(), ServerError> {
    let db_url = config.db_path.to_string();
    let pool = livtet_data::connect_with_migrations(
        &db_url,
        &[livtet_data::Kind::Business, livtet_data::Kind::Client],
    )
    .await
    .map_err(|e| ServerError::Db(e.to_string()))?;

    let db: DatabaseConnection =
        livtet_data::orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(pool.clone());
    let device_id = resolve_device_id(&db, config.device_id.as_deref()).await?;
    let engine = SyncEngine::new(db, device_id.clone());

    let pair_waiters: PairWaiters = Arc::new(Mutex::new(HashMap::new()));
    set_pair_waiters(pair_waiters.clone());

    let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();
    let notifier: Notifier = {
        let tx = out_tx.clone();
        Arc::new(move |method: &str, params: serde_json::Value| {
            let line = rpc::encode_notification(method, params);
            let _ = tx.send(line);
        })
    };

    let state = Arc::new(DaemonState {
        config,
        pool,
        engine,
        device_id,
        request_log: new_request_log(REQUEST_LOG_CAP),
        notifier,
        started_at: time::OffsetDateTime::now_utc(),
        requests_served: Arc::new(AtomicU64::new(0)),
        last_request_at: Arc::new(std::sync::Mutex::new(None)),
        pair_waiters,
        server: Mutex::new(None),
        bound: Mutex::new(None),
    });

    let writer = tokio::spawn(writer_task(out_rx));

    start_server(&state).await?;

    let loop_result = read_loop(&state, &out_tx).await;
    let _ = stop_server(&state).await;

    // The notifier holds a clone of the sender, so closing our handle closes
    // the channel for every clone, letting the writer drain and exit.
    drop(out_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;

    loop_result
}

/// Resolve this daemon's device id: an explicit override, else
/// `client_settings.device_id`, else a freshly generated + persisted ULID.
async fn resolve_device_id(
    db: &DatabaseConnection,
    override_id: Option<&str>,
) -> Result<String, ServerError> {
    use livtet_data::client_entities::client_settings;

    if let Some(id) = override_id
        && !id.trim().is_empty()
    {
        return Ok(id.to_string());
    }

    let existing = client_settings::Entity::find_by_id("device_id".to_string())
        .one(db)
        .await
        .map_err(|e| ServerError::Db(e.to_string()))?;
    if let Some(row) = existing {
        return Ok(row.value);
    }

    let generated = ulid::Ulid::generate().to_string();
    let model = client_settings::ActiveModel {
        key: Set("device_id".to_string()),
        value: Set(generated.clone()),
        updated_at: Set(primitive_now()),
    };
    model
        .insert(db)
        .await
        .map_err(|e| ServerError::Db(e.to_string()))?;
    Ok(generated)
}

/// Bind the embedded sync HTTP server (port `0` → ephemeral) and spawn it.
pub async fn start_server(state: &Arc<DaemonState>) -> Result<(), ServerError> {
    if state.server.lock().await.is_some() {
        return Err(ServerError::Server("server already running".to_string()));
    }

    let bind_addr = format!("{}:{}", state.config.host, state.config.port);
    let listener = tokio::net::TcpListener::bind(bind_addr.as_str())
        .await
        .map_err(|e| ServerError::Server(format!("failed to bind {bind_addr}: {e}")))?;
    let local = listener
        .local_addr()
        .map_err(|e| ServerError::Server(format!("failed to read local addr: {e}")))?;
    let acceptor = poem::listener::TcpAcceptor::from_tokio(listener)
        .map_err(|e| ServerError::Server(format!("failed to build acceptor: {e}")))?;

    let engine = Arc::new(RwLock::new(state.engine.clone()));
    let pair_waiters = state.pair_waiters.clone();
    let log = state.request_log.clone();
    let notifier = state.notifier.clone();
    let served = state.requests_served.clone();
    let last = state.last_request_at.clone();

    let app = make_sync_routes(engine, pair_waiters)
        .around(move |endpoint, req| {
            let log = log.clone();
            let notifier = notifier.clone();
            let served = served.clone();
            let last = last.clone();
            async move {
                let http_method = req.method().to_string();
                let path = req.uri().path().to_string();
                let device_id = req
                    .headers()
                    .get("x-device-id")
                    .and_then(|value| value.to_str().ok())
                    .map(|value| value.to_string());

                let response = endpoint.get_response(req).await;
                let status = response.status().as_u16();
                let at = now_string();

                let is_pair_request = http_method == "POST" && path == PAIR_PATH;
                let is_sync_data =
                    path == PUSH_PATH || path == CHANGES_PATH || path == PULL_FULL_PATH;

                requests::record(
                    &log,
                    REQUEST_LOG_CAP,
                    RequestRecord {
                        method: http_method.clone(),
                        path: path.clone(),
                        status,
                        at: at.clone(),
                        device_id,
                    },
                );
                let _ = served.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut guard) = last.lock() {
                    *guard = Some(at.clone());
                }

                requests::notify(
                    &notifier,
                    rpc::method::NOTIFY_REQUEST_RECEIVED,
                    serde_json::json!({
                        "method": http_method.clone(),
                        "path": path.clone(),
                        "status": status,
                        "at": at,
                    }),
                );
                if is_pair_request {
                    requests::notify(
                        &notifier,
                        rpc::method::NOTIFY_PAIRING_REQUESTED,
                        serde_json::json!({ "path": path }),
                    );
                } else if is_sync_data {
                    requests::notify(
                        &notifier,
                        rpc::method::NOTIFY_SYNC_COMPLETED,
                        serde_json::json!({ "path": path, "status": status }),
                    );
                }

                Ok(response)
            }
        })
        .boxed();

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let join = tokio::spawn(async move {
        let signal = async move {
            while shutdown_rx.changed().await.is_ok() {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        };
        if let Err(error) = Server::new_with_acceptor(acceptor)
            .name("livtet-sync-server")
            .run_with_graceful_shutdown(app, signal, Some(Duration::from_secs(5)))
            .await
        {
            tracing::error!(error = %error, "embedded sync http server error");
        }
    });

    *state.server.lock().await = Some(ServerHandle {
        shutdown_tx: Some(shutdown_tx),
        join: Some(join),
    });
    *state.bound.lock().await = Some(local);

    requests::notify(
        &state.notifier,
        rpc::method::NOTIFY_SERVER_STARTED,
        serde_json::json!({ "host": state.config.host, "port": local.port() }),
    );
    tracing::info!(addr = %local, "embedded sync http server started");
    Ok(())
}

/// Signal the embedded HTTP server to stop and await its task.
pub async fn stop_server(state: &Arc<DaemonState>) -> Result<(), ServerError> {
    let handle = state.server.lock().await.take();
    if let Some(mut handle) = handle {
        if let Some(tx) = handle.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        if let Some(join) = handle.join.take() {
            let _ = join.await;
        }
        *state.bound.lock().await = None;
        requests::notify(
            &state.notifier,
            rpc::method::NOTIFY_SERVER_STOPPED,
            serde_json::json!({}),
        );
        tracing::info!("embedded sync http server stopped");
    }
    Ok(())
}

/// Drain encoded NDJSON lines to stdout.
async fn writer_task(mut rx: mpsc::UnboundedReceiver<String>) {
    let stdout = tokio::io::stdout();
    let mut writer = tokio::io::BufWriter::new(stdout);
    while let Some(line) = rx.recv().await {
        if writer.write_all(line.as_bytes()).await.is_err() {
            break;
        }
        if writer.write_all(b"\n").await.is_err() {
            break;
        }
        if writer.flush().await.is_err() {
            break;
        }
    }
    let _ = writer.flush().await;
}

/// Read NDJSON requests from stdin, dispatch them, and queue the responses.
async fn read_loop(
    state: &Arc<DaemonState>,
    out_tx: &mpsc::UnboundedSender<String>,
) -> Result<(), ServerError> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match rpc::parse_message(trimmed) {
            Ok(rpc::RpcMessage::Request(request)) => {
                let is_shutdown = request.method == rpc::method::SHUTDOWN;
                let response = dispatch(request, state, out_tx).await;
                let _ = out_tx.send(rpc::encode_response(&response));
                if is_shutdown {
                    break;
                }
            }
            // Inbound notifications/responses from the parent are ignored.
            Ok(_) => {}
            Err(error) => {
                let response =
                    rpc::RpcResponse::error(None, rpc::error_code::PARSE_ERROR, error.to_string());
                let _ = out_tx.send(rpc::encode_response(&response));
            }
        }
    }

    Ok(())
}

/// A dispatch failure carrying the JSON-RPC error code to report.
struct RpcFailure {
    code: i64,
    message: String,
}

impl From<ServerError> for RpcFailure {
    fn from(error: ServerError) -> Self {
        Self {
            code: rpc::error_code::INTERNAL_ERROR,
            message: error.to_string(),
        }
    }
}

impl From<livtet_sync::SyncError> for RpcFailure {
    fn from(error: livtet_sync::SyncError) -> Self {
        Self {
            code: rpc::error_code::INTERNAL_ERROR,
            message: error.to_string(),
        }
    }
}

impl From<livtet_data::orm::DbErr> for RpcFailure {
    fn from(error: livtet_data::orm::DbErr) -> Self {
        Self {
            code: rpc::error_code::INTERNAL_ERROR,
            message: error.to_string(),
        }
    }
}

impl From<serde_json::Error> for RpcFailure {
    fn from(error: serde_json::Error) -> Self {
        Self {
            code: rpc::error_code::INTERNAL_ERROR,
            message: error.to_string(),
        }
    }
}

fn invalid_params(message: impl Into<String>) -> RpcFailure {
    RpcFailure {
        code: rpc::error_code::INVALID_PARAMS,
        message: message.into(),
    }
}

/// Parse request params into `T`, treating an absent/null `params` as `{}`.
fn parse_params<T: serde::de::DeserializeOwned>(
    request: &rpc::RpcRequest,
) -> Result<T, RpcFailure> {
    let value = request.params.clone().unwrap_or(serde_json::Value::Null);
    let value = if value.is_null() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        value
    };
    serde_json::from_value(value).map_err(|e| invalid_params(format!("invalid params: {e}")))
}

fn finish(
    request: &rpc::RpcRequest,
    outcome: Result<serde_json::Value, RpcFailure>,
) -> rpc::RpcResponse {
    match outcome {
        Ok(value) => rpc::RpcResponse::ok(request.id.clone(), value),
        Err(failure) => rpc::RpcResponse::error(request.id.clone(), failure.code, failure.message),
    }
}

/// Route a JSON-RPC request to its handler.
async fn dispatch(
    request: rpc::RpcRequest,
    state: &Arc<DaemonState>,
    _out_tx: &mpsc::UnboundedSender<String>,
) -> rpc::RpcResponse {
    match request.method.as_str() {
        rpc::method::HEALTH => finish(&request, handle_health(state).await),
        rpc::method::STATUS => finish(&request, handle_status(state).await),
        rpc::method::REQUESTS_RECENT => {
            finish(&request, handle_requests_recent(&request, state).await)
        }
        rpc::method::PAIRING_BEGIN => finish(&request, handle_pairing_begin(&request, state).await),
        rpc::method::PAIRING_LIST => finish(&request, handle_pairing_list(state).await),
        rpc::method::PAIRING_APPROVE => {
            finish(&request, handle_pairing_approve(&request, state).await)
        }
        rpc::method::PAIRING_REJECT => {
            finish(&request, handle_pairing_reject(&request, state).await)
        }
        rpc::method::DEVICES_LIST => finish(&request, handle_devices_list(state).await),
        rpc::method::DEVICES_REVOKE => {
            finish(&request, handle_devices_revoke(&request, state).await)
        }
        rpc::method::CONFLICTS_LIST => finish(&request, handle_conflicts_list(state).await),
        rpc::method::CONFLICTS_RESOLVE => {
            finish(&request, handle_conflicts_resolve(&request, state).await)
        }
        rpc::method::SERVER_START => finish(&request, handle_server_start(state).await),
        rpc::method::SERVER_STOP => finish(&request, handle_server_stop(state).await),
        rpc::method::SHUTDOWN => finish(&request, handle_shutdown(state).await),
        other => rpc::RpcResponse::error(
            request.id.clone(),
            rpc::error_code::METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        ),
    }
}

async fn handle_health(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    let running = state.server.lock().await.is_some();
    let uptime = (time::OffsetDateTime::now_utc() - state.started_at).whole_seconds();
    Ok(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "uptime_secs": uptime,
        "server_running": running,
        "port": current_port(state).await,
    }))
}

async fn handle_status(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::{paired_devices, pending_pairings};

    let db = state.engine.db();
    let latest_version = state.engine.get_latest_version().await?;
    let paired_device_count = paired_devices::Entity::find().count(db).await?;

    // Only count pairings a remote device has actually claimed; the local
    // desktop's own unclaimed tickets are not "pending pairings".
    let pending_pairing_count = pending_pairings::Entity::find()
        .filter(pending_pairings::Column::StatusId.eq(DbId::from(PairingStatus::Pending)))
        .filter(pending_pairings::Column::ExpiresAt.gt(primitive_now()))
        .filter(pending_pairings::Column::DeviceTypeId.is_not_null())
        .count(db)
        .await?;

    let running = state.server.lock().await.is_some();
    let last_request_at = state
        .last_request_at
        .lock()
        .ok()
        .and_then(|guard| guard.clone());

    Ok(serde_json::json!({
        "device_id": state.device_id,
        "server_running": running,
        "host": state.config.host,
        "port": current_port(state).await,
        "latest_version": latest_version,
        "paired_device_count": paired_device_count,
        "pending_pairing_count": pending_pairing_count,
        "requests_served": state.requests_served.load(Ordering::Relaxed),
        "last_request_at": last_request_at,
    }))
}

async fn handle_requests_recent(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let params: RecentParams = parse_params(request)?;
    let limit = params.limit.unwrap_or(50).min(REQUEST_LOG_CAP);
    let records = requests::recent(&state.request_log, limit);
    Ok(serde_json::to_value(records)?)
}

async fn handle_pairing_begin(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::pending_pairings;

    let params: PairingBeginParams = parse_params(request)?;
    let ttl = params.ttl_secs.unwrap_or(300).min(i64::MAX as u64) as i64;
    let now = time::OffsetDateTime::now_utc();
    let expires = now + time::Duration::seconds(ttl);

    let token = ulid::Ulid::generate().to_string();
    let desktop_id = state
        .device_id
        .parse::<DbId>()
        .map_err(|_| invalid_params(format!("device id is not a ULID: {}", state.device_id)))?;

    let (host, port) = current_host_port(state).await;
    let listen_on = format!("{host}:{port}")
        .parse::<Address>()
        .map_err(|e| RpcFailure {
            code: rpc::error_code::INTERNAL_ERROR,
            message: format!("invalid listen address: {e}"),
        })?;

    let model = pending_pairings::ActiveModel {
        token: Set(token.clone()),
        desktop_id: Set(desktop_id),
        listen_on: Set(Some(listen_on)),
        status_id: Set(Some(DbId::from(PairingStatus::Pending))),
        device_name: Set(None),
        device_type_id: Set(None),
        device_id: Set(None),
        created_at: Set(primitive_now()),
        expires_at: Set(time::PrimitiveDateTime::new(expires.date(), expires.time())),
    };
    model.insert(state.engine.db()).await?;

    let uri = format!("livtet://sync?host={host}&port={port}&token={token}");
    Ok(serde_json::json!({
        "token": token,
        "uri": uri,
        "expires_at": format_offset(expires),
    }))
}

async fn handle_pairing_list(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::pending_pairings;

    // A ticket only becomes a pairable device once a remote claims it, which
    // stamps its `device_type_id`. Without this filter the desktop lists the
    // tickets it minted itself as unknown devices.
    let rows = pending_pairings::Entity::find()
        .filter(pending_pairings::Column::StatusId.eq(DbId::from(PairingStatus::Pending)))
        .filter(pending_pairings::Column::ExpiresAt.gt(primitive_now()))
        .filter(pending_pairings::Column::DeviceTypeId.is_not_null())
        .all(state.engine.db())
        .await?;

    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "token": row.token,
                "desktop_id": row.desktop_id.to_string(),
                "listen_on": row.listen_on.as_ref().map(|addr| addr.to_string()),
                "status_id": row.status_id.map(|id| id.to_string()),
                "device_name": row.device_name,
                "device_type_id": row.device_type_id.map(|id| id.to_string()),
                "device_id": row.device_id.map(|id| id.to_string()),
                "created_at": format_primitive(row.created_at),
                "expires_at": format_primitive(row.expires_at),
            })
        })
        .collect();

    Ok(serde_json::Value::Array(list))
}

async fn handle_pairing_approve(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::{paired_devices, pending_pairings};
    use livtet_data::orm::sea_query::OnConflict;

    let params: TokenParams = parse_params(request)?;
    let db = state.engine.db();

    let pending = pending_pairings::Entity::find_by_id(params.token.clone())
        .one(db)
        .await?
        .ok_or_else(|| invalid_params(format!("unknown pairing token: {}", params.token)))?;

    // Fail closed: only a ticket a remote device actually claimed (which stamps
    // its `device_type_id`) is a pairing. Approving an unclaimed ticket would
    // mint a phantom device for the desktop itself.
    if pending.device_type_id.is_none() {
        return Err(invalid_params(format!(
            "pairing token has not been claimed by a remote device: {}",
            params.token
        )));
    }

    let session_token = ulid::Ulid::generate().to_string();
    let device_id = pending.device_id.unwrap_or_else(DbId::new);

    let model = paired_devices::ActiveModel {
        device_id: Set(device_id),
        name: Set(pending.device_name.clone()),
        listen_on: Set(pending.listen_on.clone()),
        device_type_id: Set(pending.device_type_id),
        paired_at: Set(primitive_now()),
        last_sync_at: Set(None),
    };
    paired_devices::Entity::insert(model)
        .on_conflict(
            OnConflict::column(paired_devices::Column::DeviceId)
                .update_columns([
                    paired_devices::Column::Name,
                    paired_devices::Column::ListenOn,
                    paired_devices::Column::DeviceTypeId,
                    paired_devices::Column::PairedAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;

    // `session_token` is added by a later client migration and is not on the
    // entity, so persist it with a targeted statement through the pool.
    livtet_data::sql::query("UPDATE paired_devices SET session_token = ? WHERE device_id = ?")
        .bind(session_token.clone())
        .bind(device_id.to_bytes().to_vec())
        .execute(&state.pool)
        .await
        .map_err(|e| ServerError::Db(e.to_string()))?;

    let mut pending_model: pending_pairings::ActiveModel = pending.into();
    pending_model.status_id = Set(Some(DbId::from(PairingStatus::Approved)));
    pending_model.update(db).await?;

    let _ =
        livtet_sync_http::apply_pairing_decision(&params.token, "approved", &session_token).await;

    Ok(serde_json::json!({
        "device_id": device_id.to_string(),
        "session_token": session_token,
    }))
}

async fn handle_pairing_reject(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::pending_pairings;

    let params: TokenParams = parse_params(request)?;
    let db = state.engine.db();

    let pending = pending_pairings::Entity::find_by_id(params.token.clone())
        .one(db)
        .await?
        .ok_or_else(|| invalid_params(format!("unknown pairing token: {}", params.token)))?;

    let mut model: pending_pairings::ActiveModel = pending.into();
    model.status_id = Set(Some(DbId::from(PairingStatus::Rejected)));
    model.update(db).await?;

    let _ = livtet_sync_http::apply_pairing_decision(&params.token, "rejected", "").await;

    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_devices_list(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::paired_devices;

    let rows = paired_devices::Entity::find()
        .all(state.engine.db())
        .await?;
    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "device_id": row.device_id.to_string(),
                "name": row.name,
                "listen_on": row.listen_on.as_ref().map(|addr| addr.to_string()),
                "device_type_id": row.device_type_id.map(|id| id.to_string()),
                "paired_at": format_primitive(row.paired_at),
                "last_sync_at": row.last_sync_at.map(format_primitive),
            })
        })
        .collect();

    Ok(serde_json::Value::Array(list))
}

async fn handle_devices_revoke(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::paired_devices;

    let params: DeviceParams = parse_params(request)?;
    let device_id = params
        .device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(&params.device_id).ok())
        .ok_or_else(|| invalid_params(format!("invalid device id: {}", params.device_id)))?;

    let result = paired_devices::Entity::delete_by_id(device_id)
        .exec(state.engine.db())
        .await?;

    Ok(serde_json::json!({ "ok": true, "removed": result.rows_affected }))
}

async fn handle_conflicts_list(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    let conflicts = state.engine.list_conflicts().await?;
    Ok(serde_json::to_value(conflicts)?)
}

async fn handle_conflicts_resolve(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let params: ResolveParams = parse_params(request)?;
    if !matches!(params.resolution.as_str(), "local" | "remote" | "merged") {
        return Err(invalid_params(format!(
            "invalid resolution: {}",
            params.resolution
        )));
    }

    let resolved = state
        .engine
        .resolve_conflict(
            params.id,
            &params.resolution,
            params.merged_payload.as_deref(),
        )
        .await?;

    Ok(serde_json::json!({ "ok": resolved }))
}

async fn handle_server_start(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    if state.server.lock().await.is_some() {
        return Ok(serde_json::json!({ "ok": true, "already_running": true }));
    }
    start_server(state).await?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_server_stop(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    stop_server(state).await?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_shutdown(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    stop_server(state).await?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn current_port(state: &Arc<DaemonState>) -> u16 {
    match *state.bound.lock().await {
        Some(addr) => addr.port(),
        None => state.config.port,
    }
}

async fn current_host_port(state: &Arc<DaemonState>) -> (String, u16) {
    match *state.bound.lock().await {
        Some(addr) => (addr.ip().to_string(), addr.port()),
        None => (state.config.host.clone(), state.config.port),
    }
}

fn primitive_now() -> time::PrimitiveDateTime {
    let now = time::OffsetDateTime::now_utc();
    time::PrimitiveDateTime::new(now.date(), now.time())
}

fn now_string() -> String {
    format_offset(time::OffsetDateTime::now_utc())
}

// `format_description!` requires the `time/macros` feature, which the
// workspace doesn't enable, so parse the description at runtime instead.
fn format_offset(dt: time::OffsetDateTime) -> String {
    time::format_description::parse_borrowed::<2>("[year]-[month]-[day] [hour]:[minute]:[second]")
        .ok()
        .and_then(|fmt| dt.format(&fmt).ok())
        .unwrap_or_default()
}

fn format_primitive(dt: time::PrimitiveDateTime) -> String {
    time::format_description::parse_borrowed::<2>("[year]-[month]-[day] [hour]:[minute]:[second]")
        .ok()
        .and_then(|fmt| dt.format(&fmt).ok())
        .unwrap_or_default()
}
