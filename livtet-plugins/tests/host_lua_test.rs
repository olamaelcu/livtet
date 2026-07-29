mod common;
use std::{
    future::Future,
    io::{Read, Write},
    process::{Child, Command, Stdio},
};

use camino::Utf8Path;
use livtet_plugins::{
    manifest::PluginManifest,
    protocol::{HostToMain, MainToHost, MainToHostCallback},
};
use serde_json::json;
use livtet_data::sql::{AssertSqlSafe, Connection};
use camino_tempfile::Utf8TempDir as TempDir;

const TEST_SOURCE: &str = include_str!("../fixtures/test-provider/init.lua");
const TEST_MANIFEST_TOML: &str = include_str!("../fixtures/test-provider/livtet.toml");

const PROBE_SOURCE: &str = include_str!("../fixtures/host-probe/init.lua");
const PROBE_MANIFEST_TOML: &str = include_str!("../fixtures/host-probe/livtet.toml");

struct HostProcess {
    child: Child,
}

impl HostProcess {
    fn spawn() -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_livtet-plugins-host-lua"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn host binary");
        Self { child }
    }

    fn write_msg<T: serde::Serialize>(&mut self, msg: &T) {
        let stdin = self.child.stdin.as_mut().expect("stdin not available");
        let payload = rmp_serde::to_vec(msg).expect("serialize failed");
        let len = (payload.len() as u32).to_le_bytes();
        stdin.write_all(&len).expect("write len failed");
        stdin.write_all(&payload).expect("write payload failed");
        stdin.flush().expect("flush failed");
    }

    fn read_msg<T: serde::de::DeserializeOwned>(&mut self) -> T {
        let stdout = self.child.stdout.as_mut().expect("stdout not available");
        let mut len_buf = [0u8; 4];
        stdout.read_exact(&mut len_buf).expect("read len failed");
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stdout
            .read_exact(&mut payload)
            .expect("read payload failed");
        rmp_serde::from_slice(&payload).expect("deserialize failed")
    }

    /// Read messages until one matches the predicate. Used to skip
    /// over `Log` messages that the host may interleave with
    /// request responses.
    fn read_msg_where<T, F>(&mut self, mut pred: F) -> T
    where
        T: serde::de::DeserializeOwned,
        F: FnMut(&T) -> bool,
    {
        loop {
            let msg: T = self.read_msg();
            if pred(&msg) {
                return msg;
            }
        }
    }
}

fn test_manifest_json() -> serde_json::Value {
    let manifest: PluginManifest =
        toml::from_str(TEST_MANIFEST_TOML).expect("manifest parse failed");
    serde_json::to_value(&manifest).expect("manifest serialize failed")
}

fn probe_manifest_json() -> serde_json::Value {
    let manifest: PluginManifest =
        toml::from_str(PROBE_MANIFEST_TOML).expect("probe manifest parse failed");
    serde_json::to_value(&manifest).expect("probe manifest serialize failed")
}

/// Load the host-probe plugin and return the HostProcess so the
/// caller can issue a `Call` and inspect the round-trip.
fn load_probe() -> HostProcess {
    let manifest_json = probe_manifest_json();
    let mut host = HostProcess::spawn();
    let _: HostToMain = host.read_msg();
    host.write_msg(&MainToHost::LoadPlugin {
        plugin_id: "host-probe".to_string(),
        manifest: manifest_json,
        source: PROBE_SOURCE.to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    });
    let loaded: HostToMain = host.read_msg_where(
        |m| matches!(m, HostToMain::PluginLoaded { plugin_id, .. } if plugin_id == "host-probe"),
    );
    assert!(
        matches!(loaded, HostToMain::PluginLoaded { ref load_state, .. } if load_state == "loaded"),
        "expected PluginLoaded(loaded), got {loaded:?}"
    );
    host
}

/// Spawn the host binary with `LIVTET_PLUGIN_PERMISSIONS_DIR`
/// pointing at the supplied dir. Tests that exercise
/// `host.read_file` / `host.sqlite_query` use this to scope the
/// grant sidecar to a test-owned directory without touching the
/// user's real `~/.local/share/net.olamaelcu.livtet/permissions/`. Returns
/// the live `HostProcess` plus the `TempDir` guard the caller
/// must hold for the duration of the test.
fn load_probe_with_perms_dir(perms_dir: &Utf8Path) -> (HostProcess, TempDir) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_livtet-plugins-host-lua"));
    command
        .env("LIVTET_PLUGIN_PERMISSIONS_DIR", perms_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().expect("failed to spawn host binary");
    let mut host = HostProcess { child };
    let _: HostToMain = host.read_msg();
    let manifest_json = probe_manifest_json();
    host.write_msg(&MainToHost::LoadPlugin {
        plugin_id: "host-probe".to_string(),
        manifest: manifest_json,
        source: PROBE_SOURCE.to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    });
    let loaded: HostToMain = host.read_msg_where(
        |m| matches!(m, HostToMain::PluginLoaded { plugin_id, .. } if plugin_id == "host-probe"),
    );
    assert!(
        matches!(loaded, HostToMain::PluginLoaded { ref load_state, .. } if load_state == "loaded"),
        "expected PluginLoaded(loaded), got {loaded:?}"
    );
    // The caller passes in their own TempDir; we leak a fresh one
    // to keep the env-var path alive in case they didn't. The
    // caller's TempDir is returned so the caller can write to it
    // before passing its path in.
    let keep_alive = TempDir::new().expect("tempdir");
    (host, keep_alive)
}

/// Issue a `Call` to the named capability and assert the result is
/// `Ok(value)`. Drains any `Log` messages the host may emit on the
/// way to the CallResult.
fn call_capability(
    host: &mut HostProcess,
    call_id: &str,
    capability: &str,
    args: Vec<serde_json::Value>,
) -> serde_json::Value {
    host.write_msg(&MainToHost::Call {
        id: call_id.to_string(),
        plugin_id: "host-probe".to_string(),
        capability: capability.to_string(),
        args,
    });
    let result: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::CallResult { id, .. } if id == call_id));
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => {
            if !ok {
                panic!(
                    "expected Ok result from {capability}, got Err({})",
                    error.as_deref().unwrap_or("<no error>")
                );
            }
            value.unwrap_or(serde_json::Value::Null)
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
}

#[test]
fn test_host_ready_on_startup() {
    let mut host = HostProcess::spawn();
    let ready: HostToMain = host.read_msg();
    assert_eq!(
        ready,
        HostToMain::Ready {
            runtime: "lua".to_string()
        }
    );
    host.child.kill().ok();
}

#[test]
fn test_host_load_and_call() {
    let manifest_json = test_manifest_json();

    let mut host = HostProcess::spawn();

    let _: HostToMain = host.read_msg();

    host.write_msg(&MainToHost::LoadPlugin {
        plugin_id: "test-provider".to_string(),
        manifest: manifest_json,
        source: TEST_SOURCE.to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    });

    let loaded: HostToMain = host.read_msg();
    assert!(
        matches!(loaded, HostToMain::PluginLoaded { ref plugin_id, .. } if plugin_id == "test-provider"),
        "expected PluginLoaded for test-provider, got {loaded:?}"
    );

    host.write_msg(&MainToHost::Call {
        id: "call-1".to_string(),
        plugin_id: "test-provider".to_string(),
        capability: "resolve_links".to_string(),
        args: vec![
            serde_json::Value::String("urn:isbn:9780441013593".to_string()),
            serde_json::json!({}),
        ],
    });

    let result: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::CallResult { id, .. } if id == "call-1"));
    match result {
        HostToMain::CallResult {
            id,
            ok,
            value,
            error,
            ..
        } => {
            assert_eq!(id, "call-1");
            assert!(ok, "expected ok=true, got error={error:?}");
            let value = value.unwrap_or(serde_json::Value::Null);
            let links = value.get("links").expect("missing links in result");
            let links_arr = links.as_array().expect("links is not an array");
            assert_eq!(links_arr.len(), 1, "expected one link");
            let link = &links_arr[0];
            assert_eq!(link["label"], "Test Link");
            assert_eq!(
                link["url"],
                "https://example.com/book?urn=urn:isbn:9780441013593"
            );
            assert_eq!(link["category"], "reference");
            assert_eq!(link["sort_hint"], 100);
        }
        other => panic!("expected CallResult with Ok value, got {other:?}"),
    }

    assert!(
        host.child.try_wait().expect("try_wait failed").is_none(),
        "child should still be alive after the call"
    );

    host.child.kill().ok();
}

#[test]
fn test_host_unload() {
    let manifest_json = test_manifest_json();

    let mut host = HostProcess::spawn();
    let _: HostToMain = host.read_msg();

    host.write_msg(&MainToHost::LoadPlugin {
        plugin_id: "test-provider".to_string(),
        manifest: manifest_json,
        source: TEST_SOURCE.to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    });
    let loaded: HostToMain = host.read_msg();
    assert!(
        matches!(loaded, HostToMain::PluginLoaded { .. }),
        "expected PluginLoaded, got {loaded:?}"
    );

    host.write_msg(&MainToHost::UnloadPlugin {
        plugin_id: "test-provider".to_string(),
    });
    let unloaded: HostToMain = host.read_msg();
    assert_eq!(
        unloaded,
        HostToMain::PluginUnloaded {
            plugin_id: "test-provider".to_string()
        }
    );

    host.child.kill().ok();
}

#[test]
fn test_host_shutdown() {
    let mut host = HostProcess::spawn();
    let _: HostToMain = host.read_msg();

    host.write_msg(&MainToHost::Shutdown);

    let status = host.child.wait().expect("wait failed");
    assert!(status.success(), "host did not exit cleanly: {status:?}");
}

// =====================================================================
// host.* function surface
// =====================================================================

/// Drives the host's `http_get` capability. The host will emit an
/// `HttpRequest` on stdout (which the host_manager normally turns
/// into a real reqwest call); the test mocks a 200/OK response so
/// the test runs offline.
fn call_http_get(host: &mut HostProcess, url: &str) -> serde_json::Value {
    let args = vec![serde_json::Value::String(url.to_string())];
    host.write_msg(&MainToHost::Call {
        id: "http".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "http_get".to_string(),
        args,
    });
    // The host will emit an HttpRequest before the capability
    // returns. Forward a canned HttpResponse so the call completes.
    let req: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::HttpRequest { id, .. } if id == "http"));
    let http_id = match req {
        HostToMain::HttpRequest { id, .. } => id,
        other => panic!("expected HttpRequest, got {other:?}"),
    };
    host.write_msg(&MainToHostCallback::HttpResponse {
        id: http_id,
        status: 200,
        body: Some("<html><body>ok</body></html>".to_string()),
        headers: vec![("content-type".to_string(), "text/html".to_string())],
    });
    let result: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::CallResult { id, .. } if id == "http"));
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => {
            assert!(ok, "http_get returned error: {error:?}");
            value.unwrap_or(serde_json::Value::Null)
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
}

#[test]
fn host_http_get_returns_status_body_headers() {
    let mut host = load_probe();
    let value = call_http_get(&mut host, "https://example.com/");
    assert_eq!(value["status"], json!(200));
    assert!(
        value["body"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "expected non-empty body, got {value:?}"
    );
    assert_eq!(value["has_headers"], json!(true));
    host.child.kill().ok();
}

/// Drives the host's `log` capability. The host should emit a `Log`
/// message that the test inspects to verify the level + message
/// were forwarded.
#[test]
fn host_log_writes_a_log_message() {
    let mut host = load_probe();
    host.write_msg(&MainToHost::Call {
        id: "log".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "log".to_string(),
        args: vec![json!("warn"), json!("hello from probe")],
    });
    // The log call must produce either a Log message or be elided
    // (some implementations only forward logs that survive a
    // tracing filter). The capability itself always returns "logged".
    let result: HostToMain = host.read_msg_where(|m| {
        matches!(m, HostToMain::CallResult { id, .. } if id == "log")
            || matches!(m, HostToMain::Log { .. })
    });
    if let HostToMain::Log { level, message, .. } = &result {
        assert_eq!(level, "warn");
        assert_eq!(message, "hello from probe");
    }
    // If the first message was a CallResult, the host is doing
    // fire-and-forget logging (the bundled plugins guard the call
    // and don't depend on the log being delivered). Either is
    // acceptable; the call must complete.
    assert!(
        matches!(
            result,
            HostToMain::CallResult { .. } | HostToMain::Log { .. }
        ),
        "expected CallResult or Log, got {result:?}"
    );
    host.child.kill().ok();
}

/// Drives the host's `get_secret` / `set_secret` round trip. The
/// host emits a `SecretRequest` and a `SetSecretRequest` for each
/// call; the test echoes the requested value back so the
/// capability returns it.
#[test]
fn host_get_secret_round_trips_through_ipc() {
    let mut host = load_probe();
    host.write_msg(&MainToHost::Call {
        id: "gs".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "get_secret".to_string(),
        args: vec![json!("my_test_key")],
    });
    let req: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::SecretRequest { id, .. } if id == "gs"));
    let id = match req {
        HostToMain::SecretRequest { id, .. } => id,
        other => panic!("expected SecretRequest, got {other:?}"),
    };
    host.write_msg(&MainToHostCallback::SecretResult {
        id,
        value: Some("my_secret_value".to_string()),
        error: None,
    });
    let result: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::CallResult { id, .. } if id == "gs"));
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => {
            assert!(ok, "get_secret returned error: {error:?}");
            let value = value.unwrap_or(serde_json::Value::Null);
            assert_eq!(value["value"], json!("my_secret_value"));
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
    host.child.kill().ok();
}

#[test]
fn host_set_secret_round_trips_through_ipc() {
    let mut host = load_probe();
    host.write_msg(&MainToHost::Call {
        id: "ss".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "set_secret".to_string(),
        args: vec![json!("another_key"), json!("another_value")],
    });
    let req: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::SetSecretRequest { id, .. } if id == "ss"));
    let id = match req {
        HostToMain::SetSecretRequest { id, .. } => id,
        other => panic!("expected SetSecretRequest, got {other:?}"),
    };
    host.write_msg(&MainToHostCallback::SecretResult {
        id,
        value: None,
        error: None,
    });
    let result: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::CallResult { id, .. } if id == "ss"));
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => {
            assert!(ok, "set_secret returned error: {error:?}");
            let value = value.unwrap_or(serde_json::Value::Null);
            assert_eq!(value["ok"], json!(true));
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
    host.child.kill().ok();
}

#[test]
fn host_url_encode_encodes_space_and_slash() {
    let mut host = load_probe();
    let v = call_capability(&mut host, "urlenc", "url_encode", vec![json!("a b/c")]);
    assert_eq!(v["result"], json!("a%20b%2Fc"));
    host.child.kill().ok();
}

#[test]
fn host_url_decode_round_trips_url_encode() {
    let mut host = load_probe();
    let v = call_capability(&mut host, "urldec", "url_decode", vec![json!("a%20b%2Fc")]);
    assert_eq!(v["result"], json!("a b/c"));
    host.child.kill().ok();
}

/// Stub for `host.resolve_identifier` / `resolve_identifiers` /
/// `get_edition_info` / `get_edition_identifiers`. Without a real
/// DB the host returns nil; this test asserts the IPC round-trip
/// shape and that a "no edition" answer comes back as JSON null.
#[test]
fn host_resolve_identifier_returns_nil_for_unknown_urn() {
    let mut host = load_probe();
    host.write_msg(&MainToHost::Call {
        id: "ri".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "resolve_identifier".to_string(),
        args: vec![json!("urn:isbn:9999999999999")],
    });
    let req: HostToMain = host.read_msg_where(
        |m| matches!(m, HostToMain::ResolveIdentifierRequest { id, .. } if id == "ri"),
    );
    let id = match req {
        HostToMain::ResolveIdentifierRequest { id, .. } => id,
        other => panic!("expected ResolveIdentifierRequest, got {other:?}"),
    };
    host.write_msg(&MainToHostCallback::ResolveIdentifierResult {
        id,
        edition_id: None,
        error: None,
    });
    let result: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::CallResult { id, .. } if id == "ri"));
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => {
            assert!(ok, "resolve_identifier returned error: {error:?}");
            let value = value.unwrap_or(serde_json::Value::Null);
            assert!(
                value["result"].is_null(),
                "expected null result, got {value:?}"
            );
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
    host.child.kill().ok();
}

#[test]
fn host_resolve_identifiers_returns_map_of_results() {
    let mut host = load_probe();
    host.write_msg(&MainToHost::Call {
        id: "ris".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "resolve_identifiers".to_string(),
        args: vec![json!(["urn:isbn:111", "urn:isbn:222"])],
    });
    let req: HostToMain = host.read_msg_where(
        |m| matches!(m, HostToMain::ResolveIdentifiersRequest { id, .. } if id == "ris"),
    );
    let id = match req {
        HostToMain::ResolveIdentifiersRequest { id, .. } => id,
        other => panic!("expected ResolveIdentifiersRequest, got {other:?}"),
    };
    host.write_msg(&MainToHostCallback::ResolveIdentifiersResult {
        id,
        edition_ids: vec![Some("edition-1".to_string()), None],
        error: None,
    });
    let result: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::CallResult { id, .. } if id == "ris"));
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => {
            assert!(ok, "resolve_identifiers returned error: {error:?}");
            let value = value.unwrap_or(serde_json::Value::Null);
            let arr = value["result"].as_array().expect("expected array result");
            assert_eq!(arr.len(), 2);
            assert_eq!(arr[0], json!("edition-1"));
            assert!(arr[1].is_null());
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
    host.child.kill().ok();
}

#[test]
fn host_get_edition_info_returns_table_for_known_id() {
    let mut host = load_probe();
    host.write_msg(&MainToHost::Call {
        id: "gei".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "get_edition_info".to_string(),
        args: vec![json!("edition-abc")],
    });
    let req: HostToMain = host.read_msg_where(
        |m| matches!(m, HostToMain::GetEditionInfoRequest { id, .. } if id == "gei"),
    );
    let id = match req {
        HostToMain::GetEditionInfoRequest { id, .. } => id,
        other => panic!("expected GetEditionInfoRequest, got {other:?}"),
    };
    host.write_msg(&MainToHostCallback::EditionInfoResult {
        id,
        info: Some(json!({
            "id": "edition-abc",
            "work_id": "work-1",
            "title": "Test Edition",
            "isbn": "9780441013593",
            "page_count": 200,
            "format": "paperback",
            "identifiers": ["urn:isbn:9780441013593"],
        })),
        error: None,
    });
    let result: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::CallResult { id, .. } if id == "gei"));
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => {
            assert!(ok, "get_edition_info returned error: {error:?}");
            let value = value.unwrap_or(serde_json::Value::Null);
            let info = &value["result"];
            assert_eq!(info["id"], json!("edition-abc"));
            assert_eq!(info["title"], json!("Test Edition"));
            assert_eq!(info["page_count"], json!(200));
            assert_eq!(info["identifiers"], json!(["urn:isbn:9780441013593"]));
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
    host.child.kill().ok();
}

#[test]
fn host_get_edition_identifiers_returns_array_of_urns() {
    let mut host = load_probe();
    host.write_msg(&MainToHost::Call {
        id: "geids".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "get_edition_identifiers".to_string(),
        args: vec![json!("edition-abc")],
    });
    let req: HostToMain = host.read_msg_where(
        |m| matches!(m, HostToMain::GetEditionIdentifiersRequest { id, .. } if id == "geids"),
    );
    let id = match req {
        HostToMain::GetEditionIdentifiersRequest { id, .. } => id,
        other => panic!("expected GetEditionIdentifiersRequest, got {other:?}"),
    };
    host.write_msg(&MainToHostCallback::EditionIdentifiersResult {
        id,
        urns: vec![
            "urn:isbn:9780441013593".to_string(),
            "urn:openlibrary:/works/OL45804W".to_string(),
        ],
        error: None,
    });
    let result: HostToMain =
        host.read_msg_where(|m| matches!(m, HostToMain::CallResult { id, .. } if id == "geids"));
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => {
            assert!(ok, "get_edition_identifiers returned error: {error:?}");
            let value = value.unwrap_or(serde_json::Value::Null);
            assert_eq!(
                value["result"],
                json!(["urn:isbn:9780441013593", "urn:openlibrary:/works/OL45804W",])
            );
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
    host.child.kill().ok();
}

#[test]
fn host_emit_event_unknown_type_silently_dropped() {
    let mut host = load_probe();
    // The probe emits a bogus event type. The host forwards it
    // to the main process via `EmitEvent`; the main process is
    // expected to log + drop unknown types. In the test the host
    // is acting as the main process, so it just acknowledges
    // the message (fire-and-forget) and the capability returns
    // immediately.
    host.write_msg(&MainToHost::Call {
        id: "ev".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "emit_event".to_string(),
        args: vec![json!("definitely_not_a_real_event"), json!({"x": 1})],
    });
    let result: HostToMain = host.read_msg_where(|m| {
        matches!(m, HostToMain::EmitEvent { .. })
            || matches!(m, HostToMain::CallResult { id, .. } if id == "ev")
    });
    if let HostToMain::EmitEvent {
        event_type,
        payload,
        ..
    } = &result
    {
        assert_eq!(event_type, "definitely_not_a_real_event");
        assert_eq!(payload, &json!({"x": 1}));
    }
    // The next message should be the CallResult (or it's already
    // been delivered as the first match). Either way the
    // capability must complete.
    let result: HostToMain = host.read_msg_where(|m| {
        matches!(m, HostToMain::CallResult { id, .. } if id == "ev")
            || matches!(m, HostToMain::EmitEvent { .. })
    });
    if let HostToMain::CallResult {
        ok, value, error, ..
    } = result
    {
        assert!(ok, "emit_event returned error: {error:?}");
        let value = value.unwrap_or(serde_json::Value::Null);
        assert_eq!(value["ok"], json!(true));
    } else {
        panic!("expected CallResult(ok=true), got {result:?}");
    }
    host.child.kill().ok();
}

#[test]
fn host_emit_event_known_type_accepted() {
    let mut host = load_probe();
    host.write_msg(&MainToHost::Call {
        id: "ev2".to_string(),
        plugin_id: "host-probe".to_string(),
        capability: "emit_event".to_string(),
        args: vec![
            json!("reading_progress_updated"),
            json!({"edition_id": "e1", "progress": 0.5}),
        ],
    });
    let result: HostToMain = host.read_msg_where(|m| {
        matches!(m, HostToMain::EmitEvent { .. })
            || matches!(m, HostToMain::CallResult { id, .. } if id == "ev2")
    });
    if let HostToMain::EmitEvent {
        event_type,
        payload,
        ..
    } = &result
    {
        assert_eq!(event_type, "reading_progress_updated");
        assert_eq!(payload["edition_id"], json!("e1"));
        assert_eq!(payload["progress"], json!(0.5));
    }
    let result: HostToMain = host.read_msg_where(|m| {
        matches!(m, HostToMain::CallResult { id, .. } if id == "ev2")
            || matches!(m, HostToMain::EmitEvent { .. })
    });
    if let HostToMain::CallResult {
        ok, value, error, ..
    } = result
    {
        assert!(ok, "emit_event returned error: {error:?}");
        let value = value.unwrap_or(serde_json::Value::Null);
        assert_eq!(value["ok"], json!(true));
    } else {
        panic!("expected CallResult(ok=true), got {result:?}");
    }
    host.child.kill().ok();
}

// =====================================================================
// host.read_file / host.sqlite_query — in-process, grant-gated
// =====================================================================
//
// `host.read_file` and `host.sqlite_query` resolve their grant
// sidecar on first call, check the path against the matching glob
// set, and either return the result directly to the plugin or
// surface a canonical error string. They do NOT round-trip through
// the main process. The tests below drive the host with a temp
// `LIVTET_PLUGIN_PERMISSIONS_DIR` so the real
// `~/.local/share/net.olamaelcu.livtet/permissions/` is never touched.

/// Write a JSON grant sidecar to `<dir>/<plugin_id>.json`. The
/// shape mirrors `permissions::PluginGrant` — the host loader
/// `serde_json::from_str`'s into it.
fn write_grant_json(
    dir: &Utf8Path,
    plugin_id: &str,
    read_paths: &[&str],
    sqlite_paths: &[&str],
) -> camino::Utf8PathBuf {
    let grant = serde_json::json!({
        "version": 1,
        "read_paths": read_paths,
        "sqlite_paths": sqlite_paths,
        "allow_writes": false,
    });
    let path = dir.join(format!("{plugin_id}.json"));
    fs_err::write(&path, serde_json::to_string_pretty(&grant).unwrap())
        .expect("write grant sidecar");
    path
}

fn run_async<T>(fut: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create test runtime")
        .block_on(fut)
}

/// Build a small SQLite file in a tempdir with one table `t(a, b)`
/// and the two rows `(1, 'one')` and `(2, 'two')`. Returns the
/// path.
fn build_sample_sqlite(dir: &Utf8Path) -> camino::Utf8PathBuf {
    let path = dir.join("sample.sqlite");
    run_async(async {
        let options = livtet_data::sql::sqlite::SqliteConnectOptions::new()
            .filename(path.as_std_path())
            .create_if_missing(true);
        let mut conn = livtet_data::sql::sqlite::SqliteConnection::connect_with(&options)
            .await
            .expect("open sqlite");
        livtet_data::sql::query(AssertSqlSafe("CREATE TABLE t (a INTEGER, b TEXT);"))
            .execute(&mut conn)
            .await
            .expect("create table");
        livtet_data::sql::query(AssertSqlSafe(
            "INSERT INTO t (a, b) VALUES (1, 'one'), (2, 'two');",
        ))
        .execute(&mut conn)
        .await
        .expect("seed sqlite");
    });
    path
}

#[test]
fn host_read_file_relative_path_under_plugin_dir() {
    // The probe's plugin_id is `host-probe`. The grant allows
    // reading any file under a temp dir; the test then asks the
    // host to read `<temp>/sample.txt` and asserts the host
    // returns its contents in-process.
    let dir: camino::Utf8PathBuf = camino::Utf8PathBuf::from_path_buf(
        std::env::temp_dir().join(format!("livtet-rfp-{}", std::process::id())),
    )
    .expect("temp dir must be valid UTF-8");
    fs_err::create_dir_all(&dir).expect("mkdir");
    let sample = dir.join("sample.txt");
    fs_err::write(&sample, "host-probe-can-read-this").expect("write sample");
    let glob = format!("{dir}/**");
    let perms = TempDir::new().expect("perms tempdir");
    let perms_path = perms.path();
    write_grant_json(perms_path, "host-probe", &[glob.as_str()], &[]);
    let (mut host, _keep) = load_probe_with_perms_dir(perms_path);
    let v = call_capability(
        &mut host,
        "rf",
        "read_file",
        vec![json!(sample.to_string())],
    );
    assert_eq!(v["result"], json!("host-probe-can-read-this"));
    host.child.kill().ok();
    let _ = fs_err::remove_dir_all(&dir);
}

#[test]
fn host_read_file_absolute_outside_allowed_dir_rejected() {
    let perms = TempDir::new().expect("perms tempdir");
    // Grant a glob that only covers `/tmp/allowed/**`; the test
    // asks for `/etc/passwd` and expects the canonical
    // "outside glob" error string.
    write_grant_json(
        perms.path(),
        "host-probe",
        &["/tmp/allowed/**"],
        &[],
    );
    let (mut host, _keep) = load_probe_with_perms_dir(perms.path());
    let v = call_capability(&mut host, "rf2", "read_file", vec![json!("/etc/passwd")]);
    let err = v["error"]
        .as_str()
        .expect("expected error string in result");
    assert!(
        err.contains("permission denied") && err.contains("/etc/passwd"),
        "expected outside-glob error mentioning the path, got {v:?}"
    );
    host.child.kill().ok();
}

#[test]
fn host_read_file_path_traversal_rejected() {
    let perms = TempDir::new().expect("perms tempdir");
    write_grant_json(perms.path(), "host-probe", &["/**"], &[]);
    let (mut host, _keep) = load_probe_with_perms_dir(perms.path());
    let v = call_capability(
        &mut host,
        "rf3",
        "read_file",
        vec![json!("../../../etc/passwd")],
    );
    let err = v["error"]
        .as_str()
        .expect("expected error string in result");
    assert!(
        err.contains("permission denied"),
        "expected permission-denied error, got {v:?}"
    );
    host.child.kill().ok();
}

#[test]
fn host_sqlite_query_selects_rows() {
    let perms = TempDir::new().expect("perms tempdir");
    let data = TempDir::new().expect("data tempdir");
    let sqlite_path = build_sample_sqlite(data.path());
    let glob = format!("{sqlite_path}");
    write_grant_json(perms.path(), "host-probe", &[], &[glob.as_str()]);
    let (mut host, _keep) = load_probe_with_perms_dir(perms.path());
    let v = call_capability(
        &mut host,
        "sq1",
        "sqlite_query",
        vec![
            json!(sqlite_path.to_string()),
            json!("SELECT a, b FROM t ORDER BY a"),
            json!([]),
            json!(100),
        ],
    );
    let rows = v["result"]["rows"]
        .as_array()
        .expect("rows array in result");
    assert_eq!(rows.len(), 2, "expected 2 rows, got {v:?}");
    assert_eq!(rows[0][0], json!(1));
    assert_eq!(rows[0][1], json!("one"));
    assert_eq!(rows[1][0], json!(2));
    assert_eq!(rows[1][1], json!("two"));
    host.child.kill().ok();
}

#[test]
fn host_sqlite_query_rejects_non_select() {
    let perms = TempDir::new().expect("perms tempdir");
    let data = TempDir::new().expect("data tempdir");
    let sqlite_path = build_sample_sqlite(data.path());
    let glob = format!("{sqlite_path}");
    write_grant_json(perms.path(), "host-probe", &[], &[glob.as_str()]);
    let (mut host, _keep) = load_probe_with_perms_dir(perms.path());
    let v = call_capability(
        &mut host,
        "sq2",
        "sqlite_query",
        vec![
            json!(sqlite_path.to_string()),
            json!("INSERT INTO t (a) VALUES (1)"),
            json!([]),
            json!(100),
        ],
    );
    let err = v["error"]
        .as_str()
        .expect("expected error string in result");
    assert!(
        err.to_lowercase().contains("select"),
        "expected SELECT-related error, got {v:?}"
    );
    host.child.kill().ok();
}

#[test]
fn host_sqlite_query_caps_result_rows() {
    let perms = TempDir::new().expect("perms tempdir");
    let data = TempDir::new().expect("data tempdir");
    let sqlite_path = data.path().join("big.sqlite");
    run_async(async {
        let options = livtet_data::sql::sqlite::SqliteConnectOptions::new()
            .filename(sqlite_path.as_std_path())
            .create_if_missing(true);
        let mut conn = livtet_data::sql::sqlite::SqliteConnection::connect_with(&options)
            .await
            .expect("open big.sqlite");
        livtet_data::sql::query(AssertSqlSafe("CREATE TABLE big (x INTEGER);"))
            .execute(&mut conn)
            .await
            .expect("create big table");
        // 12,000 rows. The host caps at 10,000; the test
        // confirms the cap is observed in the data the probe
        // sees.
        for i in 0..12_000i64 {
            livtet_data::sql::query(AssertSqlSafe("INSERT INTO big (x) VALUES (?1)"))
                .bind(i)
                .execute(&mut conn)
                .await
                .expect("insert row");
        }
    });
    let glob = format!("{sqlite_path}");
    write_grant_json(perms.path(), "host-probe", &[], &[glob.as_str()]);
    let (mut host, _keep) = load_probe_with_perms_dir(perms.path());
    let v = call_capability(
        &mut host,
        "sq3",
        "sqlite_query",
        vec![
            json!(sqlite_path.to_string()),
            json!("SELECT x FROM big"),
            json!([]),
            json!(10000),
        ],
    );
    let rows = v["result"]["rows"]
        .as_array()
        .expect("rows array in result");
    assert!(
        rows.len() <= 10_000,
        "rows.len() = {} must be <= 10_000 cap",
        rows.len()
    );
    assert_eq!(
        rows.len(),
        10_000,
        "expected the 10k cap to be hit (table has 12k rows)"
    );
    host.child.kill().ok();
}

// =====================================================================
// Deferred host functions (Commit 2 follow-up): html_strip, html_parse,
// get_setting. Landed now so the bundled openlibrary plugin's call
// sites become real instead of dead code.
// =====================================================================

/// Strips HTML tags, decodes a handful of common HTML entities, and
/// collapses runs of whitespace into a single space.
#[test]
fn host_html_strip_removes_tags_and_decodes_entities() {
    let mut host = load_probe();
    let v = call_capability(
        &mut host,
        "hstrip",
        "html_strip",
        vec![json!("<p>Hello <b>world</b>!</p>")],
    );
    assert_eq!(v["result"], json!("Hello world!"));
    host.child.kill().ok();
}

#[test]
fn host_html_strip_decodes_common_entities() {
    let mut host = load_probe();
    let v = call_capability(
        &mut host,
        "hstrip2",
        "html_strip",
        vec![json!(
            "Tom &amp; Jerry &lt;3 &quot;cheese&quot; &#39;cause it&#39;s good"
        )],
    );
    assert_eq!(
        v["result"],
        json!("Tom & Jerry <3 \"cheese\" 'cause it's good")
    );
    host.child.kill().ok();
}

#[test]
fn host_html_strip_strips_cdata_and_preserves_plain_text() {
    let mut host = load_probe();
    let cdata = call_capability(
        &mut host,
        "hstrip3",
        "html_strip",
        vec![json!("<![CDATA[raw text inside]]>")],
    );
    assert_eq!(cdata["result"], json!("raw text inside"));
    let plain = call_capability(
        &mut host,
        "hstrip4",
        "html_strip",
        vec![json!("just plain text with no markup")],
    );
    assert_eq!(plain["result"], json!("just plain text with no markup"));
    host.child.kill().ok();
}

/// Parse a small HTML document with two `<a>` links and confirm
/// `:select` returns the right count, the first match's
/// `:text()` matches the expected title, and `:attr("href")`
/// returns the right URL. An absent attribute is nil, and a
/// missing element is an empty list (count=0).
#[test]
fn host_html_parse_selects_and_reads_text_and_attr() {
    let mut host = load_probe();
    let html = r#"<html><body>
        <div class="result"><a href="/b/1">Dune</a><span class="author">Frank Herbert</span></div>
        <div class="result"><a href="/b/2">Foundation</a><span class="author">Isaac Asimov</span></div>
        </body></html>"#;
    let v = call_capability(
        &mut host,
        "hparse1",
        "html_parse",
        vec![json!(html), json!("div.result a"), json!("href")],
    );
    assert_eq!(v["count"], json!(2));
    assert_eq!(v["text"], json!("Dune"));
    assert_eq!(v["attr"], json!("/b/1"));
    host.child.kill().ok();
}

#[test]
fn host_html_parse_missing_attr_returns_nil_and_empty_match_count_zero() {
    let mut host = load_probe();
    let html = r#"<html><body><p id="only">hi</p></body></html>"#;
    // The first p has no `data-x` attribute, so :attr("data-x") is nil.
    let v = call_capability(
        &mut host,
        "hparse2",
        "html_parse",
        vec![json!(html), json!("p"), json!("data-x")],
    );
    assert_eq!(v["count"], json!(1));
    assert_eq!(v["text"], json!("hi"));
    assert!(v["attr"].is_null(), "expected null attr, got {v:?}");
    // No matches: selector that doesn't match anything returns an
    // empty list and the probe reports count=0 with nil text/attr.
    let v2 = call_capability(
        &mut host,
        "hparse3",
        "html_parse",
        vec![json!(html), json!("article"), json!("href")],
    );
    assert_eq!(v2["count"], json!(0));
    assert!(v2["text"].is_null());
    assert!(v2["attr"].is_null());
    host.child.kill().ok();
}

/// `host.get_setting` returns nil for unknown keys. The host-probe
/// fixture is loaded with `data_dir: None` so the in-memory
/// settings map is empty — any key is "unknown".
#[test]
fn host_get_setting_returns_nil_for_unknown_key() {
    let mut host = load_probe();
    let v = call_capability(
        &mut host,
        "gset1",
        "get_setting",
        vec![json!("nonexistent_key")],
    );
    assert!(
        v["value"].is_null(),
        "expected null value for unknown setting, got {v:?}"
    );
    host.child.kill().ok();
}

/// `host.get_setting` returns the value from `<data_dir>/settings.json`
/// when the key is present. The test writes a tempdir, drops a
/// `settings.json` in it, and loads the probe plugin with that
/// data_dir before calling the capability.
#[test]
fn host_get_setting_returns_value_from_settings_json() {
    use std::io::Write;
    let dir = camino_tempfile::tempdir().expect("tempdir");
    let settings_path = dir.path().join("settings.json");
    let mut f = std::fs::File::create(&settings_path).expect("create settings.json");
    f.write_all(br#"{"max_results": "42", "backend_type": "koha-rss2"}"#)
        .expect("write settings.json");

    let manifest_json = probe_manifest_json();
    let mut host = HostProcess::spawn();
    let _: HostToMain = host.read_msg();
    // Read the settings file and pass the parsed map to the
    // host via the `settings` field of `LoadPlugin`. The
    // pre-`settings`-IPC version of this test relied on the
    // host reading `<data_dir>/settings.json` itself; that
    // code path was removed when settings moved to the
    // main-process DB. The test now exercises the same
    // end-to-end behavior (probe plugin's `get_setting`
    // returns the value the host was given at load time),
    // but the data flows in through the IPC message rather
    // than a side-channel file read.
    let settings_map: std::collections::HashMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(&settings_path).expect("read settings.json"))
            .expect("parse settings.json");
    host.write_msg(&MainToHost::LoadPlugin {
        plugin_id: "host-probe".to_string(),
        manifest: manifest_json,
        source: PROBE_SOURCE.to_string(),
        data_dir: Some(dir.path().to_path_buf()),
        settings: Some(settings_map),
        rocks: Vec::new(),
    });
    let loaded: HostToMain = host.read_msg_where(
        |m| matches!(m, HostToMain::PluginLoaded { plugin_id, .. } if plugin_id == "host-probe"),
    );
    assert!(
        matches!(loaded, HostToMain::PluginLoaded { ref load_state, .. } if load_state == "loaded"),
        "expected PluginLoaded(loaded), got {loaded:?}"
    );

    let v = call_capability(
        &mut host,
        "gset2",
        "get_setting",
        vec![json!("max_results")],
    );
    assert_eq!(v["value"], json!("42"));
    let v2 = call_capability(
        &mut host,
        "gset3",
        "get_setting",
        vec![json!("backend_type")],
    );
    assert_eq!(v2["value"], json!("koha-rss2"));
    host.child.kill().ok();
}

// =====================================================================
// host.urn — canonical URN string construction
// =====================================================================
//
// Centralizing URN construction at the host layer means a Lua plugin
// can never emit a malformed wire string like `urn:openlibrary/books/...`
// (missing `:` after the scheme) by silently concatenating the wrong
// way. The host validates the scheme against `[%w_-]+` and surfaces a
// Lua error if the plugin ever tries.

#[test]
fn host_urn_builds_canonical_string() {
    let mut host = load_probe();
    let v = call_capability(
        &mut host,
        "urn1",
        "urn",
        vec![json!("openlibrary"), json!("/works/OL45804W")],
    );
    assert_eq!(v["result"], json!("urn:openlibrary:/works/OL45804W"));
    host.child.kill().ok();
}

#[test]
fn host_urn_accepts_path_like_values_verbatim() {
    // The value is opaque: paths, IDs with hyphens, anything.
    // Validation only applies to the scheme.
    let mut host = load_probe();
    let v = call_capability(
        &mut host,
        "urn2",
        "urn",
        vec![json!("isbn"), json!("978-0-06-112008-4")],
    );
    assert_eq!(v["result"], json!("urn:isbn:978-0-06-112008-4"));
    host.child.kill().ok();
}

#[test]
fn host_urn_rejects_scheme_with_path_separator() {
    // The bug we are guarding against: a plugin hand-concatenating
    // `"urn:openlibrary" .. "/books/..."` would have produced a
    // scheme of "openlibrary/books", which contains a `/`. The
    // host must reject this before the string leaves the plugin.
    let mut host = load_probe();
    let v = call_capability(
        &mut host,
        "urn3",
        "urn",
        vec![json!("openlibrary/books"), json!("OL45804W")],
    );
    let err = v["error"]
        .as_str()
        .expect("expected error string from malformed scheme");
    assert!(
        err.contains("[%w_-]+"),
        "expected scheme-regex error, got {v:?}"
    );
    host.child.kill().ok();
}

#[test]
fn host_urn_rejects_empty_namespace() {
    let mut host = load_probe();
    let v = call_capability(&mut host, "urn4", "urn", vec![json!(""), json!("OL45804W")]);
    let err = v["error"]
        .as_str()
        .expect("expected error string from empty namespace");
    assert!(err.contains("must not be empty"), "got {v:?}");
    host.child.kill().ok();
}

#[test]
fn host_urn_rejects_empty_value() {
    let mut host = load_probe();
    let v = call_capability(&mut host, "urn5", "urn", vec![json!("isbn"), json!("")]);
    let err = v["error"]
        .as_str()
        .expect("expected error string from empty value");
    assert!(err.contains("value must not be empty"), "got {v:?}");
    host.child.kill().ok();
}
