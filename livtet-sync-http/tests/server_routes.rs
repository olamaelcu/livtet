//! Direct HTTP coverage of the `poem` route handlers that the typed
//! `ReqwestHttpClient` contract tests don't reach: listing conflicts, the
//! pair-status SSE endpoint's not-found arm, and the `get_file` request
//! validation (malformed id, unknown inventory row). Driven with raw
//! `reqwest` GETs against a real loopback server, the same harness
//! `client_contract.rs` uses.

use std::net::SocketAddr;

use livtet_sync_http::SyncServerInstance;
use livtet_test_utils::sync::Device;

const LOOPBACK_ANY_PORT: ([u8; 4], u16) = ([127, 0, 0, 1], 0);

async fn serve(device: &Device) -> (SyncServerInstance, String) {
    let mut server = SyncServerInstance::new();
    let addr = server
        .start_on(
            device.conn.clone(),
            device.engine.device_id().to_string(),
            SocketAddr::from(LOOPBACK_ANY_PORT),
        )
        .await
        .expect("start server");
    (server, format!("http://{addr}"))
}

#[tokio::test]
async fn get_conflicts_returns_an_empty_list_for_a_fresh_engine() {
    let device = Device::new("server").await;
    let (mut server, url) = serve(&device).await;

    let resp = reqwest::Client::new()
        .get(format!("{url}/sync/conflicts"))
        .send()
        .await
        .expect("GET /sync/conflicts");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert!(
        body.as_array().expect("conflicts is an array").is_empty(),
        "a fresh engine has no conflicts"
    );

    server.stop().await.unwrap();
}

#[tokio::test]
async fn pair_status_for_an_unknown_token_is_not_found() {
    let device = Device::new("server").await;
    let (mut server, url) = serve(&device).await;

    // No pairing waiter is registered for this token, so the SSE handler
    // short-circuits with 404 rather than opening a stream.
    let resp = reqwest::Client::new()
        .get(format!("{url}/sync/pair/status/no-such-token"))
        .send()
        .await
        .expect("GET pair status");
    assert_eq!(resp.status(), 404);

    server.stop().await.unwrap();
}

#[tokio::test]
async fn file_with_a_malformed_id_is_bad_request() {
    let device = Device::new("server").await;
    let (mut server, url) = serve(&device).await;

    // Not valid hex -> the id guard rejects it before any DB lookup.
    let resp = reqwest::Client::new()
        .get(format!("{url}/sync/files/not-hex"))
        .send()
        .await
        .expect("GET file");
    assert_eq!(resp.status(), 400);

    server.stop().await.unwrap();
}

#[tokio::test]
async fn file_for_an_unknown_inventory_id_is_not_found() {
    let device = Device::new("server").await;
    let (mut server, url) = serve(&device).await;

    // Well-formed 16-byte hex id with no matching digital_inventory row.
    let hex_id = "0".repeat(32);
    let resp = reqwest::Client::new()
        .get(format!("{url}/sync/files/{hex_id}"))
        .send()
        .await
        .expect("GET file");
    assert_eq!(resp.status(), 404);

    server.stop().await.unwrap();
}
