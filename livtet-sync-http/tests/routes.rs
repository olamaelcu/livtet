//! The poem routes, driven in-process with `poem::test::TestClient` (no
//! socket).

use std::{collections::HashMap, sync::Arc};

use livtet_data::orm::{ConnectionTrait, DbBackend, Statement};
use livtet_sync::{Conflict, FullDump, PullResponse, PushResponse, SyncEngine, SyncStatus};
use livtet_sync_http::pairing::PairingWaiter;
use livtet_sync_http::routes::SyncIdentity;
use livtet_sync_http::{
    OfferDecision, OfferDecisionSink, PairWaiters, make_sync_routes, make_sync_routes_with_sink,
};
use livtet_test_utils::sync::{Device, change};
use livtet_types::DbId;
use poem::http::{HeaderValue, header::AUTHORIZATION};
use poem::{Endpoint, EndpointExt, http::StatusCode, test::TestClient};
use serde::de::DeserializeOwned;
use tokio::sync::{Mutex, RwLock};

/// The session token every test client presents, seeded by [`test_auth`].
const TEST_SESSION: &str = "test-session";

/// Seed a paired device carrying [`TEST_SESSION`]. Idempotent, so the
/// middleware can call it on every request.
async fn seed_session(conn: &livtet_data::orm::DatabaseConnection) {
    let statement = Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "INSERT OR IGNORE INTO paired_devices (device_id, paired_at, session_token) \
         VALUES (?, strftime('%Y-%m-%d %H:%M:%S','now'), ?)",
        [Vec::<u8>::from([7u8; 16]).into(), TEST_SESSION.into()],
    );
    let _ = conn.execute_raw(statement).await;
}

fn client(device: &Device) -> TestClient<impl Endpoint> {
    client_with_waiters(device, Arc::new(Mutex::new(HashMap::new())))
}

/// `client`, but over a caller-supplied waiter map so a test can register a
/// pairing identity before the routes are built.
fn client_with_waiters(device: &Device, waiters: PairWaiters) -> TestClient<impl Endpoint> {
    let engine = SyncEngine::new(device.conn.clone(), device.engine.device_id().to_string());
    let device_id = device.engine.device_id().to_string();
    let identity = SyncIdentity {
        node_id: format!("node-{device_id}"),
        device_id,
    };
    let conn = device.conn.clone();
    TestClient::new(
        make_sync_routes(Arc::new(RwLock::new(engine)), waiters, identity).around(
            move |endpoint, mut req: poem::Request| {
                let conn = conn.clone();
                async move {
                    seed_session(&conn).await;
                    req.headers_mut().insert(
                        AUTHORIZATION,
                        HeaderValue::from_static("Bearer test-session"),
                    );
                    endpoint.call(req).await
                }
            },
        ),
    )
}

/// The routes with no session injected, for asserting the 401 arms.
fn unauthenticated(device: &Device, waiters: PairWaiters) -> TestClient<impl Endpoint> {
    let engine = SyncEngine::new(device.conn.clone(), device.engine.device_id().to_string());
    let device_id = device.engine.device_id().to_string();
    let identity = SyncIdentity {
        node_id: format!("node-{device_id}"),
        device_id,
    };
    TestClient::new(make_sync_routes(
        Arc::new(RwLock::new(engine)),
        waiters,
        identity,
    ))
}

async fn json<T: DeserializeOwned>(response: poem::test::TestResponse) -> T {
    response.assert_status_is_ok();
    response
        .0
        .into_body()
        .into_json()
        .await
        .expect("JSON response body")
}

/// Create one conflict by pushing a change older than a local edit.
async fn make_conflict(device: &Device) -> Conflict {
    device
        .execute("UPDATE works SET title = 'Local' WHERE id = (SELECT id FROM works LIMIT 1)")
        .await;
    let edit = device.logged("work", "UPDATE").await.pop().expect("logged");
    let response = device
        .engine
        .push_changes(vec![change(
            "work",
            &edit.entity_id,
            "DELETE",
            edit.version - 1,
            "{}",
        )])
        .await
        .unwrap();
    response.conflicts.into_iter().next().expect("conflict")
}

/// A discovered peer confirms the node id it saw in the mDNS TXT record before
/// posting a pairing claim.
#[tokio::test]
async fn identity_reports_the_node_and_device_ids() {
    let device = Device::new("peer").await;
    let identity: SyncIdentity = json(client(&device).get("/sync/identity").send().await).await;

    assert_eq!(identity.device_id, "peer");
    assert_eq!(identity.node_id, "node-peer");
}

/// Data routes are session-gated; identity, the claim, and its status stream
/// are the only public surface.
#[tokio::test]
async fn protected_routes_reject_missing_or_unknown_sessions() {
    let device = Device::new("server").await;
    let waiters: PairWaiters = Arc::new(Mutex::new(HashMap::new()));
    let client = unauthenticated(&device, waiters);

    for path in [
        "/sync/status",
        "/sync/changes?since_version=0",
        "/sync/pull-full",
        "/sync/conflicts",
    ] {
        client
            .get(path)
            .send()
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    client
        .post("/sync/push")
        .content_type("application/json")
        .body("[]")
        .send()
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    client
        .get("/sync/status")
        .header("authorization", "Bearer not-a-session")
        .send()
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // A discovered peer confirms the node id before claiming; that is public.
    client
        .get("/sync/identity")
        .send()
        .await
        .assert_status_is_ok();
}

#[tokio::test]
async fn a_paired_session_reaches_the_data_routes() {
    let device = Device::new("server").await;
    let response = client(&device).get("/sync/status").send().await;
    response.assert_status_is_ok();
}

/// The device [`seed_session`] pairs, as the string `DbId` renders.
fn session_device_id() -> String {
    DbId::from_bytes([7u8; 16]).to_string()
}

/// The routes plus a sink that records what the sender would be told.
fn client_with_decisions(
    device: &Device,
) -> (
    TestClient<impl Endpoint>,
    Arc<std::sync::Mutex<Vec<OfferDecision>>>,
) {
    let seen: Arc<std::sync::Mutex<Vec<OfferDecision>>> = Arc::default();
    let engine = SyncEngine::new(device.conn.clone(), device.engine.device_id().to_string());
    let device_id = device.engine.device_id().to_string();
    let identity = SyncIdentity {
        node_id: format!("node-{device_id}"),
        device_id,
    };
    let conn = device.conn.clone();
    let sink: OfferDecisionSink = {
        let seen = Arc::clone(&seen);
        Arc::new(move |decision: OfferDecision| {
            seen.lock().expect("decisions lock").push(decision);
        })
    };
    let routes = make_sync_routes_with_sink(
        Arc::new(RwLock::new(engine)),
        Arc::new(Mutex::new(HashMap::new())),
        identity,
        sink,
    )
    .around(move |endpoint, mut req: poem::Request| {
        let conn = conn.clone();
        async move {
            seed_session(&conn).await;
            req.headers_mut().insert(
                AUTHORIZATION,
                HeaderValue::from_static("Bearer test-session"),
            );
            endpoint.call(req).await
        }
    });
    (TestClient::new(routes), seen)
}

/// Write a `file_offer` row addressed to `to_device_id`.
async fn seed_offer(device: &Device, offer_id: &str, to_device_id: &str) {
    let payload = serde_json::json!({
        "offer_id": offer_id,
        "inventory_id": "00000000000000000000000000000001",
        "edition_id": "00000000000000000000000000000002",
        "work_id": "00000000000000000000000000000003",
        "to_device_id": to_device_id,
        "title": "Offered Book",
        "authors": ["Ada Lovelace"],
    })
    .to_string();
    livtet_data::sql::query(
        "INSERT INTO change_log (entity_type, entity_id, operation, version, payload, \
             changed_at, device_id) \
         VALUES ('file_offer', '00000000000000000000000000000001', 'OFFER', \
             (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log), ?, datetime('now'), ?)",
    )
    .bind(payload)
    .bind(device.engine.device_id().to_string())
    .execute(device.pool())
    .await
    .expect("seed offer");
}

/// The device an offer was addressed to is the one that may answer it, and the
/// answer is what the sender is told.
#[tokio::test]
async fn an_offer_is_answered_by_its_addressee() {
    let device = Device::new("server").await;
    seed_offer(&device, "offer-1", &session_device_id()).await;
    let (cli, seen) = client_with_decisions(&device);

    cli.post("/sync/offers/offer-1/decision")
        .content_type("application/json")
        .body(r#"{ "accepted": true }"#)
        .send()
        .await
        .assert_status_is_ok();

    assert_eq!(
        seen.lock().expect("decisions lock").as_slice(),
        [OfferDecision {
            offer_id: "offer-1".to_string(),
            device_id: session_device_id(),
            accepted: true,
        }]
    );
}

/// An offer names its target; another paired device may not answer it.
#[tokio::test]
async fn an_offer_cannot_be_answered_by_another_device() {
    let device = Device::new("server").await;
    seed_offer(&device, "offer-2", &DbId::new().to_string()).await;
    let (cli, seen) = client_with_decisions(&device);

    cli.post("/sync/offers/offer-2/decision")
        .content_type("application/json")
        .body(r#"{ "accepted": true }"#)
        .send()
        .await
        .assert_status(StatusCode::FORBIDDEN);

    assert!(seen.lock().expect("decisions lock").is_empty());
}

/// An offer id this desktop never sent has no answer to record.
#[tokio::test]
async fn an_unknown_offer_has_nothing_to_answer() {
    let device = Device::new("server").await;
    let (cli, _seen) = client_with_decisions(&device);

    cli.post("/sync/offers/missing/decision")
        .content_type("application/json")
        .body(r#"{ "accepted": false }"#)
        .send()
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

/// The decision route is session-gated like every other data route.
#[tokio::test]
async fn answering_an_offer_needs_a_session() {
    let device = Device::new("server").await;
    let cli = unauthenticated(&device, Arc::new(Mutex::new(HashMap::new())));

    cli.post("/sync/offers/offer-3/decision")
        .content_type("application/json")
        .body(r#"{ "accepted": true }"#)
        .send()
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn status_reports_the_device_and_latest_version() {
    let device = Device::seeded("server").await;
    let status: SyncStatus = json(client(&device).get("/sync/status").send().await).await;
    assert_eq!(status.device_id, "server");
    assert_eq!(
        status.latest_version,
        device.engine.get_latest_version().await.unwrap()
    );
}

#[tokio::test]
async fn changes_are_paged_by_query_parameters() {
    let device = Device::seeded("server").await;
    let cli = client(&device);

    let page: PullResponse = json(
        cli.get("/sync/changes")
            .query("since_version", &0)
            .query("limit", &5)
            .send()
            .await,
    )
    .await;
    assert_eq!(page.changes.len(), 5);
    assert!(page.has_more);
    assert_eq!(page.latest_version, 5);

    let rest: PullResponse = json(
        cli.get("/sync/changes")
            .query("since_version", &page.latest_version)
            .query("limit", &100_000)
            .send()
            .await,
    )
    .await;
    assert_eq!(
        rest.changes.first().map(|c| c.version),
        Some(page.latest_version + 1)
    );
    assert!(!rest.has_more);
}

#[tokio::test]
async fn changes_default_to_pages_of_one_hundred() {
    let device = Device::seeded("server").await;
    // The seed size is random; make sure there is more than one default page.
    while device.change_log().await.len() <= 100 {
        device.execute("UPDATE works SET title = title").await;
    }

    let page: PullResponse = json(
        client(&device)
            .get("/sync/changes")
            .query("since_version", &0)
            .send()
            .await,
    )
    .await;
    assert_eq!(page.changes.len(), 100);
    assert!(page.has_more);
}

#[tokio::test]
async fn changes_without_a_since_version_is_a_bad_request() {
    let device = Device::new("server").await;
    client(&device)
        .get("/sync/changes")
        .send()
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn full_dump_matches_the_engine() {
    let device = Device::seeded("server").await;
    let dump: FullDump = json(client(&device).get("/sync/pull-full").send().await).await;
    let direct = device.engine.pull_full().await.unwrap();

    assert_eq!(dump.version, direct.version);
    assert_eq!(dump.device_id, "server");
    assert_eq!(
        serde_json::to_value(&dump.entities).unwrap(),
        serde_json::to_value(&direct.entities).unwrap()
    );
}

#[tokio::test]
async fn push_accepts_an_empty_batch() {
    let device = Device::seeded("server").await;
    let response: PushResponse = json(
        client(&device)
            .post("/sync/push")
            .body_json(&serde_json::json!([]))
            .send()
            .await,
    )
    .await;
    assert!(response.accepted);
    assert!(response.conflicts.is_empty());
}

#[tokio::test]
async fn push_with_a_malformed_body_is_a_bad_request() {
    let device = Device::new("server").await;
    client(&device)
        .post("/sync/push")
        .content_type("application/json")
        .body("{not json")
        .send()
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn conflicts_are_listed_and_resolved() {
    let device = Device::seeded("server").await;
    let conflict = make_conflict(&device).await;
    let cli = client(&device);

    let listed: Vec<Conflict> = json(cli.get("/sync/conflicts").send().await).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, conflict.id);

    let path = format!("/sync/conflicts/{}/resolve", conflict.id);
    let ok: serde_json::Value = json(
        cli.post(&path)
            .body_json(&serde_json::json!({ "resolution": "remote", "merged_payload": null }))
            .send()
            .await,
    )
    .await;
    assert_eq!(ok, serde_json::json!({ "ok": true }));

    let listed: Vec<Conflict> = json(cli.get("/sync/conflicts").send().await).await;
    assert!(listed.is_empty());

    // Already resolved: gone.
    cli.post(&path)
        .body_json(&serde_json::json!({ "resolution": "remote", "merged_payload": null }))
        .send()
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn resolving_with_an_unknown_resolution_is_a_bad_request() {
    let device = Device::seeded("server").await;
    let conflict = make_conflict(&device).await;

    client(&device)
        .post(format!("/sync/conflicts/{}/resolve", conflict.id))
        .body_json(&serde_json::json!({ "resolution": "mine", "merged_payload": null }))
        .send()
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(device.engine.list_conflicts().await.unwrap().len(), 1);
}

/// The route's `:id` segment matches any text, so a non-numeric id is caught
/// by the `Path<i64>` extractor rather than reaching the handler.
#[tokio::test]
async fn resolving_a_non_numeric_conflict_id_is_a_bad_request() {
    let device = Device::new("server").await;

    client(&device)
        .post("/sync/conflicts/not-a-number/resolve")
        .body_json(&serde_json::json!({ "resolution": "local", "merged_payload": null }))
        .send()
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

mod files {
    use super::*;

    /// Point one inventory row at a real temp file holding `contents`,
    /// returning the row's hex id (and the temp dir, kept alive).
    async fn inventory_file(
        device: &Device,
        name: &str,
        contents: &[u8],
    ) -> (String, camino_tempfile::Utf8TempDir) {
        let dir = camino_tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        fs_err::write(&path, contents).unwrap();
        let id: String = livtet_data::sql::query_scalar(
            "SELECT lower(hex(id)) FROM digital_inventory ORDER BY id LIMIT 1",
        )
        .fetch_one(device.pool())
        .await
        .unwrap();
        livtet_data::sql::query(
            "UPDATE digital_inventory SET file_path = ? WHERE lower(hex(id)) = ?",
        )
        .bind(path.as_str())
        .bind(&id)
        .execute(device.pool())
        .await
        .unwrap();
        (id, dir)
    }

    #[tokio::test]
    async fn a_non_hex_or_wrong_length_id_is_a_bad_request() {
        let device = Device::new("server").await;
        let cli = client(&device);
        for id in ["not-hex", "abcd"] {
            cli.get(format!("/sync/files/{id}"))
                .send()
                .await
                .assert_status(StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn an_unknown_inventory_id_is_not_found() {
        let device = Device::seeded("server").await;
        client(&device)
            .get(format!("/sync/files/{}", hex::encode([0u8; 16])))
            .send()
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_file_missing_from_disk_is_not_found() {
        let device = Device::seeded("server").await;
        let (id, dir) = inventory_file(&device, "book.epub", b"x").await;
        drop(dir);
        client(&device)
            .get(format!("/sync/files/{id}"))
            .send()
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_whole_file_is_served_with_its_content_type() {
        let device = Device::seeded("server").await;
        let (id, _dir) = inventory_file(&device, "book.pdf", b"%PDF-1.7 hello").await;

        let response = client(&device)
            .get(format!("/sync/files/{id}"))
            .send()
            .await;
        response.assert_status_is_ok();
        response.assert_header("content-type", "application/pdf");
        response.assert_header("accept-ranges", "bytes");
        response.assert_bytes(b"%PDF-1.7 hello").await;
    }

    #[tokio::test]
    async fn a_byte_range_is_served_as_partial_content() {
        let device = Device::seeded("server").await;
        let (id, _dir) = inventory_file(&device, "book.epub", b"0123456789").await;

        let response = client(&device)
            .get(format!("/sync/files/{id}"))
            .header("range", "bytes=2-5")
            .send()
            .await;
        response.assert_status(StatusCode::PARTIAL_CONTENT);
        response.assert_header("content-type", "application/epub+zip");
        response.assert_header("content-range", "bytes 2-5/10");
        response.assert_bytes(b"2345").await;
    }

    #[tokio::test]
    async fn an_unsatisfiable_range_is_rejected() {
        let device = Device::seeded("server").await;
        let (id, _dir) = inventory_file(&device, "book.epub", b"0123456789").await;

        client(&device)
            .get(format!("/sync/files/{id}"))
            .header("range", "bytes=50-60")
            .send()
            .await
            .assert_status(StatusCode::RANGE_NOT_SATISFIABLE);
    }

    /// `If-Range` guards a resumed download: when the hash the client holds no
    /// longer matches the stored one, the file has changed underneath it, so
    /// the `Range` must be ignored and the whole file served at 200 rather
    /// than splicing new bytes into an old partial download.
    #[tokio::test]
    async fn a_stale_if_range_serves_the_whole_file_despite_the_range() {
        let device = Device::seeded("server").await;
        let (id, _dir) = inventory_file(&device, "book.epub", b"0123456789").await;

        let response = client(&device)
            .get(format!("/sync/files/{id}"))
            .header("if-range", "blake3:not-the-stored-hash")
            .header("range", "bytes=2-5")
            .send()
            .await;
        response.assert_status(StatusCode::OK);
        response.assert_header("content-length", "10");
        response.assert_header("accept-ranges", "bytes");
        response.assert_bytes(b"0123456789").await;
    }

    /// A matching `If-Range` means the client's partial download is still
    /// valid, so the `Range` is honoured as normal.
    #[tokio::test]
    async fn a_current_if_range_still_honours_the_range() {
        let device = Device::seeded("server").await;
        let (id, _dir) = inventory_file(&device, "book.epub", b"0123456789").await;

        let stored: String = livtet_data::sql::query_scalar(
            "SELECT file_hash FROM digital_inventory WHERE lower(hex(id)) = ?",
        )
        .bind(&id)
        .fetch_one(device.pool())
        .await
        .expect("the seeder stores a file_hash");

        let response = client(&device)
            .get(format!("/sync/files/{id}"))
            .header("if-range", stored)
            .header("range", "bytes=2-5")
            .send()
            .await;
        response.assert_status(StatusCode::PARTIAL_CONTENT);
        response.assert_bytes(b"2345").await;
    }

    /// An inventory row with no `file_path` is a different miss from an
    /// inventory row that does not exist, and from a path that is gone.
    #[tokio::test]
    async fn a_null_file_path_is_not_found() {
        let device = Device::seeded("server").await;
        let (id, _dir) = inventory_file(&device, "book.epub", b"0123456789").await;

        livtet_data::sql::query(
            "UPDATE digital_inventory SET file_path = NULL WHERE lower(hex(id)) = ?",
        )
        .bind(&id)
        .execute(device.pool())
        .await
        .expect("clear the path");

        client(&device)
            .get(format!("/sync/files/{id}"))
            .send()
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
}

mod pair_status {
    use super::*;

    #[tokio::test]
    async fn an_unregistered_identity_is_not_found() {
        let device = Device::new("server").await;

        client(&device)
            .get("/sync/pair/status/never-minted")
            .send()
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    /// A registered identity opens an SSE stream.
    ///
    /// Only the status and content type are asserted. The stream's own body
    /// loop waits on the broadcast channel with a 300-second idle timeout, so
    /// reading the body here would block for that long — and the
    /// `async_stream::stream!` body carries no coverage regions anyway, so
    /// draining it would buy nothing.
    #[tokio::test]
    async fn a_registered_identity_opens_an_event_stream() {
        let device = Device::new("server").await;
        let waiters: PairWaiters = Arc::new(Mutex::new(HashMap::new()));
        let (tx, _rx) = tokio::sync::broadcast::channel(4);
        waiters.lock().await.insert(
            "pair-node".to_string(),
            PairingWaiter {
                claim: "claim-1".to_string(),
                sender: tx,
            },
        );

        let response = client_with_waiters(&device, waiters)
            .get("/sync/pair/status/pair-node?claim=claim-1")
            .send()
            .await;
        response.assert_status_is_ok();
        response.assert_header("content-type", "text/event-stream");
    }

    /// The stream carries the session token, so it only opens for the device
    /// that filed the claim: a known node id alone is not enough.
    #[tokio::test]
    async fn a_missing_or_wrong_claim_is_unauthorized() {
        let device = Device::new("server").await;
        let waiters: PairWaiters = Arc::new(Mutex::new(HashMap::new()));
        let (tx, _rx) = tokio::sync::broadcast::channel(4);
        waiters.lock().await.insert(
            "pair-node".to_string(),
            PairingWaiter {
                claim: "claim-1".to_string(),
                sender: tx,
            },
        );

        let client = client_with_waiters(&device, waiters);
        client
            .get("/sync/pair/status/pair-node")
            .send()
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
        client
            .get("/sync/pair/status/pair-node?claim=wrong")
            .send()
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    /// The handler's `Json` extractor rejects a body it cannot parse before
    /// the handler body runs.
    #[tokio::test]
    async fn a_malformed_pair_body_is_a_bad_request() {
        let device = Device::new("server").await;

        client(&device)
            .post("/sync/pair")
            .content_type("application/json")
            .body("{not json")
            .send()
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}
