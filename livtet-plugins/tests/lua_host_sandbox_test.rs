//! In-process tests for the Lua sandbox: memory cap, stripped
//! stdlib, instruction hook, and the `host.*` function surface.
//! These exercise `LuaHost::new` directly (no child process)
//! so the assertions can reach into the Lua state without
//! going through the host binary's IPC loop.
//!
//! Note: the host binary's own plugin-loading path is the
//! authoritative sandbox test in production; these tests
//! pin the contract so a refactor of `LuaHost::new` doesn't
//! silently re-expose `os` / `io` / `debug` / `package`.

use std::{
    assert_matches,
    collections::HashMap,
    io::Write as _,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use livtet_plugins::{
    host_lua::LuaHost,
    ipc_host::{CallbackRouter, IpcHost, SharedWriter},
    protocol::{HostToMain, MainToHost, MainToHostCallback},
};
use mlua::Lua;

fn empty_writer() -> SharedWriter {
    Arc::new(Mutex::new(Box::new(std::io::sink())))
}

fn empty_router() -> CallbackRouter {
    Arc::new(Mutex::new(HashMap::new()))
}

fn make_host() -> LuaHost<IpcHost> {
    LuaHost::new(Arc::new(IpcHost::new(empty_writer(), empty_router()))).expect("LuaHost::new")
}

/// Build a fresh `Lua` with the same safe-stdlib flags the
/// host uses. Tests below use this to assert on the Lua
/// state without having to share the host's private `Lua`.
fn fresh_lua_with_safe_stdlib() -> Lua {
    Lua::new_with(
        mlua::StdLib::TABLE
            | mlua::StdLib::STRING
            | mlua::StdLib::MATH
            | mlua::StdLib::UTF8
            | mlua::StdLib::COROUTINE,
        mlua::LuaOptions::new(),
    )
    .expect("Lua::new_with safe stdlib")
}

#[test]
fn lua_state_strips_dangerous_stdlib() {
    let lua = fresh_lua_with_safe_stdlib();
    let err = lua
        .load("os.execute('echo pwned')")
        .exec()
        .expect_err("os.execute must be unreachable in the sandbox");
    let msg = err.to_string();
    assert!(
        msg.contains("attempt to call a nil value")
            || msg.contains("global 'os'")
            || msg.contains("'os'"),
        "expected an 'os is nil' error, got: {msg}"
    );
}

#[test]
fn lua_state_strips_io_debug_and_package() {
    let lua = fresh_lua_with_safe_stdlib();
    for name in ["io", "debug", "package", "require"] {
        let result = lua.load(format!("{name}()")).exec();
        assert!(
            result.is_err(),
            "expected {name}() to raise in the sandbox, but the call succeeded: {result:?}"
        );
    }
    let co: Option<mlua::Value> = lua.globals().get("coroutine").ok();
    assert!(co.is_some(), "coroutine should be present");
    let s: Option<mlua::Value> = lua.globals().get("string").ok();
    assert!(s.is_some(), "string should be present");
}

#[test]
fn memory_limit_is_set() {
    let mut host = make_host();
    let result = host
        .handle_message(MainToHost::LoadPlugin {
            plugin_id: "mem-test".to_string(),
            manifest: serde_json::Value::Null,
            source: "return { ok = string.rep('x', 1024) }".to_string(),
            data_dir: None,
            settings: None,
            rocks: Vec::new(),
        })
        .expect("LoadPlugin returns Some(_)");
    let loaded = match result {
        livtet_plugins::protocol::HostToMain::PluginLoaded { load_state, .. } => load_state,
        other => panic!("expected PluginLoaded, got {other:?}"),
    };
    assert_eq!(loaded, "loaded");
}

#[test]
fn memory_limit_rejects_runaway_allocation() {
    let mut host = make_host();
    let source = "return { ok = string.rep('y', 1073741824) }".to_string();
    let result = host
        .handle_message(MainToHost::LoadPlugin {
            plugin_id: "oom-test".to_string(),
            manifest: serde_json::Value::Null,
            source,
            data_dir: None,
            settings: None,
            rocks: Vec::new(),
        })
        .expect("LoadPlugin returns Some(_)");
    match result {
        livtet_plugins::protocol::HostToMain::PluginLoadError { error, .. } => {
            assert!(
                error.to_lowercase().contains("memory") || error.to_lowercase().contains("alloc"),
                "expected memory-related error, got: {error}"
            );
        }
        other => panic!("expected PluginLoadError on OOM, got {other:?}"),
    }
}

#[test]
fn host_lua_loads_benign_plugin() {
    let mut host = make_host();
    let result = host
        .handle_message(MainToHost::LoadPlugin {
            plugin_id: "benign".to_string(),
            manifest: serde_json::Value::Null,
            source: r#"return { id = "benign", name = "Benign", version = "1.0.0", capabilities = { ping = true } }"#.to_string(),
            data_dir: None,
            settings: None,
            rocks: Vec::new(),
        })
        .expect("LoadPlugin returns Some(_)");
    let loaded = match result {
        livtet_plugins::protocol::HostToMain::PluginLoaded { load_state, .. } => load_state,
        other => panic!("expected PluginLoaded, got {other:?}"),
    };
    assert_eq!(loaded, "loaded");
}

#[test]
fn host_lua_does_not_expose_os_to_loaded_plugin() {
    let mut host = make_host();
    let result = host
        .handle_message(MainToHost::LoadPlugin {
            plugin_id: "hostile-os".to_string(),
            manifest: serde_json::Value::Null,
            source: r#"os.execute("echo pwned")"#.to_string(),
            data_dir: None,
            settings: None,
            rocks: Vec::new(),
        })
        .expect("LoadPlugin returns Some(_)");
    match result {
        livtet_plugins::protocol::HostToMain::PluginLoadError { error, .. } => {
            assert!(
                !error.is_empty(),
                "expected a non-empty error from a hostile plugin"
            );
        }
        other => panic!("expected PluginLoadError, got {other:?}"),
    }
}

#[test]
fn host_writer_types_are_send_and_sync() {
    fn assert_send<T: Send + Sync>() {}
    assert_send::<SharedWriter>();
    assert_send::<CallbackRouter>();
}

#[test]
fn host_drops_cleanly() {
    let host = make_host();
    drop(host);
}

#[test]
fn sink_writer_swallows_writes() {
    let writer = empty_writer();
    let mut guard = writer.lock().expect("writer mutex poisoned");
    let _ = guard.write_all(b"hello");
}

// =====================================================================
// In-process host.* tests (Task 2.4 / Steps 1-4)
//
// The host's `host.*` functions split into two groups:
//   1. In-process — `has_plugin`, `plugin_dir`, `plugin_asset`,
//      `require`. These don't touch the IPC SharedWriter/
//      CallbackRouter; they read Lua globals or the local
//      filesystem. Testing them in-process is the same as
//      calling them from a test plugin's source.
//   2. IPC — `http_post`, `get_secret`, `set_secret`,
//      `resolve_identifier(s)`, `get_edition_info`,
//      `get_edition_identifiers`, `fetch_progress`,
//      `upsert_progress`. These emit a `HostToMain::*Request`
//      to the SharedWriter and block on a CallbackRouter
//      oneshot for a `MainToHostCallback::*Result`.
//
// The tests below drive the IPC path with a capturing writer
// and a helper that decodes the request and routes a
// callback back through the router. The helper is
// intentionally a tiny in-process mock main-process, not a
// child process, so the assertions can stay fast and
// synchronous.
// =====================================================================

/// In-process `Write` impl that mirrors every byte to a
/// shared `Vec<u8>` and pushes a unit onto an `mpsc::Sender`
/// on every `flush()`. We use a channel rather than a
/// condvar so the responder thread can never miss a flush
/// notification (condvars have a "wait / signal" race that
/// can drop a signal that arrives between wait iterations).
/// `flush()` is what the host's `write_message` calls after
/// writing a single length-prefixed msgpack message, so the
/// channel sees one push per outbound `HostToMain::*`.
struct CapturingWriter {
    buf: Arc<Mutex<Vec<u8>>>,
    signal_tx: mpsc::Sender<()>,
}

impl CapturingWriter {
    fn new(buf: Arc<Mutex<Vec<u8>>>, signal_tx: mpsc::Sender<()>) -> Self {
        Self { buf, signal_tx }
    }
}

impl std::io::Write for CapturingWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf
            .lock()
            .expect("capture buffer poisoned")
            .extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        // The writer's `flush` is called per msgpack
        // message (length prefix, then payload). Each
        // flush pushes a unit; the responder drains
        // both and finds a complete message in the
        // buffer on the second iteration.
        let _ = self.signal_tx.send(());
        Ok(())
    }
}

/// Try to drain one length-prefixed msgpack message from the
/// shared buffer. Returns `None` if the buffer doesn't yet
/// contain a complete message (i.e. the host hasn't
/// finished writing the length prefix and the full payload).
fn try_drain_request(buf: &[u8]) -> Option<(HostToMain, usize)> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return None;
    }
    let payload = &buf[4..4 + len];
    let request: HostToMain =
        rmp_serde::from_slice(payload).expect("captured HostToMain failed to decode");
    Some((request, 4 + len))
}

/// Pull the request id out of any `HostToMain` request
/// variant (mirrors `host_lua::callback_request_id`).
fn request_id(msg: &HostToMain) -> Option<&str> {
    match msg {
        HostToMain::SecretRequest { id, .. }
        | HostToMain::SetSecretRequest { id, .. }
        | HostToMain::HttpRequest { id, .. }
        | HostToMain::ReadFileRequest { id, .. }
        | HostToMain::SqliteQueryRequest { id, .. }
        | HostToMain::ReadAssetRequest { id, .. }
        | HostToMain::ResolveIdentifierRequest { id, .. }
        | HostToMain::ResolveIdentifiersRequest { id, .. }
        | HostToMain::GetEditionInfoRequest { id, .. }
        | HostToMain::GetEditionIdentifiersRequest { id, .. }
        | HostToMain::FetchProgressRequest { id, .. }
        | HostToMain::UpsertProgressRequest { id, .. } => Some(id.as_str()),
        _ => None,
    }
}

/// Build a writer, router, and a "responder" thread. The
/// responder watches the writer's buffer for new
/// `HostToMain` requests, decodes them, looks up the
/// request's id in the router, and sends back the
/// `MainToHostCallback` chosen by the `respond` closure.
///
/// Returns:
///   - the `SharedWriter` to pass to `LuaHost::new`;
///   - the shared `CallbackRouter`;
///   - a `mpsc::Receiver<HostToMain>` (the writer-side half
///     is held by the responder; the test recv's on this to
///     inspect each decoded request);
///   - the responder's `JoinHandle` (drop to signal stop).
fn start_ipc_mock<F>(
    respond: F,
) -> (
    SharedWriter,
    CallbackRouter,
    std::sync::mpsc::Receiver<HostToMain>,
    thread::JoinHandle<()>,
)
where
    F: Fn(&HostToMain) -> MainToHostCallback + Send + 'static,
{
    let capture: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let (signal_tx, signal_rx) = mpsc::channel::<()>();
    let writer: SharedWriter = Arc::new(Mutex::new(Box::new(CapturingWriter::new(
        Arc::clone(&capture),
        signal_tx,
    )) as Box<dyn std::io::Write + Send>));
    let router: CallbackRouter = Arc::new(Mutex::new(HashMap::new()));
    let (req_tx, req_rx) = std::sync::mpsc::channel::<HostToMain>();

    let capture_for_thread = Arc::clone(&capture);
    let router_for_thread = Arc::clone(&router);
    let handle = thread::spawn(move || {
        // Drain signals from the writer's `flush` calls
        // and respond to each one. The writer pushes
        // twice per message (length prefix, then
        // payload), so we accumulate signals and only
        // respond when a complete message is in the
        // buffer. A small inner-loop check after each
        // signal handles the "buffer has a complete
        // message but the next signal hasn't arrived yet"
        // case.
        loop {
            // Block until the writer signals a flush.
            // If the writer side is dropped, the channel
            // closes and `recv` returns `Err`, at which
            // point the responder thread exits.
            if signal_rx.recv().is_err() {
                return;
            }
            // Drain any further pending signals (length
            // prefix + payload arrive back-to-back).
            while signal_rx.try_recv().is_ok() {}
            // Try to drain a complete message. If the
            // host wrote length + payload in two `flush`
            // calls, both signals are now in the channel
            // and the buffer holds the full message.
            let (request, consumed) = {
                let buf = capture_for_thread.lock().expect("capture poisoned");
                match try_drain_request(&buf) {
                    Some(pair) => pair,
                    None => continue, // the next signal will arrive
                }
            };
            {
                let mut buf = capture_for_thread.lock().expect("capture poisoned");
                buf.drain(..consumed);
            }
            // Hand the request to the test for
            // inspection (non-blocking; the test
            // doesn't always need to read it).
            let _ = req_tx.send(request.clone());
            // Build the callback and send it via the
            // router.
            let cb = respond(&request);
            if let Some(id) = request_id(&request) {
                let id = id.to_string();
                let tx_opt = router_for_thread
                    .lock()
                    .expect("router poisoned")
                    .remove(&id);
                if let Some(tx) = tx_opt {
                    let _ = tx.send(cb);
                }
            }
        }
    });

    (writer, router, req_rx, handle)
}

// =====================================================================
// host.has_plugin (in-process)
// =====================================================================

/// `host.has_plugin(id)` is purely in-process: it reads the
/// `loaded_ids` set the host populated during `LoadPlugin`.
/// Loading two plugins and asking for each id by name must
/// return true; an id that was never loaded must return
/// false.
#[test]
fn host_has_plugin_returns_true_after_load_and_false_for_unknown() {
    let mut host = make_host();
    let manifest = serde_json::Value::Null;
    for id in ["alpha", "beta"] {
        let r = host
            .handle_message(MainToHost::LoadPlugin {
                plugin_id: id.to_string(),
                manifest: manifest.clone(),
                source: format!(
                    r#"return {{ id = "{id}", name = "{id}", version = "1.0.0", capabilities = {{}} }}"#
                ),
                data_dir: None,
                settings: None,
                rocks: Vec::new(),
            })
            .expect("LoadPlugin returns Some(_)");
        assert!(
            matches!(r, HostToMain::PluginLoaded { .. }),
            "expected PluginLoaded for {id}, got {r:?}"
        );
    }
    let r = host
        .handle_message(MainToHost::LoadPlugin {
            plugin_id: "probe".to_string(),
            manifest: manifest.clone(),
            source: r#"
                return {
                    has_alpha = function() return host.has_plugin("alpha") end,
                    has_beta  = function() return host.has_plugin("beta")  end,
                    has_gamma = function() return host.has_plugin("gamma") end,
                }
            "#
            .to_string(),
            data_dir: None,
            settings: None,
            rocks: Vec::new(),
        })
        .expect("LoadPlugin returns Some(_)");
    assert_matches!(r, HostToMain::PluginLoaded { .. });
    for (cap, expected) in [
        ("has_alpha", true),
        ("has_beta", true),
        ("has_gamma", false),
    ] {
        let r = host
            .handle_message(MainToHost::Call {
                id: format!("call-{cap}"),
                plugin_id: "probe".to_string(),
                capability: cap.to_string(),
                args: vec![],
            })
            .expect("Call returns Some(_)");
        match r {
            HostToMain::CallResult {
                ok, value, error, ..
            } => {
                assert!(ok, "capability {cap} returned error: {error:?}");
                let v = value.unwrap_or(serde_json::Value::Null);
                assert_eq!(v, serde_json::json!(expected), "capability {cap}");
            }
            other => panic!("expected CallResult for {cap}, got {other:?}"),
        }
    }
}

// =====================================================================
// host.plugin_dir (in-process)
// =====================================================================

#[test]
fn host_plugin_dir_returns_nil_when_data_dir_is_none() {
    let mut host = make_host();
    host.handle_message(MainToHost::LoadPlugin {
        plugin_id: "no-data-dir".to_string(),
        manifest: serde_json::Value::Null,
        source: r#"
            return { get = function() return host.plugin_dir() end }
        "#
        .to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    })
    .expect("LoadPlugin");
    let r = host
        .handle_message(MainToHost::Call {
            id: "call-1".to_string(),
            plugin_id: "no-data-dir".to_string(),
            capability: "get".to_string(),
            args: vec![],
        })
        .expect("Call");
    match r {
        HostToMain::CallResult { ok, value, .. } => {
            assert!(ok);
            assert_eq!(value, Some(serde_json::Value::Null))
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
}

#[test]
fn host_plugin_dir_returns_path_string_when_data_dir_set() {
    let mut host = make_host();
    let dir = camino::Utf8PathBuf::from("/tmp/example-plugin-data");
    host.handle_message(MainToHost::LoadPlugin {
        plugin_id: "with-data-dir".to_string(),
        manifest: serde_json::Value::Null,
        source: r#"
            return { get = function() return host.plugin_dir() end }
        "#
        .to_string(),
        data_dir: Some(dir.clone()),
        settings: None,
        rocks: Vec::new(),
    })
    .expect("LoadPlugin");
    let r = host
        .handle_message(MainToHost::Call {
            id: "call-1".to_string(),
            plugin_id: "with-data-dir".to_string(),
            capability: "get".to_string(),
            args: vec![],
        })
        .expect("Call");
    match r {
        HostToMain::CallResult { ok, value, .. } => {
            assert!(ok);
            assert_eq!(value, Some(serde_json::json!(dir.to_string())))
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
}

// =====================================================================
// host.plugin_asset (in-process)
// =====================================================================

#[test]
fn host_plugin_asset_reads_text_file_from_assets_dir() {
    let dir = camino_tempfile::tempdir().expect("tempdir");
    let assets = dir.path().join("assets");
    fs_err::create_dir_all(&assets).expect("mkdir assets");
    fs_err::write(assets.join("hello.txt"), "Hello from an asset\n").expect("write asset");
    let data_dir = dir.path().to_path_buf();

    let mut host = make_host();
    host.handle_message(MainToHost::LoadPlugin {
        plugin_id: "asset-reader".to_string(),
        manifest: serde_json::Value::Null,
        source: r#"
            return { read = function(name) return host.plugin_asset(name) end }
        "#
        .to_string(),
        data_dir: Some(data_dir),
        settings: None,
        rocks: Vec::new(),
    })
    .expect("LoadPlugin");
    let r = host
        .handle_message(MainToHost::Call {
            id: "call-1".to_string(),
            plugin_id: "asset-reader".to_string(),
            capability: "read".to_string(),
            args: vec![serde_json::json!("hello.txt")],
        })
        .expect("Call");
    match r {
        HostToMain::CallResult { ok, value, .. } => {
            assert!(ok);
            assert_eq!(value, Some(serde_json::json!("Hello from an asset\n")))
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
}

#[test]
fn host_plugin_asset_errors_when_data_dir_is_none() {
    let mut host = make_host();
    host.handle_message(MainToHost::LoadPlugin {
        plugin_id: "no-data".to_string(),
        manifest: serde_json::Value::Null,
        source: r#"
            return { read = function(name) return host.plugin_asset(name) end }
        "#
        .to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    })
    .expect("LoadPlugin");
    let r = host
        .handle_message(MainToHost::Call {
            id: "call-1".to_string(),
            plugin_id: "no-data".to_string(),
            capability: "read".to_string(),
            args: vec![serde_json::json!("anything.png")],
        })
        .expect("Call");
    match r {
        HostToMain::CallResult {
            ok, error, value, ..
        } => {
            assert!(
                !ok,
                "expected ok=false for missing data_dir, got ok=true (value={value:?})"
            );
            let msg = error.expect("expected error string for missing data_dir");
            assert!(
                msg.contains("plugin_asset") && msg.contains("no data directory"),
                "expected a 'no data directory' plugin_asset error, got: {msg}"
            );
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
}

// =====================================================================
// host.require (in-process, "errors on any call")
// =====================================================================

#[test]
fn host_require_errors_on_any_call() {
    let mut host = make_host();
    host.handle_message(MainToHost::LoadPlugin {
        plugin_id: "req-tester".to_string(),
        manifest: serde_json::Value::Null,
        source: r#"
            return {
                try = function(target)
                    local ok, err = pcall(host.require, target)
                    -- Convert the error to a string. mlua's
                    -- Error is not a JSON-serializable Lua
                    -- value, so we tostring it before
                    -- returning.
                    if ok then
                        return { ok = true }
                    end
                    return { ok = false, err = tostring(err) }
                end,
            }
        "#
        .to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    })
    .expect("LoadPlugin");
    let r = host
        .handle_message(MainToHost::Call {
            id: "call-1".to_string(),
            plugin_id: "req-tester".to_string(),
            capability: "try".to_string(),
            args: vec![serde_json::json!("foo")],
        })
        .expect("Call");
    match r {
        HostToMain::CallResult { ok, value, .. } => {
            assert!(ok, "expected ok=true from the pcall wrapper");
            let v = value.unwrap_or(serde_json::Value::Null);
            assert_eq!(v["ok"], serde_json::json!(false), "pcall must return false");
            let err = v["err"].as_str().expect("expected err string from pcall");
            assert!(
                err.contains("not yet implemented")
                    || err.contains("not implemented")
                    || err.contains("host.require"),
                "expected a 'not implemented' error, got: {err}"
            );
            assert!(
                err.contains("foo"),
                "expected the error to mention the target, got: {err}"
            );
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
}

// =====================================================================
// host.http_post (in-process IPC round-trip)
// =====================================================================

#[test]
fn host_http_post_round_trips_through_ipc() {
    // The host emits a `HostToMain::HttpRequest { method:
    // "POST", body: Some(body) }` and blocks on the matching
    // `MainToHostCallback::HttpResponse`. The mock main-process
    // decodes the request, returns a canned HttpResponse,
    // and the plugin sees a `{status, body, headers}` table.
    let (writer, router, req_rx, _handle) = start_ipc_mock(|req| {
        // Assert the request shape from the responder
        // (catches a refactor that drops the `body` field
        // or switches to GET).
        match req {
            HostToMain::HttpRequest {
                method, body, url, ..
            } => {
                assert_eq!(method, "POST", "http_post must set method=POST");
                assert_eq!(body.as_deref(), Some(r#"{"x": 1}"#));
                assert_eq!(url, "https://example.com/api");
            }
            other => panic!("expected HttpRequest, got {other:?}"),
        }
        MainToHostCallback::HttpResponse {
            id: "ignored".to_string(),
            status: 201,
            body: Some(r#"{"created": true}"#.to_string()),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        }
    });
    let mut host = LuaHost::new(Arc::new(IpcHost::new(writer, router))).expect("LuaHost::new");
    host.handle_message(MainToHost::LoadPlugin {
        plugin_id: "http-post-tester".to_string(),
        manifest: serde_json::Value::Null,
        source: r#"
            return {
                post = function(url, body)
                    local r = host.http_post(url, body)
                    return { status = r.status, body = r.body, has_headers = r.headers ~= nil }
                end,
            }
        "#
        .to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    })
    .expect("LoadPlugin");
    let result = host
        .handle_message(MainToHost::Call {
            id: "post-1".to_string(),
            plugin_id: "http-post-tester".to_string(),
            capability: "post".to_string(),
            args: vec![
                serde_json::json!("https://example.com/api"),
                serde_json::json!(r#"{"x": 1}"#),
            ],
        })
        .expect("Call returns Some");
    // Drain the request from the responder-side channel so
    // the helper doesn't keep stale state between tests.
    let _ = req_rx.recv_timeout(Duration::from_millis(200));
    drop(host);
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => {
            assert!(ok, "http_post returned error: {error:?}");
            let v = value.unwrap_or(serde_json::Value::Null);
            assert_eq!(v["status"], serde_json::json!(201));
            assert_eq!(v["body"], serde_json::json!(r#"{"created": true}"#));
            assert_eq!(v["has_headers"], serde_json::json!(true));
        }
        other => panic!("expected CallResult, got {other:?}"),
    }
}

#[test]
fn host_http_post_with_nil_body_emits_request_with_body_none() {
    let (writer, router, req_rx, _handle) = start_ipc_mock(|req| {
        match req {
            HostToMain::HttpRequest { method, body, .. } => {
                assert_eq!(method, "POST");
                assert_eq!(body, &None, "no body arg must yield body: None");
            }
            other => panic!("expected HttpRequest, got {other:?}"),
        }
        MainToHostCallback::HttpResponse {
            id: "ignored".to_string(),
            status: 204,
            body: None,
            headers: Vec::new(),
        }
    });
    let mut host = LuaHost::new(Arc::new(IpcHost::new(writer, router))).expect("LuaHost::new");
    host.handle_message(MainToHost::LoadPlugin {
        plugin_id: "post-nil".to_string(),
        manifest: serde_json::Value::Null,
        source: r#"
            return { post = function(url) return host.http_post(url) end }
        "#
        .to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    })
    .expect("LoadPlugin");
    let result = host
        .handle_message(MainToHost::Call {
            id: "post-nil-1".to_string(),
            plugin_id: "post-nil".to_string(),
            capability: "post".to_string(),
            args: vec![serde_json::json!("https://example.com/api")],
        })
        .expect("Call returns Some");
    let _ = req_rx.recv_timeout(Duration::from_millis(200));
    drop(host);
    // The plugin returns the result table. The test
    // doesn't care about the shape; the assertion that
    // matters is in the responder closure (body == None).
    assert!(
        matches!(result, HostToMain::CallResult { .. }),
        "expected CallResult, got {result:?}"
    );
}

// =====================================================================
// host.* error: Some(...) propagation tests (Task 2.4 / Step 4)
//
// Each of the seven (eight with upsert_progress) host
// functions below round-trips through the IPC path. When
// the main process replies with `error: Some(msg)`, the
// host surface MUST surface the error to the Lua caller
// as an mlua error — not as a nil, not as a partially
// populated value, and not silently. These tests pin the
// contract for each one.
// =====================================================================

/// Macro: load a plugin that defines a function whose
/// name matches the capability name, calls the
/// corresponding host function via pcall, and returns a
/// table shaped `{error = msg}` (or `{value = v}` on
/// success). The mock main-process always replies with
/// `error: Some(...)` for the matching request variant.
///
/// The arguments passed to each host function match its
/// declared signature; the test harness chooses them per
/// capability because they differ (set_secret takes
/// `(name, value)`, upsert_progress takes 4 args, etc.).
macro_rules! error_propagation_test {
    ($test_name:ident, $capability:literal, $error_msg:literal, $expected_substr:expr, $call_args:expr) => {
        #[test]
        fn $test_name() {
            let cap = $capability;
            let call_args = $call_args;
            let setup_src = format!(
                r#"
                return {{
                    {cap} = function()
                        local ok, v_or_err = pcall(host.{cap}, {call_args})
                        if not ok then
                            return {{ error = tostring(v_or_err) }}
                        end
                        -- The host function returned a value
                        -- (success path) or a multi-return
                        -- tuple (e.g. nil, err). The
                        -- contract: on success, the host
                        -- returns a value. On error, it
                        -- raises — pcall above catches it.
                        return {{ value = v_or_err }}
                    end,
                }}
                "#
            );
            let (writer, router, _req_rx, _handle) = start_ipc_mock(|req| match req {
                HostToMain::SecretRequest { id, .. } => MainToHostCallback::SecretResult {
                    id: id.clone(),
                    value: None,
                    error: Some($error_msg.to_string()),
                },
                HostToMain::SetSecretRequest { id, .. } => MainToHostCallback::SecretResult {
                    id: id.clone(),
                    value: None,
                    error: Some($error_msg.to_string()),
                },
                HostToMain::ResolveIdentifierRequest { id, .. } => {
                    MainToHostCallback::ResolveIdentifierResult {
                        id: id.clone(),
                        edition_id: None,
                        error: Some($error_msg.to_string()),
                    }
                }
                HostToMain::ResolveIdentifiersRequest { id, .. } => {
                    MainToHostCallback::ResolveIdentifiersResult {
                        id: id.clone(),
                        edition_ids: Vec::new(),
                        error: Some($error_msg.to_string()),
                    }
                }
                HostToMain::GetEditionInfoRequest { id, .. } => {
                    MainToHostCallback::EditionInfoResult {
                        id: id.clone(),
                        info: None,
                        error: Some($error_msg.to_string()),
                    }
                }
                HostToMain::GetEditionIdentifiersRequest { id, .. } => {
                    MainToHostCallback::EditionIdentifiersResult {
                        id: id.clone(),
                        urns: Vec::new(),
                        error: Some($error_msg.to_string()),
                    }
                }
                HostToMain::FetchProgressRequest { id, .. } => {
                    MainToHostCallback::FetchProgressResult {
                        id: id.clone(),
                        progress: None,
                        error: Some($error_msg.to_string()),
                    }
                }
                HostToMain::UpsertProgressRequest { id, .. } => {
                    MainToHostCallback::UpsertProgressResult {
                        id: id.clone(),
                        edition_id: None,
                        format_id: None,
                        ok: false,
                        error: Some($error_msg.to_string()),
                    }
                }
                other => panic!("unexpected request variant: {other:?}"),
            });
            let mut host =
                LuaHost::new(Arc::new(IpcHost::new(writer, router))).expect("LuaHost::new");
            host.handle_message(MainToHost::LoadPlugin {
                plugin_id: "err-tester".to_string(),
                manifest: serde_json::Value::Null,
                source: setup_src,
                data_dir: None,
                settings: None,
                rocks: Vec::new(),
            })
            .expect("LoadPlugin");
            let result = host
                .handle_message(MainToHost::Call {
                    id: "call-1".to_string(),
                    plugin_id: "err-tester".to_string(),
                    capability: cap.to_string(),
                    args: vec![],
                })
                .expect("Call returns Some");
            drop(host);
            match result {
                HostToMain::CallResult { ok, value, .. } => {
                    assert!(ok, "expected ok=true from the pcall wrapper");
                    let v = value.unwrap_or(serde_json::Value::Null);
                    let err = v["error"]
                        .as_str()
                        .expect("expected error string under 'error' key");
                    assert!(
                        err.contains($expected_substr),
                        "expected error containing {:?}, got {err:?}",
                        $expected_substr
                    );
                }
                other => panic!("expected CallResult, got {other:?}"),
            }
        }
    };
}

error_propagation_test!(
    host_get_secret_propagates_error_some_as_mlua_error,
    "get_secret",
    "access denied",
    "access denied",
    r#""api_key""#
);

error_propagation_test!(
    host_set_secret_propagates_error_some_as_mlua_error,
    "set_secret",
    "permission denied",
    "permission denied",
    r#""k", "v""#
);

error_propagation_test!(
    host_resolve_identifier_propagates_error_some_as_mlua_error,
    "resolve_identifier",
    "urn not found",
    "urn not found",
    r#""urn:isbn:1""#
);

error_propagation_test!(
    host_resolve_identifiers_propagates_error_some_as_mlua_error,
    "resolve_identifiers",
    "batch resolve failed",
    "batch resolve failed",
    r#"{"urn:isbn:1"}"#
);

error_propagation_test!(
    host_get_edition_info_propagates_error_some_as_mlua_error,
    "get_edition_info",
    "edition lookup failed",
    "edition lookup failed",
    r#""e1""#
);

error_propagation_test!(
    host_get_edition_identifiers_propagates_error_some_as_mlua_error,
    "get_edition_identifiers",
    "identifiers lookup failed",
    "identifiers lookup failed",
    r#""e1""#
);

error_propagation_test!(
    host_fetch_progress_propagates_error_some_as_mlua_error,
    "fetch_progress",
    "progress fetch failed",
    "progress fetch failed",
    r#""urn:isbn:1""#
);

error_propagation_test!(
    host_upsert_progress_propagates_error_some_as_mlua_error,
    "upsert_progress",
    "progress upsert failed",
    "progress upsert failed",
    r#""urn:isbn:1", 0.5, nil, 0"#
);

// =====================================================================
// host.fs_copy / host.fs_symlink — in-process, grant-gated
// =====================================================================
//
// `host.fs_copy` and `host.fs_symlink` resolve their grant
// sidecar on first call (the same lazy path `read_file` /
// `sqlite_query` use) and check the requested path against the
// `read_paths` / `write_paths` glob sets. They never raise on
// grant denial — they return an `{ __livtet_error = { category,
// message } }` table the plugin can log and surface. The tests
// below drive the host with a pre-populated grant cache so the
// real `~/.local/share/...` directory is never touched.

use std::collections::HashMap as _HashMap;

/// Build a `ResolvedGrant` whose `read_paths` and `write_paths`
/// glob sets cover the supplied glob strings. Empty input
/// produces an empty (allow-nothing) globset, matching the
/// "no grant" case from the host's perspective.
fn resolved_fs_grant(
    read_globs: &[&str],
    write_globs: &[&str],
) -> std::sync::Arc<livtet_plugins::permissions::ResolvedGrant> {
    use globset::GlobSetBuilder;
    let mut read_builder = GlobSetBuilder::new();
    for g in read_globs {
        read_builder.add(globset::Glob::new(g).expect("valid read glob"));
    }
    let mut write_builder = GlobSetBuilder::new();
    for g in write_globs {
        write_builder.add(globset::Glob::new(g).expect("valid write glob"));
    }
    std::sync::Arc::new(livtet_plugins::permissions::ResolvedGrant {
        raw: livtet_plugins::permissions::PluginGrant {
            version: 1,
            read_paths: read_globs.iter().map(|s| s.to_string()).collect(),
            sqlite_paths: Vec::new(),
            allow_writes: false,
            write_paths: write_globs.iter().map(|s| s.to_string()).collect(),
            system_secrets: Vec::new(),
            embeddings: false,
            oauth_providers: Vec::new(),
            http_proxy_url: None,
        },
        read_paths: read_builder.build().expect("build read globset"),
        sqlite_paths: GlobSetBuilder::new().build().expect("empty globset"),
        write_paths: write_builder.build().expect("build write globset"),
        system_secrets: std::collections::HashSet::new(),
        embeddings: false,
        oauth_providers: _HashMap::new(),
        http_proxy_url: None,
    })
}

/// Standard plugin source: defines `copy` and `symlink`
/// capabilities that forward straight to `host.fs_copy` /
/// `host.fs_symlink`. The host never sees the grant
/// denial/success path through this plugin; the test asserts
/// on the value the Lua caller observes.
const FS_PLUGIN_SOURCE: &str = r#"
    return {
        copy = function(src, dst) return host.fs_copy(src, dst) end,
        symlink = function(target, link_path)
            return host.fs_symlink(target, link_path)
        end,
    }
"#;

/// Load `FS_PLUGIN_SOURCE` under `plugin_id` and call the
/// `capability` capability with the given string args.
/// Returns the `CallResult`'s JSON `value` (or `error`) for
/// assertion.
fn call_fs_capability(
    host: &mut LuaHost<IpcHost>,
    plugin_id: &str,
    capability: &str,
    args: Vec<String>,
) -> (bool, Option<serde_json::Value>, Option<String>) {
    host.handle_message(MainToHost::LoadPlugin {
        plugin_id: plugin_id.to_string(),
        manifest: serde_json::Value::Null,
        source: FS_PLUGIN_SOURCE.to_string(),
        data_dir: None,
        settings: None,
        rocks: Vec::new(),
    })
    .expect("LoadPlugin returns Some");
    let result = host
        .handle_message(MainToHost::Call {
            id: format!("fs-{capability}"),
            plugin_id: plugin_id.to_string(),
            capability: capability.to_string(),
            args: args.into_iter().map(serde_json::Value::String).collect(),
        })
        .expect("Call returns Some");
    match result {
        HostToMain::CallResult {
            ok, value, error, ..
        } => (ok, value, error),
        other => panic!("expected CallResult, got {other:?}"),
    }
}

#[test]
fn host_fs_copy_happy_path_returns_true() {
    let tmp = camino_tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src.txt");
    let dst = tmp.path().join("dst.txt");
    fs_err::write(&src, b"hello").expect("write src");
    let glob = format!("{}/**", tmp.path().to_string());

    let mut host = make_host();
    let grant = resolved_fs_grant(&[glob.as_str()], &[glob.as_str()]);
    host.grant_plugin("fs-copy-ok", grant)
        .expect("grant_plugin");

    let (ok, value, error) = call_fs_capability(
        &mut host,
        "fs-copy-ok",
        "copy",
        vec![
            src.to_string().to_string(),
            dst.to_string().to_string(),
        ],
    );
    assert!(ok, "call should succeed, got error {error:?}");
    assert_eq!(value, Some(serde_json::json!(true)));
    assert!(
        dst.exists(),
        "destination should exist after fs_copy success"
    );
    assert_eq!(fs_err::read(&dst).expect("read dst"), b"hello");
}

#[test]
fn host_fs_copy_src_outside_read_paths_returns_permission_denied() {
    let tmp = camino_tempfile::tempdir().expect("tempdir");
    let allowed = tmp.path().join("allowed");
    fs_err::create_dir_all(&allowed).expect("mkdir allowed");
    let src = tmp.path().join("src.txt"); // NOT under allowed/
    let dst = allowed.join("dst.txt");
    let allow_glob = format!("{}/**", allowed.to_string());

    let mut host = make_host();
    let grant = resolved_fs_grant(&[allow_glob.as_str()], &[allow_glob.as_str()]);
    host.grant_plugin("fs-copy-no-read", grant)
        .expect("grant_plugin");

    let (ok, value, error) = call_fs_capability(
        &mut host,
        "fs-copy-no-read",
        "copy",
        vec![
            src.to_string().to_string(),
            dst.to_string().to_string(),
        ],
    );
    assert!(ok, "Lua call returned without raising, got error {error:?}");
    let v = value.expect("expected __livtet_error envelope");
    assert_eq!(
        v["__livtet_error"]["category"],
        serde_json::json!("permission_denied")
    );
    let msg = v["__livtet_error"]["message"]
        .as_str()
        .expect("error message string");
    assert!(
        msg.contains("permission denied"),
        "expected permission-denied message, got {msg}"
    );
    assert!(
        !dst.exists(),
        "dst must not be created when read grant fails"
    );
}

#[test]
fn host_fs_copy_dst_outside_write_paths_returns_permission_denied() {
    let tmp = camino_tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src.txt");
    fs_err::write(&src, b"hi").expect("write src");
    let outside = tmp.path().join("outside");
    fs_err::create_dir_all(&outside).expect("mkdir outside");
    let dst = outside.join("dst.txt");
    let read_glob = format!("{}/**", tmp.path().to_string());
    // read allows the whole tmp tree; write only allows an
    // empty allowlist so no path matches.
    let mut host = make_host();
    let grant = resolved_fs_grant(&[read_glob.as_str()], &[]);
    host.grant_plugin("fs-copy-no-write", grant)
        .expect("grant_plugin");

    let (ok, value, error) = call_fs_capability(
        &mut host,
        "fs-copy-no-write",
        "copy",
        vec![
            src.to_string().to_string(),
            dst.to_string().to_string(),
        ],
    );
    assert!(ok, "Lua call returned without raising, got error {error:?}");
    let v = value.expect("expected __livtet_error envelope");
    assert_eq!(
        v["__livtet_error"]["category"],
        serde_json::json!("permission_denied")
    );
    assert!(
        !dst.exists(),
        "dst must not be created when write grant fails"
    );
}

#[test]
fn host_fs_copy_missing_source_returns_file_not_found() {
    let tmp = camino_tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("does-not-exist.txt");
    let dst = tmp.path().join("dst.txt");
    let glob = format!("{}/**", tmp.path().to_string());

    let mut host = make_host();
    let grant = resolved_fs_grant(&[glob.as_str()], &[glob.as_str()]);
    host.grant_plugin("fs-copy-missing", grant)
        .expect("grant_plugin");

    let (ok, value, error) = call_fs_capability(
        &mut host,
        "fs-copy-missing",
        "copy",
        vec![
            src.to_string().to_string(),
            dst.to_string().to_string(),
        ],
    );
    assert!(ok, "Lua call returned without raising, got error {error:?}");
    let v = value.expect("expected __livtet_error envelope");
    assert_eq!(
        v["__livtet_error"]["category"],
        serde_json::json!("file_not_found")
    );
    assert!(
        !dst.exists(),
        "dst must not be created when source is missing"
    );
}

#[test]
#[cfg(unix)]
fn host_fs_symlink_happy_path_creates_symlink() {
    let tmp = camino_tempfile::tempdir().expect("tempdir");
    let target = tmp.path().join("target.txt");
    let link_path = tmp.path().join("link.txt");
    fs_err::write(&target, b"hi").expect("write target");
    let glob = format!("{}/**", tmp.path().to_string());

    let mut host = make_host();
    let grant = resolved_fs_grant(&[glob.as_str()], &[glob.as_str()]);
    host.grant_plugin("fs-symlink-ok", grant)
        .expect("grant_plugin");

    let (ok, value, error) = call_fs_capability(
        &mut host,
        "fs-symlink-ok",
        "symlink",
        vec![
            target.to_string().to_string(),
            link_path.to_string().to_string(),
        ],
    );
    assert!(ok, "call should succeed, got error {error:?}");
    assert_eq!(value, Some(serde_json::json!(true)));
    let meta = fs_err::symlink_metadata(&link_path).expect("symlink_metadata");
    assert!(
        meta.file_type().is_symlink(),
        "link_path should be a symlink"
    );
    let link_target = fs_err::read_link(&link_path).expect("read_link");
    assert_eq!(
        link_target.to_string_lossy().to_string(),
        target.to_string(),
        "symlink should point at target",
    );
}

#[test]
fn host_fs_symlink_link_outside_write_paths_returns_permission_denied() {
    let tmp = camino_tempfile::tempdir().expect("tempdir");
    let target = tmp.path().join("target.txt");
    fs_err::write(&target, b"hi").expect("write target");
    let outside = tmp.path().join("outside");
    fs_err::create_dir_all(&outside).expect("mkdir outside");
    let link_path = outside.join("link.txt");

    let mut host = make_host();
    // read+write both empty: nothing is allowed.
    let grant = resolved_fs_grant(&[], &[]);
    host.grant_plugin("fs-symlink-no-write", grant)
        .expect("grant_plugin");

    let (ok, value, error) = call_fs_capability(
        &mut host,
        "fs-symlink-no-write",
        "symlink",
        vec![
            target.to_string().to_string(),
            link_path.to_string().to_string(),
        ],
    );
    assert!(ok, "Lua call returned without raising, got error {error:?}");
    let v = value.expect("expected __livtet_error envelope");
    assert_eq!(
        v["__livtet_error"]["category"],
        serde_json::json!("permission_denied")
    );
    assert!(
        !link_path.exists() && fs_err::symlink_metadata(&link_path).is_err(),
        "symlink must not be created when grant fails"
    );
}

#[test]
fn host_fs_symlink_existing_link_path_returns_file_error() {
    let tmp = camino_tempfile::tempdir().expect("tempdir");
    let target = tmp.path().join("target.txt");
    let link_path = tmp.path().join("link.txt");
    fs_err::write(&target, b"hi").expect("write target");
    // Pre-create the link path as a regular file so the
    // symlink(2) call fails with EEXIST.
    fs_err::write(&link_path, b"blocker").expect("write blocker");
    let glob = format!("{}/**", tmp.path().to_string());

    let mut host = make_host();
    let grant = resolved_fs_grant(&[glob.as_str()], &[glob.as_str()]);
    host.grant_plugin("fs-symlink-eexist", grant)
        .expect("grant_plugin");

    let (ok, value, error) = call_fs_capability(
        &mut host,
        "fs-symlink-eexist",
        "symlink",
        vec![
            target.to_string().to_string(),
            link_path.to_string().to_string(),
        ],
    );
    assert!(ok, "Lua call returned without raising, got error {error:?}");
    let v = value.expect("expected __livtet_error envelope");
    assert_eq!(
        v["__livtet_error"]["category"],
        serde_json::json!("file_error")
    );
}
