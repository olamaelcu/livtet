use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use livtet_plugins::embedded_host::EmbeddedHost;
use livtet_plugins::host_trait::HostHttp;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock_test() -> MutexGuard<'static, ()> {
    match TEST_LOCK.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct EnvGuard {
    vars: Vec<&'static str>,
}

impl EnvGuard {
    fn set_rewrite(from: &str, to: &str) -> Self {
        unsafe {
            std::env::set_var("LIVTET_HTTP_REWRITE_FROM", from);
            std::env::set_var("LIVTET_HTTP_REWRITE_TO", to);
        }
        Self {
            vars: vec!["LIVTET_HTTP_REWRITE_FROM", "LIVTET_HTTP_REWRITE_TO"],
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for var in &self.vars {
            unsafe {
                std::env::set_var(var, "");
            }
        }
    }
}

fn bind_port() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Length: {len}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{body}",
        len = body.len(),
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

fn write_200(stream: &mut TcpStream, body: &str) {
    write_response(stream, 200, body);
}

struct HttpRequest {
    request_line: String,
    headers: Vec<String>,
    body: Option<String>,
}

fn read_request(stream: &mut TcpStream) -> HttpRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();

    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).unwrap();
    assert!(n > 0, "server: empty request");
    let raw = String::from_utf8_lossy(&buf[..n]);

    let first_line_end = raw.find('\n').unwrap_or(raw.len());
    let request_line = raw[..first_line_end].trim().to_string();

    let headers_end = raw
        .find("\r\n\r\n")
        .map(|p| p + 4)
        .or_else(|| raw.find("\n\n").map(|p| p + 2));

    let (headers, body) = match headers_end {
        Some(body_start) => {
            let header_section = &raw[first_line_end + 1..body_start.min(raw.len()) - 2];
            let mut headers = Vec::new();
            let mut content_length = None;
            for line in header_section.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(val) = trimmed
                    .to_lowercase()
                    .strip_prefix("content-length:")
                {
                    content_length = val.trim().parse::<usize>().ok();
                }
                headers.push(trimmed.to_string());
            }

            let body = match (body_start < raw.len(), content_length) {
                (true, Some(expected_len)) => {
                    let body_region = &buf[body_start..];
                    let available = body_region.len();
                    if available >= expected_len {
                        Some(
                            String::from_utf8_lossy(&body_region[..expected_len])
                                .to_string(),
                        )
                    } else {
                        let mut full_body = body_region.to_vec();
                        let mut remaining = vec![0u8; expected_len - available];
                        stream.read_exact(&mut remaining).ok();
                        full_body.extend_from_slice(&remaining);
                        Some(String::from_utf8_lossy(&full_body).to_string())
                    }
                }
                _ => None,
            };
            (headers, body)
        }
        None => (Vec::new(), None),
    };

    HttpRequest {
        request_line,
        headers,
        body,
    }
}

fn spawn_server<F>(listener: TcpListener, handler: F) -> std::thread::JoinHandle<()>
where
    F: FnOnce(&mut TcpStream) + Send + 'static,
{
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        handler(&mut stream);
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn http_get_returns_200_with_body() {
    let _lock = lock_test();
    let (listener, port) = bind_port();
    let _guard = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port}"),
    );

    let server = spawn_server(listener, |stream| {
        let _req = read_request(stream);
        write_200(stream, "hello");
    });

    let host = EmbeddedHost::new();
    let resp = host
        .http_get("https://embedded.test", &[])
        .expect("http_get should succeed");

    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.as_deref(), Some("hello"));

    server.join().unwrap();
}

#[test]
fn http_get_passes_headers() {
    let _lock = lock_test();
    let (listener, port) = bind_port();
    let _guard = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port}"),
    );

    let server = spawn_server(listener, |stream| {
        let req = read_request(stream);
        let echo = req
            .headers
            .iter()
            .find(|h| h.to_lowercase().starts_with("x-custom:"))
            .cloned()
            .unwrap_or_else(|| "x-custom: NOT_FOUND".to_string());
        let body = format!("received: {echo}");
        write_200(stream, &body);
    });

    let host = EmbeddedHost::new();
    let resp = host
        .http_get(
            "https://embedded.test/",
            &[("x-custom".to_string(), "test-value".to_string())],
        )
        .expect("http_get should succeed");

    assert_eq!(resp.status, 200);
    let body = resp.body.unwrap();
    assert!(
        body.to_lowercase().contains("x-custom: test-value"),
        "response body {body:?} should contain the sent header"
    );

    server.join().unwrap();
}

#[test]
fn http_post_sends_body() {
    let _lock = lock_test();
    let (listener, port) = bind_port();
    let _guard = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port}"),
    );

    let server = spawn_server(listener, |stream| {
        let req = read_request(stream);
        let method_ok = req.request_line.starts_with("POST");
        let body_echo = format!(
            "method={} body={}",
            if method_ok { "POST" } else { &req.request_line },
            req.body.as_deref().unwrap_or("(no body)")
        );
        write_200(stream, &body_echo);
    });

    let host = EmbeddedHost::new();
    let resp = host
        .http_post("https://embedded.test/", Some("hello world"), &[])
        .expect("http_post should succeed");

    assert_eq!(resp.status, 200);
    let body = resp.body.unwrap();
    assert!(body.contains("method=POST"), "body {body:?}");
    assert!(body.contains("body=hello world"), "body {body:?}");

    server.join().unwrap();
}

#[test]
fn http_put_sends_body() {
    let _lock = lock_test();
    let (listener, port) = bind_port();
    let _guard = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port}"),
    );

    let server = spawn_server(listener, |stream| {
        let req = read_request(stream);
        let method_ok = req.request_line.starts_with("PUT");
        let echo = format!(
            "method={} body={}",
            if method_ok { "PUT" } else { &req.request_line },
            req.body.as_deref().unwrap_or("(no body)")
        );
        write_200(stream, &echo);
    });

    let host = EmbeddedHost::new();
    let resp = host
        .http_put("https://embedded.test/", Some("put-body"), &[])
        .expect("http_put should succeed");

    assert_eq!(resp.status, 200);
    let body = resp.body.unwrap();
    assert!(body.contains("method=PUT"), "body {body:?}");
    assert!(body.contains("body=put-body"), "body {body:?}");

    server.join().unwrap();
}

#[test]
fn http_patch_sends_body() {
    let _lock = lock_test();
    let (listener, port) = bind_port();
    let _guard = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port}"),
    );

    let server = spawn_server(listener, |stream| {
        let req = read_request(stream);
        let method_ok = req.request_line.starts_with("PATCH");
        let echo = format!(
            "method={} body={}",
            if method_ok { "PATCH" } else { &req.request_line },
            req.body.as_deref().unwrap_or("(no body)")
        );
        write_200(stream, &echo);
    });

    let host = EmbeddedHost::new();
    let resp = host
        .http_patch("https://embedded.test/", Some("patch-body"), &[])
        .expect("http_patch should succeed");

    assert_eq!(resp.status, 200);
    let body = resp.body.unwrap();
    assert!(body.contains("method=PATCH"), "body {body:?}");
    assert!(body.contains("body=patch-body"), "body {body:?}");

    server.join().unwrap();
}

#[test]
fn http_delete_no_body() {
    let _lock = lock_test();
    let (listener, port) = bind_port();
    let _guard = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port}"),
    );

    let server = spawn_server(listener, |stream| {
        let req = read_request(stream);
        let method_ok = req.request_line.starts_with("DELETE");
        let echo = format!(
            "method={} has_body={}",
            if method_ok {
                "DELETE"
            } else {
                &req.request_line
            },
            req.body.is_some()
        );
        write_200(stream, &echo);
    });

    let host = EmbeddedHost::new();
    let resp = host
        .http_delete("https://embedded.test/", &[])
        .expect("http_delete should succeed");

    assert_eq!(resp.status, 200);
    let body = resp.body.unwrap();
    assert!(body.contains("method=DELETE"), "body {body:?}");
    assert!(body.contains("has_body=false"), "body {body:?}");

    server.join().unwrap();
}

#[test]
fn http_get_unreachable_returns_error() {
    let _lock = lock_test();
    let (listener, port) = bind_port();
    drop(listener);

    let _guard = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port}"),
    );

    let host = EmbeddedHost::new();
    let result = host.http_get("https://embedded.test/", &[]);
    assert!(
        result.is_err(),
        "expected error for unreachable port, got Ok"
    );

    let (listener2, port2) = bind_port();
    let _guard2 = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port2}"),
    );

    let server2 = spawn_server(listener2, |stream| {
        let _req = read_request(stream);
        write_200(stream, "recovered");
    });

    let resp2 = host
        .http_get("https://embedded.test/", &[])
        .expect("worker should survive earlier error");
    assert_eq!(resp2.body.as_deref(), Some("recovered"));

    server2.join().unwrap();
}

#[test]
fn http_get_rewrites_url() {
    let _lock = lock_test();
    let (listener, port) = bind_port();
    let _guard = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port}"),
    );

    let server = spawn_server(listener, |stream| {
        let req = read_request(stream);
        let echo =
            if req.request_line.contains("/test-path") && req.request_line.contains("query=1") {
                "rewrite_ok"
            } else {
                "rewrite_fail"
            };
        write_200(stream, echo);
    });

    let host = EmbeddedHost::new();
    let resp = host
        .http_get("https://embedded.test/test-path?query=1", &[])
        .expect("http_get should succeed");

    assert_eq!(resp.status, 200);
    let body = resp.body.unwrap();
    assert_eq!(
        body, "rewrite_ok",
        "server should see /test-path?query=1 after rewrite; body={body:?}"
    );

    server.join().unwrap();
}

#[test]
fn http_post_preserves_status() {
    let _lock = lock_test();
    let (listener, port) = bind_port();
    let _guard = EnvGuard::set_rewrite(
        "https://embedded.test",
        &format!("http://127.0.0.1:{port}"),
    );

    let server = spawn_server(listener, |stream| {
        let _req = read_request(stream);
        write_response(stream, 300, "redirect");
    });

    let host = EmbeddedHost::new();
    let resp = host
        .http_post("https://embedded.test/", Some("data"), &[])
        .expect("http_post should succeed");

    assert_eq!(resp.status, 300);
    assert_eq!(resp.body.as_deref(), Some("redirect"));

    server.join().unwrap();
}
