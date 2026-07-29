//! Mock-host integration test for the `import_*` capability family.
//!
//! Drives the real fixture Lua plugin (`fixtures/import-fixture/`)
//! through an in-process `LuaHost<MockHost>`. The plugin returns
//! canned `import_detect` / `import_list_items` / `import_items`
//! responses for a Calibre-style `metadata.db` source, and the test
//! pins the wire-format shapes against the typed return types in
//! `livtet_plugins::import` (`ImportDetection`, `ImportPreviewItem`,
//! `ImportRecord`, `ImportFile`).
//!
//! The fixture returns canned responses only — it doesn't exercise
//! `host.fs_copy` / `host.fs_symlink`. A future slice can extend the
//! fixture to call those on `import_items` and the mock-host can
//! grow recording state for `host.fs_copy(src, dst)` /
//! `host.fs_symlink(target, link_path)` calls.
//!
//! Modeled after `mocked_http_test.rs` (same `MockHost` shape, same
//! `Arc<MockHost>` + `LuaHost<MockHost>` wiring, same `call_capability`
//! dispatch). That test inlines a `MockHost`; this one mirrors the
//! pattern rather than sharing the helper, because the import
//! capability family doesn't exercise `HostHttp` and a shared helper
//! would force every consumer to import every trait impl.

use std::sync::Arc;

use livtet_plugins::{
    host_lua::LuaHost,
    host_trait::{
        GetEmbeddingResponse, HostBase, HostDatabase, HostEmbeddings, HostError, HostFiles,
        HostHttp, HostHttpResponse, HostLog, HostOAuth, HostSecrets, HostSettings,
        HostSystemSecrets, SimilarEdition, StoreEmbeddingResponse,
    },
    import::{ImportDetection, ImportFile, ImportPreviewItem, ImportRecord},
    protocol::HostToMain,
    system_secrets::PluginSystemSecret,
};

/// Inlined fixture source. See `fixtures/import-fixture/init.lua` for
/// the canned responses; if you change this string, also change the
/// fixture file (and vice versa) so the test stays hermetic.
const IMPORT_FIXTURE_SOURCE: &str = include_str!("../fixtures/import-fixture/init.lua");

/// Test host that stubs every host function. The import_* capabilities
/// don't touch any of them, so we return `Err(Unsupported)` (or `None`
/// for the read-flavored traits) and hold no per-call state. A future
/// slice that exercises `host.fs_copy` / `host.fs_symlink` will need
/// to add recording state here; see the module-level docstring.
struct MockHost;

impl HostBase for MockHost {}

impl HostHttp for MockHost {
    fn http_get(
        &self,
        _url: &str,
        _headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        Err(HostError::Unsupported)
    }
    fn http_post(
        &self,
        _url: &str,
        _body: Option<&str>,
        _headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        Err(HostError::Unsupported)
    }
    fn http_put(
        &self,
        _url: &str,
        _body: Option<&str>,
        _headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        Err(HostError::Unsupported)
    }
    fn http_patch(
        &self,
        _url: &str,
        _body: Option<&str>,
        _headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        Err(HostError::Unsupported)
    }
    fn http_delete(
        &self,
        _url: &str,
        _headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        Err(HostError::Unsupported)
    }
}

impl HostLog for MockHost {
    fn log(&self, _plugin_id: &str, _level: &str, _message: &str) {
        // Drop plugin log lines on the floor — the fixture doesn't
        // log anything interesting and surfacing them via
        // `tracing::*!` would add noise to `cargo test` output.
    }
}

impl HostSecrets for MockHost {
    fn get_secret(&self, _plugin_id: &str, _name: &str) -> Result<Option<String>, HostError> {
        Err(HostError::Unsupported)
    }
    fn set_secret(&self, _plugin_id: &str, _name: &str, _value: &str) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
}

impl HostSettings for MockHost {
    fn get_setting(&self, _plugin_id: &str, _key: &str) -> Option<String> {
        None
    }
    fn set_setting(&self, _plugin_id: &str, _key: &str, _value: &str) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
}

impl HostSystemSecrets for MockHost {
    fn get_system_secret(&self, _name: PluginSystemSecret) -> Option<String> {
        None
    }
}

impl HostDatabase for MockHost {
    fn resolve_identifier(&self, _urn: &str) -> Result<Option<String>, HostError> {
        Err(HostError::Unsupported)
    }
    fn resolve_identifiers(&self, _urns: &[String]) -> Result<Vec<Option<String>>, HostError> {
        Err(HostError::Unsupported)
    }
    fn get_edition_info(&self, _edition_id: &str) -> Result<Option<serde_json::Value>, HostError> {
        Err(HostError::Unsupported)
    }
    fn get_edition_identifiers(&self, _edition_id: &str) -> Result<Vec<String>, HostError> {
        Err(HostError::Unsupported)
    }
    fn fetch_progress(
        &self,
        _urn: &str,
    ) -> Result<Option<livtet_plugins::progress_entry::ProgressEntry>, HostError> {
        Err(HostError::Unsupported)
    }
    fn upsert_progress(
        &self,
        _urn: &str,
        _progress: f64,
        _last_location: Option<String>,
        _total_reading_time_secs: i64,
    ) -> Result<serde_json::Value, HostError> {
        Err(HostError::Unsupported)
    }
}

impl HostFiles for MockHost {
    fn read_file(&self, _path: &str) -> Result<Option<String>, HostError> {
        Err(HostError::Unsupported)
    }
    fn plugin_asset(&self, _plugin_dir: &str, _filename: &str) -> Result<Vec<u8>, HostError> {
        Err(HostError::Unsupported)
    }
}

impl HostEmbeddings for MockHost {
    fn store_embedding(
        &self,
        _edition_id: &str,
        _model: &str,
        _vector_bytes: &[u8],
    ) -> Result<StoreEmbeddingResponse, HostError> {
        Err(HostError::Unsupported)
    }
    fn get_embedding(
        &self,
        _edition_id: &str,
        _model: &str,
    ) -> Result<Option<GetEmbeddingResponse>, HostError> {
        Err(HostError::Unsupported)
    }
    fn find_similar_editions(
        &self,
        _query_vector: &[u8],
        _model: &str,
        _limit: usize,
    ) -> Result<Vec<SimilarEdition>, HostError> {
        Err(HostError::Unsupported)
    }
}

/// OAuth isn't exercised by the fixture; returning `Unsupported` keeps
/// `MockHost` plug-compatible with `LuaHost<MockHost>` without forcing
/// the test to wire a fake OAuth provider.
impl HostOAuth for MockHost {
    fn redeem_token(&self, _plugin_id: &str, _provider: &str) -> Result<String, HostError> {
        Err(HostError::Unsupported)
    }
    fn get_valid_token(&self, _plugin_id: &str, _provider: &str) -> Result<String, HostError> {
        Err(HostError::Unsupported)
    }
    fn revoke_token(&self, _plugin_id: &str, _provider: &str) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    fn authorize(&self, _plugin_id: &str, _provider: &str) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
}

/// Build an in-process `LuaHost<MockHost>` and load the
/// `import-fixture` plugin from the inlined source. Returns
/// `(mock, host)` so future tests that grow the mock's recording
/// surface have a handle on the mock without re-plumbing the host.
fn make_host_typed() -> (Arc<MockHost>, LuaHost<MockHost>) {
    let mock = Arc::new(MockHost);
    let mut host = LuaHost::new(Arc::clone(&mock) as Arc<MockHost>).expect("LuaHost::new");
    match host.load_plugin_source("import-fixture", IMPORT_FIXTURE_SOURCE, None, None) {
        HostToMain::PluginLoaded { .. } => {}
        other => panic!("import-fixture load_plugin_source: {other:?}"),
    }
    (mock, host)
}

/// Run a single capability call and panic on anything other than
/// `Ok(value)`. Mirrors the dispatch-shape assertions in
/// `mocked_http_test.rs`.
fn call(
    host: &mut LuaHost<MockHost>,
    cap: &str,
    args: Vec<serde_json::Value>,
) -> serde_json::Value {
    let result = host.call_capability("call-1", "import-fixture", cap, &args);
    match result {
        HostToMain::CallResult {
            ok: true,
            value: Some(v),
            ..
        } => v,
        other => panic!("expected CallResult(Ok(_)) from {cap}, got {other:?}"),
    }
}

/// `provider.import_detect(source)` for a Calibre-shaped source
/// should return `{confidence=1.0, format_name="Calibre SQLite",
/// estimated_count=3}`. The plugin branches on
/// `source.path:find("metadata%.db$") ~= nil`, so a path that ends in
/// `metadata.db` is the canonical "yes I handle this" signal.
#[test]
fn import_detect_returns_confidence_and_format_name() {
    let (_mock, mut host) = make_host_typed();
    let result = call(
        &mut host,
        "import_detect",
        vec![serde_json::json!({
            "type": "file",
            "path": "/tmp/test-calibre/metadata.db",
        })],
    );
    let det: ImportDetection = serde_json::from_value(result).expect("import_detect deserialise");
    assert!(
        (det.confidence - 1.0).abs() < 0.001,
        "confidence should be 1.0"
    );
    assert_eq!(det.format_name.as_deref(), Some("Calibre SQLite"));
    assert_eq!(det.estimated_count, Some(3));
}

/// `provider.import_list_items(source, options)` should return three
/// preview rows for the canonical Calibre source. The first row's
/// title is pinned so the test catches accidental ordering
/// regressions in the fixture.
#[test]
fn import_list_items_returns_three_previews() {
    let (_mock, mut host) = make_host_typed();
    let result = call(
        &mut host,
        "import_list_items",
        vec![
            serde_json::json!({
                "type": "file",
                "path": "/tmp/test-calibre/metadata.db",
            }),
            serde_json::json!({}),
        ],
    );
    let items: Vec<ImportPreviewItem> =
        serde_json::from_value(result).expect("import_list_items deserialise");
    assert_eq!(items.len(), 3, "expected three previews; got {items:?}");
    assert_eq!(items[0].title, "Test Book 1");
    assert!(
        items[0]
            .identifiers
            .iter()
            .any(|i| i == "urn:isbn:9780000000001"),
        "first preview should carry the ISBN-13 URN; got {:?}",
        items[0].identifiers
    );
}

/// `provider.import_items(source, options)` should return three
/// records, each carrying one file (the fixture models a Calibre
/// library where every book has exactly one attached file). The
/// first record's file format is pinned to "epub" so the test catches
/// accidental format regressions in the fixture.
#[test]
fn import_items_returns_records_with_files() {
    let (_mock, mut host) = make_host_typed();
    let result = call(
        &mut host,
        "import_items",
        vec![
            serde_json::json!({
                "type": "file",
                "path": "/tmp/test-calibre/metadata.db",
            }),
            serde_json::json!({ "selected_items": ["1", "2", "3"] }),
        ],
    );
    let records: Vec<ImportRecord> =
        serde_json::from_value(result).expect("import_items deserialise");
    assert_eq!(records.len(), 3, "expected three records; got {records:?}");
    assert_eq!(
        records[0].files.len(),
        1,
        "first record should have exactly one file; got {:?}",
        records[0].files
    );
    let first_file: &ImportFile = &records[0].files[0];
    assert_eq!(first_file.format, "epub");
    assert_eq!(first_file.path, "/tmp/livtet-fixture/book1.epub");
    assert_eq!(records[0].series_name.as_deref(), Some("Test Series"));
    assert_eq!(records[0].series_position, Some(1));
}

/// A source whose path doesn't end in `metadata.db` should be
/// declined. `import_detect` returns nil for declined sources, which
/// the host serialises as JSON `null`. The plugin's other two
/// import_* capabilities return empty arrays instead of nil — see
/// `import_list_items_declines_unknown_source` for that branch.
#[test]
fn import_detect_declines_unrecognized_source() {
    let (_mock, mut host) = make_host_typed();
    let result = call(
        &mut host,
        "import_detect",
        vec![serde_json::json!({
            "type": "file",
            "path": "/tmp/something-else.csv",
        })],
    );
    assert!(
        result.is_null(),
        "plugin should return nil for unrecognized sources; got {result:?}"
    );
}

/// `import_list_items` for an unrecognized source returns an empty
/// list (rather than nil) — the UI iterates the result, so an empty
/// list is the friendly "nothing to import" signal.
///
/// Note: the fixture returns Lua `{}` for the declined case, which
/// mlua serializes as a JSON object (`{}`) rather than a JSON array
/// (`[]`). We accept either shape — the existing
/// `test_search_capability_returns_empty_for_blank_query` test in
/// `host_manager_test.rs` documents this behaviour. Both shapes mean
/// "no items" from the caller's perspective.
#[test]
fn import_list_items_declines_unknown_source() {
    let (_mock, mut host) = make_host_typed();
    let result = call(
        &mut host,
        "import_list_items",
        vec![
            serde_json::json!({
                "type": "file",
                "path": "/tmp/something-else.csv",
            }),
            serde_json::json!({}),
        ],
    );
    match &result {
        serde_json::Value::Array(arr) => {
            assert!(arr.is_empty(), "expected empty list; got {arr:?}")
        }
        serde_json::Value::Object(obj) if obj.is_empty() => {}
        other => panic!("expected empty array or empty object, got {other:?}"),
    }
}
