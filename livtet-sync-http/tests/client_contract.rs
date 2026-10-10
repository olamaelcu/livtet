//! The `reqwest` client against the `poem` server over a real loopback
//! socket: every `SyncHttpClient` method, the `SyncSession` wrapper, and
//! the server instance lifecycle.

use std::net::SocketAddr;

use livtet_sync_http::{
    ReqwestHttpClient, ReqwestSyncClient, SyncHttpClient, SyncHttpError, SyncServerInstance,
};
use livtet_test_utils::sync::{Device, change};

const LOOPBACK_ANY_PORT: ([u8; 4], u16) = ([127, 0, 0, 1], 0);

/// A server over `device`, listening on an ephemeral loopback port.
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

async fn connected_client(url: &str) -> ReqwestHttpClient {
    let mut client = ReqwestHttpClient::new();
    client.connect(url).await.expect("connect");
    client
}

#[tokio::test]
async fn server_binds_the_requested_address_and_reports_the_port() {
    let device = Device::new("server").await;
    let mut server = SyncServerInstance::new();
    let addr = server
        .start_on(
            device.conn.clone(),
            "server".into(),
            SocketAddr::from(LOOPBACK_ANY_PORT),
        )
        .await
        .unwrap();

    assert!(addr.ip().is_loopback());
    assert_ne!(addr.port(), 0);
    assert!(server.is_running());
    server.stop().await.unwrap();
}

#[tokio::test]
async fn server_refuses_a_second_start_and_can_restart_after_stop() {
    let device = Device::new("server").await;
    let (mut server, url) = serve(&device).await;

    let again = server
        .start_on(
            device.conn.clone(),
            "server".into(),
            SocketAddr::from(LOOPBACK_ANY_PORT),
        )
        .await;
    assert!(again.is_err());

    server.stop().await.unwrap();
    assert!(!server.is_running());
    assert!(ReqwestHttpClient::new().connect(&url).await.is_err());
    server.stop().await.expect("stopping twice is a no-op");

    let addr = server
        .start_on(
            device.conn.clone(),
            "server".into(),
            SocketAddr::from(LOOPBACK_ANY_PORT),
        )
        .await
        .unwrap();
    connected_client(&format!("http://{addr}")).await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn calls_before_connect_fail_with_not_connected() {
    let client = ReqwestHttpClient::new();
    assert!(client.base_url().is_none());
    assert!(matches!(
        client.status().await,
        Err(SyncHttpError::NotConnected)
    ));
    assert!(matches!(
        client.pull_since(0, 10).await,
        Err(SyncHttpError::NotConnected)
    ));
    assert!(matches!(
        client.push(vec![]).await,
        Err(SyncHttpError::NotConnected)
    ));
}

#[tokio::test]
async fn connecting_to_nothing_fails_and_stays_disconnected() {
    // A port nothing is serving: reserved by binding and dropping a listener.
    let port = livtet_test_utils::fault::reserve_port();

    let mut client = ReqwestHttpClient::new();
    let result = client.connect(&format!("http://127.0.0.1:{port}")).await;
    assert!(
        matches!(result, Err(SyncHttpError::Transport(_))),
        "{result:?}"
    );
    assert!(client.base_url().is_none());
}

#[tokio::test]
async fn connect_remembers_the_base_url_without_a_trailing_slash() {
    let device = Device::new("server").await;
    let (mut server, url) = serve(&device).await;

    let client = connected_client(&format!("{url}/")).await;
    assert_eq!(client.base_url(), Some(url.as_str()));
    server.stop().await.unwrap();
}

#[tokio::test]
async fn reads_over_the_wire_match_the_engine() {
    let device = Device::seeded("server").await;
    let (mut server, url) = serve(&device).await;
    let client = connected_client(&url).await;

    let status = client.status().await.unwrap();
    assert_eq!(status.device_id, "server");
    assert_eq!(
        status.latest_version,
        device.engine.get_latest_version().await.unwrap()
    );

    let remote = client.pull_since(3, 10).await.unwrap();
    let local = device.engine.pull_changes(3, 10).await.unwrap();
    assert_eq!(remote.has_more, local.has_more);
    assert_eq!(remote.latest_version, local.latest_version);
    assert_eq!(
        serde_json::to_value(&remote.changes).unwrap(),
        serde_json::to_value(&local.changes).unwrap()
    );

    let remote = client.pull_full().await.unwrap();
    let local = device.engine.pull_full().await.unwrap();
    assert_eq!(remote.version, local.version);
    assert_eq!(
        serde_json::to_value(&remote.entities).unwrap(),
        serde_json::to_value(&local.entities).unwrap()
    );

    server.stop().await.unwrap();
}

#[tokio::test]
async fn push_conflicts_and_resolution_round_trip() {
    let device = Device::seeded("server").await;
    let (mut server, url) = serve(&device).await;
    let client = connected_client(&url).await;

    device
        .execute("UPDATE works SET title = 'Local' WHERE id = (SELECT id FROM works LIMIT 1)")
        .await;
    let edit = device.logged("work", "UPDATE").await.pop().expect("logged");

    let response = client
        .push(vec![change(
            "work",
            &edit.entity_id,
            "DELETE",
            edit.version - 1,
            "{}",
        )])
        .await
        .unwrap();
    assert!(!response.accepted);
    assert_eq!(response.conflicts.len(), 1);
    let conflict_id = response.conflicts[0].id;

    let invalid = client.resolve_conflict(conflict_id, "mine", None).await;
    assert!(
        matches!(invalid, Err(SyncHttpError::Status { code: 400, .. })),
        "{invalid:?}"
    );

    assert!(
        client
            .resolve_conflict(conflict_id, "local", None)
            .await
            .unwrap()
    );
    // The server answers 404 once it is resolved; the client reports false.
    assert!(
        !client
            .resolve_conflict(conflict_id, "local", None)
            .await
            .unwrap()
    );
    assert!(device.engine.list_conflicts().await.unwrap().is_empty());

    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_session_pairs_the_local_engine_with_the_remote_server() {
    let server_device = Device::seeded("server").await;
    let phone = Device::new("phone").await;
    let (mut server, url) = serve(&server_device).await;

    let mut session = ReqwestSyncClient::with_default_http(&phone.conn, "phone");
    assert!(session.base_url().is_none());
    session.connect(&url).await.unwrap();
    assert_eq!(session.base_url(), Some(url.as_str()));

    assert_eq!(session.engine().device_id(), "phone");
    assert_eq!(session.local_latest_version().await.unwrap(), 0);
    assert_eq!(
        session.status().await.unwrap().latest_version,
        server_device.engine.get_latest_version().await.unwrap()
    );
    let page = session.pull_since(0, 5).await.unwrap();
    assert_eq!(page.changes.len(), 5);

    server.stop().await.unwrap();
}

/// `with_http` is the bring-your-own-`reqwest::Client` constructor, for
/// callers that need their own timeouts, proxy or TLS configuration. Its
/// sibling `with_default_http` is covered above; this is the same contract
/// over a caller-supplied client.
#[tokio::test]
async fn a_session_accepts_a_caller_supplied_http_client() {
    let server_device = Device::seeded("server").await;
    let phone = Device::new("phone").await;
    let (mut server, url) = serve(&server_device).await;

    let mut session = ReqwestSyncClient::with_http(&phone.conn, "phone", reqwest::Client::new());
    assert!(session.base_url().is_none());
    session.connect(&url).await.expect("connect");
    assert_eq!(session.base_url(), Some(url.as_str()));
    assert_eq!(
        session.status().await.expect("status").latest_version,
        server_device.engine.get_latest_version().await.unwrap()
    );

    server.stop().await.unwrap();
}

/// `start` is the `0.0.0.0:{port}` convenience over `start_on`. A port is
/// reserved by binding and dropping a listener, the same idiom
/// `connecting_to_nothing_fails_and_stays_disconnected` uses.
#[tokio::test]
async fn start_binds_every_interface_on_the_given_port() {
    let device = Device::new("server").await;
    let port = livtet_test_utils::fault::reserve_port();

    let mut server = SyncServerInstance::new();
    server
        .start(
            device.conn.clone(),
            device.engine.device_id().to_string(),
            port,
        )
        .await
        .expect("start on a reserved port");
    assert!(server.is_running());

    // Reachable on loopback, which 0.0.0.0 covers.
    let client = connected_client(&format!("http://127.0.0.1:{port}")).await;
    assert_eq!(
        client.status().await.expect("status").device_id,
        device.engine.device_id()
    );

    server.stop().await.unwrap();
    assert!(!server.is_running());
}

#[test]
fn a_default_server_instance_is_not_running() {
    assert!(!SyncServerInstance::default().is_running());
}

#[test]
fn a_default_http_client_has_no_base_url() {
    assert!(ReqwestHttpClient::default().base_url().is_none());
}

/// A base url that passes the `join_url` scheme check but is not a parseable
/// URL fails as `Url`, not as `Transport` — the parse happens before any
/// request is attempted. `http://[` starts an IPv6 literal that never closes.
#[tokio::test]
async fn an_unparseable_base_url_fails_before_any_request() {
    let mut client = ReqwestHttpClient::new();
    let err = client
        .connect("http://[")
        .await
        .expect_err("an unparseable base url must not connect");
    assert!(matches!(err, SyncHttpError::Url(_)), "{err:?}");
    assert!(
        !err.to_string().contains("must start with http"),
        "this must be the parse failure, not join_url's own scheme check: {err}"
    );
    assert!(client.base_url().is_none());
}
