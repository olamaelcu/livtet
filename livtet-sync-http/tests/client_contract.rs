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
    // Bind then drop a listener to get a port nothing is serving.
    let port = std::net::TcpListener::bind(SocketAddr::from(LOOPBACK_ANY_PORT))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();

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
