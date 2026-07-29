use livtet_plugin::protocol::{HostToMain, MainToHost, MainToHostCallback};
use serde_json::json;

#[test]
fn test_main_to_host_load_plugin_roundtrip() {
    let msg = MainToHost::LoadPlugin {
        plugin_id: "abc".into(),
        manifest: json!({"id": "abc"}),
        source: "/path/to/plugin.lua".into(),
        data_dir: None,
        settings: None,
        rocks: vec!["dkjson".into(), "luasocket".into()],
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: MainToHost = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_main_to_host_load_plugin_roundtrip_with_empty_rocks() {
    // Empty `rocks` list — the typical case for a plugin that
    // doesn't depend on any LuaRocks modules. Must round-trip
    // back to an empty Vec on the other end.
    let msg = MainToHost::LoadPlugin {
        plugin_id: "abc".into(),
        manifest: json!({"id": "abc"}),
        source: "/path/to/plugin.lua".into(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: MainToHost = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_main_to_host_unload_plugin_roundtrip() {
    let msg = MainToHost::UnloadPlugin {
        plugin_id: "abc".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: MainToHost = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_main_to_host_call_roundtrip() {
    let msg = MainToHost::Call {
        id: "call-1".into(),
        plugin_id: "abc".into(),
        capability: "search".into(),
        args: vec![json!("query")],
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: MainToHost = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_main_to_host_shutdown_roundtrip() {
    let msg = MainToHost::Shutdown;
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: MainToHost = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_call_result_ok_roundtrip() {
    let msg = HostToMain::CallResult {
        id: "call-1".into(),
        ok: true,
        value: Some(json!({"ok": true})),
        error: None,
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_call_result_err_roundtrip() {
    let msg = HostToMain::CallResult {
        id: "call-1".into(),
        ok: false,
        value: None,
        error: Some("something went wrong".into()),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_http_request_roundtrip() {
    let msg = HostToMain::HttpRequest {
        id: "req-1".into(),
        plugin_id: "abc".into(),
        method: "GET".into(),
        url: "https://example.com".into(),
        body: Some("body".into()),
        headers: vec![("Authorization".into(), "Bearer token".into())],
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_log_roundtrip() {
    let msg = HostToMain::Log {
        plugin_id: "abc".into(),
        level: "info".into(),
        message: "hello".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_plugin_loaded_roundtrip() {
    let msg = HostToMain::PluginLoaded {
        plugin_id: "abc".into(),
        load_state: "active".into(),
        missing_optional: vec!["opt-dep".into()],
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_plugin_unloaded_roundtrip() {
    let msg = HostToMain::PluginUnloaded {
        plugin_id: "abc".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_ready_roundtrip() {
    let msg = HostToMain::Ready {
        runtime: "lua".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_plugin_load_error_roundtrip() {
    let msg = HostToMain::PluginLoadError {
        plugin_id: "abc".into(),
        error: "failed to compile".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_main_to_host_callback_http_response_roundtrip() {
    let msg = MainToHostCallback::HttpResponse {
        id: "req-1".into(),
        status: 200,
        body: Some("response".into()),
        headers: vec![("Content-Type".into(), "text/plain".into())],
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: MainToHostCallback = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_main_to_host_callback_secret_result_roundtrip() {
    let msg = MainToHostCallback::SecretResult {
        id: "sec-1".into(),
        value: Some("secret".into()),
        error: None,
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: MainToHostCallback = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_main_to_host_type_tag() {
    let msg = MainToHost::Shutdown;
    let value = serde_json::to_value(&msg).unwrap();
    let obj = value.as_object().unwrap();
    assert_eq!(obj.get("type").unwrap().as_str().unwrap(), "shutdown");
}

#[test]
fn test_host_to_main_type_tag() {
    let msg = HostToMain::Ready {
        runtime: "lua".into(),
    };
    let value = serde_json::to_value(&msg).unwrap();
    let obj = value.as_object().unwrap();
    assert_eq!(obj.get("type").unwrap().as_str().unwrap(), "ready");
}

#[test]
fn test_main_to_host_callback_type_tag() {
    let msg = MainToHostCallback::SecretResult {
        id: "sec-1".into(),
        value: None,
        error: None,
    };
    let value = serde_json::to_value(&msg).unwrap();
    let obj = value.as_object().unwrap();
    assert_eq!(obj.get("type").unwrap().as_str().unwrap(), "secret_result");
}

// =====================================================================
// Round-trip tests for the 12 MainToHost variants that didn't have a
// `*_roundtrip` test in the original coverage set. The wire format is
// MessagePack; the variants in this section are all fire-and-forget
// (EmitEvent) or request variants the host function fires and blocks
// on. The "shape" each test pins is "serialize -> deserialize -> equal",
// which catches accidental field renames / drops / type changes that
// would silently break IPC compatibility.
// =====================================================================

#[test]
fn test_host_to_main_secret_request_roundtrip() {
    let msg = HostToMain::SecretRequest {
        id: "sec-1".into(),
        plugin_id: "openlibrary".into(),
        name: "api_key".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_set_secret_request_roundtrip() {
    let msg = HostToMain::SetSecretRequest {
        id: "sec-1".into(),
        plugin_id: "openlibrary".into(),
        name: "api_key".into(),
        value: "sk_live_xyz".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_read_file_request_roundtrip() {
    let msg = HostToMain::ReadFileRequest {
        id: "rf-1".into(),
        plugin_id: "koreader".into(),
        path: "metadata.lua".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_sqlite_query_request_roundtrip() {
    let msg = HostToMain::SqliteQueryRequest {
        id: "sq-1".into(),
        plugin_id: "koreader".into(),
        path: "stats.sqlite".into(),
        sql: "SELECT * FROM book WHERE id = ?".into(),
        params: vec![json!("abc123")],
        limit: Some(50),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_sqlite_query_request_roundtrip_with_omitted_limit() {
    // The `limit` field is `skip_serializing_if = "Option::is_none"`,
    // so an absent `limit` (the typical "give me the cap" call) must
    // round-trip back to `None` on the other end. Catches a future
    // refactor that accidentally tightens the skip rule.
    let msg = HostToMain::SqliteQueryRequest {
        id: "sq-2".into(),
        plugin_id: "koreader".into(),
        path: "stats.sqlite".into(),
        sql: "SELECT 1".into(),
        params: vec![],
        limit: None,
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_emit_event_roundtrip() {
    let msg = HostToMain::EmitEvent {
        plugin_id: "openlibrary".into(),
        event_type: "search".into(),
        payload: json!({"query": "the lord of the rings", "limit": 10}),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_read_asset_request_roundtrip() {
    let msg = HostToMain::ReadAssetRequest {
        id: "asset-1".into(),
        plugin_id: "koreader".into(),
        filename: "icon.png".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_resolve_identifier_request_roundtrip() {
    let msg = HostToMain::ResolveIdentifierRequest {
        id: "ri-1".into(),
        plugin_id: "openlibrary".into(),
        urn: "urn:isbn:9780441013593".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_resolve_identifiers_request_roundtrip() {
    let msg = HostToMain::ResolveIdentifiersRequest {
        id: "ris-1".into(),
        plugin_id: "openlibrary".into(),
        urns: vec![
            "urn:isbn:9780441013593".into(),
            "urn:isbn:9780441172719".into(),
        ],
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_get_edition_info_request_roundtrip() {
    let msg = HostToMain::GetEditionInfoRequest {
        id: "gei-1".into(),
        plugin_id: "openlibrary".into(),
        edition_id: "01HFNX5G3N6F2B4J1Z2X3Y4W5V".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_get_edition_identifiers_request_roundtrip() {
    let msg = HostToMain::GetEditionIdentifiersRequest {
        id: "geids-1".into(),
        plugin_id: "openlibrary".into(),
        edition_id: "01HFNX5G3N6F2B4J1Z2X3Y4W5V".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_fetch_progress_request_roundtrip() {
    let msg = HostToMain::FetchProgressRequest {
        id: "fp-1".into(),
        plugin_id: "koreader".into(),
        urn: "urn:isbn:9780441013593".into(),
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_upsert_progress_request_roundtrip() {
    let msg = HostToMain::UpsertProgressRequest {
        id: "up-1".into(),
        plugin_id: "koreader".into(),
        urn: "urn:isbn:9780441013593".into(),
        progress: 0.5,
        last_location: Some("epubcfi(/6/14[chap03])".into()),
        total_reading_time_secs: 1800,
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_host_to_main_upsert_progress_request_roundtrip_with_none_last_location() {
    // `last_location` is intentionally NOT `skip_serializing_if` — it
    // sits mid-payload and rmp_serde encodes the enum as a positional
    // array, so eliding would shift every subsequent field. This
    // test pins the "None on the wire as nil" contract.
    let msg = HostToMain::UpsertProgressRequest {
        id: "up-2".into(),
        plugin_id: "koreader".into(),
        urn: "urn:isbn:9780441013593".into(),
        progress: 0.0,
        last_location: None,
        total_reading_time_secs: 0,
    };
    let encoded = rmp_serde::to_vec(&msg).unwrap();
    let decoded: HostToMain = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(msg, decoded);
}

// Type-tag smoke tests for the new request variants. The protocol
// uses snake_case in the `type` field; pinning the strings here
// catches an accidental rename of a variant.
#[test]
fn test_host_to_main_secret_request_type_tag() {
    let msg = HostToMain::SecretRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        name: "n".into(),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "secret_request");
}

#[test]
fn test_host_to_main_set_secret_request_type_tag() {
    let msg = HostToMain::SetSecretRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        name: "n".into(),
        value: "v".into(),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "set_secret_request");
}

#[test]
fn test_host_to_main_read_file_request_type_tag() {
    let msg = HostToMain::ReadFileRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        path: "f".into(),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "read_file_request");
}

#[test]
fn test_host_to_main_sqlite_query_request_type_tag() {
    let msg = HostToMain::SqliteQueryRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        path: "f".into(),
        sql: "SELECT 1".into(),
        params: vec![],
        limit: None,
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "sqlite_query_request");
}

#[test]
fn test_host_to_main_emit_event_type_tag() {
    let msg = HostToMain::EmitEvent {
        plugin_id: "p".into(),
        event_type: "search".into(),
        payload: json!({}),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "emit_event");
}

#[test]
fn test_host_to_main_read_asset_request_type_tag() {
    let msg = HostToMain::ReadAssetRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        filename: "icon.png".into(),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "read_asset_request");
}

#[test]
fn test_host_to_main_resolve_identifier_request_type_tag() {
    let msg = HostToMain::ResolveIdentifierRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        urn: "u".into(),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "resolve_identifier_request");
}

#[test]
fn test_host_to_main_resolve_identifiers_request_type_tag() {
    let msg = HostToMain::ResolveIdentifiersRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        urns: vec![],
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "resolve_identifiers_request");
}

#[test]
fn test_host_to_main_get_edition_info_request_type_tag() {
    let msg = HostToMain::GetEditionInfoRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        edition_id: "e".into(),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "get_edition_info_request");
}

#[test]
fn test_host_to_main_get_edition_identifiers_request_type_tag() {
    let msg = HostToMain::GetEditionIdentifiersRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        edition_id: "e".into(),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "get_edition_identifiers_request");
}

#[test]
fn test_host_to_main_fetch_progress_request_type_tag() {
    let msg = HostToMain::FetchProgressRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        urn: "u".into(),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "fetch_progress_request");
}

#[test]
fn test_host_to_main_upsert_progress_request_type_tag() {
    let msg = HostToMain::UpsertProgressRequest {
        id: "x".into(),
        plugin_id: "p".into(),
        urn: "u".into(),
        progress: 0.5,
        last_location: None,
        total_reading_time_secs: 0,
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["type"], "upsert_progress_request");
}
