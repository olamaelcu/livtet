//! The poem routes, driven in-process with `poem::test::TestClient` (no
//! socket).

use std::{collections::HashMap, sync::Arc};

use livtet_sync::{Conflict, FullDump, PullResponse, PushResponse, SyncEngine, SyncStatus};
use livtet_sync_http::{PairWaiters, make_sync_routes};
use livtet_test_utils::sync::{Device, change};
use poem::{Endpoint, http::StatusCode, test::TestClient};
use serde::de::DeserializeOwned;
use tokio::sync::{Mutex, RwLock};

fn client(device: &Device) -> TestClient<impl Endpoint> {
    let engine = SyncEngine::new(device.conn.clone(), device.engine.device_id().to_string());
    let waiters: PairWaiters = Arc::new(Mutex::new(HashMap::new()));
    TestClient::new(make_sync_routes(Arc::new(RwLock::new(engine)), waiters))
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
}
