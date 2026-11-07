//! Pairing over iroh, exposed to the mobile clients.
//!
//! Pairing never touches HTTP and never needs a host or port: the peer is
//! addressed by its iroh node id, which the `_livtet-sync._tcp` TXT `id` record
//! carries. Dialling resolves the addresses of that id (mDNS on the LAN, the
//! relay off it), so the phone can pair with a desktop it can only reach
//! outbound.
//!
//! The endpoint's secret key is persisted at the path the caller names, because
//! the node id derived from it *is* this device's identity: the desktop records
//! the connection's node id when the claim is filed, and every later
//! `pair_status` poll must present the same one.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use camino::Utf8Path;
use livtet_core::data::SharedState;
use livtet_core::data::migrator::Kind;
use livtet_iroh::{DiscoveryService, SecretKey};
use livtet_mdns::Advertiser;
use livtet_sync::SyncEngine;
use livtet_sync_server::wire_state::WireState;
use livtet_sync_iroh::{
    Client, PairingOutcome, spawn_server,
};

use crate::error::LivtetError;

/// How long to wait between `pair_status` polls while the reader decides.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// What became of a pairing attempt.
///
/// `Pending` is not a failure: the claim is filed and the desktop has not
/// answered yet, so the caller can wait for the reader or poll again with
/// [`sync_pairing_status`] using the same `claim`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SyncPairingResult {
    /// The desktop approved; `session_token` authenticates later sync requests.
    Paired {
        device_id: String,
        session_token: String,
    },
    /// The desktop declined, or the claim could not be filed.
    Refused { reason: String },
    /// Filed and undecided; `claim` is what [`sync_pairing_status`] continues.
    Pending { claim: String },
}

/// File a pairing claim with `peer_node_id` and wait for the reader's decision.
///
/// Binds a sync endpoint using (or creating) the secret key at
/// `secret_key_path`, files the claim, then polls until the decision lands or
/// `wait_secs` elapses. A `Pending` result leaves the claim valid on the peer.
#[uniffi::export(async_runtime = "tokio")]
pub async fn pair_sync_device(
    secret_key_path: String,
    peer_node_id: String,
    device_id: String,
    name: String,
    device_type: String,
    wait_secs: u32,
) -> Result<SyncPairingResult, LivtetError> {
    let client = connect(secret_key_path, peer_node_id).await?;
    let outcome = client
        .pair(&device_id, &name, &device_type)
        .await
        .map_err(|error| LivtetError::Sync(format!("could not file the pairing claim: {error}")))?;

    match outcome {
        PairingOutcome::Pending { claim } => await_decision(&client, claim, wait_secs).await,
        PairingOutcome::Paired {
            device_id,
            session_token,
        } => Ok(SyncPairingResult::Paired {
            device_id,
            session_token,
        }),
        PairingOutcome::Refused { reason } => Ok(SyncPairingResult::Refused { reason }),
    }
}

/// Continue waiting on a claim already filed by [`pair_sync_device`].
///
/// Rebinding derives the same node id from the same persisted key, which is
/// what the peer keyed the claim on.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_pairing_status(
    secret_key_path: String,
    peer_node_id: String,
    claim: String,
    wait_secs: u32,
) -> Result<SyncPairingResult, LivtetError> {
    let client = connect(secret_key_path, peer_node_id).await?;
    await_decision(&client, claim, wait_secs).await
}

/// What a peer says about itself, as returned by [`peer_sync_identity`].
///
/// The node id is the pairing key already known to the caller; the device id is
/// the peer's own ULID, which is what a paired-devices list needs to show.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncPeerIdentity {
    pub node_id: String,
    pub device_id: String,
    pub protocol: u32,
}

/// Ask `node_id` who it is, over the sync protocol.
///
/// Pairing replies with the *claiming* device's id, not the peer's, so the app
/// uses this to learn the identity of the device it just paired with. Failure is
/// not fatal to pairing: the caller keeps the node id and shows what it has.
#[uniffi::export(async_runtime = "tokio")]
pub async fn peer_sync_identity(
    secret_key_path: String,
    peer_node_id: String,
) -> Result<SyncPeerIdentity, LivtetError> {
    let client = connect(secret_key_path, peer_node_id).await?;
    let identity = client
        .identity()
        .await
        .map_err(|error| LivtetError::Sync(format!("could not read the peer identity: {error}")))?;
    Ok(SyncPeerIdentity {
        node_id: identity.node_id,
        device_id: identity.device_id,
        protocol: identity.protocol,
    })
}

/// Bind a sync endpoint from the persisted key and aim a client at `node_id`.
pub(crate) async fn connect(
    secret_key_path: String,
    peer_node_id: String,
) -> Result<Client, LivtetError> {
    let secret = load_or_create_secret(Utf8Path::new(&secret_key_path))?;
    let discovery = DiscoveryService::bind_with_secret(secret)
        .await
        .map_err(|error| {
            LivtetError::Sync(format!("could not start the sync endpoint: {error}"))
        })?;
    Ok(Client::new(discovery.endpoint().clone(), peer_node_id))
}

/// Poll `pair_status` until the peer decides or `wait_secs` runs out.
async fn await_decision(
    client: &Client,
    first_claim: String,
    wait_secs: u32,
) -> Result<SyncPairingResult, LivtetError> {
    let deadline = Instant::now() + Duration::from_secs(u64::from(wait_secs));
    let mut claim = first_claim;

    loop {
        if Instant::now() >= deadline {
            return Ok(SyncPairingResult::Pending { claim });
        }
        tokio::time::sleep(POLL_INTERVAL).await;

        match client.pair_status(&claim).await.map_err(|error| {
            LivtetError::Sync(format!("could not read the pairing status: {error}"))
        })? {
            PairingOutcome::Pending { claim: next } => claim = next,
            PairingOutcome::Paired {
                device_id,
                session_token,
            } => {
                return Ok(SyncPairingResult::Paired {
                    device_id,
                    session_token,
                });
            }
            PairingOutcome::Refused { reason } => return Ok(SyncPairingResult::Refused { reason }),
        }
    }
}

/// Read the 32-byte iroh secret key at `path`, creating it on first use.
///
/// Stored as hex, the same form the desktop daemon keeps in the OS keyring. The
/// file holds the device's node identity, so it is never rewritten once valid;
/// a malformed file is an error rather than a silent regeneration, which would
/// strand the desktop's record of this device under an unreachable node id.
fn load_or_create_secret(path: &Utf8Path) -> Result<SecretKey, LivtetError> {
    if path.exists() {
        let encoded = std::fs::read_to_string(path)
            .map_err(|error| LivtetError::Sync(format!("could not read {path}: {error}")))?;
        let bytes: [u8; 32] = hex::decode(encoded.trim())
            .ok()
            .and_then(|decoded| decoded.try_into().ok())
            .ok_or_else(|| {
                LivtetError::Sync(format!("{path} does not hold a 32-byte hex secret key"))
            })?;
        return Ok(SecretKey::from_bytes(&bytes));
    }

    let secret = SecretKey::generate();
    if let Some(parent) = path.parent().filter(|parent| !parent.as_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| LivtetError::Sync(format!("could not create {parent}: {error}")))?;
    }
    std::fs::write(path, hex::encode(secret.to_bytes()))
        .map_err(|error| LivtetError::Sync(format!("could not write {path}: {error}")))?;
    Ok(secret)
}

/// The advertisement this process is running, keyed by secret key path.
static ADVERTISED: LazyLock<Mutex<HashMap<String, RunningAdvertisement>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A bound endpoint, its mDNS record, and the task answering connections.
struct RunningAdvertisement {
    node_id: String,
    discovery: DiscoveryService,
    advertiser: Advertiser,
    serving: tokio::task::JoinHandle<()>,
    /// The wire this endpoint answers from, so a decision made on this device
    /// reaches the claimant polling the same endpoint.
    wire: Arc<WireState>,
}

/// Announce this device on `_livtet-sync._tcp` so peers list it.
///
/// Returns the node id the advertisement carries. It comes from the same
/// persisted key that pairing uses, so a desktop that discovers this device and
/// a reader who types the id by hand are naming one identity.
///
/// Idempotent per `secret_key_path`: a second call returns the id already
/// advertised instead of binding another endpoint, so re-opening the screen
/// does not leave duplicate records on the network.
///
/// The endpoint answers the sync wire from this device's own library, so a peer
/// that claims it — the desktop, to pull books the phone holds — gets the same
/// authorization and file handling the desktop's own daemon serves. Claims wait
/// on this device's reader: [`sync_pending_pairings`] lists them and
/// [`sync_decide_pairing`] answers.
#[uniffi::export(async_runtime = "tokio")]
pub async fn advertise_sync_device(
    secret_key_path: String,
    db_path: String,
    device_id: String,
    name: String,
    device_type: String,
) -> Result<String, LivtetError> {
    if let Ok(hosts) = ADVERTISED.lock()
        && let Some(host) = hosts.get(&secret_key_path)
    {
        return Ok(host.node_id.clone());
    }

    let secret = load_or_create_secret(Utf8Path::new(&secret_key_path))?;
    let discovery = DiscoveryService::bind_with_secret(secret)
        .await
        .map_err(|error| {
            LivtetError::Sync(format!("could not start the sync endpoint: {error}"))
        })?;
    let node_id = discovery.node_id().as_str().to_string();
    let (ip, port) = advertised_endpoint(&discovery);

    let wire = Arc::new(
        wire_state(&db_path, &device_id, Some(node_id.clone()))
            .await?
            .counting(),
    );
    let serving = spawn_server(
        discovery.endpoint().clone(),
        livtet_sync_server::wire::handler(Arc::clone(&wire)),
    );

    let advertiser = Advertiser::start(&node_id, &device_id, Some(&name), &device_type, ip, port)
        .map_err(|error| {
        LivtetError::Sync(format!("could not advertise this device: {error}"))
    })?;

    if let Ok(mut hosts) = ADVERTISED.lock() {
        hosts.insert(
            secret_key_path,
            RunningAdvertisement {
                node_id: node_id.clone(),
                discovery,
                advertiser,
                serving,
                wire,
            },
        );
    }
    Ok(node_id)
}

/// Withdraw the advertisement and stop answering. A no-op when nothing is
/// advertised for this key path.
#[uniffi::export]
pub fn stop_advertising_sync_device(secret_key_path: String) {
    let running = ADVERTISED
        .lock()
        .ok()
        .and_then(|mut hosts| hosts.remove(&secret_key_path));
    if let Some(running) = running {
        running.advertiser.shutdown();
        running.serving.abort();
        // Dropping the endpoint's last handle closes the socket, which unwinds
        // the accept loop the aborted task was driving.
        drop(running.discovery);
    }
}

/// One peer waiting on this device's reader to approve a pairing.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncPendingPairing {
    /// The claiming peer's node id; what [`sync_decide_pairing`] is addressed to.
    pub node_id: String,
    /// The claimant's own device id, when it sent one.
    pub device_id: Option<String>,
    /// The name it advertises.
    pub name: Option<String>,
}

/// One device this endpoint has paired with.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncPairedDevice {
    /// The peer's device id: what [`sync_send_book`] is addressed to.
    pub device_id: String,
    /// The peer's iroh node id, when the pairing recorded one.
    pub node_id: Option<String>,
    /// The name it advertises.
    pub name: Option<String>,
    /// Whether this device may dial the peer, or only be dialled by it.
    pub outbound: bool,
}

/// A book this device offered to a peer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncSentBook {
    /// The offer id the peer echoes when it decides.
    pub offer_id: String,
    /// The inventory row whose bytes the peer will fetch from this device.
    pub inventory_id: String,
}

/// The wire for an advertised endpoint.
fn advertisement(secret_key_path: &str) -> Result<Arc<WireState>, LivtetError> {
    ADVERTISED
        .lock()
        .ok()
        .and_then(|hosts| hosts.get(secret_key_path).map(|host| Arc::clone(&host.wire)))
        .ok_or_else(|| {
            LivtetError::Sync(
                "this device is not advertising a sync endpoint; call \
                 advertise_sync_device first"
                    .to_string(),
            )
        })
}

/// The peers waiting on this device's reader to approve a pairing.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_pending_pairings(
    secret_key_path: String,
) -> Result<Vec<SyncPendingPairing>, LivtetError> {
    let wire = advertisement(&secret_key_path)?;
    let claims = livtet_sync_server::pairs::pending_claims(&wire)
        .await
        .map_err(|error| LivtetError::Sync(error.to_string()))?;
    Ok(claims
        .into_iter()
        .map(|claim| SyncPendingPairing {
            node_id: claim.node_id,
            device_id: claim.device_id,
            name: claim.name,
        })
        .collect())
}

/// Answer the claim `node_id` filed.
///
/// Approving mints the session token the peer presents on every later request
/// and returns it, so the caller can record which device it just let in. A
/// rejection returns `None`; the claimant's next poll learns it was refused.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_decide_pairing(
    secret_key_path: String,
    node_id: String,
    accept: bool,
) -> Result<Option<String>, LivtetError> {
    let wire = advertisement(&secret_key_path)?;
    if accept {
        let approval = livtet_sync_server::pairs::approve_claim(&wire, &node_id)
            .await
            .map_err(|error| LivtetError::Sync(error.to_string()))?;
        Ok(Some(approval.session_token))
    } else {
        livtet_sync_server::pairs::reject_claim(&wire, &node_id)
            .await
            .map_err(|error| LivtetError::Sync(error.to_string()))?;
        Ok(None)
    }
}

/// The devices this endpoint has paired with.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_paired_devices(
    secret_key_path: String,
) -> Result<Vec<SyncPairedDevice>, LivtetError> {
    use livtet_core::data::orm::EntityTrait;

    let wire = advertisement(&secret_key_path)?;
    let rows = livtet_core::data::client_entities::paired_devices::Entity::find()
        .all(wire.engine.db())
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| SyncPairedDevice {
            device_id: row.device_id.to_string(),
            node_id: None,
            name: row.name,
            outbound: row.outbound,
        })
        .collect())
}

/// Offer `edition_id`'s file to a paired device.
///
/// The offer is a change-log row here, so it travels the same version cursor the
/// peer already pulls: the next pull it makes from this device carries the offer
/// and the catalog rows that describe it. The peer then fetches the bytes from
/// this device, which is why the pairing has to be one it can dial.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_send_book(
    secret_key_path: String,
    edition_id: String,
    device_id: String,
) -> Result<SyncSentBook, LivtetError> {
    let wire = advertisement(&secret_key_path)?;
    let offer = livtet_sync_server::pairs::offer_book(&wire, &edition_id, &device_id)
        .await
        .map_err(|error| LivtetError::Sync(error.to_string()))?;
    Ok(SyncSentBook {
        offer_id: offer.offer_id,
        inventory_id: offer.inventory_id,
    })
}

/// This device's pairing link, and the code a QR encoder renders.
///
/// The format lives in `livtet-sync` so every client writes and reads the same
/// string; see [`PairLink`](livtet_sync::PairLink) for the shapes it accepts.
#[uniffi::export]
pub fn pair_link(node_id: String, name: Option<String>) -> String {
    livtet_sync::PairLink::new(node_id, name).to_uri()
}

/// A pairing link taken apart.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncPairLink {
    /// The protocol version the link was written for.
    pub version: u16,
    /// The node to dial.
    pub node_id: String,
    /// The name the code advertised, when it carried one.
    pub name: Option<String>,
}

/// Read a pairing link a scanner handed over.
///
/// Accepts the current `livtet://sync?v=2&nodeId=` form, the older
/// `livtet://pair?node=` one, and the web form of either.
#[uniffi::export]
pub fn parse_pair_link(link: String) -> Result<SyncPairLink, LivtetError> {
    let parsed = livtet_sync::PairLink::parse(&link)
        .map_err(|error| LivtetError::InvalidInput(error.to_string()))?;
    Ok(SyncPairLink {
        version: parsed.version,
        node_id: parsed.node_id,
        name: parsed.name,
    })
}

/// The wire state an advertised endpoint answers from.
///
/// Opens the library the same way a pull does — business and client migrations
/// over one connection — so the engine, the pool the pairing columns live in,
/// and the file paths all describe the library the reader sees.
async fn wire_state(
    db_path: &str,
    device_id: &str,
    node_id: Option<String>,
) -> Result<WireState, LivtetError> {
    let shared = SharedState::connect(db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    // `SyncEngine` works over SeaORM's connection, not the raw sqlx pool.
    let db =
        livtet_core::data::orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(shared.pool.clone());
    let engine = SyncEngine::new(db, device_id.to_string());
    let state = WireState::new(engine, shared.pool.clone(), device_id).counting();
    Ok(match node_id {
        Some(node_id) => state.with_node_id(node_id),
        None => state,
    })
}

/// The address and port to publish: the endpoint's IPv4 socket when it bound
/// one, else whatever it bound, else "all interfaces" at the parsed port.
fn advertised_endpoint(discovery: &DiscoveryService) -> (std::net::IpAddr, u16) {
    let sockets = discovery.bound_sockets();
    let chosen = sockets
        .iter()
        .find(|socket| socket.is_ipv4())
        .or_else(|| sockets.first());
    match chosen {
        Some(socket) => (socket.ip(), socket.port()),
        None => (std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use livtet_sync_iroh::{Handler, Identity, Outcome, PROTOCOL_VERSION, Request, Response};

    /// How many `PairStatus` polls this fake peer answers `Pending` before approving.
    const DECIDES_AFTER_POLLS: u32 = 2;

    /// A slow reader is normal, so a claim has to survive being asked about again.
    ///
    /// The app loops `sync_pairing_status` while it waits, one call per round.
    /// This peer stays undecided for the first rounds and only then approves, so
    /// the test fails if polling restarts the claim, loses it, or reports the
    /// first answer forever.
    #[tokio::test]
    async fn a_pending_claim_keeps_polling_until_the_peer_approves() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let peer = DiscoveryService::bind().await.expect("peer endpoint");
        let peer_node_id = peer.node_id().as_str().to_string();
        let polls = Arc::new(AtomicU32::new(0));

        let handler: Handler = Arc::new({
            let polls = Arc::clone(&polls);
            move |request, _remote| {
                let polls = Arc::clone(&polls);
                Box::pin(async move {
                    match request {
                        Request::Pair { .. } => Outcome::reply(Response::PairingPending {
                            claim: "claim-9".to_string(),
                        }),
                        Request::PairStatus { .. } => {
                            let round = polls.fetch_add(1, Ordering::SeqCst);
                            if round < DECIDES_AFTER_POLLS {
                                Outcome::reply(Response::PairingPending {
                                    claim: "claim-9".to_string(),
                                })
                            } else {
                                Outcome::reply(Response::Paired {
                                    device_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
                                    session_token: "token-after-wait".to_string(),
                                })
                            }
                        }
                        other => Outcome::reply(Response::Refused {
                            reason: format!("unexpected request: {other:?}"),
                        }),
                    }
                })
            }
        });
        let _serving = spawn_server(peer.endpoint().clone(), handler);

        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let key_path = dir.path().join("sync.key").to_string();
        let filed = pair_sync_device(
            key_path.clone(),
            peer_node_id.clone(),
            "01ARZ3NDEKTSV4RRFFQ69G5FAW".to_string(),
            "Reader's phone".to_string(),
            "android".to_string(),
            0,
        )
        .await
        .expect("pair call");
        let SyncPairingResult::Pending { claim } = filed else {
            panic!("a zero-second wait should return the claim, not {filed:?}");
        };

        let first = sync_pairing_status(key_path.clone(), peer_node_id.clone(), claim.clone(), 1)
            .await
            .expect("first round");
        assert!(
            matches!(first, SyncPairingResult::Pending { .. }),
            "an undecided peer must stay pending, got {first:?}"
        );

        let mut claim = match first {
            SyncPairingResult::Pending { claim } => claim,
            other => panic!("expected pending, got {other:?}"),
        };
        for _ in 0..8 {
            match sync_pairing_status(key_path.clone(), peer_node_id.clone(), claim.clone(), 2)
                .await
                .expect("later round")
            {
                SyncPairingResult::Pending { claim: next } => claim = next,
                SyncPairingResult::Paired { session_token, .. } => {
                    assert_eq!(session_token, "token-after-wait");
                    return;
                }
                other => panic!("peer refused unexpectedly: {other:?}"),
            }
        }
        panic!("the approval never arrived across polling rounds");
    }

    /// A paired-devices row must show the peer's own device id, not the phone's.
    ///
    /// The pairing reply carries the claiming device's id, which is why the app
    /// asks `peer_sync_identity` separately; this pins the value that comes back.
    #[tokio::test]
    async fn the_peer_identity_reports_the_peers_own_device_id() {
        let peer = DiscoveryService::bind().await.expect("peer endpoint");
        let peer_node_id = peer.node_id().as_str().to_string();

        let handler: Handler = Arc::new(|request, _remote| {
            Box::pin(async move {
                match request {
                    Request::Identity => Outcome::reply(Response::Identity(Identity {
                        protocol: PROTOCOL_VERSION,
                        node_id: "aa11bb22cc33dd44ee55ff6677889900".to_string(),
                        device_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
                    })),
                    other => Outcome::reply(Response::Refused {
                        reason: format!("unexpected request: {other:?}"),
                    }),
                }
            })
        });
        let _serving = spawn_server(peer.endpoint().clone(), handler);

        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let identity = peer_sync_identity(dir.path().join("sync.key").to_string(), peer_node_id)
            .await
            .expect("identity");

        assert_eq!(identity.node_id, "aa11bb22cc33dd44ee55ff6677889900");
        assert_eq!(identity.device_id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(identity.protocol, PROTOCOL_VERSION);
    }

    /// The phone must show up in the same browse the desktop's nearby list runs.
    ///
    /// This is the visibility contract: the record the FFI registers carries the
    /// node id a peer dials, the name a reader sees, and the `android` type the
    /// desktop keys its label off.
    #[tokio::test]
    async fn an_advertised_device_appears_to_a_nearby_browse() {
        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let key_path = dir.path().join("sync.key").to_string();
        let db_path = dir.path().join("livtet.db").to_string();

        let node_id = advertise_sync_device(
            key_path.clone(),
            db_path,
            "01ARZ3NDEKTSV4RRFFQ69G5FAW".to_string(),
            "Reader's phone".to_string(),
            "android".to_string(),
        )
        .await
        .expect("advertise");

        let peers = tokio::task::spawn_blocking(|| livtet_mdns::browse_for(Duration::from_secs(3)))
            .await
            .expect("browse task")
            .expect("browse");

        let mine = peers
            .iter()
            .find(|peer| peer.node_id == node_id)
            .unwrap_or_else(|| panic!("advertised node {node_id} missing from {peers:?}"));
        assert_eq!(mine.name, "Reader's phone");
        assert_eq!(mine.device_type, "android");

        stop_advertising_sync_device(key_path);
    }
    /// Reopening the screen must not strand the previous advertisement.
    ///
    /// The registry is keyed by key path, so a stop that failed to remove the
    /// entry (or an advertise that refused to rebind after one) would leave the
    /// device invisible for the rest of the process's life.
    #[tokio::test]
    async fn advertising_stops_and_restarts_with_the_same_identity() {
        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let key_path = dir.path().join("sync.key").to_string();
        let db_path = dir.path().join("livtet.db").to_string();

        let first = advertise_sync_device(
            key_path.clone(),
            db_path.clone(),
            "01ARZ3NDEKTSV4RRFFQ69G5FAW".to_string(),
            "Reader's phone".to_string(),
            "android".to_string(),
        )
        .await
        .expect("advertise");

        stop_advertising_sync_device(key_path.clone());

        let second = advertise_sync_device(
            key_path.clone(),
            db_path,
            "01ARZ3NDEKTSV4RRFFQ69G5FAW".to_string(),
            "Reader's phone".to_string(),
            "android".to_string(),
        )
        .await
        .expect("advertise again");

        assert_eq!(
            first, second,
            "the persisted key must yield one identity across restarts"
        );
        stop_advertising_sync_device(key_path);
    }

    /// File a claim against a peer that approves it, over a real iroh connection.
    ///
    /// Both endpoints live in this process: the peer answers `Pair` with a pending
    /// claim and `PairStatus` with an approval, so the test covers the whole path
    /// the phone takes — bind an endpoint from the persisted key, dial by node id,
    /// file the claim, poll, and surface the token — without a database or a second
    /// machine.
    #[tokio::test]
    async fn a_claim_is_filed_and_approved_over_iroh() {
        let peer = DiscoveryService::bind().await.expect("peer endpoint");
        let peer_node_id = peer.node_id().as_str().to_string();

        let handler: Handler = Arc::new(|request, _remote| {
            Box::pin(async move {
                match request {
                    Request::Pair { .. } => Outcome::reply(Response::PairingPending {
                        claim: "claim-1".to_string(),
                    }),
                    Request::PairStatus { claim } => {
                        assert_eq!(claim, "claim-1", "the filed claim is what gets polled");
                        Outcome::reply(Response::Paired {
                            device_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
                            session_token: "token-1".to_string(),
                        })
                    }
                    other => Outcome::reply(Response::Refused {
                        reason: format!("unexpected request: {other:?}"),
                    }),
                }
            })
        });
        let _serving = livtet_sync_iroh::spawn_server(peer.endpoint().clone(), handler);

        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let result = pair_sync_device(
            dir.path().join("sync.key").to_string(),
            peer_node_id,
            "01ARZ3NDEKTSV4RRFFQ69G5FAW".to_string(),
            "Reader's phone".to_string(),
            "android".to_string(),
            10,
        )
        .await
        .expect("pair call");

        assert_eq!(
            result,
            SyncPairingResult::Paired {
                device_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
                session_token: "token-1".to_string(),
            }
        );
    }

    #[test]
    fn a_key_is_created_once_and_reused() {
        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let path = dir.path().join("sync.key");

        let first = load_or_create_secret(&path).expect("create");
        let second = load_or_create_secret(&path).expect("reuse");

        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            hex::encode(first.to_bytes())
        );
    }

    /// A phone holds a book, a peer claims it, and the book arrives on the peer.
    ///
    /// This is the direction the build did not have: pairing was outward-only, so
    /// a desktop could offer a book to a phone and never the reverse. The peer
    /// claims the phone, the phone approves on its own screen, the offer rides the
    /// change log the peer pulls, and the bytes come back over the same wire.
    #[tokio::test]
    async fn a_peer_that_claims_this_device_receives_an_offered_book() {
        use crate::mutations::NewEdition;
        use crate::LivtetStore;

        let phone_dir = camino_tempfile::Utf8TempDir::new().expect("phone dir");
        let peer_dir = camino_tempfile::Utf8TempDir::new().expect("peer dir");

        let phone_db = phone_dir.path().join("livtet.db").to_string();
        let phone_key = phone_dir.path().join("sync.key").to_string();
        let peer_key = peer_dir.path().join("sync.key").to_string();

        // The phone's library: one work, one edition, one real file.
        let store = LivtetStore::open(
            phone_db.clone(),
            phone_dir.path().join("index").to_string(),
        )
        .await
        .expect("store");
        let work = store
            .create_work("Reverse Direction".to_string(), None, None)
            .await
            .expect("work");
        let edition = store
            .create_edition(NewEdition {
                work_id: work.id,
                title: None,
                description: None,
                published_date: None,
                format_id: None,
                language_id: None,
                notes: None,
                cover_image_url: None,
            })
            .await
            .expect("edition");
        let bytes = b"an epub the desktop asked for".to_vec();
        let book = phone_dir.path().join("reverse.epub");
        std::fs::write(&book, &bytes).expect("write book");
        store
            .set_edition_file(
                edition.id,
                livtet_core::types::DiskPath::from_path(book.as_path()),
                Some("epub".to_string()),
                Some(bytes.len() as i64),
            )
            .await
            .expect("attach the file");
        drop(store);

        let phone_node = advertise_sync_device(
            phone_key.clone(),
            phone_db.clone(),
            "01ARZ3NDEKTSV4RRFFQ69G5FAW".to_string(),
            "Reader's phone".to_string(),
            "android".to_string(),
        )
        .await
        .expect("advertise");

        // The peer files the claim; the phone never reaches out.
        let peer_device = "01ARZ3NDEKTSV4RRFFQ69G5FAX".to_string();
        let claim = match pair_sync_device(
            peer_key.clone(),
            phone_node.clone(),
            peer_device.clone(),
            "Studio".to_string(),
            "desktop".to_string(),
            0,
        )
        .await
        .expect("claim")
        {
            SyncPairingResult::Pending { claim } => claim,
            other => panic!("a fresh claim is pending, got {other:?}"),
        };

        let pending = sync_pending_pairings(phone_key.clone())
            .await
            .expect("pending claims");
        let claimer = pending
            .iter()
            .find(|entry| entry.device_id.as_deref() == Some(peer_device.as_str()))
            .unwrap_or_else(|| panic!("the claim is listed: {pending:?}"))
            .node_id
            .clone();

        let issued = sync_decide_pairing(phone_key.clone(), claimer, true)
            .await
            .expect("approve")
            .expect("an approval mints a token");

        match sync_pairing_status(peer_key.clone(), phone_node.clone(), claim, 5)
            .await
            .expect("poll")
        {
            SyncPairingResult::Paired { session_token, .. } => assert_eq!(
                session_token, issued,
                "the claimant is handed the token the phone minted"
            ),
            other => panic!("the approval reaches the claimant, got {other:?}"),
        }

        // The offer rides the change log; the peer is the one that pulls.
        let sent = sync_send_book(phone_key.clone(), edition.id.to_string(), peer_device)
            .await
            .expect("offer the book");

        let client = connect(peer_key.clone(), phone_node.clone())
            .await
            .expect("connect")
            .with_token(issued);
        let page = client.changes(0, None).await.expect("changes");
        assert!(
            page.changes.iter().any(|change| {
                change.entity_type == "file_offer" && change.entity_id == sent.inventory_id
            }),
            "the offer travels the cursor, {} changes seen",
            page.changes.len()
        );

        let (header, body) = client
            .file(&sent.inventory_id, 0, None, None)
            .await
            .expect("fetch the file");
        assert_eq!(header.length, bytes.len() as u64);
        assert_eq!(body.read_all().await.expect("body"), bytes);

        stop_advertising_sync_device(phone_key);
    }

    #[test]
    fn a_malformed_key_is_an_error_rather_than_a_new_identity() {
        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let path = dir.path().join("sync.key");
        std::fs::write(&path, "not-a-key").expect("write");

        let error = load_or_create_secret(&path).expect_err("must reject");

        assert!(
            matches!(&error, LivtetError::Sync(message) if message.contains("32-byte hex")),
            "unexpected error: {error}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "not-a-key",
            "the bad file must be left alone"
        );
    }
}
