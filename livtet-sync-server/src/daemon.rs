//! The sync server daemon: SQLite + embedded sync HTTP server + stdio RPC loop.

use std::collections::HashMap;
use std::collections::HashSet;
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
use livtet_sync_http::{PairWaiters, make_sync_routes_with_sink, set_pair_waiters};
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
    self, DeviceNameParams, DeviceParams, OfferBookParams, OfferDecisionParams, OfferParams,
    PairingBeginParams, PairingClaimParams, RecentParams, ResolveParams, TokenParams,
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
    /// How an answered offer reaches the desktop, whichever transport carried it.
    pub offer_sink: livtet_sync_http::OfferDecisionSink,
    /// Serialises this daemon's own writes.
    ///
    /// RPCs now run concurrently, so a background pull and a reader-triggered
    /// one can both try to apply a page; SQLite allows one writer, and the
    /// loser gets `SQLITE_BUSY`. Waiting here is cheaper than failing there.
    pub write_gate: tokio::sync::Mutex<()>,
    /// The `iroh` discovery endpoint, bound at startup when the network allows.
    pub discovery: Mutex<Option<Arc<DiscoveryService>>>,
    /// The mDNS advertisement of the HTTP pairing surface, while the server runs.
    pub advertiser: Mutex<Option<Advertiser>>,
    /// The running HTTP server handle, if any.
    pub server: Mutex<Option<ServerHandle>>,
    /// The address the HTTP server is actually bound to.
    pub bound: Mutex<Option<std::net::SocketAddr>>,
}

/// Keyring service under which the discovery secret is stored.
const DISCOVERY_KEYRING_SERVICE: &str = "net.olamaelcu.livtet.sync";
/// Keyring account (item) name for the discovery secret.
const DISCOVERY_KEYRING_ACCOUNT: &str = "discovery-node-key";
/// Explicit key override for tests and headless debug runs; unset normally.
#[cfg(debug_assertions)]
const DISCOVERY_KEY_ENV: &str = "LIVTET_DISCOVERY_KEY";

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
    let discovery_secret = discovery_secret_key();
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

    // A decision arrives from the peer, but the reader it belongs to is on the
    // other side of the stdio control channel: the sink is the seam, and both
    // transports route through it.
    let decision_notifier = notifier.clone();
    let offer_sink: livtet_sync_http::OfferDecisionSink = Arc::new(move |decision| {
        requests::notify(
            &decision_notifier,
            rpc::method::NOTIFY_OFFER_DECIDED,
            serde_json::json!({
                "offer_id": decision.offer_id,
                "device_id": decision.device_id,
                "accepted": decision.accepted,
            }),
        );
    });

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
        offer_sink,
        write_gate: tokio::sync::Mutex::new(()),
        discovery: Mutex::new(discovery),
        advertiser: Mutex::new(None),
        server: Mutex::new(None),
        bound: Mutex::new(None),
    });

    let writer = tokio::spawn(writer_task(out_rx));

    start_server(&state).await?;
    advertise(&state).await;

    // The framed transport runs on the discovery endpoint that is already
    // bound: no port, no listener of its own, and it closes when the endpoint
    // does.
    let endpoint = state
        .discovery
        .lock()
        .await
        .as_ref()
        .map(|service| service.endpoint().clone());
    if let Some(endpoint) = endpoint {
        livtet_sync_iroh::spawn_server(endpoint, crate::wire::handler(Arc::clone(&state)));
    }
    spawn_pull_loop(Arc::clone(&state));
    let _ = cleanup_old_pairings(&state).await;
    let _ = promote_mutual_pairings(&state).await;

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

    let offer_sink = state.offer_sink.clone();

    let app = make_sync_routes_with_sink(engine, pair_waiters, identity, offer_sink)
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
                // `shutdown` stays inline: its answer must be queued before the
                // loop breaks and the writer drains, or the parent never learns
                // the daemon left.
                if request.method == rpc::method::SHUTDOWN {
                    let response = dispatch(request, state, out_tx).await;
                    let _ = out_tx.send(rpc::encode_response(&response));
                    break;
                }
                // Everything else runs concurrently. A pull holds the RPC for
                // as long as its HTTP calls, file writes and `apply_changes`
                // take, and dispatching inline starved every other call for
                // that long: the settings page's status and discovery polls
                // timed out while a single peer was being read.
                let state = Arc::clone(state);
                let out_tx = out_tx.clone();
                tokio::spawn(async move {
                    let response = dispatch(request, &state, &out_tx).await;
                    let _ = out_tx.send(rpc::encode_response(&response));
                });
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
        rpc::method::OFFERS_LIST => finish(&request, handle_offers_list(state).await),
        rpc::method::OFFERS_COVER => finish(&request, handle_offers_cover(&request, state).await),
        rpc::method::OFFERS_DECIDE => finish(&request, handle_offers_decide(&request, state).await),
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

/// Whether a pending row is a real remote claim.
///
/// A row with no socket origin arrived over iroh, where the connection's node
/// id is the identity and cannot be spoofed; a row whose origin is this
/// device's own address is this machine talking to itself, and is not a peer.
fn is_remote_claim(origin: Option<&livtet_types::Address>, self_addr: &str) -> bool {
    origin.is_none_or(|addr| addr.to_string() != self_addr)
}

async fn handle_status(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::{paired_devices, pending_pairings};

    let db = state.engine.db();
    let latest_version = state.engine.get_latest_version().await?;
    let paired_device_count = paired_devices::Entity::find().count(db).await?;

    // A pending pairing counts only when it is a real remote claim; see
    // `is_remote_claim`. The table holds a handful of rows, so filtering in
    // Rust keeps the rule in one place instead of in two queries.
    let self_addr = self_address(state).await;
    let pending_pairing_count = pending_pairings::Entity::find()
        .filter(pending_pairings::Column::StatusId.eq(DbId::from(PairingStatus::Pending)))
        .filter(pending_pairings::Column::ExpiresAt.gt(primitive_now()))
        .all(db)
        .await?
        .into_iter()
        .filter(|row| is_remote_claim(row.origin_addr.as_ref(), &self_addr))
        .count();

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

    // A claim that arrived over iroh carries no socket origin and is admitted;
    // one whose origin is this device's own address is not a peer.
    let self_addr = self_address(state).await;
    let rows: Vec<_> = pending_pairings::Entity::find()
        .filter(pending_pairings::Column::StatusId.eq(DbId::from(PairingStatus::Pending)))
        .filter(pending_pairings::Column::ExpiresAt.gt(primitive_now()))
        .all(state.engine.db())
        .await?
        .into_iter()
        .filter(|row| is_remote_claim(row.origin_addr.as_ref(), &self_addr))
        .collect();

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
    // A claim filed over iroh has no socket origin — the connection's node id
    // is the identity, and a peer cannot forge it. A claim that carries an
    // origin (the HTTP transport) must not be this device talking to itself.
    if pending_node_id == state.device_id
        || pending
            .origin_addr
            .as_ref()
            .is_some_and(|addr| addr.to_string() == self_addr)
    {
        return Err(invalid_params(format!(
            "pairing node id did not originate from a remote device: {pending_node_id}"
        )));
    }

    let session_token = ulid::Ulid::generate().to_string();
    let device_id = pending.device_id.unwrap_or_else(DbId::new);
    // Only a claim that names where *it* serves can be dialled back. The
    // fallback address is this device's own, which would mean pulling from
    // ourselves, so it stays "unknown" and the pairing stays one-way.
    let peer_address = pending
        .listen_on
        .as_ref()
        .filter(|address| address.to_string() != self_addr)
        .cloned();

    // A claim over iroh carries no socket origin: the connection's node id is
    // the identity, and it is what the row dials later.
    let claimed_over_iroh = pending.origin_addr.is_none();
    let model = paired_devices::ActiveModel {
        device_id: Set(device_id),
        name: Set(pending.device_name.clone()),
        listen_on: Set(peer_address.clone()),
        device_type_id: Set(pending.device_type_id),
        paired_at: Set(primitive_now()),
        last_sync_at: Set(None),
        // A pairing is mutual when the peer can be dialled: it named where it
        // serves, or it claimed over iroh, where the node id — the pairing key
        // itself — is the address. A claim that arrived over HTTP with no
        // address comes from a peer too old to speak iroh, and stays one-way.
        outbound: Set(peer_address.is_some() || claimed_over_iroh),
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
    livtet_data::sql::query(
        "UPDATE paired_devices SET session_token = ?, node_id = ? WHERE device_id = ?",
    )
    .bind(session_token.clone())
    .bind(pending_node_id.clone())
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
    use livtet_data::client_entities::paired_devices;
    use livtet_data::orm::{EntityTrait, Set, sea_query::OnConflict};
    use livtet_sync_iroh::{Request, Response};
    use livtet_types::Address;

    // Pairing is dialled by node id, so the claimant needs no reachable port
    // and the approver's address never enters the exchange.
    let endpoint = state
        .discovery
        .lock()
        .await
        .as_ref()
        .map(|service| service.endpoint().clone())
        .ok_or_else(|| {
            ServerError::Server("peer discovery is not bound, so pairing cannot dial".to_string())
        })?;
    let connection = livtet_sync_iroh::dial(&endpoint, &params.node_id)
        .await
        .map_err(|error| ServerError::Server(format!("pairing dial: {error}")))?;

    // Who is on the other end? The connection authenticates the node id, and
    // the identity frame names the device id the pairing row is keyed by.
    let exchange = livtet_sync_iroh::call(&connection, &Request::Identity)
        .await
        .map_err(|error| ServerError::Server(format!("peer identity: {error}")))?;
    let identity = match exchange.response {
        Response::Identity(identity) => identity,
        other => {
            return Err(ServerError::Server(format!(
                "peer identity: unexpected reply {other:?}"
            )));
        }
    };
    let peer_device_id = identity
        .device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(&identity.device_id).ok())
        .ok_or_else(|| {
            ServerError::Server(format!("invalid peer device id: {}", identity.device_id))
        })?;

    let our_node = discovery_node_id(state).await;
    let our_name = state
        .device_name
        .lock()
        .await
        .clone()
        .unwrap_or_else(|| crate::mdns::friendly_name(&state.device_id));
    let exchange = livtet_sync_iroh::call(
        &connection,
        &Request::Pair {
            device_id: state.device_id.clone(),
            name: our_name,
            device_type: "desktop".to_string(),
            node_id: our_node,
        },
    )
    .await
    .map_err(|error| ServerError::Server(format!("pairing claim: {error}")))?;
    let session_token = match exchange.response {
        Response::Paired { session_token, .. } if !session_token.is_empty() => session_token,
        Response::Paired { .. } => {
            return Err(ServerError::Server(
                "approval carried no session token".to_string(),
            ));
        }
        Response::Refused { reason } => {
            return Err(ServerError::Server(format!(
                "pairing was refused: {reason}"
            )));
        }
        other => {
            return Err(ServerError::Server(format!(
                "pairing claim: unexpected reply {other:?}"
            )));
        }
    };

    let listen_on = params
        .listen_on
        .trim()
        .parse::<std::net::SocketAddr>()
        .ok()
        .map(Address::new);
    let db = state.engine.db();
    let model = paired_devices::ActiveModel {
        device_id: Set(peer_device_id),
        name: Set(params.device_name.clone()),
        listen_on: Set(listen_on),
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
                    paired_devices::Column::PairedAt,
                    paired_devices::Column::Outbound,
                ])
                .to_owned(),
        )
        .exec(db)
        .await
        .map_err(|error| ServerError::Server(format!("storing paired device: {error}")))?;

    // `session_token` and `node_id` are not on the entity; write them through
    // the pool. The node id is what every later dial addresses.
    livtet_data::sql::query(
        "UPDATE paired_devices SET session_token = ?, node_id = ? WHERE device_id = ?",
    )
    .bind(session_token)
    .bind(params.node_id.clone())
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
                // Which way the pairing points: an outbound row is one this
                // side claimed, so it holds the token the peer issued and can
                // be pulled from. An approved row holds a token *this* side
                // issued, which only lets the peer pull from here. Both are
                // paired devices; only one can be pulled from.
                "outbound": row.outbound,
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
    use livtet_data::entities::{authors, digital_inventory, editions, work_authors, works};

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
    // The reader on the other side is asked about this book before any catalog
    // row lands, so the title and authors cannot come from its own tables: the
    // offer carries them.
    let work = works::Entity::find_by_id(edition.work_id)
        .one(db)
        .await?
        .ok_or_else(|| invalid_params(format!("unknown work: {}", edition.work_id)))?;
    let title = edition
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| work.title.clone());
    let mut authors: Vec<String> = authors::Entity::find()
        .filter(
            authors::Column::Id.is_in(
                work_authors::Entity::find()
                    .filter(work_authors::Column::WorkId.eq(edition.work_id))
                    .all(db)
                    .await?
                    .into_iter()
                    .map(|link| link.author_id)
                    .collect::<Vec<_>>(),
            ),
        )
        .all(db)
        .await?
        .into_iter()
        .map(|author| author.name)
        .collect();
    authors.sort();
    let payload = serde_json::json!({
        "offer_id": offer_id,
        "inventory_id": inventory_id,
        "edition_id": edition_id.to_hex(),
        "work_id": edition.work_id.to_hex(),
        "to_device_id": device_id.to_string(),
        "title": title,
        "authors": authors,
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

/// One pull pass against an outbound peer, with its address corrected first
/// when the peer turns out to have moved.
///
/// The address a pull dials is the pairing row's, and that row is written once
/// — when the pairing is approved. The peer's daemon, however, binds its HTTP
/// server on every launch and advertises the port it picked over mDNS. A peer
/// that restarted therefore keeps a pairing that no longer resolves, and every
/// pull fails silently until the two are paired again. So a failed pull
/// re-resolves the address from the peer's current advertisement and retries
/// once; a peer that is simply unreachable, or a failure that was not about
/// reaching it, is reported unchanged.
async fn pull_device(
    state: &Arc<DaemonState>,
    device_id: DbId,
    background: bool,
) -> Result<serde_json::Value, ServerError> {
    let outcome = match pull_device_once(state, device_id, background).await {
        Ok(outcome) => outcome,
        Err(error) => match re_resolve_peer_address(state, device_id).await {
            Ok(true) => {
                tracing::info!(%device_id, %error, "peer address changed; retrying the pull");
                pull_device_once(state, device_id, background).await?
            }
            Ok(false) => return Err(error),
            Err(browse_error) => {
                // The pull's own failure is the one worth reporting: a browse
                // that heard nothing is how a peer that is simply off looks.
                tracing::debug!(%device_id, %browse_error, "could not re-resolve the peer");
                return Err(error);
            }
        },
    };

    report_pulled(state, device_id, &outcome);
    Ok(outcome)
}

/// Correct a paired peer's stored address from its mDNS advertisement.
///
/// Returns whether the row changed, which is what makes a retry worth
/// spending. A peer that is not advertising — asleep, off the network, or on a
/// link that drops multicast — leaves the pairing untouched.
async fn re_resolve_peer_address(
    state: &Arc<DaemonState>,
    device_id: DbId,
) -> Result<bool, ServerError> {
    use livtet_data::client_entities::paired_devices;

    let own_device_id = state.device_id.clone();
    // `mdns-sd`'s receiver is blocking, so the window runs off the async
    // runtime, as it does for `discovery.nearby`.
    let peers = tokio::task::spawn_blocking(crate::mdns::browse_nearby)
        .await
        .map_err(|error| ServerError::Server(format!("mDNS browse task failed: {error}")))??;

    let wanted = device_id.to_string();
    let Some(peer) = peers
        .iter()
        .find(|peer| peer.device_id == wanted && peer.device_id != own_device_id)
    else {
        return Ok(false);
    };
    let Some(address) = preferred_address(peer) else {
        return Ok(false);
    };

    let db = state.engine.db();
    let Some(row) = paired_devices::Entity::find_by_id(device_id)
        .one(db)
        .await
        .map_err(|error| ServerError::Server(format!("paired device lookup: {error}")))?
    else {
        return Ok(false);
    };
    if !address_changed(row.listen_on.as_ref(), &address) {
        return Ok(false);
    }

    let mut model: paired_devices::ActiveModel = row.into();
    model.listen_on = Set(Some(address.clone()));
    model
        .update(db)
        .await
        .map_err(|error| ServerError::Server(format!("paired device update: {error}")))?;
    tracing::info!(%device_id, %address, "paired device address refreshed from its advertisement");
    Ok(true)
}

/// The address to dial for an advertised peer: the first routable one it
/// listed, else whatever it listed at all.
///
/// An advertisement carries every interface's address, including loopback and
/// link-local ones a peer on another machine cannot reach. Falling back rather
/// than giving up keeps two instances on one machine workable.
fn preferred_address(peer: &NearbyPeer) -> Option<livtet_types::Address> {
    let candidates: Vec<std::net::SocketAddr> = peer
        .addresses
        .iter()
        .filter_map(|address| address.trim().parse::<std::net::IpAddr>().ok())
        .map(|ip| std::net::SocketAddr::new(ip, peer.port))
        .collect();
    candidates
        .iter()
        .copied()
        .find(is_routable)
        .or_else(|| candidates.first().copied())
        .map(livtet_types::Address::new)
}

/// Whether a peer could be reached at `candidate` from another machine.
fn is_routable(candidate: &std::net::SocketAddr) -> bool {
    let ip = candidate.ip();
    !ip.is_loopback() && !ip.is_unspecified() && !is_link_local(ip)
}

/// Link-local addresses are scoped to one link and never reach a peer.
fn is_link_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// Whether the stored address differs from the freshly resolved one.
fn address_changed(
    stored: Option<&livtet_types::Address>,
    resolved: &livtet_types::Address,
) -> bool {
    stored.map(|address| address.0) != Some(resolved.0)
}

/// One pull pass against an outbound peer, using the address on file.
async fn pull_device_once(
    state: &Arc<DaemonState>,
    device_id: DbId,
    background: bool,
) -> Result<serde_json::Value, ServerError> {
    use livtet_data::client_entities::paired_devices;
    use livtet_data::orm::{EntityTrait, Set};
    use livtet_sync::{ApplyOptions, PullResponse};

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
    // The transport dials by node id over iroh, so no inbound port on the peer
    // has to be reachable; a row from before that keeps the HTTP path.
    let peer_client = crate::peer::peer_for(state, device_id).await?;

    let cursor_key = format!("sync.cursor.{device_id}");
    let mut cursor = read_setting(state, &cursor_key)
        .await?
        .unwrap_or_else(|| "0".to_string());
    let mut applied = 0;
    let mut offered = 0;
    let mut conflicts = 0;

    loop {
        let since_version = cursor.parse::<i64>().unwrap_or(0);
        let pull: PullResponse = peer_client.changes(since_version, 100).await?;
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

        // A book this device offered is a question, not a delivery: record it
        // for the reader to answer and hold the catalog rows that describe it,
        // so a declined book never reaches the library.
        let mut offers: Vec<(livtet_sync::SyncChange, serde_json::Value)> = Vec::new();
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
            offers.push((change.clone(), payload));
        }

        let changes: Vec<_> = pull
            .changes
            .into_iter()
            .filter(|change| change.entity_type != "file_offer")
            .collect();
        // Rows describing an offered book wait with it; everything else is a
        // library change and applies now.
        let held_ids: HashSet<String> = offers
            .iter()
            .flat_map(|(_, payload)| offer_row_ids(payload))
            .collect();
        let (deferred, metadata): (Vec<_>, Vec<_>) = changes
            .into_iter()
            .partition(|change| held_ids.contains(change.entity_id.as_str()));
        {
            // The write gate is held only here: the page is already fetched, and
            // everything below touches the database. A background pull and a
            // reader-triggered one otherwise race for SQLite's single writer.
            let _write = state.write_gate.lock().await;

            if !metadata.is_empty() {
                let outcome = state
                    .engine
                    .apply_changes(
                        metadata,
                        ApplyOptions {
                            inventory_paths: HashMap::new(),
                        },
                    )
                    .await
                    .map_err(ServerError::from)?;
                applied += outcome.applied;
                conflicts += outcome.conflicts.len();
            }

            for (change, payload) in &offers {
                let ids = offer_row_ids(payload);
                let held: Vec<livtet_sync::SyncChange> = deferred
                    .iter()
                    .filter(|row| ids.contains(&row.entity_id))
                    .cloned()
                    .collect();
                if record_inbound_offer(state, change, payload, &held).await? {
                    offered += 1;
                }
            }

            cursor = pull.latest_version.to_string();
            write_setting(state, &cursor_key, &cursor).await?;
        }
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

/// Load the discovery secret from the OS keyring, creating it on first run.
///
/// The 32-byte secret determines the iroh node id. It is never written to the
/// data directory and never enters `client_settings`, which syncs to peers.
/// If the keyring is unavailable, this run gets an ephemeral key: the daemon
/// still works, but the node id changes on the next start.
fn discovery_secret_key() -> SecretKey {
    if let Some(secret) = discovery_secret_from_env() {
        return secret;
    }

    let entry = match keyring::Entry::new(DISCOVERY_KEYRING_SERVICE, DISCOVERY_KEYRING_ACCOUNT) {
        Ok(entry) => entry,
        Err(error) => {
            tracing::warn!(%error, "OS keyring unavailable; using an ephemeral node id");
            return SecretKey::generate();
        }
    };

    match entry.get_password() {
        Ok(encoded) => match decode_discovery_secret(&encoded) {
            Some(secret) => return secret,
            None => tracing::warn!("stored discovery key is malformed; replacing it"),
        },
        Err(keyring::Error::NoEntry) => {}
        Err(error) => {
            tracing::warn!(%error, "OS keyring read failed; trying to store a new key");
        }
    }

    let secret = SecretKey::generate();
    let encoded = hex::encode(secret.to_bytes());
    if let Err(error) = entry.set_password(&encoded) {
        tracing::warn!(%error, "OS keyring write failed; using an ephemeral node id");
    }
    secret
}

/// Explicit override for tests and headless debug runs; never used in release.
#[cfg(debug_assertions)]
fn discovery_secret_from_env() -> Option<SecretKey> {
    let encoded = std::env::var(DISCOVERY_KEY_ENV).ok()?;
    decode_discovery_secret(&encoded)
}

#[cfg(not(debug_assertions))]
fn discovery_secret_from_env() -> Option<SecretKey> {
    None
}

fn decode_discovery_secret(encoded: &str) -> Option<SecretKey> {
    let bytes: [u8; 32] = hex::decode(encoded).ok()?.try_into().ok()?;
    Some(SecretKey::from_bytes(&bytes))
}

/// Fetch one offered file (and its cover) into the receiver's directories.
async fn download_offer(
    peer: &crate::peer::Peer,
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

    let file_path = books_dir.join(format!("{inventory_id}-{file_name}"));
    peer.download_file(inventory_id, &file_path).await?;

    let mut cover_path = None;
    if payload
        .get("has_cover")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        if let Some((content_type, bytes)) = peer.cover(inventory_id).await? {
            let extension = if content_type.contains("jpeg") {
                "jpg"
            } else if content_type.contains("webp") {
                "webp"
            } else {
                "png"
            };
            let path = covers_dir.join(format!("{inventory_id}.{extension}"));
            fs_err::tokio::write(path.as_std_path(), &bytes)
                .await
                .map_err(|error| ServerError::Server(format!("write received cover: {error}")))?;
            cover_path = Some(path.to_string());
        }
    }

    Ok((
        inventory_id.to_string(),
        livtet_sync::InventoryPaths {
            file_path: Some(file_path.to_string()),
            cover_path,
        },
    ))
}

/// The inventory, edition, and work ids an offer names.
///
/// A pull holds these rows back until the reader decides; the offer payload
/// carries them in the same hex form `change_log` rows write.
fn offer_row_ids(payload: &serde_json::Value) -> HashSet<String> {
    ["inventory_id", "edition_id", "work_id"]
        .iter()
        .filter_map(|key| payload.get(*key).and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

/// Record one offer from a pull, holding the rows that describe it.
///
/// Returns `true` when this is a newly pending offer, so the caller can count
/// it and the app can be told to ask. A re-offer refreshes a pending row; a
/// decided one stays decided, because the reader's answer outranks a retry.
async fn record_inbound_offer(
    state: &Arc<DaemonState>,
    change: &livtet_sync::SyncChange,
    payload: &serde_json::Value,
    held: &[livtet_sync::SyncChange],
) -> Result<bool, ServerError> {
    use livtet_data::client_entities::inbound_offers;

    let text = |key: &str| {
        payload
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    let offer_id =
        text("offer_id").ok_or_else(|| ServerError::Server("offer has no id".to_string()))?;
    let inventory_id = text("inventory_id")
        .ok_or_else(|| ServerError::Server("offer has no inventory id".to_string()))?;
    let edition_id = text("edition_id")
        .ok_or_else(|| ServerError::Server("offer has no edition id".to_string()))?;
    let work_id =
        text("work_id").ok_or_else(|| ServerError::Server("offer has no work id".to_string()))?;
    // A sender that predates the title in the payload leaves nothing to show;
    // the dialog still needs a line, so the offer keeps a neutral one.
    let title = text("title").unwrap_or_else(|| "Untitled".to_string());
    let authors = payload
        .get("authors")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]))
        .to_string();
    let deferred = serde_json::to_string(held).unwrap_or_else(|_| "[]".to_string());
    let from_device_id = change.device_id.clone();

    let existing = inbound_offers::Entity::find_by_id(offer_id.clone())
        .one(state.engine.db())
        .await
        .map_err(|error| ServerError::Server(format!("inbound offer lookup: {error}")))?;
    let fresh = existing.is_none();
    if let Some(row) = &existing
        && row.status != "pending"
    {
        return Ok(false);
    }

    livtet_data::sql::query(
        "INSERT INTO inbound_offers (offer_id, from_device_id, inventory_id, edition_id, \
             work_id, title, authors, file_name, has_cover, offered_at, received_at, \
             status, deferred_changes) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?) \
         ON CONFLICT(offer_id) DO UPDATE SET \
             from_device_id = excluded.from_device_id, \
             inventory_id = excluded.inventory_id, \
             edition_id = excluded.edition_id, \
             work_id = excluded.work_id, \
             title = excluded.title, \
             authors = excluded.authors, \
             file_name = excluded.file_name, \
             has_cover = excluded.has_cover, \
             offered_at = excluded.offered_at, \
             deferred_changes = excluded.deferred_changes \
         WHERE inbound_offers.status = 'pending'",
    )
    .bind(offer_id.clone())
    .bind(from_device_id.clone())
    .bind(inventory_id)
    .bind(edition_id)
    .bind(work_id)
    .bind(title.clone())
    .bind(authors)
    .bind(text("file_name"))
    .bind(
        payload
            .get("has_cover")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    )
    .bind(change.changed_at.clone())
    .bind(now_string())
    .bind(deferred)
    .execute(&state.pool)
    .await
    .map_err(|error| ServerError::Server(format!("could not record the offer: {error}")))?;

    if fresh {
        requests::notify(
            &state.notifier,
            rpc::method::NOTIFY_OFFER_RECEIVED,
            serde_json::json!({
                "offer_id": offer_id,
                "from_device_id": from_device_id,
                "title": title,
            }),
        );
    }
    Ok(fresh)
}

/// Books offered to this desktop that are still waiting on a decision.
async fn handle_offers_list(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::{inbound_offers, paired_devices};

    let mut rows = inbound_offers::Entity::find()
        .filter(inbound_offers::Column::Status.eq("pending"))
        .all(state.engine.db())
        .await?;
    rows.sort_by(|left, right| right.received_at.cmp(&left.received_at));

    let names: HashMap<String, Option<String>> = paired_devices::Entity::find()
        .all(state.engine.db())
        .await?
        .into_iter()
        .map(|device| (device.device_id.to_string(), device.name))
        .collect();

    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "offer_id": row.offer_id,
                "from_device_id": row.from_device_id,
                "from_name": names.get(&row.from_device_id).cloned().flatten(),
                "title": row.title,
                "authors": serde_json::from_str::<Vec<String>>(&row.authors).unwrap_or_default(),
                "file_name": row.file_name,
                "has_cover": row.has_cover,
                "offered_at": row.offered_at,
                "received_at": row.received_at,
            })
        })
        .collect();
    Ok(serde_json::Value::Array(list))
}

/// One inbound offer's cover, fetched from the sender and encoded for the UI.
///
/// The bytes have not been downloaded yet — that happens on acceptance — so
/// this reaches back over the same token the pull used. `null` means there is
/// nothing to show, which is not an error: plenty of books have no cover.
async fn handle_offers_cover(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    use base64::Engine as _;
    use livtet_data::client_entities::inbound_offers;

    let params: OfferParams = parse_params(request)?;
    let row = inbound_offers::Entity::find_by_id(params.offer_id)
        .one(state.engine.db())
        .await?
        .ok_or_else(|| invalid_params("no such offer".to_string()))?;
    if !row.has_cover {
        return Ok(serde_json::Value::Null);
    }

    let peer = crate::peer::peer_by_id(state, &row.from_device_id).await?;
    let Some((content_type, bytes)) = peer.cover(&row.inventory_id).await? else {
        return Ok(serde_json::Value::Null);
    };
    Ok(serde_json::json!({
        "content_type": content_type,
        "data_url": format!(
            "data:{content_type};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        ),
    }))
}

/// Take or decline an inbound offer, then tell the sender what happened.
///
/// Accepting is what downloads the file and applies the held rows, so nothing
/// of a declined book ever reaches the library. The decision stands locally
/// whether or not the sender is still reachable; `delivered` says which.
async fn handle_offers_decide(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::inbound_offers;
    use livtet_sync::{ApplyOptions, InventoryPaths};

    let params: OfferDecisionParams = parse_params(request)?;
    let row = inbound_offers::Entity::find_by_id(params.offer_id.clone())
        .one(state.engine.db())
        .await?
        .ok_or_else(|| invalid_params(format!("no such offer: {}", params.offer_id)))?;
    if row.status != "pending" {
        return Err(invalid_params(format!(
            "offer {} was already {}",
            params.offer_id, row.status
        )));
    }

    let peer = crate::peer::peer_by_id(state, &row.from_device_id).await?;
    let mut applied = 0;
    let mut conflicts = 0;
    // Held for the local writes below, released before the peer is told: a
    // network call must not block another reader's pull.
    let mut write = None;

    if params.accepted {
        let books_dir = state.config.books_dir.clone().ok_or_else(|| RpcFailure {
            code: rpc::error_code::INTERNAL_ERROR,
            message: "no books directory configured".to_string(),
        })?;
        let covers_dir = state.config.covers_dir.clone().ok_or_else(|| RpcFailure {
            code: rpc::error_code::INTERNAL_ERROR,
            message: "no covers directory configured".to_string(),
        })?;
        fs_err::tokio::create_dir_all(books_dir.as_std_path())
            .await
            .map_err(|error| RpcFailure {
                code: rpc::error_code::INTERNAL_ERROR,
                message: format!("books directory: {error}"),
            })?;
        fs_err::tokio::create_dir_all(covers_dir.as_std_path())
            .await
            .map_err(|error| RpcFailure {
                code: rpc::error_code::INTERNAL_ERROR,
                message: format!("covers directory: {error}"),
            })?;

        let offer = serde_json::json!({
            "inventory_id": row.inventory_id,
            "file_name": row.file_name,
            "has_cover": row.has_cover,
        });
        let (inventory_id, paths) = download_offer(&peer, &offer, &books_dir, &covers_dir)
            .await
            .map_err(|error| RpcFailure {
                code: rpc::error_code::INTERNAL_ERROR,
                message: error.to_string(),
            })?;

        let held: Vec<livtet_sync::SyncChange> =
            serde_json::from_str(&row.deferred_changes).unwrap_or_default();
        write = Some(state.write_gate.lock().await);
        if !held.is_empty() {
            let outcome = state
                .engine
                .apply_changes(
                    held,
                    ApplyOptions {
                        inventory_paths: HashMap::from([(
                            inventory_id,
                            InventoryPaths {
                                file_path: paths.file_path.clone(),
                                cover_path: paths.cover_path.clone(),
                            },
                        )]),
                    },
                )
                .await
                .map_err(ServerError::from)?;
            applied = outcome.applied;
            conflicts = outcome.conflicts.len();
        }
    }

    livtet_data::sql::query(
        "UPDATE inbound_offers SET status = ?, decided_at = ?, deferred_changes = '[]' \
         WHERE offer_id = ?",
    )
    .bind(if params.accepted {
        "accepted"
    } else {
        "rejected"
    })
    .bind(now_string())
    .bind(row.offer_id.clone())
    .execute(&state.pool)
    .await
    .map_err(|error| RpcFailure {
        code: rpc::error_code::INTERNAL_ERROR,
        message: format!("could not record the decision: {error}"),
    })?;

    drop(write.take());
    let delivered = peer
        .offer_decision(&row.offer_id, params.accepted)
        .await
        .is_ok();

    Ok(serde_json::json!({
        "offer_id": row.offer_id,
        "accepted": params.accepted,
        "delivered": delivered,
        "applied": applied,
        "conflicts": conflicts,
        // The receiving side indexes what it accepted; it needs the edition
        // the offer carried, which its own tables now hold but which the
        // caller cannot name from the offer id alone.
        "edition_id": row.edition_id,
    }))
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
        match DiscoveryService::bind_with_secret(discovery_secret_key()).await {
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
pub(crate) async fn discovery_node_id(state: &Arc<DaemonState>) -> String {
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
        let interval = Duration::from_secs(state.config.pull_interval_secs.max(1));
        let mut ticker = tokio::time::interval(interval);
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

/// Tell the parent process that a pull changed this library.
///
/// The pull runs on the daemon's own clock, so nothing else would tell the UI:
/// `sync.completed` reports requests this device *served*. Only a pull that
/// applied rows is reported — a pull that found nothing changed the catalog by
/// definition not at all, and a UI refetch would be pure noise.
fn report_pulled(state: &Arc<DaemonState>, device_id: DbId, outcome: &serde_json::Value) {
    let applied = outcome
        .get("applied")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if applied == 0 {
        return;
    }
    requests::notify(
        &state.notifier,
        rpc::method::NOTIFY_SYNC_PULLED,
        serde_json::json!({
            "device_id": device_id.to_string(),
            "applied": applied,
            "conflicts": outcome
                .get("conflicts")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        }),
    );
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

/// Make pairings approved before pairing became mutual pull-capable.
///
/// Approval used to write `outbound = false`, which `pull_device` refuses: the
/// device that *approved* a claim could never read the device that filed it,
/// even though both sides already held the same token. The flag only gates
/// pulling, so flipping it on rows that have a token grants the missing half
/// without re-pairing anything.
async fn promote_mutual_pairings(state: &Arc<DaemonState>) -> Result<(), ServerError> {
    // Approval used to store the address the claim was *served* on — this
    // device's own, and a wildcard one at that — so those rows have nothing to
    // dial. A peer address is never `0.0.0.0`/`[::]`; anything specific is.
    let updated = livtet_data::sql::query(
        "UPDATE paired_devices SET outbound = 1 \
         WHERE outbound = 0 AND session_token IS NOT NULL AND session_token != '' \
           AND listen_on IS NOT NULL \
           AND listen_on NOT LIKE '0.0.0.0:%' AND listen_on NOT LIKE '[::]:%'",
    )
    .execute(&state.pool)
    .await
    .map_err(|e| ServerError::Db(format!("promoting mutual pairings: {e}")))?;

    if updated.rows_affected() > 0 {
        tracing::info!(
            devices = updated.rows_affected(),
            "pairings approved before they were mutual now pull in both directions"
        );
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{address_changed, preferred_address};
    use crate::mdns::NearbyPeer;

    fn peer(addresses: &[&str], port: u16) -> NearbyPeer {
        NearbyPeer {
            node_id: "node".to_string(),
            device_id: "01M6BYRPJ5XQ4HH7R8CEZ194HY".to_string(),
            name: "Jacky's MacBook Pro".to_string(),
            device_type: "desktop".to_string(),
            version: "0.2.1".to_string(),
            hostname: "jackys-macbook-pro.local".to_string(),
            addresses: addresses
                .iter()
                .map(|address| address.to_string())
                .collect(),
            port,
        }
    }

    /// A peer advertises every interface it has. Dialling loopback or a
    /// link-local address reaches this machine, never the peer.
    #[test]
    fn the_routable_address_wins_over_the_local_ones() {
        let advertised = peer(
            &["127.0.0.1", "fe80::1c2b:3eff:fe4d:5a6b", "192.168.1.226"],
            7710,
        );

        let chosen = preferred_address(&advertised).expect("an address to dial");

        assert_eq!(chosen.to_string(), "192.168.1.226:7710");
    }

    /// Two instances on one machine still sync: the fallback is better than
    /// refusing to try.
    #[test]
    fn a_loopback_only_peer_is_still_reachable() {
        let advertised = peer(&["127.0.0.1"], 7711);

        let chosen = preferred_address(&advertised).expect("an address to dial");

        assert_eq!(chosen.to_string(), "127.0.0.1:7711");
    }

    #[test]
    fn a_peer_without_addresses_offers_nothing_to_dial() {
        assert!(preferred_address(&peer(&[], 7710)).is_none());
    }

    /// The address is only worth rewriting when it actually moved: a peer that
    /// came back on the port it advertised before must not churn its row (and
    /// trigger a pointless retry) on every failed pull.
    #[test]
    fn only_a_moved_address_counts_as_a_change() {
        let stored: livtet_types::Address = "192.168.1.226:7710".parse().expect("stored address");
        let same: livtet_types::Address = "192.168.1.226:7710".parse().expect("same address");
        let moved: livtet_types::Address = "192.168.1.226:7711".parse().expect("moved address");

        assert!(!address_changed(Some(&stored), &same));
        assert!(address_changed(Some(&stored), &moved));
        assert!(
            address_changed(None, &moved),
            "a pairing with no address has to take the resolved one"
        );
    }
}
