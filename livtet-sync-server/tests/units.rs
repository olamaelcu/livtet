//! The daemon's pure pieces: argument parsing, error rendering, the request
//! ring buffer, and JSON-RPC framing.
//!
//! `rpc_health.rs` covers the daemon end to end over a real stdio pipe;
//! these cover the decisions that test cannot reach from outside.

use livtet_sync_server::{
    RequestRecord, RpcMessage, RpcResponse, ServerConfig, ServerError, encode_notification,
    encode_request, encode_response, error_code, new_request_log, parse_message, recent_requests,
    record_request,
};

fn args(items: &[&str]) -> impl Iterator<Item = String> + use<> {
    items
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .into_iter()
}

// ─── config ─────────────────────────────────────────────────────────────────

#[test]
fn no_arguments_yields_the_documented_defaults() {
    let config = ServerConfig::from_args(args(&[])).expect("defaults");
    assert_eq!(config.db_path, "livtet-sync.db");
    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 0, "0 asks the OS for an ephemeral port");
    assert_eq!(config.device_id, None);
}

#[test]
fn every_flag_is_parsed() {
    let config = ServerConfig::from_args(args(&[
        "--db",
        "/tmp/x.db",
        "--host",
        "0.0.0.0",
        "--port",
        "3121",
        "--device-id",
        "dev-7",
    ]))
    .expect("full argument list");
    assert_eq!(config.db_path, "/tmp/x.db");
    assert_eq!(config.host, "0.0.0.0");
    assert_eq!(config.port, 3121);
    assert_eq!(config.device_id.as_deref(), Some("dev-7"));
}

/// A flag whose value is missing must not silently fall back to the default.
#[test]
fn a_flag_without_its_value_is_an_error_naming_the_flag() {
    for flag in ["--db", "--host", "--port", "--device-id"] {
        let err =
            ServerConfig::from_args(args(&[flag])).expect_err("a dangling flag must not parse");
        let message = err.to_string();
        assert!(
            message.contains(flag),
            "the error must name {flag}: {message}"
        );
        assert!(matches!(err, ServerError::Config(_)), "got {err:?}");
    }
}

#[test]
fn a_non_numeric_port_is_rejected_and_echoed() {
    let err = ServerConfig::from_args(args(&["--port", "http"])).expect_err("not a port");
    let message = err.to_string();
    assert!(
        message.contains("http"),
        "must echo the bad value: {message}"
    );
    // 65536 overflows u16 and must be caught the same way.
    assert!(ServerConfig::from_args(args(&["--port", "65536"])).is_err());
    assert!(
        ServerConfig::from_args(args(&["--port", "65535"])).is_ok(),
        "the top of the u16 range is valid"
    );
}

#[test]
fn an_unknown_argument_is_rejected_rather_than_ignored() {
    let err = ServerConfig::from_args(args(&["--colour", "blue"])).expect_err("unknown flag");
    assert!(err.to_string().contains("--colour"), "got {err}");
}

// ─── error ──────────────────────────────────────────────────────────────────

#[test]
fn each_error_variant_renders_with_its_own_prefix() {
    let cases = [
        (
            ServerError::Io("broken pipe".into()),
            "io error: broken pipe",
        ),
        (ServerError::Db("locked".into()), "database error: locked"),
        (ServerError::Http("bind".into()), "http error: bind"),
        (ServerError::Rpc("bad frame".into()), "rpc error: bad frame"),
        (ServerError::Config("nope".into()), "config error: nope"),
        (ServerError::Server("down".into()), "server error: down"),
    ];
    for (err, expected) in cases {
        assert_eq!(err.to_string(), expected);
    }
}

/// Each `From` impl routes to the variant that describes where the failure
/// came from, so the daemon's logs stay diagnosable.
#[test]
fn conversions_route_to_the_right_variant() {
    let io: ServerError = std::io::Error::other("nope").into();
    assert!(matches!(io, ServerError::Io(_)), "got {io:?}");

    let json: ServerError = serde_json::from_str::<ServerConfigShape>("{")
        .expect_err("malformed JSON")
        .into();
    assert!(matches!(json, ServerError::Rpc(_)), "got {json:?}");

    let sync: ServerError = livtet_sync::SyncError::UnknownEntityType {
        type_name: "widget".into(),
    }
    .into();
    assert!(matches!(sync, ServerError::Db(_)), "got {sync:?}");
}

/// Only used as a deserialization target for the error-conversion test.
#[derive(Debug, serde::Deserialize)]
struct ServerConfigShape {
    #[allow(dead_code)]
    unused: i32,
}

// ─── requests: the bounded ring buffer ──────────────────────────────────────

fn record_at(n: usize) -> RequestRecord {
    RequestRecord {
        method: "GET".to_string(),
        path: format!("/sync/{n}"),
        status: 200,
        at: "2026-01-01T00:00:00Z".to_string(),
        device_id: Some(format!("dev-{n}")),
    }
}

#[test]
fn recent_returns_newest_first_and_honours_the_limit() {
    let log = new_request_log(8);
    for n in 0..4 {
        record_request(&log, 8, record_at(n));
    }

    let newest = recent_requests(&log, 2);
    assert_eq!(newest.len(), 2);
    assert_eq!(newest[0].path, "/sync/3", "newest first");
    assert_eq!(newest[1].path, "/sync/2");

    assert_eq!(
        recent_requests(&log, 99).len(),
        4,
        "limit may exceed the log"
    );
    assert!(recent_requests(&log, 0).is_empty());
}

/// The log is bounded, so an old request must be evicted rather than growing
/// the buffer without limit in a long-running daemon.
#[test]
fn the_log_evicts_the_oldest_once_full() {
    let cap = 3;
    let log = new_request_log(cap);
    for n in 0..5 {
        record_request(&log, cap, record_at(n));
    }

    let all = recent_requests(&log, 99);
    assert_eq!(all.len(), cap, "the buffer must not grow past its cap");
    let paths: Vec<&str> = all.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["/sync/4", "/sync/3", "/sync/2"],
        "the three newest survive"
    );
}

#[test]
fn an_empty_log_reports_nothing() {
    assert!(recent_requests(&new_request_log(4), 10).is_empty());
}

#[test]
fn notify_fires_through_the_sink() {
    use std::sync::{Arc, Mutex};
    let seen: Arc<Mutex<Vec<(String, serde_json::Value)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let notifier: livtet_sync_server::Notifier =
        Arc::new(move |method: &str, params: serde_json::Value| {
            sink.lock().unwrap().push((method.to_string(), params));
        });

    livtet_sync_server::requests::notify(
        &notifier,
        "request.received",
        serde_json::json!({"a": 1}),
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "request.received");
    assert_eq!(seen[0].1, serde_json::json!({"a": 1}));
}

// ─── rpc framing ────────────────────────────────────────────────────────────

/// The discriminator is structural: `method` with an `id` is a request,
/// `method` without one is a notification, an `id` alone is a response.
#[test]
fn parse_message_classifies_by_method_and_id() {
    let request = parse_message(r#"{"jsonrpc":"2.0","id":1,"method":"status"}"#).expect("request");
    match request {
        RpcMessage::Request(r) => {
            assert_eq!(r.method, "status");
            assert_eq!(r.id, Some(serde_json::json!(1)));
        }
        other => panic!("expected a request, got {other:?}"),
    }

    let notification =
        parse_message(r#"{"jsonrpc":"2.0","method":"request.received"}"#).expect("notification");
    match notification {
        RpcMessage::Notification(n) => assert_eq!(n.method, "request.received"),
        other => panic!("expected a notification, got {other:?}"),
    }

    let response =
        parse_message(r#"{"jsonrpc":"2.0","id":2,"result":{"ok":true}}"#).expect("response");
    match response {
        RpcMessage::Response(r) => {
            assert_eq!(r.id, Some(serde_json::json!(2)));
            assert_eq!(r.result, Some(serde_json::json!({"ok": true})));
        }
        other => panic!("expected a response, got {other:?}"),
    }
}

#[test]
fn parse_message_rejects_frames_it_cannot_classify() {
    let not_json = parse_message("{").expect_err("malformed JSON");
    assert!(
        not_json.to_string().contains("parse error"),
        "got {not_json}"
    );

    let neither = parse_message(r#"{"jsonrpc":"2.0"}"#)
        .expect_err("a frame with neither method nor id is not JSON-RPC");
    assert!(
        neither.to_string().contains("no method or id"),
        "got {neither}"
    );

    // Well-formed JSON of the wrong shape: `method` must be a string.
    let bad_shape =
        parse_message(r#"{"jsonrpc":"2.0","method":5}"#).expect_err("method must be a string");
    assert!(
        bad_shape.to_string().contains("invalid notification"),
        "got {bad_shape}"
    );
}

#[test]
fn responses_carry_either_a_result_or_an_error_but_not_both() {
    let ok = RpcResponse::ok(Some(serde_json::json!(1)), serde_json::json!("fine"));
    assert_eq!(ok.jsonrpc, "2.0");
    assert!(ok.result.is_some() && ok.error.is_none());

    let err = RpcResponse::error(
        Some(serde_json::json!(1)),
        error_code::METHOD_NOT_FOUND,
        "no such method",
    );
    assert!(err.result.is_none());
    let error = err.error.expect("an error object");
    assert_eq!(error.code, error_code::METHOD_NOT_FOUND);
    assert_eq!(error.message, "no such method");
    assert_eq!(error.data, None);
}

/// Encoded frames must round-trip back through `parse_message`, since that is
/// exactly what the parent process does with them.
#[test]
fn encoded_frames_round_trip_through_the_parser() {
    let request = encode_request(9, "pairing.begin", serde_json::json!({"ttl": 60}));
    assert!(matches!(
        parse_message(&request).expect("request round trip"),
        RpcMessage::Request(_)
    ));

    let notification = encode_notification("request.received", serde_json::json!({"path": "/x"}));
    assert!(matches!(
        parse_message(&notification).expect("notification round trip"),
        RpcMessage::Notification(_)
    ));

    let response = encode_response(&RpcResponse::ok(
        Some(serde_json::json!(9)),
        serde_json::json!(null),
    ));
    assert!(matches!(
        parse_message(&response).expect("response round trip"),
        RpcMessage::Response(_)
    ));

    // No frame may carry a trailing newline -- the daemon adds the NDJSON
    // delimiter itself, and a doubled one would end the stream early.
    for frame in [&request, &notification, &response] {
        assert!(
            !frame.ends_with('\n'),
            "{frame:?} must not be newline-terminated"
        );
    }
}

#[test]
fn an_error_response_encodes_its_code_and_message() {
    let frame = encode_response(&RpcResponse::error(
        None,
        error_code::INVALID_PARAMS,
        "missing token",
    ));
    let value: serde_json::Value = serde_json::from_str(&frame).expect("valid JSON");
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["error"]["code"], error_code::INVALID_PARAMS);
    assert_eq!(value["error"]["message"], "missing token");
    assert!(
        value.get("result").is_none(),
        "a result member must be omitted entirely, not sent as null"
    );
}
