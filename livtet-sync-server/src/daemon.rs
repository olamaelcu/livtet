//! The sync server daemon: SQLite + iroh sync endpoint + stdio RPC loop.

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
use livtet_rendezvous::RendezvousClient;
use livtet_sync::SyncEngine;
use livtet_types::{DbId, PairingStatus};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::config::ServerConfig;
use crate::error::ServerError;
use crate::pairing::{OfferDecisionSink, PairDecisions, PairWaiters, new_pair_decisions};
use crate::requests::{self, Notifier, RequestLog, new_request_log};
use crate::rpc::{
    self, DeviceNameParams, DeviceParams, OfferBookParams, OfferDecisionParams, OfferParams,
    PairingBeginParams, PairingClaimCodeParams, PairingClaimParams, RebindParams,
    RequestsRecentParams, ResolveParams, RpcResponse, TokenParams,
};
use crate::wire_state::{RequestCounters, WireState};
use livtet_mdns::{Advertiser, NearbyPeer, browse_nearby};

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
    /// Sink for outgoing JSON-RPC notifications.
    pub notifier: Notifier,
    /// When the daemon started.
    pub started_at: time::OffsetDateTime,
    /// Outstanding pairing claims, keyed by the claimant's node id.
    pub pair_waiters: PairWaiters,
    /// Answered claims, keyed by the claim secret, so a claimant can resume
    /// after a dropped connection.
    pub pair_decisions: PairDecisions,
    /// How an answered offer reaches the desktop, whichever transport carried it.
    pub offer_sink: OfferDecisionSink,
    /// Serialises this daemon's own writes.
    ///
    /// RPCs now run concurrently, so a background pull and a reader-triggered
    /// one can both try to apply a page; SQLite allows one writer, and the
    /// loser gets `SQLITE_BUSY`. Waiting here is cheaper than failing there.
    pub write_gate: tokio::sync::Mutex<()>,
    /// Serialises endpoint lifecycle changes (`server.rebind`) against
    /// `shutdown`. RPCs run concurrently, so without this a shutdown can close
    /// the endpoint a rebind is still binding, and the new endpoint is dropped
    /// unclosed when the process exits.
    pub lifecycle: tokio::sync::Mutex<()>,
    /// The `iroh` discovery endpoint, bound at startup when the network allows.
    pub discovery: Mutex<Option<Arc<DiscoveryService>>>,
    /// The mDNS discovery advertisement, while the daemon runs.
    pub advertiser: Mutex<Option<Advertiser>>,
    /// The platform's pairing rendezvous, where short codes are published and
    /// resolved.
    ///
    /// One client per daemon: it owns the TLS configuration and the connection
    /// pool, and every code exchange has to use the same base the flags named.
    pub rendezvous: RendezvousClient,
    /// The most recent answered requests, newest last.
    pub request_log: RequestLog,
    /// The pipe back to the desktop, for the requests only it can answer.
    ///
    /// The OPDS route table lives there, so a forwarded `Opds` wire request
    /// becomes a reverse JSON-RPC call rather than an answer this process
    /// could produce.
    pub host: HostBridge,
    /// How many sync requests this daemon has answered.
    pub requests_served: Arc<AtomicU64>,
    /// When the last sync request was answered.
    pub last_request_at: Arc<Mutex<Option<String>>>,
}

impl DaemonState {
    /// The state the sync wire answers from, sharing this daemon's engine,
    /// pool, pairing maps and reporting seams.
    ///
    /// The wire takes [`WireState`] rather than `DaemonState` so an embedder can
    /// serve the same protocol without inventing a stdio pipe. Everything a peer
    /// can observe — approvals, offer decisions, the request log — is shared, so
    /// the daemon's RPCs still drive the answers the endpoint gives.
    pub async fn wire_state(&self) -> Arc<WireState> {
        let node_id = self
            .discovery
            .lock()
            .await
            .as_ref()
            .map(|service| service.node_id().0.clone());

        let state = WireState::new(
            self.engine.clone(),
            self.pool.clone(),
            self.device_id.clone(),
        )
        .with_notifier(Arc::clone(&self.notifier))
        .with_host(self.host.clone())
        .with_request_log(Arc::clone(&self.request_log))
        .with_counters(RequestCounters {
            served: Arc::clone(&self.requests_served),
            last_at: Arc::clone(&self.last_request_at),
        })
        .with_offer_sink(Arc::clone(&self.offer_sink))
        .with_pairing(
            Arc::clone(&self.pair_waiters),
            Arc::clone(&self.pair_decisions),
        )
        // Read at construction: a rename re-spawns the server, and this is the
        // name that spawn advertises.
        .with_device_name(self.device_name.lock().await.clone());

        Arc::new(match node_id {
            Some(node_id) => state.with_node_id(node_id),
            None => state,
        })
    }
}

/// A request the daemon sends its parent and waits on.
///
/// The stdio pipe normally flows one way: the desktop calls, the daemon
/// answers. OPDS pass-through needs the opposite direction — the desktop owns
/// the route table — so this keeps a second registry of in-flight ids and
/// [`crate::daemon::read_loop`] hands responses to it. Ids are daemon-owned and
/// unrelated to the parent's, because the two directions never share a call.
#[derive(Clone)]
pub struct HostBridge {
    inner: Arc<HostBridgeInner>,
}

struct HostBridgeInner {
    /// The stdout writer every line goes through.
    out: mpsc::UnboundedSender<String>,
    /// Requests waiting on an answer, keyed by the id the request carried.
    pending: std::sync::Mutex<HashMap<u64, oneshot::Sender<RpcResponse>>>,
    next_id: AtomicU64,
}

impl HostBridge {
    /// Build a bridge over the stdout line channel.
    pub fn new(out: mpsc::UnboundedSender<String>) -> Self {
        Self {
            inner: Arc::new(HostBridgeInner {
                out,
                pending: std::sync::Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// Send `method` to the desktop and wait for its answer.
    ///
    /// An answer is a [`rpc::RpcResponse`]; a `result` is returned, an `error`
    /// becomes the `Err` message. A timeout forgets the id, so a late answer is
    /// logged as unmatched rather than delivered to the next request.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self
                .inner
                .pending
                .lock()
                .map_err(|_| "the host bridge is poisoned".to_string())?;
            pending.insert(id, tx);
        }

        if self
            .inner
            .out
            .send(rpc::encode_request(id, method, params))
            .is_err()
        {
            self.forget(id);
            return Err("the control channel to the desktop is closed".to_string());
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => match (response.result, response.error) {
                (_, Some(error)) => Err(error.message),
                (Some(result), None) => Ok(result),
                (None, None) => Err("the desktop returned an empty response".to_string()),
            },
            Ok(Err(_)) => Err("the desktop dropped the request".to_string()),
            Err(_) => {
                self.forget(id);
                Err("the desktop did not answer in time".to_string())
            }
        }
    }

    /// Deliver `response` to the request waiting on its id, if any.
    pub fn answer(&self, response: RpcResponse) {
        let Some(id) = response.id.as_ref().and_then(serde_json::Value::as_u64) else {
            return;
        };
        let sender = self
            .inner
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&id));
        if let Some(sender) = sender {
            let _ = sender.send(response);
        }
    }

    fn forget(&self, id: u64) {
        if let Ok(mut pending) = self.inner.pending.lock() {
            pending.remove(&id);
        }
    }
}

/// Keyring service under which the discovery secret is stored.
const DISCOVERY_KEYRING_SERVICE: &str = "net.olamaelcu.livtet.sync";
/// Keyring account (item) name for the discovery secret.
const DISCOVERY_KEYRING_ACCOUNT: &str = "discovery-node-key";
/// Explicit key override for tests and headless debug runs; unset normally.
#[cfg(debug_assertions)]
const DISCOVERY_KEY_ENV: &str = "LIVTET_DISCOVERY_KEY";

/// How long a filed claim may wait for the reader to decide.
const PAIR_POLL_WINDOW: Duration = Duration::from_secs(300);

/// How often the claimant asks whether the claim was decided.
const PAIR_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Most request records `requests.recent` will return in one answer.
const REQUEST_RECENT_LIMIT: u64 = 200;

/// Connect + migrate the database, bind the sync endpoint, and run the stdio
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

    let pair_waiters: PairWaiters = crate::pairing::new_pair_waiters();

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
    let discovery = match bind_discovery(&config, discovery_secret).await {
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
    let offer_sink: OfferDecisionSink = Arc::new(move |decision| {
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

    // Built before the state: a base the TLS setup or the URL cannot serve is
    // a startup problem, and failing here names it once instead of on the
    // first code the reader asks for.
    let rendezvous = RendezvousClient::new(&config.rendezvous_url)
        .map_err(|error| ServerError::Config(format!("rendezvous client: {error}")))?;

    let state = Arc::new(DaemonState {
        config,
        pool,
        engine,
        device_id,
        device_name: Mutex::new(device_name),
        notifier,
        started_at: time::OffsetDateTime::now_utc(),
        pair_waiters,
        pair_decisions: new_pair_decisions(),
        offer_sink,
        write_gate: tokio::sync::Mutex::new(()),
        lifecycle: tokio::sync::Mutex::new(()),
        discovery: Mutex::new(discovery),
        advertiser: Mutex::new(None),
        rendezvous,
        request_log: new_request_log(),
        host: HostBridge::new(out_tx.clone()),
        requests_served: Arc::new(AtomicU64::new(0)),
        last_request_at: Arc::new(Mutex::new(None)),
    });

    let writer = tokio::spawn(writer_task(out_rx));

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
        livtet_sync_iroh::spawn_server(endpoint, crate::wire::handler(state.wire_state().await));
    }
    spawn_pull_loop(Arc::clone(&state));
    let _ = cleanup_old_pairings(&state).await;
    let _ = promote_mutual_pairings(&state).await;

    let loop_result = read_loop(&state, &out_tx).await;
    stop_advertiser(&state).await;
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

/// Bind the iroh endpoint on the configured UDP port.
///
/// Port `0` asks the OS for an ephemeral port. A failed bind is not fatal: the
/// caller logs it and keeps serving the RPC loop without a sync endpoint.
async fn bind_discovery(
    config: &ServerConfig,
    secret: SecretKey,
) -> Result<DiscoveryService, livtet_iroh::DiscoveryError> {
    DiscoveryService::bind_with_secret_and_port(secret, Some(config.port)).await
}

/// The UDP port the iroh endpoint actually bound.
///
/// Falls back to the configured port when discovery is unbound, so status stays
/// meaningful on a machine where the endpoint could not start.
async fn endpoint_port(state: &Arc<DaemonState>) -> u16 {
    state
        .discovery
        .lock()
        .await
        .as_ref()
        .and_then(|service| service.bound_port())
        .unwrap_or(state.config.port)
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
                // `health` stays inline too. It is a liveness probe with no
                // I/O, and dispatched concurrently it could run after a
                // `shutdown` on the next input line had already closed the
                // endpoint, reporting a port this daemon no longer serves.
                if request.method == rpc::method::HEALTH {
                    let response = dispatch(request, state, out_tx).await;
                    let _ = out_tx.send(rpc::encode_response(&response));
                    continue;
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
            // An answer to a request this daemon made (OPDS pass-through).
            Ok(rpc::RpcMessage::Response(response)) => state.host.answer(response),
            // Notifications from the parent carry nothing this daemon acts on.
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

impl From<livtet_mdns::MdnsError> for RpcFailure {
    fn from(error: livtet_mdns::MdnsError) -> Self {
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
        rpc::method::RENDEZVOUS_PUBLISH => finish(&request, handle_rendezvous_publish(state).await),
        rpc::method::PAIRING_CLAIM_CODE => {
            finish(&request, handle_pairing_claim_code(&request, state).await)
        }
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
        rpc::method::REQUESTS_RECENT => {
            finish(&request, handle_requests_recent(&request, state).await)
        }
        rpc::method::SERVER_REBIND => finish(&request, handle_server_rebind(&request, state).await),
        rpc::method::SHUTDOWN => finish(&request, handle_shutdown(state).await),
        other => rpc::RpcResponse::error(
            request.id.clone(),
            rpc::error_code::METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        ),
    }
}

async fn handle_health(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    let uptime = (time::OffsetDateTime::now_utc() - state.started_at).whole_seconds();
    Ok(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "uptime_secs": uptime,
        "node_id": discovery_node_id(state).await,
        "port": endpoint_port(state).await,
        "requests_served": state.requests_served.load(Ordering::Relaxed),
        "last_request_at": state.last_request_at.lock().await.clone(),
    }))
}

async fn handle_status(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::{paired_devices, pending_pairings};

    let db = state.engine.db();
    let latest_version = state.engine.get_latest_version().await?;
    let paired_device_count = paired_devices::Entity::find().count(db).await?;

    let pending_pairing_count = pending_pairings::Entity::find()
        .filter(pending_pairings::Column::StatusId.eq(DbId::from(PairingStatus::Pending)))
        .filter(pending_pairings::Column::ExpiresAt.gt(primitive_now()))
        .count(db)
        .await?;

    Ok(serde_json::json!({
        "device_id": state.device_id,
        "node_id": discovery_node_id(state).await,
        "port": endpoint_port(state).await,
        "latest_version": latest_version,
        "paired_device_count": paired_device_count,
        "pending_pairing_count": pending_pairing_count,
        "requests_served": state.requests_served.load(Ordering::Relaxed),
        "last_request_at": state.last_request_at.lock().await.clone(),
    }))
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
    // carry it, and remote peers dial it to file a claim. No token is minted
    // and no `pending_pairings` row is created here — rows only exist for
    // requests that actually arrived from another device.
    let device_type = params.device_type.unwrap_or_else(|| "unknown".to_string());
    let identity = match discovery_identity(state, &device_type).await {
        Some(identity) => identity,
        // Without a bound endpoint there is nothing for peers to discover, so
        // publish the device id rather than failing the move.
        None => PeerIdentity {
            node_id: NodeId(state.device_id.clone()),
            name: state.device_id.clone(),
            device_type,
        },
    };

    Ok(serde_json::json!({
        "node_id": identity.node_id.0,
        "identity": {
            "node_id": identity.node_id.0,
            "name": identity.name,
            "device_type": identity.device_type,
        },
        "expires_at": format_offset(expires),
    }))
}

async fn handle_pairing_list(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    use livtet_data::client_entities::pending_pairings;

    let rows: Vec<_> = pending_pairings::Entity::find()
        .filter(pending_pairings::Column::StatusId.eq(DbId::from(PairingStatus::Pending)))
        .filter(pending_pairings::Column::ExpiresAt.gt(primitive_now()))
        .all(state.engine.db())
        .await?;

    let list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "node_id": row.token,
                "desktop_id": row.desktop_id.to_string(),
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

    stop_advertiser(state).await;
    advertise(state).await;

    Ok(serde_json::json!({ "name": name }))
}

async fn handle_pairing_approve(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let params: TokenParams = parse_params(request)?;
    let wire = state.wire_state().await;
    let approval = crate::pairs::approve_claim(&wire, &params.token)
        .await
        .map_err(claim_failure)?;
    Ok(serde_json::json!({
        "device_id": approval.device_id,
        "session_token": approval.session_token,
    }))
}

async fn handle_pairing_reject(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let params: TokenParams = parse_params(request)?;
    let wire = state.wire_state().await;
    crate::pairs::reject_claim(&wire, &params.token)
        .await
        .map_err(claim_failure)?;
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

/// Publish this device under a short code the reader can type elsewhere.
///
/// The endpoint is bound on demand, as `pairing.begin` does: a code that names
/// a node nothing is listening on would pair with nobody, and the bind is what
/// turns the device id into an address the other device can dial.
async fn handle_rendezvous_publish(
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let _ = discovery_identity(state, "desktop").await;
    let node_id = discovery_node_id(state).await;
    let name = advertised_name(state).await;
    let published = state
        .rendezvous
        .publish(&node_id, Some(name.as_str()), "desktop")
        .await
        .map_err(|error| RpcFailure::from(ServerError::Server(error.to_string())))?;
    Ok(serde_json::json!({
        "code": published.code,
        "expires_at": published.expires_at,
    }))
}

/// Pair with the device a short code names.
///
/// The code is only a lookup: resolving it yields a node id, and from there the
/// claim is the same iroh exchange a discovered peer gets — the rendezvous
/// never carries the pairing itself.
async fn handle_pairing_claim_code(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let params: PairingClaimCodeParams = parse_params(request)?;
    let code = livtet_rendezvous::normalize_code(&params.code).map_err(code_failure)?;
    let peer = state
        .rendezvous
        .resolve(&code)
        .await
        .map_err(code_failure)?;

    let claim = PairingClaimParams {
        node_id: peer.node_id,
        device_name: params.device_name.or(peer.name),
    };
    let state = Arc::clone(state);
    // Same shape as `pairing.claim`: the reader on the far side is the one who
    // decides, so this returns and the answer arrives as a notification.
    tokio::spawn(async move {
        if let Err(error) = claim_peer(&state, claim).await {
            tracing::warn!(%error, "pairing claim from a code failed");
        }
    });
    Ok(serde_json::json!({ "status": "pending" }))
}

/// A code the platform would not accept or resolve.
///
/// `invalid_params`, not an internal error: nothing here failed — the code is
/// simply not one, or no longer one, the platform knows. The client's own
/// message is forwarded verbatim so the UI can say "that code has expired"
/// rather than invent wording for it.
fn code_failure(error: livtet_rendezvous::RendezvousError) -> RpcFailure {
    invalid_params(error.to_string())
}

/// Claim a discovered peer, poll until it decides, and store the token it
/// issues us as an outbound paired device.
///
/// The claim is two-phase: `Pair` answers with a claim secret immediately, and
/// `PairStatus` carries the decision. No connection has to survive the reader's
/// thinking time, so a dropped stream is retried rather than read as a refusal.
async fn claim_peer(
    state: &Arc<DaemonState>,
    params: PairingClaimParams,
) -> Result<(), ServerError> {
    use livtet_data::client_entities::paired_devices;
    use livtet_data::orm::{EntityTrait, Set, sea_query::OnConflict};
    use livtet_sync_iroh::{Client, PairingOutcome};

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
    let client = Client::new(endpoint, params.node_id.clone());

    // Who is on the other end? The connection authenticates the node id, and
    // the identity frame names the device id the pairing row is keyed by.
    let identity = client
        .identity()
        .await
        .map_err(|error| ServerError::Server(format!("peer identity: {error}")))?;
    let peer_device_id = identity
        .device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(&identity.device_id).ok())
        .ok_or_else(|| {
            ServerError::Server(format!("invalid peer device id: {}", identity.device_id))
        })?;

    let our_name = state
        .device_name
        .lock()
        .await
        .clone()
        .unwrap_or_else(|| livtet_mdns::friendly_name(&state.device_id));
    // This device's half of the pair: the approver stores it as its outbound
    // credential, and this device keeps it as its inbound token.
    let claimant_token = DbId::new().to_string();
    let claim = match client
        .pair(&state.device_id, &our_name, "desktop", &claimant_token)
        .await
        .map_err(|error| ServerError::Server(format!("pairing claim: {error}")))?
    {
        PairingOutcome::Pending { claim } => claim,
        PairingOutcome::Refused { reason } => {
            return Err(ServerError::Server(format!(
                "pairing was refused: {reason}"
            )));
        }
        PairingOutcome::Paired { .. } => {
            return Err(ServerError::Server(
                "pairing claim: the peer answered with a decision and no claim".to_string(),
            ));
        }
    };

    // The reader is human-paced, so the claim is polled rather than held open.
    let deadline = tokio::time::Instant::now() + PAIR_POLL_WINDOW;
    let session_token = loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(ServerError::Server(
                "the pairing was not decided in time".to_string(),
            ));
        }
        tokio::time::sleep(PAIR_POLL_INTERVAL).await;
        match client.pair_status(&claim).await {
            Ok(PairingOutcome::Paired { session_token, .. }) if !session_token.is_empty() => {
                break session_token;
            }
            Ok(PairingOutcome::Paired { .. }) => {
                return Err(ServerError::Server(
                    "approval carried no session token".to_string(),
                ));
            }
            Ok(PairingOutcome::Refused { reason }) => {
                return Err(ServerError::Server(format!(
                    "pairing was refused: {reason}"
                )));
            }
            // Still pending, or the poll itself was rate-limited or lost its
            // connection: the decision stays resumable, so try again.
            Ok(PairingOutcome::Pending { .. }) | Err(_) => continue,
        }
    };

    let db = state.engine.db();
    let model = paired_devices::ActiveModel {
        device_id: Set(peer_device_id),
        name: Set(params.device_name.clone()),
        // The peer's node id is the address; nothing listens on a stored one.
        listen_on: Set(None),
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

    // `peer_session_token` and `node_id` are not on the entity; write them through
    // the pool. This token was issued by the peer, so it only authenticates when
    // we dial them — it must not sit in `session_token`, which inbound requests
    // are checked against.
    livtet_data::sql::query(
        "UPDATE paired_devices SET session_token = ?, peer_session_token = ?, node_id = ? \
         WHERE device_id = ?",
    )
    .bind(claimant_token)
    .bind(session_token)
    .bind(params.node_id.clone())
    .bind(peer_device_id.to_bytes().to_vec())
    .execute(&state.pool)
    .await
    .map_err(|error| ServerError::Server(format!("storing the peer's session token: {error}")))?;

    requests::notify(
        &state.notifier,
        rpc::method::NOTIFY_PAIRING_CLAIMED,
        serde_json::json!({
            "device_id": peer_device_id.to_string(),
            "name": params.device_name,
            "node_id": params.node_id,
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
    let params: OfferBookParams = parse_params(request)?;
    let wire = state.wire_state().await;
    let offer = crate::pairs::offer_book(&wire, &params.edition_id, &params.device_id)
        .await
        .map_err(claim_failure)?;
    Ok(serde_json::json!({
        "offer_id": offer.offer_id,
        "inventory_id": offer.inventory_id,
        "device_id": params.device_id,
    }))
}

/// Map a shared pairing or offer failure onto the error the desktop's UI reads.
fn claim_failure(error: crate::pairs::ClaimError) -> RpcFailure {
    use crate::pairs::ClaimError;
    match error {
        ClaimError::Db(message) => RpcFailure {
            code: rpc::error_code::INTERNAL_ERROR,
            message,
        },
        other => invalid_params(other.to_string()),
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
///
/// The transport dials the peer's node id, which iroh resolves on every call,
/// so there is no stored address to correct or retry.
async fn pull_device(
    state: &Arc<DaemonState>,
    device_id: DbId,
    background: bool,
) -> Result<serde_json::Value, ServerError> {
    let outcome = pull_device_once(state, device_id, background).await?;

    report_pulled(state, device_id, &outcome);
    Ok(outcome)
}

/// One pull pass against an outbound peer, dialling its node id.
async fn pull_device_once(
    state: &Arc<DaemonState>,
    device_id: DbId,
    background: bool,
) -> Result<serde_json::Value, ServerError> {
    use livtet_data::client_entities::paired_devices;
    use livtet_data::orm::{EntityTrait, Set};
    use livtet_sync::PullResponse;

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
    // has to be reachable.
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
                    .apply_changes(metadata)
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

/// Where an accepted offer's bytes were written on this device.
///
/// Not the sender's paths: `digital_inventory` is device-specific and no longer
/// synced, so the receiver's paths are the only ones that exist here.
struct DownloadedPaths {
    file_path: String,
    cover_path: Option<String>,
}

/// Fetch one offered file (and its cover) into the receiver's directories.
async fn download_offer(
    peer: &crate::peer::Peer,
    payload: &serde_json::Value,
    books_dir: &camino::Utf8Path,
    covers_dir: &camino::Utf8Path,
) -> Result<(String, DownloadedPaths), ServerError> {
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
        && let Some((content_type, bytes)) = peer.cover(inventory_id).await?
    {
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

    Ok((
        inventory_id.to_string(),
        DownloadedPaths {
            file_path: file_path.to_string(),
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
                .apply_offer_changes(held.clone())
                .await
                .map_err(ServerError::from)?;
            applied = outcome.applied;
            conflicts = outcome.conflicts.len();
        }

        // `digital_inventory` is not synced, so accepting is where this device records
        // its own copy. The hash and size describe the file and come from the held row;
        // the paths are the ones just written here.
        let (file_hash, file_size_bytes) = held
            .iter()
            .find(|change| change.entity_type == "digital_inventory")
            .and_then(|change| serde_json::from_str::<serde_json::Value>(&change.payload).ok())
            .map(|payload| {
                (
                    payload
                        .get("file_hash")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    payload
                        .get("file_size_bytes")
                        .and_then(serde_json::Value::as_i64),
                )
            })
            .unwrap_or((None, None));
        livtet_sync::adopt_inventory(
            state.engine.db(),
            DbId::from_hex(&inventory_id)
                .map_err(|error| ServerError::Server(format!("inventory id: {error}")))?,
            DbId::from_hex(&row.edition_id)
                .map_err(|error| ServerError::Server(format!("edition id: {error}")))?,
            &paths.file_path,
            paths.cover_path.as_deref(),
            file_hash.as_deref(),
            file_size_bytes,
        )
        .await
        .map_err(ServerError::from)?;
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

    // Remove the peer side first, while the outbound token still exists. This
    // is best-effort: an offline peer must not block the local revoke.
    use livtet_data::sql::Row;
    let node_id: Option<String> =
        livtet_data::sql::query("SELECT node_id FROM paired_devices WHERE device_id = ?")
            .bind(device_id.to_bytes().to_vec())
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
            .and_then(|row| row.try_get::<Option<String>, _>("node_id").ok().flatten());
    let mut remote_removed = false;
    if node_id.is_some()
        && let Ok(client) = crate::peer::peer_for(state, device_id).await
    {
        remote_removed = client.revoke().await.unwrap_or(false);
    }

    let result = paired_devices::Entity::delete_by_id(device_id)
        .exec(state.engine.db())
        .await?;

    if let Some(node_id) = node_id {
        let _ = livtet_data::sql::query(
            "DELETE FROM pending_pairings WHERE token = ? OR device_id = ?",
        )
        .bind(node_id)
        .bind(device_id.to_bytes().to_vec())
        .execute(&state.pool)
        .await;
    }

    Ok(serde_json::json!({
        "ok": true,
        "removed": result.rows_affected,
        "remote_removed": remote_removed,
    }))
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

/// The most recent answered sync requests, newest first.
async fn handle_requests_recent(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let params: RequestsRecentParams = parse_params(request)?;
    let limit = params.limit.unwrap_or(50).min(REQUEST_RECENT_LIMIT) as usize;
    Ok(serde_json::to_value(requests::recent(
        &state.request_log,
        limit,
    ))?)
}

/// Rebind the iroh endpoint on a new UDP port without restarting the daemon.
///
/// The old endpoint is closed first: a failed bind must not leave this daemon
/// advertising an endpoint it no longer serves. When the bind fails there is no
/// endpoint left, and the error says so.
async fn handle_server_rebind(
    request: &rpc::RpcRequest,
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value, RpcFailure> {
    let _lifecycle = state.lifecycle.lock().await;
    let params: RebindParams = parse_params(request)?;

    stop_advertiser(state).await;
    if let Some(service) = state.discovery.lock().await.take() {
        service.shutdown().await;
    }

    match DiscoveryService::bind_with_secret_and_port(discovery_secret_key(), Some(params.port))
        .await
    {
        Ok(service) => {
            let service = Arc::new(service);
            let endpoint = service.endpoint().clone();
            let node_id = service.node_id().0;
            let port = service.bound_port().unwrap_or(params.port);
            *state.discovery.lock().await = Some(Arc::clone(&service));
            livtet_sync_iroh::spawn_server(
                endpoint,
                crate::wire::handler(state.wire_state().await),
            );
            advertise(state).await;
            tracing::info!(%node_id, port, "sync endpoint rebound");
            Ok(serde_json::json!({ "node_id": node_id, "port": port }))
        }
        Err(error) => Err(RpcFailure {
            code: rpc::error_code::INTERNAL_ERROR,
            message: format!(
                "could not bind the sync endpoint on port {}: {error}; no endpoint is bound",
                params.port
            ),
        }),
    }
}

async fn handle_shutdown(state: &Arc<DaemonState>) -> Result<serde_json::Value, RpcFailure> {
    // Wait for an in-flight `server.rebind` to finish storing its endpoint, so
    // the close below covers the endpoint that is actually current.
    let _lifecycle = state.lifecycle.lock().await;
    stop_advertiser(state).await;
    shutdown_discovery(state).await;
    Ok(serde_json::json!({ "ok": true }))
}

fn primitive_now() -> time::PrimitiveDateTime {
    let now = time::OffsetDateTime::now_utc();
    time::PrimitiveDateTime::new(now.date(), now.time())
}

pub(crate) fn now_string() -> String {
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
/// advertise. A failed bind is not fatal: the daemon logs it and pairing is
/// unavailable until the endpoint can bind.
async fn discovery_identity(state: &Arc<DaemonState>, device_type: &str) -> Option<PeerIdentity> {
    let mut slot = state.discovery.lock().await;
    if slot.is_none() {
        match DiscoveryService::bind_with_secret_and_port(
            discovery_secret_key(),
            Some(state.config.port),
        )
        .await
        {
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

/// The name this device advertises: the chosen one, else the generated
/// fallback.
///
/// Normalized the way `livtet_mdns` does it — trimmed, blank counts as absent —
/// so a rendezvous code carries the same name a nearby peer sees on this
/// device, rather than the raw setting.
async fn advertised_name(state: &Arc<DaemonState>) -> String {
    state
        .device_name
        .lock()
        .await
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| livtet_mdns::friendly_name(&state.device_id))
}

/// Announce the sync endpoint for app discovery.
///
/// The record carries the node id a client pairs with, the device name and the
/// version. The SRV port is the endpoint's UDP port; it is informational, and
/// nothing dials it as an HTTP surface.
async fn advertise(state: &Arc<DaemonState>) {
    let (ip, port) = {
        let slot = state.discovery.lock().await;
        slot.as_ref()
            .and_then(|service| {
                service
                    .bound_sockets()
                    .into_iter()
                    .find(std::net::SocketAddr::is_ipv4)
            })
            .map(|addr| (addr.ip(), addr.port()))
            .unwrap_or((
                std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
                state.config.port,
            ))
    };
    let node_id = discovery_node_id(state).await;
    let device_name = state.device_name.lock().await.clone();
    match Advertiser::start(
        &node_id,
        &state.device_id,
        device_name.as_deref(),
        "desktop",
        ip,
        port,
    ) {
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
/// The pull runs on the daemon's own clock, so nothing else would tell the UI.
/// Only a pull that applied rows is reported — a pull that found nothing
/// changed the catalog by definition not at all, and a UI refetch would be pure
/// noise.
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

/// Make pairings that already name a node id pull-capable.
///
/// Approval used to write `outbound = false`, which `pull_device` refuses, and
/// the transport change means only a row with a node id can be dialled. The
/// flag only gates pulling, so flipping it on those rows grants the missing
/// half without re-pairing anything.
async fn promote_mutual_pairings(state: &Arc<DaemonState>) -> Result<(), ServerError> {
    // Only a row that names the peer's node id can be dialled. Rows from the
    // HTTP transport have no node id and are left alone: promoting them would
    // make every pull fail on the missing address.
    let updated = livtet_data::sql::query(
        "UPDATE paired_devices SET outbound = 1 \
         WHERE outbound = 0 AND session_token IS NOT NULL AND session_token != '' \
           AND node_id IS NOT NULL AND node_id != ''",
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
