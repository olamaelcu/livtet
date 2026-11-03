//! The RPC methods `rpc_health.rs` does not reach: pairing approval and
//! rejection over iroh claims, device revocation, conflict listing and
//! resolution, and how the dispatcher answers a method or a params object it
//! cannot use.
//!
//! One daemon serves most of the file — spawning the binary per test is the
//! expensive part, and none of these leave state that affects the others.

use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use livtet_sync_server::rpc::{self, StdioClient};
use livtet_types::{DbId, PairingStatus};

/// Deterministic 32-byte key so tests never touch the OS keyring.
const TEST_DISCOVERY_KEY: &str = "0101010101010101010101010101010101010101010101010101010101010101";

fn temp_db_path(tag: &str) -> Utf8PathBuf {
    // Concurrent tests can read the same nanosecond; a counter keeps the
    // SQLite file (and its migration run) unique within the test binary.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "livtet-sync-server-{tag}-{}-{nanos}-{seq}.db",
        std::process::id(),
    ));
    Utf8PathBuf::from_path_buf(path).expect("temp dir path is valid UTF-8")
}

fn cleanup(path: &Utf8Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(format!("{}-wal", path));
    let _ = fs::remove_file(format!("{}-shm", path));
}

async fn spawn(db_path: &Utf8Path) -> StdioClient {
    let binary = env!("CARGO_BIN_EXE_livtet-sync-server");
    // The file directories are part of every pull's preconditions; without them
    // a pull stops before it dials anything, which would hide whether a pairing
    // is actually reachable.
    let books = db_path.with_extension("books");
    let covers = db_path.with_extension("covers");
    for dir in [&books, &covers] {
        let _ = fs::create_dir_all(dir);
    }
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
    match StdioClient::spawn_with_env(
        binary,
        &args,
        &[("LIVTET_DISCOVERY_KEY", TEST_DISCOVERY_KEY)],
    )
    .await
    {
        Ok(client) => client,
        Err(error) => {
            cleanup(db_path);
            panic!("failed to spawn daemon: {error}");
        }
    }
}

async fn ok_result(
    client: &mut StdioClient,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let response = client
        .request(method, params)
        .await
        .unwrap_or_else(|e| panic!("{method} transport: {e}"));
    assert!(
        response.error.is_none(),
        "{method} returned an error: {:?}",
        response.error
    );
    response.result.unwrap_or_else(|| panic!("{method} result"))
}

async fn error_code(client: &mut StdioClient, method: &str, params: serde_json::Value) -> i64 {
    let response = client
        .request(method, params)
        .await
        .unwrap_or_else(|e| panic!("{method} transport: {e}"));
    assert!(
        response.result.is_none(),
        "{method} unexpectedly succeeded: {:?}",
        response.result
    );
    response
        .error
        .unwrap_or_else(|| panic!("{method} error object"))
        .code
}

/// Ask the daemon to exit, then wait for it.
///
/// Always end a daemon this way rather than with `kill`. The daemon is a
/// separate process running the instrumented binary, and a SIGKILLed process
/// never flushes its coverage profile — so killing it silently discards every
/// line these tests just exercised in it.
async fn shutdown(client: &mut StdioClient) {
    let response = client
        .request(rpc::method::SHUTDOWN, serde_json::json!({}))
        .await
        .expect("shutdown transport");
    assert!(
        response.error.is_none(),
        "shutdown error: {:?}",
        response.error
    );
    let _ = tokio::time::timeout(Duration::from_secs(5), client.wait()).await;
}

/// A pool over the daemon's own database file, for seeding and asserting rows.
async fn pool_for(db_path: &Utf8Path) -> livtet_data::sql::SqlitePool {
    let options: livtet_data::sql::sqlite::SqliteConnectOptions =
        db_path.to_string().parse().expect("sqlite connect options");
    livtet_data::sql::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open daemon database")
}

/// Write the row the iroh accept loop creates when a peer files a claim.
///
/// It carries no socket origin and no stored address: the connection's node id
/// is the identity, and the pairing key.
async fn seed_claim(
    pool: &livtet_data::sql::SqlitePool,
    node_id: &str,
    device_id: DbId,
    desktop_id: DbId,
) {
    livtet_data::sql::query(
        "INSERT INTO pending_pairings \
             (token, desktop_id, listen_on, status_id, device_name, device_type_id, \
              device_id, origin_addr, created_at, expires_at) \
         VALUES (?, ?, NULL, ?, 'Remote Phone', NULL, ?, NULL, datetime('now'), \
                 datetime('now', '+5 minutes'))",
    )
    .bind(node_id)
    .bind(desktop_id.to_bytes().to_vec())
    .bind(DbId::from(PairingStatus::Pending).to_bytes().to_vec())
    .bind(device_id.to_bytes().to_vec())
    .execute(pool)
    .await
    .expect("seed an iroh claim");
}

#[tokio::test]
async fn devices_revoke_reports_what_it_removed_and_validates_the_id() {
    let db_path = temp_db_path("revoke");
    let mut client = spawn(&db_path).await;

    // A well-formed id with no matching row removes nothing, and says so
    // rather than erroring.
    let revoked = ok_result(
        &mut client,
        rpc::method::DEVICES_REVOKE,
        serde_json::json!({ "device_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV" }),
    )
    .await;
    assert_eq!(revoked["ok"].as_bool(), Some(true));
    assert_eq!(
        revoked["removed"].as_u64(),
        Some(0),
        "no device was paired, so nothing can be removed"
    );

    // The handler accepts the hex form as well as the ULID form.
    let hex = ok_result(
        &mut client,
        rpc::method::DEVICES_REVOKE,
        serde_json::json!({ "device_id": "0190b1c2d3e4f5a6b7c8d9e0f1a2b3c4" }),
    )
    .await;
    assert_eq!(hex["removed"].as_u64(), Some(0));

    // Neither form: the caller's mistake.
    assert_eq!(
        error_code(
            &mut client,
            rpc::method::DEVICES_REVOKE,
            serde_json::json!({ "device_id": "not-an-id" }),
        )
        .await,
        rpc::error_code::INVALID_PARAMS
    );

    shutdown(&mut client).await;
    cleanup(&db_path);
}

#[tokio::test]
async fn conflicts_list_is_empty_and_resolve_validates_its_resolution() {
    let db_path = temp_db_path("conflicts");
    let mut client = spawn(&db_path).await;

    let conflicts = ok_result(
        &mut client,
        rpc::method::CONFLICTS_LIST,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        conflicts.as_array().map(Vec::len),
        Some(0),
        "a fresh database has no conflicts: {conflicts}"
    );

    // Only three resolutions are accepted; anything else is rejected before
    // the engine is touched.
    for bad in ["theirs", "", "LOCAL"] {
        assert_eq!(
            error_code(
                &mut client,
                rpc::method::CONFLICTS_RESOLVE,
                serde_json::json!({ "id": 1, "resolution": bad }),
            )
            .await,
            rpc::error_code::INVALID_PARAMS,
            "resolution {bad:?} must be rejected"
        );
    }

    // A valid resolution against a conflict that does not exist resolves
    // nothing, rather than erroring.
    for good in ["local", "remote", "merged"] {
        let resolved = ok_result(
            &mut client,
            rpc::method::CONFLICTS_RESOLVE,
            serde_json::json!({ "id": 9_999, "resolution": good, "merged_payload": "{}" }),
        )
        .await;
        assert_eq!(
            resolved["ok"].as_bool(),
            Some(false),
            "there is no conflict 9999 to resolve with {good:?}"
        );
    }

    shutdown(&mut client).await;
    cleanup(&db_path);
}

#[tokio::test]
async fn the_dispatcher_rejects_unknown_methods_and_unusable_params() {
    let db_path = temp_db_path("dispatch");
    let mut client = spawn(&db_path).await;

    assert_eq!(
        error_code(&mut client, "does.not.exist", serde_json::json!({})).await,
        rpc::error_code::METHOD_NOT_FOUND
    );

    // The HTTP lifecycle is gone, not merely unhandled.
    for method in ["server.start", "server.stop", "requests.recent"] {
        assert_eq!(
            error_code(&mut client, method, serde_json::json!({})).await,
            rpc::error_code::METHOD_NOT_FOUND,
            "{method} must no longer exist"
        );
    }

    // A method that needs params, called without the field it needs.
    assert_eq!(
        error_code(
            &mut client,
            rpc::method::PAIRING_REJECT,
            serde_json::json!({})
        )
        .await,
        rpc::error_code::INVALID_PARAMS
    );

    // A params object of the wrong shape entirely.
    assert_eq!(
        error_code(
            &mut client,
            rpc::method::CONFLICTS_RESOLVE,
            serde_json::json!({ "id": "not-a-number", "resolution": "local" }),
        )
        .await,
        rpc::error_code::INVALID_PARAMS
    );

    shutdown(&mut client).await;
    cleanup(&db_path);
}

/// `request_params` is the typed sibling of `request`, serialising a params
/// struct rather than taking a pre-built `Value`.
#[tokio::test]
async fn typed_params_take_the_same_path_as_a_json_value() {
    let db_path = temp_db_path("typed");
    let mut client = spawn(&db_path).await;

    let begin = ok_result(
        &mut client,
        rpc::method::PAIRING_BEGIN,
        serde_json::json!({}),
    )
    .await;
    let node_id = begin["node_id"].as_str().expect("node_id").to_string();

    let response = client
        .request_params(
            rpc::method::PAIRING_REJECT,
            &rpc::TokenParams { token: node_id },
        )
        .await
        .expect("request_params transport");
    // `pairing.begin` publishes an identity but files no claim, so rejecting
    // it is the handler's invalid-params answer. Reaching that answer proves
    // the typed params survived the transport.
    assert_eq!(
        response.error.map(|error| error.code),
        Some(rpc::error_code::INVALID_PARAMS),
        "typed params must reach the handler"
    );

    shutdown(&mut client).await;
    cleanup(&db_path);
}

/// The identity published by `pairing.begin` is not itself a claim, so
/// rejecting it fails; only a request a remote device actually filed can be
/// rejected.
#[tokio::test]
async fn pairing_reject_requires_a_remote_claim() {
    let db_path = temp_db_path("reject");
    let mut client = spawn(&db_path).await;

    let status = ok_result(&mut client, rpc::method::STATUS, serde_json::json!({})).await;
    let local_device_id: DbId = status["device_id"]
        .as_str()
        .expect("device_id")
        .parse()
        .expect("local device id");

    let begin = ok_result(
        &mut client,
        rpc::method::PAIRING_BEGIN,
        serde_json::json!({}),
    )
    .await;
    let local_node_id = begin["node_id"].as_str().expect("node_id").to_string();

    // No pending row exists for the locally published identity.
    assert_eq!(
        error_code(
            &mut client,
            rpc::method::PAIRING_REJECT,
            serde_json::json!({ "token": local_node_id }),
        )
        .await,
        rpc::error_code::INVALID_PARAMS
    );

    // A claim filed over iroh is rejectable: the connection authenticates the
    // node id, so no socket origin is recorded or checked.
    let remote_node_id = "aa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899";
    let remote_device_id = DbId::new();
    let pool = pool_for(&db_path).await;
    seed_claim(&pool, remote_node_id, remote_device_id, local_device_id).await;

    let rejected = ok_result(
        &mut client,
        rpc::method::PAIRING_REJECT,
        serde_json::json!({ "token": remote_node_id }),
    )
    .await;
    assert_eq!(rejected["ok"].as_bool(), Some(true));

    let status_id: Vec<u8> =
        livtet_data::sql::query_scalar("SELECT status_id FROM pending_pairings WHERE token = ?")
            .bind(remote_node_id)
            .fetch_one(&pool)
            .await
            .expect("read the rejected row");
    assert_eq!(
        status_id,
        DbId::from(PairingStatus::Rejected).to_bytes().to_vec(),
        "the pending row is marked rejected"
    );
    pool.close().await;

    shutdown(&mut client).await;
    cleanup(&db_path);
}

/// The whole pairing handshake, through to a mutual paired device.
///
/// The claim row is exactly what the iroh accept loop writes: keyed by the
/// remote node id, with no socket origin and no stored address. Approval must
/// mint the token, record the node id, and make the pairing pullable — the
/// node id is the only address there is.
#[tokio::test]
async fn approving_an_iroh_claim_pairs_the_device() {
    let db_path = temp_db_path("approve");
    let mut client = spawn(&db_path).await;

    let status = ok_result(&mut client, rpc::method::STATUS, serde_json::json!({})).await;
    let local_device_id: DbId = status["device_id"]
        .as_str()
        .expect("device_id")
        .parse()
        .expect("local device id");

    // The claim is keyed by the *remote* node identity.
    let remote_node_id = "aa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899";
    let remote_device_id: DbId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("device id");
    let pool = pool_for(&db_path).await;
    seed_claim(&pool, remote_node_id, remote_device_id, local_device_id).await;

    let approved = ok_result(
        &mut client,
        rpc::method::PAIRING_APPROVE,
        serde_json::json!({ "token": remote_node_id }),
    )
    .await;
    let session_token = approved["session_token"].as_str().expect("session_token");
    assert_eq!(
        session_token.len(),
        26,
        "the session token is a ULID: {session_token}"
    );
    assert_eq!(
        approved["device_id"].as_str(),
        Some(remote_device_id.to_string().as_str()),
        "the claimed device id carries through to the pairing"
    );

    // The device is now listed, carrying the name the claim supplied.
    let devices = ok_result(
        &mut client,
        rpc::method::DEVICES_LIST,
        serde_json::json!({}),
    )
    .await;
    let entries = devices.as_array().expect("devices array");
    assert_eq!(entries.len(), 1, "exactly one device paired: {devices}");
    assert_eq!(
        entries[0]["device_id"].as_str(),
        Some(remote_device_id.to_string().as_str())
    );
    assert_eq!(entries[0]["name"].as_str(), Some("Remote Phone"));
    assert_eq!(
        entries[0]["outbound"],
        serde_json::Value::Bool(true),
        "an approved iroh claim is mutual and pullable: {devices}"
    );
    assert!(
        entries[0].get("listen_on").is_none(),
        "there is no stored HTTP address to report: {devices}"
    );

    // The session token and node id are persisted, not just returned: the node
    // id is what every later dial addresses.
    let (stored_token, stored_node, outbound, listen_on): (
        String,
        Option<String>,
        i64,
        Option<String>,
    ) = livtet_data::sql::query_as(
        "SELECT session_token, node_id, outbound, listen_on \
         FROM paired_devices WHERE device_id = ?",
    )
    .bind(remote_device_id.to_bytes().to_vec())
    .fetch_one(&pool)
    .await
    .expect("read back the pairing");
    assert_eq!(stored_token, session_token);
    assert_eq!(stored_node.as_deref(), Some(remote_node_id));
    assert_eq!(outbound, 1, "an approved claim is mutual");
    assert_eq!(
        listen_on, None,
        "the node id is the address; no socket address is stored"
    );

    // Approving the same ticket twice must not pair a second device: the
    // upsert keys on device_id.
    let second = ok_result(
        &mut client,
        rpc::method::PAIRING_APPROVE,
        serde_json::json!({ "token": remote_node_id }),
    )
    .await;
    assert_eq!(
        second["device_id"].as_str(),
        Some(remote_device_id.to_string().as_str())
    );
    let devices = ok_result(
        &mut client,
        rpc::method::DEVICES_LIST,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        devices.as_array().map(Vec::len),
        Some(1),
        "re-approving must update the device, not add another: {devices}"
    );

    // And now revoke really removes something, which the no-op revoke test
    // above cannot show.
    let revoked = ok_result(
        &mut client,
        rpc::method::DEVICES_REVOKE,
        serde_json::json!({ "device_id": remote_device_id.to_string() }),
    )
    .await;
    assert_eq!(revoked["removed"].as_u64(), Some(1));
    let devices = ok_result(
        &mut client,
        rpc::method::DEVICES_LIST,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(devices.as_array().map(Vec::len), Some(0));

    pool.close().await;
    shutdown(&mut client).await;
    cleanup(&db_path);
}

/// Pairings that already name a node id get their missing half on the next
/// start: the token is already held by both sides, and only the flag gated it.
/// A row with no node id has nothing to dial and stays one-way.
#[tokio::test]
async fn starting_up_promotes_pairings_with_a_node_id() {
    let db_path = temp_db_path("promote");
    let mut client = spawn(&db_path).await;
    // Wait for the daemon to open and migrate the database before seeding.
    ok_result(&mut client, rpc::method::STATUS, serde_json::json!({})).await;

    let pool = pool_for(&db_path).await;

    let peer = DbId::new().to_bytes().to_vec();
    let legacy = DbId::new().to_bytes().to_vec();
    for (id, name, node_id, token) in [
        (
            peer.clone(),
            "Node Peer",
            Some("aa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899"),
            "legacy-token-peer",
        ),
        (legacy.clone(), "Address Only", None, "legacy-token-own"),
    ] {
        livtet_data::sql::query(
            "INSERT INTO paired_devices (device_id, name, listen_on, device_type_id, \
                 paired_at, last_sync_at, session_token, node_id, outbound) \
             VALUES (?, ?, NULL, NULL, datetime('now'), NULL, ?, ?, 0)",
        )
        .bind(id)
        .bind(name)
        .bind(token)
        .bind(node_id)
        .execute(&pool)
        .await
        .expect("seed a legacy pairing");
    }

    shutdown(&mut client).await;
    let mut restarted = spawn(&db_path).await;
    ok_result(&mut restarted, rpc::method::STATUS, serde_json::json!({})).await;

    let promoted: i64 =
        livtet_data::sql::query_scalar("SELECT outbound FROM paired_devices WHERE device_id = ?")
            .bind(peer)
            .fetch_one(&pool)
            .await
            .expect("read the promoted row");
    assert_eq!(promoted, 1, "a node id is enough to pull");

    let untouched: i64 =
        livtet_data::sql::query_scalar("SELECT outbound FROM paired_devices WHERE device_id = ?")
            .bind(legacy)
            .fetch_one(&pool)
            .await
            .expect("read the address-only row");
    assert_eq!(
        untouched, 0,
        "a row without a node id has no address to dial"
    );

    pool.close().await;
    shutdown(&mut restarted).await;
}

/// `kill` is the escape hatch for a daemon that will not answer `shutdown`.
/// It is deliberately the only test that uses it: a SIGKILLed child never
/// flushes its coverage profile, so every other test here exits cleanly.
#[tokio::test]
async fn kill_terminates_a_daemon_that_is_not_asked_nicely() {
    let db_path = temp_db_path("kill");
    let mut client = spawn(&db_path).await;

    // Confirm it is alive and answering before killing it.
    let health = ok_result(&mut client, rpc::method::HEALTH, serde_json::json!({})).await;
    assert_eq!(health["status"].as_str(), Some("ok"));

    client.kill();
    // `wait` must return once the child is gone rather than hanging.
    let exited = tokio::time::timeout(Duration::from_secs(5), client.wait()).await;
    assert!(exited.is_ok(), "the killed daemon must be reaped");

    cleanup(&db_path);
}
