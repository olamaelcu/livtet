//! The sync server daemon: SQLite + embedded sync HTTP server + stdio RPC loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use livtet_data::orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};
use livtet_iroh::{DiscoveryService, NodeId, PeerIdentity, SecretKey};
use livtet_sync::SyncEngine;
use livtet_sync_http::routes::{CHANGES_PATH, PAIR_PATH, PULL_FULL_PATH, PUSH_PATH, SyncIdentity};
use livtet_sync_http::{PairWaiters, make_sync_routes, set_pair_waiters};
use livtet_types::{DbId, PairingStatus};
use poem::{Endpoint, EndpointExt, Server};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, RwLock, mpsc, watch};

use crate::config::ServerConfig;
use crate::error::ServerError;
use crate::mdns::{Advertiser, NearbyPeer, browse_nearby};
use crate::requests::{
    self, Notifier, REQUEST_LOG_CAP, RequestLog, RequestRecord, new_request_log,
};
use crate::rpc::{
    self, DeviceNameParams, DeviceParams, OfferBookParams, PairingBeginParams, PairingClaimParams,
    RecentParams, ResolveParams, TokenParams,
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
    /// Name advertised to nearby peers; `None` falls back to one generated
    /// from the device id. Mutation re-registers the mDNS record.
    pub device_name: Mutex<Option<String>>,
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
    /// The `iroh` discovery endpoint, bound at startup when the network allows.
    pub discovery: Mutex<Option<Arc<DiscoveryService>>>,
    /// The mDNS advertisement of the HTTP pairing surface, while the server runs.
    pub advertiser: Mutex<Option<Advertiser>>,
    /// The running HTTP server handle, if any.
    pub server: Mutex<Option<ServerHandle>>,
    /// The address the HTTP server is actually bound to.
    pub bound: Mutex<Option<std::net::SocketAddr>>,
}

/// File beside the database holding the iroh discovery secret.
///
/// It is deliberately not a `client_settings` row: that table syncs to paired
/// peers, and the secret must never leave this device.
const DISCOVERY_KEY_FILE: &str = "discovery.key";

/// A managed embedded HTTP server with graceful shutdown.
pub struct ServerHandle {
    shutdown_tx: Option<watch::Sender<bool>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

/// Connect + migrate the database, start the HTTP server, and run the stdio
/// RPC loop until the parent sends `shutdown` or closes stdin.
pub async fn run_async(config: ServerConfig) -> Result<(), ServerError> {
    let db_url = config.db_path.to_string();
    let pool = livtet_data::connect_with_migrations_foreign_keys(
        &db_url,
        &[livtet_data::Kind::Business, livtet_data::Kind::Client],
        false,
    )
    .await
    .map_err(|e| ServerError::Db(e.to_string()))?;

    let db: DatabaseConnection =
        livtet_data::orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(pool.clone());
    let device_id = resolve_device_id(&db, config.device_id.as_deref()).await?;
    let device_name = config.device_name.clone();
    let discovery_secret = discovery_secret_key(&config)?;
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

    // Bind discovery once, up front: the node id is needed both for the
    // identity route and for the mDNS TXT record. Losing the network is not
    // fatal — the daemon falls back to the device id and pairing by address.
    let discovery = match DiscoveryService::bind_with_secret(discovery_secret).await {
        Ok(service) => Some(Arc::new(service)),
        Err(error) => {
            tracing::warn!(%error, "peer discovery unavailable; using the device id");
            None
        }
    };

    let state = Arc::new(DaemonState {
        config,
        pool,
        engine,
        device_id,
        device_name: Mutex::new(device_name),
        request_log: new_request_log(REQUEST_LOG_CAP),
        notifier,
        started_at: time::OffsetDateTime::now_utc(),
        requests_served: Arc::new(AtomicU64::new(0)),
        last_request_at: Arc::new(std::sync::Mutex::new(None)),
        pair_waiters,
        discovery: Mutex::new(discovery),
        advertiser: Mutex::new(None),
        server: Mutex::new(None),
        bound: Mutex::new(None),
    });

    let writer = tokio::spawn(writer_task(out_rx));

    start_server(&state).await?;
    advertise(&state).await;
    spawn_pull_loop(Arc::clone(&state));
    let _ = cleanup_old_pairings(&state).await;

    let loop_result = read_loop(&state, &out_tx).await;
    let _ = stop_server(&state).await;
    shutdown_discovery(&state).await;

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

    let identity = SyncIdentity {
        node_id: discovery_node_id(state).await,
        device_id: state.device_id.clone(),
    };

    let app = make_sync_routes(engine, pair_waiters, identity)
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
    stop_advertiser(state).await;
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
        rpc::method::DISCOVERY_NEARBY => finish(&request, handle_discovery_nearby(state).await),
        rpc::method::DISCOVERY_RENAME => {
            finish(&request, handle_discovery_rename(&request, state).await)
        }
        rpc::method::PAIRING_APPROVE => {
            finish(&request, handle_pairing_approve(&request, state).await)
        }
        rpc::method::PAIRING_REJECT => {
            finish(&request, handle_pairing_reject(&request, state).await)
        }
        rpc::method::PAIRING_CLAIM => finish(&request, handle_pairing_claim(&request, state).await),
        rpc::method::DEVICES_LIST => finish(&request, handle_devices_list(state).await),
        rpc::method::DEVICES_OFFER_BOOK => {
            finish(&request, handle_devices_offer_book(&request, state).await)
        }
        rpc::method::DEVICES_PULL => finish(&request, handle_devices_pull(&request, state).await),
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

    // A pending pairing counts only when it arrived from another host. Tickets
    // this daemon minted carry no origin, and a request from the daemon's own
    // address is itself, so both are excluded by the `!=` comparison.
    let self_addr = self_address(state).await;
    let pending_pairing_count = pending_pairings::Entity::find()
        .filter(pending_pairings::Column::StatusId.eq(DbId::from(PairingStatus::Pending)))
        .filter(pending_pairings::Column::ExpiresAt.gt(primitive_now()))
        .filter(pending_pairings::Column::OriginAddr.ne(self_addr))
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
    let params: PairingBeginParams = parse_params(request)?;
    let ttl = params.ttl_secs.unwrap_or(300).min(i64::MAX as u64) as i64;
    let now = time::OffsetDateTime::now_utc();
    let expires = now + time::Duration::seconds(ttl);

    // The ticket is this device's own node identity: discovery advertisements
    // carry it, and remote peers send it back on `/sync/pair`. No token is
    // minted and no `pending_pairings` row is created here — rows only exist
    // for requests that actually arrived from another device.
    let device_type = params.device_type.unwrap_or_else(|| "unknown".to_string());
    let identity = match discovery_identity(state, &device_type).await {
        Some(identity) => identity,
        // Without a bound endpoint there is nothing for peers to discover;
        // pairing still works for callers that were given the address and
        // device id directly, so publish the device id rather than failing.
        None => PeerIdentity {
            node_id: NodeId(state.device_id.clone()),
            name: state.device_id.clone(),
            device_type,
        },
    };

    let (host, port) = current_host_port(state).await;

    Ok(serde_json::json!({
        "node_id": identity.node_id.0,
        "identity": {
            "node_id": identity.node_id.0,
            "name": identity.name,
            "device_type": identity.device_type,
        },
        "listen_on": format!("{host}:{port}"),
        "expires_at": format_offset(expires),
    }))
}

async fn handle_pairing_list(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::pending_pairings;

    // A ticket is a pairable device only when the request came from another
    // host. Tickets this daemon minted have no origin, so they never appear;
    // the `!=` also drops anything sent from the daemon's own address.
    let self_addr = self_address(state).await;
    let rows = pending_pairings::Entity::find()
        .filter(pending_pairings::Column::StatusId.eq(DbId::from(PairingStatus::Pending)))
        .filter(pending_pairings::Column::ExpiresAt.gt(primitive_now()))
        .filter(pending_pairings::Column::OriginAddr.ne(self_addr))
        .all(state.engine.db())
        .await?;

    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "node_id": row.token,
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

async fn handle_discovery_nearby(
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let own_node_id = discovery_node_id(state).await;
    let own_device_id = state.device_id.clone();

    // `mdns-sd`'s receiver is blocking, so the window runs off the async
    // runtime. The daemon is stopped inside the browse before it returns.
    let peers = tokio::task::spawn_blocking(browse_nearby)
        .await
        .map_err(|error| RpcFailure {
            code: rpc::error_code::INTERNAL_ERROR,
            message: format!("mDNS browse task failed: {error}"),
        })??;

    // Our own advertisement can surface through the multicast socket. Exclude
    // by both identities: the node id is iroh's endpoint id once discovery is
    // bound, while the device id is the ULID fallback.
    let nearby: Vec<NearbyPeer> = peers
        .into_iter()
        .filter(|peer| peer.node_id != own_node_id && peer.device_id != own_device_id)
        .collect();

    Ok(serde_json::to_value(nearby)?)
}

/// Change the name peers see, re-registering the mDNS record when serving.
///
/// The value is normalized here (trimmed, blank → `None`) so the CLI flag,
/// the settings field, and this RPC all mean the same thing: `None` restores
/// the generated fallback.
async fn handle_discovery_rename(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let params: DeviceNameParams = parse_params(request)?;
    let name = params
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    *state.device_name.lock().await = name.clone();

    let serving = state.server.lock().await.is_some();
    if serving {
        stop_advertiser(state).await;
        advertise(state).await;
    }

    Ok(serde_json::json!({ "name": name }))
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
        .ok_or_else(|| invalid_params(format!("unknown pairing node id: {}", params.token)))?;

    // Fail closed: only a request that originated elsewhere is a pairing.
    // `pending.token` holds the remote node identity; a request carrying our
    // own identity, or one recorded without an origin, is not a peer.
    let self_addr = self_address(state).await;
    let pending_node_id = pending.token.clone();
    if pending_node_id == state.device_id
        || pending
            .origin_addr
            .as_ref()
            .is_none_or(|addr| addr.to_string() == self_addr)
    {
        return Err(invalid_params(format!(
            "pairing node id did not originate from a remote device: {pending_node_id}"
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
        outbound: Set(false),
    };
    paired_devices::Entity::insert(model)
        .on_conflict(
            OnConflict::column(paired_devices::Column::DeviceId)
                .update_columns([
                    paired_devices::Column::Name,
                    paired_devices::Column::ListenOn,
                    paired_devices::Column::DeviceTypeId,
                    paired_devices::Column::PairedAt,
                    paired_devices::Column::Outbound,
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

    let _ = livtet_sync_http::apply_pairing_decision(&pending_node_id, "approved", &session_token)
        .await;

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
        .ok_or_else(|| invalid_params(format!("unknown pairing node id: {}", params.token)))?;

    // Fail closed: `pending.token` holds the remote node identity. Rejecting a
    // request that carries our own identity, or one with no recorded origin,
    // would be acting on something this device did not receive from a peer.
    let self_addr = self_address(state).await;
    let pending_node_id = pending.token.clone();
    if pending_node_id == state.device_id
        || pending
            .origin_addr
            .as_ref()
            .is_none_or(|addr| addr.to_string() == self_addr)
    {
        return Err(invalid_params(format!(
            "pairing node id did not originate from a remote device: {pending_node_id}"
        )));
    }

    let mut model: pending_pairings::ActiveModel = pending.into();
    model.status_id = Set(Some(DbId::from(PairingStatus::Rejected)));
    model.update(db).await?;

    let _ = livtet_sync_http::apply_pairing_decision(&pending_node_id, "rejected", "").await;

    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_pairing_claim(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let params: PairingClaimParams = parse_params(request)?;
    let state = Arc::clone(state);
    // The peer's approval is human-paced, so the RPC returns immediately and
    // the result arrives as a `pairing.claimed` notification.
    tokio::spawn(async move {
        if let Err(error) = claim_peer(&state, params).await {
            tracing::warn!(%error, "pairing claim failed");
        }
    });
    Ok(serde_json::json!({ "status": "pending" }))
}

/// Claim a discovered peer, follow its decision stream, and store the token it
/// issues us as an outbound paired device.
async fn claim_peer(
    state: &Arc<DaemonState>,
    params: PairingClaimParams,
) -> Result<(), ServerError> {
    use futures_util::StreamExt;
    use livtet_data::client_entities::paired_devices;
    use livtet_data::orm::{EntityTrait, Set, sea_query::OnConflict};
    use livtet_types::Address;

    let base = format!("http://{}", params.listen_on.trim_end_matches('/'));
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(310))
        .build()
        .map_err(|error| ServerError::Server(format!("pairing client: {error}")))?;

    let identity: serde_json::Value = http
        .get(format!("{base}/sync/identity"))
        .send()
        .await
        .map_err(|error| ServerError::Server(format!("peer identity: {error}")))?
        .json()
        .await
        .map_err(|error| ServerError::Server(format!("peer identity body: {error}")))?;
    let peer_device_id = identity
        .get("device_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ServerError::Server("peer identity carried no device id".to_string()))?;
    let peer_device_id = peer_device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(peer_device_id).ok())
        .ok_or_else(|| ServerError::Server(format!("invalid peer device id: {peer_device_id}")))?;

    let our_node = discovery_node_id(state).await;
    let our_name = state
        .device_name
        .lock()
        .await
        .clone()
        .unwrap_or_else(|| crate::mdns::friendly_name(&state.device_id));
    let claim: serde_json::Value = http
        .post(format!("{base}/sync/pair"))
        .json(&serde_json::json!({
            "device_id": state.device_id,
            "name": our_name,
            "device_type": "desktop",
            "node_id": our_node,
        }))
        .send()
        .await
        .map_err(|error| ServerError::Server(format!("pairing claim: {error}")))?
        .json()
        .await
        .map_err(|error| ServerError::Server(format!("pairing claim body: {error}")))?;
    let secret = claim
        .get("claim")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ServerError::Server("pairing claim returned no secret".to_string()))?;

    let mut stream = http
        .get(format!("{base}/sync/pair/status/{our_node}?claim={secret}"))
        .send()
        .await
        .map_err(|error| ServerError::Server(format!("pairing status: {error}")))?
        .bytes_stream();
    let mut buffer = String::new();
    let mut decision: Option<serde_json::Value> = None;
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| ServerError::Server(format!("pairing stream: {error}")))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        if let Some(found) = buffer.lines().find_map(|line| {
            line.strip_prefix("data:")
                .and_then(|data| serde_json::from_str::<serde_json::Value>(data.trim()).ok())
        }) {
            decision = Some(found);
            break;
        }
    }
    drop(stream);

    let decision =
        decision.ok_or_else(|| ServerError::Server("pairing stream ended early".to_string()))?;
    if decision.get("event").and_then(serde_json::Value::as_str) != Some("approved") {
        return Err(ServerError::Server("pairing was rejected".to_string()));
    }
    let session_token = decision
        .get("session_token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ServerError::Server("approval carried no session token".to_string()))?;

    let listen_on: std::net::SocketAddr = params.listen_on.parse().map_err(|error| {
        ServerError::Server(format!(
            "invalid peer address {}: {error}",
            params.listen_on
        ))
    })?;
    let db = state.engine.db();
    let model = paired_devices::ActiveModel {
        device_id: Set(peer_device_id),
        name: Set(params.device_name.clone()),
        listen_on: Set(Some(Address::new(listen_on))),
        device_type_id: Set(None),
        paired_at: Set(primitive_now()),
        last_sync_at: Set(None),
        outbound: Set(true),
    };
    paired_devices::Entity::insert(model)
        .on_conflict(
            OnConflict::column(paired_devices::Column::DeviceId)
                .update_columns([
                    paired_devices::Column::Name,
                    paired_devices::Column::ListenOn,
                    paired_devices::Column::PairedAt,
                    paired_devices::Column::Outbound,
                ])
                .to_owned(),
        )
        .exec(db)
        .await
        .map_err(|error| ServerError::Server(format!("storing paired device: {error}")))?;

    // `session_token` is not on the entity; write it through the pool.
    livtet_data::sql::query("UPDATE paired_devices SET session_token = ? WHERE device_id = ?")
        .bind(session_token.to_string())
        .bind(peer_device_id.to_bytes().to_vec())
        .execute(&state.pool)
        .await
        .map_err(|error| ServerError::Server(format!("storing session token: {error}")))?;

    requests::notify(
        &state.notifier,
        rpc::method::NOTIFY_PAIRING_CLAIMED,
        serde_json::json!({
            "device_id": peer_device_id.to_string(),
            "name": params.device_name,
            "listen_on": params.listen_on,
        }),
    );

    // Pull once so an offer already waiting on the peer arrives promptly.
    if let Err(error) = pull_device(state, peer_device_id, true).await {
        tracing::warn!(%error, "initial pull after pairing failed");
    }
    Ok(())
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

async fn handle_devices_offer_book(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::paired_devices;
    use livtet_data::entities::{digital_inventory, editions};

    let params: OfferBookParams = parse_params(request)?;
    let device_id = params
        .device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(&params.device_id).ok())
        .ok_or_else(|| invalid_params(format!("invalid device id: {}", params.device_id)))?;
    let edition_id = params
        .edition_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(&params.edition_id).ok())
        .ok_or_else(|| invalid_params(format!("invalid edition id: {}", params.edition_id)))?;

    let db = state.engine.db();
    if paired_devices::Entity::find_by_id(device_id)
        .one(db)
        .await?
        .is_none()
    {
        return Err(invalid_params(format!(
            "device is not paired: {}",
            params.device_id
        )));
    }

    let edition = editions::Entity::find_by_id(edition_id)
        .one(db)
        .await?
        .ok_or_else(|| invalid_params(format!("unknown edition: {}", params.edition_id)))?;
    let inventory = digital_inventory::Entity::find()
        .filter(digital_inventory::Column::EditionId.eq(edition_id))
        .one(db)
        .await?
        .ok_or_else(|| invalid_params("this edition has no file to send"))?;
    let Some(file_path) = inventory.file_path.as_deref() else {
        return Err(invalid_params("this edition has no file to send"));
    };
    if !camino::Utf8Path::new(file_path).is_file() {
        return Err(invalid_params("the book file is missing from disk"));
    }

    let offer_id = ulid::Ulid::generate().to_string();
    let inventory_id = inventory.id.to_hex();
    let payload = serde_json::json!({
        "offer_id": offer_id,
        "inventory_id": inventory_id,
        "edition_id": edition_id.to_hex(),
        "work_id": edition.work_id.to_hex(),
        "to_device_id": device_id.to_string(),
        "file_name": camino::Utf8Path::new(file_path)
            .file_name()
            .unwrap_or("book"),
        "file_size_bytes": inventory.file_size_bytes,
        "file_hash": inventory.file_hash,
        "has_cover": inventory
            .cover_path
            .as_deref()
            .is_some_and(|path| camino::Utf8Path::new(path).is_file()),
    })
    .to_string();

    // An offer is a change-log row so it rides the same version cursor the
    // receiver already pulls. `OFFER` is deliberately not an apply operation:
    // the receiver handles the row itself and must not replay it as a table
    // mutation.
    livtet_data::sql::query(
        "INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id) \
         VALUES ('file_offer', ?, 'OFFER', (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log), ?, datetime('now'), ?)",
    )
    .bind(inventory_id.clone())
    .bind(payload)
    .bind(state.device_id.clone())
    .execute(&state.pool)
    .await
    .map_err(|error| RpcFailure {
        code: rpc::error_code::INTERNAL_ERROR,
        message: format!("could not record the file offer: {error}"),
    })?;

    // Touching `updated_at` makes the existing audit triggers emit full
    // UPDATE payloads for the work, edition, and file, so a normal pull also
    // carries the catalog record and cover path.
    livtet_data::sql::query(
        "UPDATE digital_inventory SET updated_at = datetime('now') WHERE id = ?",
    )
    .bind(inventory.id.to_bytes().to_vec())
    .execute(&state.pool)
    .await
    .map_err(offer_touch_error)?;
    livtet_data::sql::query("UPDATE editions SET updated_at = datetime('now') WHERE id = ?")
        .bind(edition_id.to_bytes().to_vec())
        .execute(&state.pool)
        .await
        .map_err(offer_touch_error)?;
    livtet_data::sql::query(
        "UPDATE works SET updated_at = datetime('now') WHERE id = (SELECT work_id FROM editions WHERE id = ?)",
    )
    .bind(edition_id.to_bytes().to_vec())
    .execute(&state.pool)
    .await
    .map_err(offer_touch_error)?;

    Ok(serde_json::json!({
        "offer_id": offer_id,
        "inventory_id": inventory_id,
        "device_id": device_id.to_string(),
    }))
}

fn offer_touch_error(error: impl std::fmt::Display) -> RpcFailure {
    RpcFailure {
        code: rpc::error_code::INTERNAL_ERROR,
        message: format!("could not stage the book metadata: {error}"),
    }
}

async fn handle_devices_pull(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let params: DeviceParams = parse_params(request)?;
    let device_id = params
        .device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(&params.device_id).ok())
        .ok_or_else(|| invalid_params(format!("invalid device id: {}", params.device_id)))?;
    let outcome = pull_device(state, device_id, false).await?;
    Ok(serde_json::to_value(outcome)?)
}

/// One pull pass against an outbound peer.
async fn pull_device(
    state: &Arc<DaemonState>,
    device_id: DbId,
    background: bool,
) -> Result<serde_json::Value, ServerError> {
    use livtet_data::client_entities::paired_devices;
    use livtet_data::orm::{EntityTrait, Set};
    use livtet_sync::{ApplyOptions, InventoryPaths, PullResponse};

    let db = state.engine.db();
    let peer = paired_devices::Entity::find_by_id(device_id)
        .one(db)
        .await
        .map_err(|error| ServerError::Server(format!("paired device lookup: {error}")))?
        .ok_or_else(|| ServerError::Server("device is not paired".to_string()))?;
    if !peer.outbound {
        return Err(ServerError::Server(
            "device was not claimed by this installation".to_string(),
        ));
    }
    let listen_on = peer
        .listen_on
        .as_ref()
        .map(ToString::to_string)
        .ok_or_else(|| ServerError::Server("paired device has no address".to_string()))?;
    let token: String = livtet_data::sql::query_scalar(
        "SELECT session_token FROM paired_devices WHERE device_id = ?",
    )
    .bind(device_id.to_bytes().to_vec())
    .fetch_one(&state.pool)
    .await
    .map_err(|error| ServerError::Server(format!("session token lookup: {error}")))?;

    let books_dir = state
        .config
        .books_dir
        .clone()
        .ok_or_else(|| ServerError::Server("no books directory configured".to_string()))?;
    let covers_dir = state
        .config
        .covers_dir
        .clone()
        .ok_or_else(|| ServerError::Server("no covers directory configured".to_string()))?;
    fs_err::tokio::create_dir_all(books_dir.as_std_path())
        .await
        .map_err(|error| ServerError::Server(format!("books directory: {error}")))?;
    fs_err::tokio::create_dir_all(covers_dir.as_std_path())
        .await
        .map_err(|error| ServerError::Server(format!("covers directory: {error}")))?;

    let base = format!("http://{}", listen_on.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| ServerError::Server(format!("pull client: {error}")))?;
    let cursor_key = format!("sync.cursor.{device_id}");
    let mut cursor = read_setting(state, &cursor_key)
        .await?
        .unwrap_or_else(|| "0".to_string());
    let mut applied = 0;
    let mut offered = 0;
    let mut conflicts = 0;

    loop {
        let response = client
            .get(format!(
                "{base}/sync/changes?since_version={cursor}&limit=100"
            ))
            .header("x-livtet-session", &token)
            .send()
            .await
            .map_err(|error| ServerError::Server(format!("pull changes: {error}")))?;
        if !response.status().is_success() {
            return Err(ServerError::Server(format!(
                "pull changes returned HTTP {}",
                response.status()
            )));
        }
        let pull: PullResponse = response
            .json()
            .await
            .map_err(|error| ServerError::Server(format!("pull changes body: {error}")))?;
        if pull.changes.is_empty() {
            if pull.has_more {
                // A page can be empty when it held only offers for another
                // device; advance past it instead of stalling.
                cursor = pull.latest_version.to_string();
                write_setting(state, &cursor_key, &cursor).await?;
                continue;
            }
            break;
        }

        let mut paths: HashMap<String, InventoryPaths> = HashMap::new();
        for change in pull
            .changes
            .iter()
            .filter(|change| change.entity_type == "file_offer")
        {
            let payload: serde_json::Value =
                serde_json::from_str(&change.payload).unwrap_or_default();
            if payload
                .get("to_device_id")
                .and_then(serde_json::Value::as_str)
                != Some(state.device_id.as_str())
            {
                continue;
            }
            let downloaded =
                download_offer(&client, &base, &token, &payload, &books_dir, &covers_dir).await?;
            paths.insert(downloaded.0, downloaded.1);
            offered += 1;
        }

        let metadata: Vec<_> = pull
            .changes
            .into_iter()
            .filter(|change| change.entity_type != "file_offer")
            .collect();
        if !metadata.is_empty() {
            let outcome = state
                .engine
                .apply_changes(
                    metadata,
                    ApplyOptions {
                        inventory_paths: paths,
                    },
                )
                .await
                .map_err(ServerError::from)?;
            applied += outcome.applied;
            conflicts += outcome.conflicts.len();
        }

        cursor = pull.latest_version.to_string();
        write_setting(state, &cursor_key, &cursor).await?;
        if !pull.has_more {
            break;
        }
    }

    let mut model: paired_devices::ActiveModel = peer.into();
    model.last_sync_at = Set(Some(primitive_now()));
    model
        .update(db)
        .await
        .map_err(|error| ServerError::Server(format!("last sync update: {error}")))?;

    if background {
        tracing::info!(%device_id, applied, offered, "pulled from paired device");
    }
    Ok(serde_json::json!({
        "device_id": device_id.to_string(),
        "applied": applied,
        "offered": offered,
        "conflicts": conflicts,
    }))
}

async fn read_setting(state: &Arc<DaemonState>, key: &str) -> Result<Option<String>, ServerError> {
    let value: Option<String> =
        livtet_data::sql::query_scalar("SELECT value FROM client_settings WHERE key = ?")
            .bind(key.to_string())
            .fetch_optional(&state.pool)
            .await
            .map_err(|error| ServerError::Server(format!("settings read: {error}")))?;
    Ok(value)
}

async fn write_setting(
    state: &Arc<DaemonState>,
    key: &str,
    value: &str,
) -> Result<(), ServerError> {
    use livtet_data::client_entities::client_settings;
    use livtet_data::orm::{Set, sea_query::OnConflict};

    let model = client_settings::ActiveModel {
        key: Set(key.to_string()),
        value: Set(value.to_string()),
        updated_at: Set(primitive_now()),
    };
    client_settings::Entity::insert(model)
        .on_conflict(
            OnConflict::column(client_settings::Column::Key)
                .update_columns([
                    client_settings::Column::Value,
                    client_settings::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(state.engine.db())
        .await
        .map_err(|error| ServerError::Server(format!("settings write: {error}")))?;
    Ok(())
}

/// Where the discovery key lives, beside the SQLite file.
fn discovery_key_path(config: &ServerConfig) -> camino::Utf8PathBuf {
    config
        .db_path
        .parent()
        .unwrap_or_else(|| camino::Utf8Path::new("."))
        .join(DISCOVERY_KEY_FILE)
}

/// Load the discovery secret, creating and persisting one on first run.
///
/// The 32-byte secret determines the iroh node id, so keeping it beside the
/// database makes this device's node id stable across restarts.
fn discovery_secret_key(config: &ServerConfig) -> Result<SecretKey, ServerError> {
    use std::io::{ErrorKind, Write};

    let path = discovery_key_path(config);
    if let Some(parent) = path.parent() {
        fs_err::create_dir_all(parent)
            .map_err(|error| ServerError::Server(format!("discovery key directory: {error}")))?;
    }

    let open = || {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)
    };

    match open() {
        Ok(mut file) => {
            let secret = SecretKey::generate();
            file.write_all(&secret.to_bytes())
                .map_err(|error| ServerError::Server(format!("discovery key write: {error}")))?;
            Ok(secret)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let bytes = fs_err::read(&path).map_err(|error| {
                ServerError::Server(format!("discovery key read: {error}"))
            })?;
            let bytes: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
                ServerError::Server(format!(
                    "discovery key has {} bytes, expected 32",
                    bytes.len()
                ))
            })?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(error) => Err(ServerError::Server(format!(
            "discovery key create: {error}"
        ))),
    }
}

/// Fetch one offered file (and its cover) into the receiver's directories.
async fn download_offer(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    payload: &serde_json::Value,
    books_dir: &camino::Utf8Path,
    covers_dir: &camino::Utf8Path,
) -> Result<(String, livtet_sync::InventoryPaths), ServerError> {
    let inventory_id = payload
        .get("inventory_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ServerError::Server("offer has no inventory id".to_string()))?;
    let file_name = payload
        .get("file_name")
        .and_then(serde_json::Value::as_str)
        .and_then(|name| camino::Utf8Path::new(name).file_name())
        .filter(|name| !name.is_empty())
        .unwrap_or("book");

    let bytes = client
        .get(format!("{base}/sync/files/{inventory_id}"))
        .header("x-livtet-session", token)
        .send()
        .await
        .map_err(|error| ServerError::Server(format!("download file: {error}")))?
        .bytes()
        .await
        .map_err(|error| ServerError::Server(format!("download file body: {error}")))?;
    let file_path = books_dir.join(format!("{inventory_id}-{file_name}"));
    fs_err::tokio::write(file_path.as_std_path(), &bytes)
        .await
        .map_err(|error| ServerError::Server(format!("write received file: {error}")))?;

    let mut cover_path = None;
    if payload
        .get("has_cover")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        let response = client
            .get(format!("{base}/sync/covers/{inventory_id}"))
            .header("x-livtet-session", token)
            .send()
            .await
            .map_err(|error| ServerError::Server(format!("download cover: {error}")))?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("image/png")
            .to_string();
        let extension = if content_type.contains("jpeg") {
            "jpg"
        } else if content_type.contains("webp") {
            "webp"
        } else {
            "png"
        };
        let bytes = response
            .bytes()
            .await
            .map_err(|error| ServerError::Server(format!("download cover body: {error}")))?;
        let path = covers_dir.join(format!("{inventory_id}.{extension}"));
        fs_err::tokio::write(path.as_std_path(), &bytes)
            .await
            .map_err(|error| ServerError::Server(format!("write received cover: {error}")))?;
        cover_path = Some(path.to_string());
    }

    Ok((
        inventory_id.to_string(),
        livtet_sync::InventoryPaths {
            file_path: Some(file_path.to_string()),
            cover_path,
        },
    ))
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
    shutdown_discovery(state).await;
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

/// The daemon's own `host:port`, used to tell a self-originated pairing from a
/// remote one. `pairing.begin` listens on it; `/sync/pair` compares to it.
async fn self_address(state: &Arc<DaemonState>) -> String {
    let (host, port) = current_host_port(state).await;
    format!("{host}:{port}")
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
/// Bind the discovery endpoint on first use and return the identity to
/// advertise. A failed bind is not fatal: the daemon logs it and pairing falls
/// back to explicit addresses.
async fn discovery_identity(state: &Arc<DaemonState>, device_type: &str) -> Option<PeerIdentity> {
    let mut slot = state.discovery.lock().await;
    if slot.is_none() {
        let secret = match discovery_secret_key(&state.config) {
            Ok(secret) => secret,
            Err(error) => {
                tracing::warn!(%error, "discovery secret key unavailable");
                return None;
            }
        };
        match DiscoveryService::bind_with_secret(secret).await {
            Ok(service) => *slot = Some(Arc::new(service)),
            Err(error) => {
                tracing::warn!(%error, "peer discovery unavailable; advertising the device id");
                return None;
            }
        }
    }
    slot.as_ref()
        .map(|service| service.identity(state.device_id.clone(), device_type.to_string()))
}

/// Close the discovery endpoint, if it was ever bound.
async fn shutdown_discovery(state: &Arc<DaemonState>) {
    if let Some(service) = state.discovery.lock().await.take() {
        service.shutdown().await;
    }
}

/// The node id to publish: the bound endpoint's, else the device id.
async fn discovery_node_id(state: &Arc<DaemonState>) -> String {
    state
        .discovery
        .lock()
        .await
        .as_ref()
        .map(|service| service.node_id().0.clone())
        .unwrap_or_else(|| state.device_id.clone())
}

/// Announce the HTTP pairing surface once the server has a bound port.
///
/// The record carries the node id the client will use as the pairing key, so a
/// device that finds us over mDNS can confirm it with `GET /sync/identity`
/// before posting a claim.
async fn advertise(state: &Arc<DaemonState>) {
    let (host, port) = current_host_port(state).await;
    let Ok(ip) = host.parse::<std::net::IpAddr>() else {
        tracing::warn!(%host, "mDNS advertising skipped: unparseable bind address");
        return;
    };
    let node_id = discovery_node_id(state).await;
    let device_name = state.device_name.lock().await.clone();
    match Advertiser::start(&node_id, &state.device_id, device_name.as_deref(), ip, port) {
        Ok(advertiser) => *state.advertiser.lock().await = Some(advertiser),
        Err(error) => tracing::warn!(%error, "mDNS advertising failed"),
    }
}

async fn stop_advertiser(state: &Arc<DaemonState>) {
    if let Some(advertiser) = state.advertiser.lock().await.take() {
        advertiser.shutdown();
    }
}

/// Pull from every outbound paired device once a minute.
///
/// A claimed peer holds the token we use to authenticate, so this side is the
/// only one that can start the transfer.
fn spawn_pull_loop(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let devices = outbound_device_ids(&state).await;
            for device_id in devices {
                if let Err(error) = pull_device(&state, device_id, true).await {
                    tracing::warn!(%device_id, %error, "pull from paired device failed");
                }
            }
        }
    });
}

async fn outbound_device_ids(state: &Arc<DaemonState>) -> Vec<DbId> {
    use livtet_data::client_entities::paired_devices;
    use livtet_data::orm::QueryFilter;

    match paired_devices::Entity::find()
        .filter(paired_devices::Column::Outbound.eq(true))
        .all(state.engine.db())
        .await
    {
        Ok(rows) => rows.into_iter().map(|row| row.device_id).collect(),
        Err(error) => {
            tracing::warn!(%error, "could not list outbound devices");
            Vec::new()
        }
    }
}

/// Delete pending pairings whose ticket window has closed.
///
/// Rows only outlive their window if the daemon exited while one was open;
/// pruning on startup keeps `pairing.list` free of dead entries.
async fn cleanup_old_pairings(state: &Arc<DaemonState>) -> Result<(), ServerError> {
    use livtet_data::client_entities::pending_pairings;
    use livtet_data::orm::{ColumnTrait, EntityTrait, QueryFilter};

    pending_pairings::Entity::delete_many()
        .filter(pending_pairings::Column::ExpiresAt.lt(primitive_now()))
        .exec(state.engine.db())
        .await
        .map_err(|e| ServerError::Db(format!("cleanup failed: {e}")))?;

    tracing::info!("cleaned up old pending pairings");
    Ok(())
}
