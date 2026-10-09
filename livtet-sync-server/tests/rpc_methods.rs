//! The RPC methods `rpc_health.rs` does not reach: server start/stop,
//! pairing rejection, device revocation, conflict listing and resolution,
//! and how the dispatcher answers a method or a params object it cannot use.
//!
//! One daemon serves the whole file — spawning the binary per test is the
//! expensive part, and none of these leave state that affects the others
//! except the server start/stop pair, which restores itself.

use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use livtet_sync_server::rpc::{self, StdioClient};

fn temp_db_path(tag: &str) -> Utf8PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "livtet-sync-server-{tag}-{}-{}.db",
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

async fn spawn(db_path: &Utf8Path) -> StdioClient {
    let binary = env!("CARGO_BIN_EXE_livtet-sync-server");
    let args = vec![
        "--db".to_string(),
        db_path.to_string(),
        "--port".to_string(),
        "0".to_string(),
    ];
    match StdioClient::spawn(binary, &args).await {
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

async fn server_running(client: &mut StdioClient) -> bool {
    let status = ok_result(client, rpc::method::STATUS, serde_json::json!({})).await;
    status["server_running"]
        .as_bool()
        .expect("status reports server_running")
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

#[tokio::test]
async fn server_start_and_stop_are_reflected_in_status() {
    let db_path = temp_db_path("start-stop");
    let mut client = spawn(&db_path).await;

    assert!(
        server_running(&mut client).await,
        "boots with the server up"
    );

    let stopped = ok_result(&mut client, rpc::method::SERVER_STOP, serde_json::json!({})).await;
    assert_eq!(stopped["ok"].as_bool(), Some(true));
    assert!(
        !server_running(&mut client).await,
        "status must reflect the stop"
    );

    // Stopping an already-stopped server is not an error.
    let again = ok_result(&mut client, rpc::method::SERVER_STOP, serde_json::json!({})).await;
    assert_eq!(again["ok"].as_bool(), Some(true));

    let started = ok_result(
        &mut client,
        rpc::method::SERVER_START,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(started["ok"].as_bool(), Some(true));
    assert_eq!(
        started.get("already_running").and_then(|v| v.as_bool()),
        None,
        "a real start must not claim it was already running"
    );
    assert!(server_running(&mut client).await);

    // A second start is a no-op that says so, rather than rebinding the port.
    let redundant = ok_result(
        &mut client,
        rpc::method::SERVER_START,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(redundant["already_running"].as_bool(), Some(true));

    shutdown(&mut client).await;
    cleanup(&db_path);
}

#[tokio::test]
async fn pairing_reject_marks_a_minted_token_and_refuses_unknown_ones() {
    let db_path = temp_db_path("reject");
    let mut client = spawn(&db_path).await;

    let begin = ok_result(
        &mut client,
        rpc::method::PAIRING_BEGIN,
        serde_json::json!({}),
    )
    .await;
    let token = begin["token"].as_str().expect("token").to_string();

    let rejected = ok_result(
        &mut client,
        rpc::method::PAIRING_REJECT,
        serde_json::json!({ "token": token }),
    )
    .await;
    assert_eq!(rejected["ok"].as_bool(), Some(true));

    // A token that was never minted is the caller's mistake.
    assert_eq!(
        error_code(
            &mut client,
            rpc::method::PAIRING_REJECT,
            serde_json::json!({ "token": "01ARZ3NDEKTSV4RRFFQ69G5FAV" }),
        )
        .await,
        rpc::error_code::INVALID_PARAMS
    );

    shutdown(&mut client).await;
    cleanup(&db_path);
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
    let token = begin["token"].as_str().expect("token").to_string();

    let response = client
        .request_params(
            rpc::method::PAIRING_REJECT,
            &rpc::TokenParams {
                token: token.clone(),
            },
        )
        .await
        .expect("request_params transport");
    assert!(
        response.error.is_none(),
        "typed params must reach the handler: {:?}",
        response.error
    );
    assert_eq!(response.result.expect("result")["ok"].as_bool(), Some(true));

    shutdown(&mut client).await;
    cleanup(&db_path);
}

/// The daemon pushes notifications as well as answering requests. Which one
/// arrives first depends on timing, so this asserts the shape of whatever
/// comes rather than demanding a particular event.
#[tokio::test]
async fn the_daemon_emits_well_formed_notifications() {
    let db_path = temp_db_path("notify");
    let mut client = spawn(&db_path).await;

    // Restarting the server is guaranteed to emit started/stopped events.
    let _ = ok_result(&mut client, rpc::method::SERVER_STOP, serde_json::json!({})).await;
    let _ = ok_result(
        &mut client,
        rpc::method::SERVER_START,
        serde_json::json!({}),
    )
    .await;

    let known = [
        rpc::method::NOTIFY_PAIRING_REQUESTED,
        rpc::method::NOTIFY_REQUEST_RECEIVED,
        rpc::method::NOTIFY_SYNC_COMPLETED,
        rpc::method::NOTIFY_SERVER_STARTED,
        rpc::method::NOTIFY_SERVER_STOPPED,
    ];

    let notification = client
        .next_notification(Duration::from_secs(5))
        .await
        .expect("the daemon must emit at least one notification");
    assert_eq!(notification.jsonrpc, "2.0");
    assert!(
        known.contains(&notification.method.as_str()),
        "undeclared notification method {:?}",
        notification.method
    );

    shutdown(&mut client).await;
    cleanup(&db_path);
}

/// The whole pairing handshake, through to a paired device.
///
/// `rpc_health.rs` and the rejection test above only ever drive
/// `pairing.approve` to failure, because a ticket this daemon minted has no
/// remote origin and a claim from loopback shares the daemon's own address —
/// both of which `handle_pairing_approve` fails closed on. Getting to the
/// success path needs a claim stamped with an address that is not the
/// daemon's, which is what the SQL below simulates. Nothing else in the
/// suite reaches the device insert, the session-token persist, or the
/// transition of the pending row to Approved.
#[tokio::test]
async fn approving_a_remote_claim_pairs_the_device() {
    let db_path = temp_db_path("approve");
    let mut client = spawn(&db_path).await;

    let status = ok_result(&mut client, rpc::method::STATUS, serde_json::json!({})).await;
    let host = status["host"].as_str().expect("host").to_string();
    let port = status["port"].as_u64().expect("port");

    let begin = ok_result(
        &mut client,
        rpc::method::PAIRING_BEGIN,
        serde_json::json!({}),
    )
    .await;
    let token = begin["token"].as_str().expect("token").to_string();

    // A remote device claims the ticket over HTTP, which fills in its name,
    // type and device id.
    let remote_device_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let response = reqwest::Client::new()
        .post(format!(
            "http://{host}:{port}{}",
            livtet_sync_http::PAIR_PATH
        ))
        .json(&serde_json::json!({
            "device_id": remote_device_id,
            "name": "Remote Phone",
            "device_type": "android",
            "token": token,
        }))
        .send()
        .await
        .expect("pair POST");
    assert!(
        response.status().is_success(),
        "pair POST: {}",
        response.status()
    );

    // The claim came from loopback, so its origin is the daemon's own
    // address and approval would still refuse. Stamp a different origin to
    // stand in for a claim from another host.
    let pool = livtet_data::sql::sqlite::SqlitePoolOptions::new()
        .connect_with(
            livtet_data::sql::sqlite::SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(false),
        )
        .await
        .expect("open db");
    livtet_data::sql::query("UPDATE pending_pairings SET origin_addr = ? WHERE token = ?")
        .bind(format!("10.0.0.5:{port}"))
        .bind(token.clone())
        .execute(&pool)
        .await
        .expect("stamp a remote origin");

    let approved = ok_result(
        &mut client,
        rpc::method::PAIRING_APPROVE,
        serde_json::json!({ "token": token }),
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
        Some(remote_device_id),
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
    assert_eq!(entries[0]["device_id"].as_str(), Some(remote_device_id));
    assert_eq!(entries[0]["name"].as_str(), Some("Remote Phone"));

    // The session token is persisted, not just returned — the remote device
    // authenticates later requests with it.
    let stored: String =
        livtet_data::sql::query_scalar("SELECT session_token FROM paired_devices LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("read back the session token");
    assert_eq!(stored, session_token);

    // Approving the same ticket twice must not pair a second device: the
    // pending row is Approved now, and the upsert keys on device_id.
    let second = ok_result(
        &mut client,
        rpc::method::PAIRING_APPROVE,
        serde_json::json!({ "token": token }),
    )
    .await;
    assert_eq!(second["device_id"].as_str(), Some(remote_device_id));
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
        serde_json::json!({ "device_id": remote_device_id }),
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

/// `kill` is the escape hatch for a daemon that will not answer `shutdown`.
/// It is deliberately the only test that uses it: a SIGKILLed child never
/// flushes its coverage profile, so every other test here exits cleanly.
#[tokio::test]
async fn kill_terminates_a_daemon_that_is_not_asked_nicely() {
    let db_path = temp_db_path("kill");
    let mut client = spawn(&db_path).await;

    // Confirm it is alive and answering before killing it.
    assert!(server_running(&mut client).await);

    client.kill();
    // `wait` must return once the child is gone rather than hanging.
    let exited = tokio::time::timeout(Duration::from_secs(5), client.wait()).await;
    assert!(exited.is_ok(), "the killed daemon must be reaped");

    cleanup(&db_path);
}
