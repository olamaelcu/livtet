//! Test utilities shared across the workspace. Stub: only the
//! minimum surface required by `livtet-plugins`'s HTTP integration
//! tests. Extend as new tests need it.

use std::{collections::HashMap, sync::Arc};

use camino::Utf8PathBuf;
use tokio::sync::Mutex;

/// A tiny in-process HTTP/1.1 server used by the
/// `livtet-plugins::repository::client` integration tests to serve
/// repository metadata over loopback. Returned by `spawn_server`.
pub struct TestServer {
    /// Base URL (e.g. `http://127.0.0.1:54321`) the client should
    /// fetch from.
    pub base_url: String,
    /// Server task handle so tests can shut the server down on
    /// drop (currently a no-op).
    _task: tokio::task::JoinHandle<()>,
}

/// Spawn a single-shot HTTP/1.1 server rooted at `root`. The server
/// reads a request line, parses the path, and serves files from
/// `root/<path>` with `Content-Type` from a small extension table.
/// Responses are framed by `build_response` / `http_response`.
pub async fn spawn_server(root: Utf8PathBuf) -> TestServer {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let base_url = format!("http://{addr}");

    let root = Arc::new(root);
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let root = Arc::clone(&root);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16 * 1024];
                let mut total = 0usize;
                while total < buf.len() {
                    match sock.read(&mut buf[total..]).await {
                        Ok(0) => break,
                        Ok(n) => {
                            total += n;
                            if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                        Err(_) => return,
                    }
                }
                let request = String::from_utf8_lossy(&buf[..total]).into_owned();
                let (status_line, body, content_type) =
                    build_response(&root, parse_request_path(&request));
                let response = format!(
                    "{status_line}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let mut payload = response.into_bytes();
                payload.extend_from_slice(&body);
                let _ = sock.write_all(&payload).await;
                let _ = sock.shutdown().await;
            });
        }
    });

    TestServer {
        base_url,
        _task: task,
    }
}

/// Parse just the request-target out of an HTTP/1.1 request line.
/// Returns `/` when the line is malformed or missing.
pub fn parse_request_path(request: &str) -> String {
    let first_line = request.lines().next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let _method = parts.next();
    parts.next().unwrap_or("/").to_string()
}

/// Build a response for `path` rooted at `root`. Serves files
/// directly; returns 404 for missing paths and 400 for path
/// traversal attempts.
pub fn build_response(
    root: &camino::Utf8Path,
    path: String,
) -> (String, Vec<u8>, &'static str) {
    use camino::Utf8Path;
    let clean = Utf8Path::new(&path);
    if clean.components().any(|c| {
        matches!(c, camino::Utf8Component::ParentDir | camino::Utf8Component::CurDir)
    }) || path.contains("..")
    {
        return ("HTTP/1.1 400 Bad Request".to_string(), Vec::new(), "text/plain");
    }
    let full = root.join(clean.as_str().trim_start_matches('/'));
    match fs_err::read(&full) {
        Ok(bytes) => {
            let content_type = http_response(&full);
            ("HTTP/1.1 200 OK".to_string(), bytes, content_type)
        }
        Err(_) => (
            "HTTP/1.1 404 Not Found".to_string(),
            b"not found".to_vec(),
            "text/plain",
        ),
    }
}

/// Pick a `Content-Type` from a file extension. Used by
/// `build_response` for served files.
pub fn http_response(path: &camino::Utf8Path) -> &'static str {
    let ext: Option<String> = path.extension().map(|s| s.to_string());
    match ext.as_deref() {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("toml") => "application/toml; charset=utf-8",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        Some("sig") => "application/octet-stream",
        Some("ltp") => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

/// Reserved for tests that need a shared mutable HashMap of canned
/// responses. Currently unused — kept for future tests.
#[allow(dead_code)]
pub type TestServerRegistry = Arc<Mutex<HashMap<String, Vec<u8>>>>;
