//! Integration test: spawn the built `livtet-sync-server` binary and drive
//! it over stdio with the JSON-RPC NDJSON protocol.

use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use livtet_sync_server::rpc::{self, StdioClient};
use livtet_types::{DbId, PairingStatus};

/// Deterministic 32-byte key so tests never touch the OS keyring.
const TEST_DISCOVERY_KEY: &str = "0101010101010101010101010101010101010101010101010101010101010101";

fn temp_db_path() -> Utf8PathBuf {
    // The wall clock alone collides: concurrent tests can read the same
    // nanosecond, and two daemons migrating one SQLite file fail with
    // "database is locked" / duplicate `core_migrations`. A per-process
    // counter makes the path unique within the test binary.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "livtet-sync-server-test-{}-{nanos}-{seq}.db",
        std::process::id(),
    ));
    Utf8PathBuf::from_path_buf(path).expect("temp dir path is valid UTF-8")
}

fn cleanup(path: &Utf8Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(format!("{}-wal", path));
    let _ = fs::remove_file(format!("{}-shm", path));
}

#[tokio::test]
async fn rpc_health_status_pairing_roundtrip() {
    let db_path = temp_db_path();
    let binary = env!("CARGO_BIN_EXE_livtet-sync-server");
    let args = vec![
        "--db".to_string(),
        db_path.to_string(),
        "--port".to_string(),
        "0".to_string(),
    ];

    let mut client = match StdioClient::spawn_with_env(
        binary,
        &args,
        &[("LIVTET_DISCOVERY_KEY", TEST_DISCOVERY_KEY)],
    )
    .await
    {
        Ok(client) => client,
        Err(error) => {
            cleanup(&db_path);
            panic!("failed to spawn daemon: {error}");
        }
    };

    // `health`
    let health = client
        .request(rpc::method::HEALTH, serde_json::json!({}))
        .await
        .expect("health response");
    assert!(health.error.is_none(), "health error: {:?}", health.error);
    let health = health.result.expect("health result");
    assert_eq!(health["status"].as_str(), Some("ok"));
    assert!(health["version"].as_str().is_some());
    assert!(
        health["node_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "health must name the node peers dial: {health}"
    );
    assert!(
        health["port"].as_u64().is_some(),
        "health reports the endpoint port: {health}"
    );
    assert!(
        health.get("server_running").is_none(),
        "the HTTP server is gone: {health}"
    );

    // `status`
    let status = client
        .request(rpc::method::STATUS, serde_json::json!({}))
        .await
        .expect("status response");
    assert!(status.error.is_none(), "status error: {:?}", status.error);
    let status = status.result.expect("status result");
    let device_id = status["device_id"]
        .as_str()
        .expect("device_id must be a non-empty string");
    assert!(!device_id.is_empty());
    assert!(
        status["node_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "status must name the node peers dial: {status}"
    );
    assert!(status["port"].as_u64().is_some());
    assert!(
        status.get("server_running").is_none() && status.get("host").is_none(),
        "no HTTP lifecycle fields remain: {status}"
    );

    // `pairing.begin`
    let begin = client
        .request(rpc::method::PAIRING_BEGIN, serde_json::json!({}))
        .await
        .expect("pairing.begin response");
    assert!(begin.error.is_none(), "begin error: {:?}", begin.error);
    let begin = begin.result.expect("begin result");
    let node_id = begin["node_id"].as_str().expect("node_id").to_string();
    assert!(!node_id.is_empty(), "node_id must be non-empty");
    assert!(
        begin["token"].is_null() && begin["uri"].is_null(),
        "the identity contract mints no token or pairing URI: {begin}"
    );
    assert!(
        begin["listen_on"].is_null(),
        "there is no HTTP endpoint to advertise: {begin}"
    );
    assert_eq!(
        begin["identity"]["node_id"].as_str(),
        Some(node_id.as_str()),
        "identity must repeat the node id: {begin}"
    );
    assert!(begin["expires_at"].as_str().is_some());

    // `pairing.list`: a freshly minted ticket exists only so a remote device
    // can claim it. Until that happens there is no device to pair with, so the
    // desktop must not see its own unclaimed ticket in the pairing list.
    let list = client
        .request(rpc::method::PAIRING_LIST, serde_json::json!({}))
        .await
        .expect("pairing.list response");
    assert!(list.error.is_none(), "list error: {:?}", list.error);
    let list = list.result.expect("list result");
    let entries = list.as_array().expect("list result is an array");
    assert!(
        !entries
            .iter()
            .any(|entry| entry["node_id"].as_str() == Some(node_id.as_str())),
        "a locally minted identity must not be listed as a pairable device: {list}"
    );

    let status = client
        .request(rpc::method::STATUS, serde_json::json!({}))
        .await
        .expect("status response");
    let status = status.result.expect("status result");
    assert_eq!(
        status["pending_pairing_count"].as_i64(),
        Some(0),
        "unclaimed ticket must not be counted as a pending pairing: {status}"
    );

    // A claim that arrived over iroh has no socket origin; it is the row the
    // accept loop writes before holding the stream for a decision.
    let remote_node_id = "aa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899";
    let remote_device_id: DbId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("device id");
    let local_device_id: DbId = device_id.parse().expect("local device id");
    let pool = livtet_data::sql::sqlite::SqlitePoolOptions::new()
        .connect_with(
            livtet_data::sql::sqlite::SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(false),
        )
        .await
        .expect("open db");
    livtet_data::sql::query(
        "INSERT INTO pending_pairings \
             (token, desktop_id, listen_on, status_id, device_name, device_type_id, \
              device_id, origin_addr, created_at, expires_at) \
         VALUES (?, ?, NULL, ?, 'Remote Phone', NULL, ?, NULL, datetime('now'), \
                 datetime('now', '+5 minutes'))",
    )
    .bind(remote_node_id)
    .bind(local_device_id.to_bytes().to_vec())
    .bind(DbId::from(PairingStatus::Pending).to_bytes().to_vec())
    .bind(remote_device_id.to_bytes().to_vec())
    .execute(&pool)
    .await
    .expect("seed an iroh claim");
    pool.close().await;

    let list = client
        .request(rpc::method::PAIRING_LIST, serde_json::json!({}))
        .await
        .expect("pairing.list response");
    let list = list.result.expect("list result");
    let entries = list.as_array().expect("list result is an array");
    assert!(
        entries
            .iter()
            .any(|entry| entry["node_id"].as_str() == Some(remote_node_id)),
        "an iroh claim must be listed: {list}"
    );

    let status = client
        .request(rpc::method::STATUS, serde_json::json!({}))
        .await
        .expect("status response");
    let status = status.result.expect("status result");
    assert_eq!(
        status["pending_pairing_count"].as_i64(),
        Some(1),
        "an iroh claim must be counted: {status}"
    );

    // `shutdown`
    let shutdown = client
        .request(rpc::method::SHUTDOWN, serde_json::json!({}))
        .await
        .expect("shutdown response");
    assert!(
        shutdown.error.is_none(),
        "shutdown error: {:?}",
        shutdown.error
    );

    // The daemon should exit on its own once shutdown is handled.
    let _ = tokio::time::timeout(Duration::from_secs(5), client.wait()).await;

    cleanup(&db_path);
}

#[tokio::test]
async fn pairing_approve_rejects_unclaimed_ticket() {
    let db_path = temp_db_path();
    let binary = env!("CARGO_BIN_EXE_livtet-sync-server");
    let args = vec![
        "--db".to_string(),
        db_path.to_string(),
        "--port".to_string(),
        "0".to_string(),
    ];

    let mut client = match StdioClient::spawn_with_env(
        binary,
        &args,
        &[("LIVTET_DISCOVERY_KEY", TEST_DISCOVERY_KEY)],
    )
    .await
    {
        Ok(client) => client,
        Err(error) => {
            cleanup(&db_path);
            panic!("failed to spawn daemon: {error}");
        }
    };

    // `pairing.begin` only publishes this device's identity; no pending row
    // exists until a remote device actually claims it.
    let begin = client
        .request(rpc::method::PAIRING_BEGIN, serde_json::json!({}))
        .await
        .expect("pairing.begin response");
    let begin = begin.result.expect("begin result");
    let node_id = begin["node_id"].as_str().expect("node_id").to_string();

    // Approving an identity with no remote claim must fail closed rather than
    // minting a phantom device for the desktop itself.
    let approve = client
        .request(
            rpc::method::PAIRING_APPROVE,
            serde_json::json!({ "token": node_id }),
        )
        .await
        .expect("pairing.approve response");
    assert!(
        approve.error.is_some(),
        "approving an unclaimed ticket must fail: {:?}",
        approve.result
    );

    // No device should have been paired.
    let devices = client
        .request(rpc::method::DEVICES_LIST, serde_json::json!({}))
        .await
        .expect("devices.list response");
    let devices = devices.result.expect("devices result");
    assert_eq!(
        devices.as_array().map(Vec::len),
        Some(0),
        "no paired device should exist after a rejected approval: {devices}"
    );

    let shutdown = client
        .request(rpc::method::SHUTDOWN, serde_json::json!({}))
        .await
        .expect("shutdown response");
    assert!(
        shutdown.error.is_none(),
        "shutdown error: {:?}",
        shutdown.error
    );
    let _ = tokio::time::timeout(Duration::from_secs(5), client.wait()).await;

    cleanup(&db_path);
}
