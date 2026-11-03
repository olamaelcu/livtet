//! Inbound book offers end to end: a pull records what a peer offered and
//! holds the rows that describe it; the reader's decision is what downloads
//! the file, applies those rows, and tells the sender.
//!
//! The peer is a stub iroh endpoint rather than a second daemon: what matters
//! here is this daemon's half of the protocol — the rows it records, the file
//! it fetches, and the decision it sends back.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use livtet_iroh::{DiscoveryService, SecretKey};
use livtet_sync::{PullResponse, SyncChange};
use livtet_sync_iroh::{ErrorCode, FileHeader, Handler, Outcome, Request, Response};

use livtet_sync_server::rpc::{self, StdioClient};
use livtet_types::DbId;

const TEST_DISCOVERY_KEY: &str = "0101010101010101010101010101010101010101010101010101010101010101";

/// What the stub peer served and what the daemon told it.
#[derive(Default)]
struct PeerState {
    /// The page every changes call returns.
    changes: Mutex<Vec<SyncChange>>,
    /// `(offer_id, accepted)` for every decision the daemon sent.
    decisions: Mutex<Vec<(String, bool)>>,
}

/// A stub peer: the node id the daemon dials, its state, and the live endpoint.
struct StubPeer {
    node_id: String,
    state: Arc<PeerState>,
    /// Kept alive for the test; dropping it closes the endpoint.
    _service: DiscoveryService,
}

/// Start the stub peer and return its node id and shared state.
async fn start_peer() -> StubPeer {
    let state = Arc::new(PeerState::default());
    let file = temp_path("peer-book");
    fs::write(&file, b"offered-book-bytes").expect("write offered book");

    let service = DiscoveryService::bind_with_secret(SecretKey::generate())
        .await
        .expect("stub endpoint");
    let node_id = service.node_id().0;

    let handler: Handler = Arc::new({
        let state = Arc::clone(&state);
        move |request: Request, _remote: String| {
            let state = Arc::clone(&state);
            let file = file.clone();
            Box::pin(async move {
                match request {
                    Request::Changes { .. } => Outcome::reply(Response::Changes(PullResponse {
                        changes: state.changes.lock().expect("changes lock").clone(),
                        has_more: false,
                        latest_version: 100,
                    })),
                    Request::File { .. } => {
                        let length = fs::metadata(&file).map(|meta| meta.len()).unwrap_or(0);
                        Outcome::file(
                            FileHeader {
                                content_type: "application/epub+zip".to_string(),
                                offset: 0,
                                length,
                                total: length,
                                etag: format!("{length:x}"),
                            },
                            file,
                        )
                    }
                    Request::Cover { .. } => Outcome::reply(Response::Error {
                        code: ErrorCode::NotFound,
                        message: "no cover".to_string(),
                    }),
                    Request::OfferDecision {
                        offer_id, accepted, ..
                    } => {
                        state
                            .decisions
                            .lock()
                            .expect("decisions lock")
                            .push((offer_id, accepted));
                        Outcome::reply(Response::OfferDecided)
                    }
                    other => Outcome::reply(Response::Error {
                        code: ErrorCode::BadRequest,
                        message: format!("unexpected request {other:?}"),
                    }),
                }
            })
        }
    });
    livtet_sync_iroh::spawn_server(service.endpoint().clone(), handler);

    StubPeer {
        node_id,
        state,
        _service: service,
    }
}

fn temp_path(tag: &str) -> Utf8PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "livtet-sync-offers-{tag}-{}-{nanos}-{seq}",
        std::process::id(),
    ));
    Utf8PathBuf::from_path_buf(path).expect("temp path is valid UTF-8")
}

fn cleanup(dir: &Utf8Path) {
    let _ = fs::remove_dir_all(dir);
}

async fn spawn_daemon(db_path: &Utf8Path, books: &Utf8Path, covers: &Utf8Path) -> StdioClient {
    let binary = env!("CARGO_BIN_EXE_livtet-sync-server");
    let args = vec![
        "--db".to_string(),
        db_path.to_string(),
        "--port".to_string(),
        "0".to_string(),
        "--books-dir".to_string(),
        books.to_string(),
        "--covers-dir".to_string(),
        covers.to_string(),
    ];
    StdioClient::spawn_with_env(
        binary,
        &args,
        &[("LIVTET_DISCOVERY_KEY", TEST_DISCOVERY_KEY)],
    )
    .await
    .expect("spawn daemon")
}

async fn ok_result(
    client: &mut StdioClient,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let response = client
        .request(method, params)
        .await
        .unwrap_or_else(|error| panic!("{method} transport: {error}"));
    assert!(
        response.error.is_none(),
        "{method} returned an error: {:?}",
        response.error
    );
    response.result.unwrap_or_else(|| panic!("{method} result"))
}

async fn shutdown(client: &mut StdioClient) {
    let _ = client
        .request(rpc::method::SHUTDOWN, serde_json::json!({}))
        .await;
    let _ = tokio::time::timeout(Duration::from_secs(5), client.wait()).await;
}

/// One offer's change rows: the offer itself plus the work its payload names,
/// which the pull must hold back.
fn offer_rows(offer: &OfferSeed<'_>) -> Vec<SyncChange> {
    let OfferSeed {
        offer_id,
        version,
        to_device_id,
        from_device_id,
        title,
        work,
        inventory,
        edition,
    } = *offer;
    let payload = serde_json::json!({
        "offer_id": offer_id,
        "inventory_id": inventory.to_hex(),
        "edition_id": edition.to_hex(),
        "work_id": work.to_hex(),
        "to_device_id": to_device_id,
        "title": title,
        "authors": ["Ada Lovelace"],
        "file_name": "offered.epub",
        "file_size_bytes": 18,
        "has_cover": false,
    })
    .to_string();

    vec![
        SyncChange {
            id: version,
            entity_type: "file_offer".to_string(),
            entity_id: inventory.to_hex(),
            operation: "OFFER".to_string(),
            version,
            payload,
            changed_at: "2026-11-01 09:00:00".to_string(),
            device_id: from_device_id.to_string(),
        },
        // The catalog row describing the offered book: held until accepted.
        SyncChange {
            id: version + 1,
            entity_type: "work".to_string(),
            entity_id: work.to_hex(),
            operation: "UPDATE".to_string(),
            version: version + 1,
            payload: serde_json::json!({
                "id": work.to_hex(),
                "title": title,
                "created_at": "2026-11-01 09:00:00",
            })
            .to_string(),
            changed_at: "2026-11-01 09:00:00".to_string(),
            device_id: from_device_id.to_string(),
        },
    ]
}

/// The identity of one offer to seed, and the ids its held row names.
#[derive(Clone, Copy)]
struct OfferSeed<'a> {
    offer_id: &'a str,
    version: i64,
    to_device_id: &'a str,
    from_device_id: &'a str,
    title: &'a str,
    work: DbId,
    inventory: DbId,
    edition: DbId,
}

/// Pair `device_id` with the stub, naming the node id a dial uses.
async fn seed_paired_device(
    pool: &livtet_data::sql::SqlitePool,
    device_id: DbId,
    node_id: Option<&str>,
    token: &str,
) {
    livtet_data::sql::query(
        "INSERT INTO paired_devices \
             (device_id, name, listen_on, device_type_id, paired_at, last_sync_at, \
              session_token, node_id, outbound) \
         VALUES (?, 'Peer', NULL, NULL, datetime('now'), NULL, ?, ?, 1)",
    )
    .bind(device_id.to_bytes().to_vec())
    .bind(token)
    .bind(node_id.map(str::to_string))
    .execute(pool)
    .await
    .expect("seed paired device");
}

/// The whole lifecycle: pull, list, decline one offer, accept the other.
#[tokio::test]
async fn a_pulled_offer_waits_for_the_reader_and_reports_its_decision() {
    let root = temp_path("lifecycle");
    let books = root.join("books");
    let covers = root.join("covers");
    let db_path = root.join("sync.db");
    for dir in [books.clone(), covers.clone()] {
        fs::create_dir_all(dir.as_std_path()).expect("create temp dir");
    }

    let peer = start_peer().await;
    let mut client = spawn_daemon(&db_path, &books, &covers).await;

    let status = ok_result(&mut client, rpc::method::STATUS, serde_json::json!({})).await;
    let device_id = status["device_id"]
        .as_str()
        .expect("status device id")
        .to_string();

    let peer_device = DbId::new();
    let accepted_work = DbId::new();
    let accepted_inventory = DbId::new();
    let accepted_edition = DbId::new();
    let declined_work = DbId::new();
    let declined_inventory = DbId::new();
    let declined_edition = DbId::new();

    let mut changes = offer_rows(&OfferSeed {
        offer_id: "offer-accept",
        version: 1,
        to_device_id: &device_id,
        from_device_id: &peer_device.to_string(),
        title: "Held Book",
        work: accepted_work,
        inventory: accepted_inventory,
        edition: accepted_edition,
    });
    changes.extend(offer_rows(&OfferSeed {
        offer_id: "offer-decline",
        version: 3,
        to_device_id: &device_id,
        from_device_id: &peer_device.to_string(),
        title: "Unwanted Book",
        work: declined_work,
        inventory: declined_inventory,
        edition: declined_edition,
    }));

    // The stub serves one page; the daemon records both offers from it.
    *peer.state.changes.lock().expect("changes lock") = changes;

    // The peer must be a paired, outbound device with a token this side holds.
    seed_paired_device(
        &pool_for(&db_path).await,
        peer_device,
        Some(&peer.node_id),
        "peer-token",
    )
    .await;

    // A device this side claimed is the only kind that can be pulled from, and
    // the desktop needs the flag to tell the two listings apart.
    let listed = ok_result(
        &mut client,
        rpc::method::DEVICES_LIST,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        listed[0]["outbound"],
        serde_json::Value::Bool(true),
        "a claimed device is outbound: {listed}"
    );

    let pulled = ok_result(
        &mut client,
        rpc::method::DEVICES_PULL,
        serde_json::json!({ "device_id": peer_device.to_string() }),
    )
    .await;
    assert_eq!(pulled["offered"].as_u64(), Some(2), "both offers recorded");
    assert_eq!(
        pulled["applied"].as_u64(),
        Some(0),
        "the offered books' rows stay held"
    );

    let pending = ok_result(&mut client, rpc::method::OFFERS_LIST, serde_json::json!({})).await;
    let pending = pending.as_array().expect("offer list");
    assert_eq!(pending.len(), 2);
    let titles: Vec<&str> = pending
        .iter()
        .map(|offer| offer["title"].as_str().expect("title"))
        .collect();
    assert!(titles.contains(&"Held Book"), "titles: {titles:?}");
    assert!(titles.contains(&"Unwanted Book"), "titles: {titles:?}");
    assert_eq!(
        pending[0]["authors"].as_array().map(Vec::len),
        Some(1),
        "the offer carries its authors for the dialog"
    );
    assert_eq!(pending[0]["from_name"].as_str(), Some("Peer"));

    let pool = pool_for(&db_path).await;
    let held: i64 = livtet_data::sql::query_scalar("SELECT count(*) FROM works")
        .fetch_one(&pool)
        .await
        .expect("count works");
    assert_eq!(held, 0, "a pull must not land an offered book");

    // Declining downloads nothing and reports the refusal to the sender.
    let declined = ok_result(
        &mut client,
        rpc::method::OFFERS_DECIDE,
        serde_json::json!({ "offer_id": "offer-decline", "accepted": false }),
    )
    .await;
    assert_eq!(declined["accepted"].as_bool(), Some(false));
    assert_eq!(declined["delivered"].as_bool(), Some(true));
    assert_eq!(declined["applied"].as_u64(), Some(0));
    assert!(
        fs::read_dir(books.as_std_path())
            .expect("books dir")
            .next()
            .is_none(),
        "a declined book leaves no file"
    );

    // Accepting downloads the file, applies the held row, and reports.
    let accepted = ok_result(
        &mut client,
        rpc::method::OFFERS_DECIDE,
        serde_json::json!({ "offer_id": "offer-accept", "accepted": true }),
    )
    .await;
    assert_eq!(accepted["accepted"].as_bool(), Some(true));
    assert_eq!(accepted["delivered"].as_bool(), Some(true));
    assert_eq!(
        accepted["applied"].as_u64(),
        Some(1),
        "the held row applied"
    );

    let files: Vec<String> = fs::read_dir(books.as_std_path())
        .expect("books dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(files.len(), 1, "books: {files:?}");
    let contents = fs::read(books.join(&files[0]).as_std_path()).expect("downloaded file");
    assert_eq!(contents, b"offered-book-bytes");

    let decided: Vec<(String, bool)> = peer
        .state
        .decisions
        .lock()
        .expect("decisions lock")
        .iter()
        .cloned()
        .collect();
    assert_eq!(
        decided,
        vec![
            ("offer-decline".to_string(), false),
            ("offer-accept".to_string(), true),
        ],
        "the sender is told each decision"
    );

    // A decision is final: answering again is a protocol error.
    let again = client
        .request(
            rpc::method::OFFERS_DECIDE,
            serde_json::json!({ "offer_id": "offer-accept", "accepted": false }),
        )
        .await
        .expect("decide transport");
    assert_eq!(
        again.error.expect("second decision rejected").code,
        rpc::error_code::INVALID_PARAMS
    );

    let pending = ok_result(&mut client, rpc::method::OFFERS_LIST, serde_json::json!({})).await;
    assert_eq!(
        pending.as_array().map(Vec::len),
        Some(0),
        "decided offers leave the pending list"
    );

    shutdown(&mut client).await;
    cleanup(&root);
}

/// A paired row with no node id has no address to dial: the pull says so
/// rather than trying a stored socket address.
#[tokio::test]
async fn a_peer_without_a_node_id_cannot_be_pulled() {
    let root = temp_path("no-node-id");
    let books = root.join("books");
    let covers = root.join("covers");
    let db_path = root.join("sync.db");
    for dir in [books.clone(), covers.clone()] {
        fs::create_dir_all(dir.as_std_path()).expect("create temp dir");
    }

    let mut client = spawn_daemon(&db_path, &books, &covers).await;
    // The daemon owns the database: wait for it to open and migrate before
    // seeding a row underneath it.
    ok_result(&mut client, rpc::method::STATUS, serde_json::json!({})).await;

    let peer_device = DbId::new();
    seed_paired_device(&pool_for(&db_path).await, peer_device, None, "peer-token").await;

    let response = client
        .request(
            rpc::method::DEVICES_PULL,
            serde_json::json!({ "device_id": peer_device.to_string() }),
        )
        .await
        .expect("pull transport");
    let error = response
        .error
        .expect("a peer without a node id is an error");
    assert!(
        error.message.contains("no node id"),
        "the refusal must name the missing address: {}",
        error.message
    );

    shutdown(&mut client).await;
    cleanup(&root);
}

/// A pull that applies rows tells the parent process, so a library view can
/// refresh without polling. A pull that finds nothing must stay quiet.
#[tokio::test]
async fn a_pull_that_applies_changes_tells_the_parent() {
    let root = temp_path("pulled");
    let books = root.join("books");
    let covers = root.join("covers");
    let db_path = root.join("sync.db");
    for dir in [books.clone(), covers.clone()] {
        fs::create_dir_all(dir.as_std_path()).expect("create temp dir");
    }

    let peer = start_peer().await;
    let mut client = spawn_daemon(&db_path, &books, &covers).await;

    let status = ok_result(&mut client, rpc::method::STATUS, serde_json::json!({})).await;
    let _device_id = status["device_id"].as_str().expect("status device id");

    // A plain catalog row: no offer, so the pull applies it immediately.
    let peer_device = DbId::new();
    let work = DbId::new();
    *peer.state.changes.lock().expect("changes lock") = vec![SyncChange {
        id: 1,
        entity_type: "work".to_string(),
        entity_id: work.to_hex(),
        operation: "UPDATE".to_string(),
        version: 1,
        payload: serde_json::json!({
            "id": work.to_hex(),
            "title": "Pulled Book",
            "created_at": "2026-11-01 09:00:00",
        })
        .to_string(),
        changed_at: "2026-11-01 09:00:00".to_string(),
        device_id: peer_device.to_string(),
    }];

    seed_paired_device(
        &pool_for(&db_path).await,
        peer_device,
        Some(&peer.node_id),
        "peer-token",
    )
    .await;

    let pulled = ok_result(
        &mut client,
        rpc::method::DEVICES_PULL,
        serde_json::json!({ "device_id": peer_device.to_string() }),
    )
    .await;
    assert_eq!(pulled["applied"].as_u64(), Some(1), "the row applied");

    // Whatever else arrived first, the pull's notification has to be among them.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut reported = None;
    while std::time::Instant::now() < deadline && reported.is_none() {
        let Some(notification) = client.next_notification(Duration::from_millis(500)).await else {
            continue;
        };
        if notification.method == rpc::method::NOTIFY_SYNC_PULLED {
            reported = Some(notification);
        }
    }
    let reported = reported.expect("a pull that applied changes must report them");
    assert_eq!(
        reported.params["applied"].as_u64(),
        Some(1),
        "the report says how much landed: {:?}",
        reported.params
    );
    assert_eq!(
        reported.params["device_id"].as_str(),
        Some(peer_device.to_string().as_str()),
        "the report names the peer it came from"
    );

    shutdown(&mut client).await;
    cleanup(&root);
}

/// A pool over the daemon's own database file, for asserting on its rows.
async fn pool_for(db_path: &Utf8Path) -> livtet_data::sql::SqlitePool {
    let options: livtet_data::sql::sqlite::SqliteConnectOptions =
        db_path.to_string().parse().expect("sqlite connect options");
    livtet_data::sql::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open daemon database")
}
