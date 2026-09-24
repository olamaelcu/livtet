//! Integration test: spawn the built `livtet-sync-server` binary and drive
//! it over stdio with the JSON-RPC NDJSON protocol.

use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use livtet_sync_server::rpc::{self, StdioClient};

fn temp_db_path() -> Utf8PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "livtet-sync-server-test-{}-{}.db",
        std::process::id(),
        nanos
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

    let mut client = match StdioClient::spawn(binary, &args).await {
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
    assert_eq!(health["server_running"].as_bool(), Some(true));
    assert!(health["port"].as_u64().unwrap_or(0) > 0);

    // `status`
    let status = client
        .request(rpc::method::STATUS, serde_json::json!({}))
        .await
        .expect("status response");
    assert!(status.error.is_none(), "status error: {:?}", status.error);
    let status = status.result.expect("status result");
    assert!(
        status["device_id"]
            .as_str()
            .map(|value| !value.is_empty())
            .unwrap_or(false),
        "device_id must be a non-empty string: {status}"
    );
    assert_eq!(status["server_running"].as_bool(), Some(true));
    assert!(status["port"].as_u64().unwrap_or(0) > 0);

    // `requests.recent`
    let recent = client
        .request(
            rpc::method::REQUESTS_RECENT,
            serde_json::json!({ "limit": 10 }),
        )
        .await
        .expect("requests.recent response");
    assert!(recent.error.is_none(), "recent error: {:?}", recent.error);
    assert!(recent.result.expect("recent result").is_array());

    // `pairing.begin`
    let begin = client
        .request(rpc::method::PAIRING_BEGIN, serde_json::json!({}))
        .await
        .expect("pairing.begin response");
    assert!(begin.error.is_none(), "begin error: {:?}", begin.error);
    let begin = begin.result.expect("begin result");
    let token = begin["token"].as_str().expect("token").to_string();
    assert!(!token.is_empty(), "token must be non-empty");
    assert!(
        begin["uri"]
            .as_str()
            .unwrap_or("")
            .starts_with("livtet://sync?"),
        "uri must be a livtet:// sync URI: {begin}"
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
            .any(|entry| entry["token"].as_str() == Some(token.as_str())),
        "unclaimed ticket must not be listed as a pairable device: {list}"
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

    // A remote device claims the ticket over the HTTP pairing endpoint.
    let host = status["host"].as_str().expect("host").to_string();
    let port = status["port"].as_u64().expect("port");
    let pair_url = format!("http://{host}:{port}{}", livtet_sync_http::PAIR_PATH);
    let response = reqwest::Client::new()
        .post(&pair_url)
        .json(&serde_json::json!({
            "device_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "name": "Remote Phone",
            "device_type": "android",
            "token": token,
        }))
        .send()
        .await
        .expect("pair POST");
    assert!(
        response.status().is_success(),
        "pair POST failed: {}",
        response.status()
    );

    // Once claimed, the pending pairing is visible and countable again.
    let list = client
        .request(rpc::method::PAIRING_LIST, serde_json::json!({}))
        .await
        .expect("pairing.list response");
    let list = list.result.expect("list result");
    let entries = list.as_array().expect("list result is an array");
    assert!(
        entries
            .iter()
            .any(|entry| entry["token"].as_str() == Some(token.as_str())),
        "claimed token missing from pairing.list: {list}"
    );

    let status = client
        .request(rpc::method::STATUS, serde_json::json!({}))
        .await
        .expect("status response");
    let status = status.result.expect("status result");
    assert_eq!(
        status["pending_pairing_count"].as_i64(),
        Some(1),
        "claimed ticket must be counted as a pending pairing: {status}"
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

    let mut client = match StdioClient::spawn(binary, &args).await {
        Ok(client) => client,
        Err(error) => {
            cleanup(&db_path);
            panic!("failed to spawn daemon: {error}");
        }
    };

    // Mint a local ticket. Nothing has claimed it, so it is not a pairing.
    let begin = client
        .request(rpc::method::PAIRING_BEGIN, serde_json::json!({}))
        .await
        .expect("pairing.begin response");
    let begin = begin.result.expect("begin result");
    let token = begin["token"].as_str().expect("token").to_string();

    // Approving an unclaimed ticket must fail closed rather than minting a
    // phantom device for the desktop itself.
    let approve = client
        .request(
            rpc::method::PAIRING_APPROVE,
            serde_json::json!({ "token": token }),
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
