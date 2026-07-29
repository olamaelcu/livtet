//! Embedded (mobile-friendly) plugin host implementation.
//!
//! HTTP calls run through `reqwest::Client` (async) executed on a
//! dedicated blocking worker thread that owns a private tokio
//! runtime. This isolation matters: the rest of `livtet-ffi` already
//! drives sync FFI handlers through `crate::runtime::block_on(...)`,
//! which leaves a tokio runtime "current" on whatever thread invoked
//! it. `reqwest::blocking`'s internal runtime detects an active
//! tokio runtime via `Handle::try_current()` and panics with
//! "Cannot start a runtime from within a runtime", tearing down the
//! `reqwest-internal-sync-runtime` worker thread and surfacing as
//! "event loop thread panicked" at the call site. Using async
//! `reqwest::Client` with an isolated single-thread runtime avoids
//! the nesting check entirely.
//!
//! Operations that require IPC (secrets, filesystem, DB) return
//! `HostError::Unsupported` — the lookup/search capabilities that
//! bundled plugins exercise do not need them.

use std::sync::Arc;

use super::host_trait::{
    GetEmbeddingResponse, HostBase, HostDatabase, HostEmbeddings, HostError, HostFiles, HostHttp,
    HostHttpResponse, HostLog, HostOAuth, HostSecrets, HostSettings, HostSystemSecrets,
    SimilarEdition, StoreEmbeddingResponse,
};
use crate::system_secrets::PluginSystemSecret;

/// Newtype wrapper for the worker-thread tokio runtime. We store a
/// channel + runtime handle to avoid holding a `Runtime` directly in
/// a `static`, which would prevent the worker thread from being
/// joined deterministically at shutdown on platforms that care.
struct HttpWorker {
    sender: std::sync::mpsc::Sender<HttpJob>,
}

enum HttpJob {
    Get {
        url: String,
        headers: Vec<(String, String)>,
        reply: std::sync::mpsc::Sender<Result<HostHttpResponse, HostError>>,
    },
    Post {
        url: String,
        body: Option<String>,
        headers: Vec<(String, String)>,
        reply: std::sync::mpsc::Sender<Result<HostHttpResponse, HostError>>,
    },
    Put {
        url: String,
        body: Option<String>,
        headers: Vec<(String, String)>,
        reply: std::sync::mpsc::Sender<Result<HostHttpResponse, HostError>>,
    },
    Patch {
        url: String,
        body: Option<String>,
        headers: Vec<(String, String)>,
        reply: std::sync::mpsc::Sender<Result<HostHttpResponse, HostError>>,
    },
    Delete {
        url: String,
        headers: Vec<(String, String)>,
        reply: std::sync::mpsc::Sender<Result<HostHttpResponse, HostError>>,
    },
}

impl HttpWorker {
    fn shared() -> &'static Self {
        use std::sync::OnceLock;
        static WORKER: OnceLock<HttpWorker> = OnceLock::new();
        WORKER.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<HttpJob>();
            std::thread::Builder::new()
                .name("livtet-embedded-http".into())
                .spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("embedded-host http worker: build runtime");
                    // `reqwest::Client::builder().build()` spawns an
                    // internal task to validate its TLS config and start
                    // the connection pool warmup. That needs a current
                    // tokio runtime or it panics. Entering once at boot
                    // makes every subsequent `rt.block_on` (and every
                    // reqwest call inside it) cheap.
                    let _guard = rt.enter();
                    // Build a `rustls::ClientConfig` directly so we
                    // skip `rustls-platform-verifier` entirely. That
                    // crate ships an Android verifier which delegates
                    // to the OS `CertPathValidator`. Android's verifier
                    // enforces OCSP revocation checking and reports
                    // "Certificate does not specify OCSP responder" as
                    // `Revoked` for any leaf whose intermediates ship
                    // only CRLs — which is most public-CA chains
                    // (including Let's Encrypt YR1 and Google WR2).
                    // That breaks HTTPS for any Android device whose
                    // CA store hands back a CRL-only intermediate
                    // chain.
                    //
                    // `webpki-roots 0.26` ships Mozilla's CA set as
                    // `TrustAnchor<'static>` tuples. `rustls`'s
                    // `RootCertStore` has a direct
                    // `FromIterator<TrustAnchor<'static>>` impl, so we
                    // can hand them to it without parsing full X.509
                    // certs. `WebPkiServerVerifier` built on that
                    // store performs chain + signature verification but
                    // **no revocation checking** (no OCSP, no CRL),
                    // which is exactly what we want for talking to
                    // public APIs from a long-running embedded
                    // client.
                    //
                    // We bypass reqwest's verifier-selection logic
                    // (which would otherwise route through
                    // `rustls-platform-verifier`) by feeding the finished
                    // `ClientConfig` to reqwest via
                    // `use_preconfigured_tls`. The `Some` wrap
                    // happens internally — we pass the bare config.
                    //
                    // `ClientConfig::builder()` would default to
                    // `get_default_or_install_from_crate_features`,
                    // which panics on Android with "Could not
                    // automatically determine the process-level
                    // CryptoProvider from Rustls crate features".
                    // `builder_with_provider(...)` plus an explicit
                    // `install_default(...)` before any rustls call
                    // sidesteps that path.
                    let provider = rustls::crypto::aws_lc_rs::default_provider();
                    let _ = rustls::crypto::CryptoProvider::install_default(provider.clone());
                    let provider_arc = Arc::new(provider);
                    let tls_roots: Vec<_> = webpki_roots::TLS_SERVER_ROOTS.to_vec();
                    let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
                        Arc::new(rustls::RootCertStore::from_iter(tls_roots)),
                        provider_arc.clone(),
                    )
                    .build()
                    .expect("webpki verifier builder should not fail with valid Mozilla roots");
                    let verifier: Arc<dyn rustls::client::danger::ServerCertVerifier> = verifier;
                    let client_config = rustls::ClientConfig::builder_with_provider(provider_arc)
                        .with_safe_default_protocol_versions()
                        .expect("default protocol versions are valid")
                        .dangerous()
                        .with_custom_certificate_verifier(verifier)
                        .with_no_client_auth();
                    let client = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(60))
                        .connect_timeout(std::time::Duration::from_secs(60))
                        .use_preconfigured_tls(client_config)
                        .build()
                        .expect("embedded-host http worker: build client");
                    // `_guard` (from `rt.enter()` above) is held for the
                    // entire worker lifetime and dropped naturally when
                    // this closure returns — `reqwest::Client`'s
                    // internal connection-pool tasks need a current
                    // tokio runtime across every `block_on`.
                    loop {
                        // Wrap each job in `catch_unwind` so a panic in
                        // reqwest / tokio doesn't kill the worker for
                        // the rest of the process lifetime. The panic
                        // is forwarded to logcat and the reply channel
                        // is closed with an error so the caller gets
                        // `Err(HostError::Http(...))` instead of
                        // nothing.
                        let job = match rx.recv() {
                            Ok(job) => job,
                            Err(e) => {
                                tracing::error!(error = %e, "rx.recv failed on worker thread");
                                continue;
                            }
                        };
                        let request_url = match &job {
                            HttpJob::Get { url, .. }
                            | HttpJob::Post { url, .. }
                            | HttpJob::Put { url, .. }
                            | HttpJob::Patch { url, .. }
                            | HttpJob::Delete { url, .. } => url.clone(),
                        };
                        tracing::debug!(
                            method = match &job {
                                HttpJob::Get { .. } => "GET",
                                HttpJob::Post { .. } => "POST",
                                HttpJob::Put { .. } => "PUT",
                                HttpJob::Patch { .. } => "PATCH",
                                HttpJob::Delete { .. } => "DELETE",
                            },
                            url = %request_url,
                            "embedded_http request received"
                        );
                        let result =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match job {
                                HttpJob::Get {
                                    url,
                                    headers,
                                    reply,
                                } => {
                                    let _ = reply.send(dispatch_get(&rt, &client, &url, &headers));
                                }
                                HttpJob::Post {
                                    url,
                                    body,
                                    headers,
                                    reply,
                                } => {
                                    let _ = reply
                                        .send(dispatch_post(&rt, &client, &url, body, &headers));
                                }
                                HttpJob::Put {
                                    url,
                                    body,
                                    headers,
                                    reply,
                                } => {
                                    let _ = reply.send(dispatch_with_method(
                                        &rt,
                                        &client,
                                        &url,
                                        reqwest::Method::PUT,
                                        body,
                                        &headers,
                                    ));
                                }
                                HttpJob::Patch {
                                    url,
                                    body,
                                    headers,
                                    reply,
                                } => {
                                    let _ = reply.send(dispatch_with_method(
                                        &rt,
                                        &client,
                                        &url,
                                        reqwest::Method::PATCH,
                                        body,
                                        &headers,
                                    ));
                                }
                                HttpJob::Delete {
                                    url,
                                    headers,
                                    reply,
                                } => {
                                    let _ = reply.send(dispatch_with_method(
                                        &rt,
                                        &client,
                                        &url,
                                        reqwest::Method::DELETE,
                                        None,
                                        &headers,
                                    ));
                                }
                            }));
                        if let Err(panic_payload) = result {
                            tracing::error!(
                                panic = %panic_msg(&panic_payload),
                                "embedded-http worker caught panic; loop continues"
                            );
                        }
                    }
                    // The loop above has no `break` — the worker runs
                    // for the lifetime of the process. `rt` is dropped
                    // by the thread's `Drop` when the thread panics or
                    // the process exits; we never reach here.
                })
                .expect("embedded-host http worker: spawn thread");
            HttpWorker { sender: tx }
        })
    }

    fn run<F>(&self, job: F) -> Result<HostHttpResponse, HostError>
    where
        F: FnOnce(std::sync::mpsc::Sender<Result<HostHttpResponse, HostError>>) -> HttpJob,
    {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let job = job(reply_tx);
        self.sender
            .send(job)
            .map_err(|e| HostError::Http(format!("worker channel: {e}")))?;
        reply_rx
            .recv()
            .map_err(|e| HostError::Http(format!("worker reply: {e}")))?
    }
}

/// Walks an error's `std::error::Error::source()` chain and returns a
/// multi-line string with the full picture.
///
/// `reqwest::Error::Display` only prints the top-level message
/// ("error sending request for url (...)") and drops the cause. That
/// makes diagnosing a slow TLS handshake vs. a connection reset vs.
/// a DNS failure impossible from logcat alone. This helper walks
/// the source chain and produces a string of the form:
///
/// ```text
/// error sending request for url (https://...)
///   Caused by[1]: reqwest::Error { kind: Request, source: hyper::Error(Connect, ConnectError("dns error: ...")) }
///   Caused by[2]: ...deeper cause...
/// ```
///
/// Capped at 10 levels to avoid pathological cycles in custom
/// `Error::source()` impls. `reqwest`'s chain doesn't cycle in
/// practice, but defensive limits are cheap.
fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    let mut depth: usize = 0;
    while let Some(e) = current {
        if depth == 0 {
            let _ = writeln!(out, "{}", e);
        } else {
            let _ = writeln!(out, "  Caused by[{}]: {}", depth, e);
        }
        current = e.source();
        depth += 1;
        if depth > 10 {
            let _ = writeln!(out, "  ... (source chain truncated at depth 10)");
            break;
        }
    }
    out.trim_end().to_string()
}

/// Rewrite a request URL when test interception is configured via
/// environment variables. When `LIVTET_HTTP_REWRITE_FROM` and
/// `LIVTET_HTTP_REWRITE_TO` are both set, any URL that starts with the
/// `FROM` prefix has that prefix replaced with `TO`. This lets
/// integration tests redirect real plugin HTTP calls (e.g.
/// `https://openlibrary.org/search.json`) to a local wiremock server
/// without modifying plugin source.
///
/// Reads the env vars on every call (cheap) so tests can set them
/// before the first HTTP request without worrying about initialization
/// ordering relative to the `HttpWorker` singleton.
fn rewrite_url(url: &str) -> String {
    let from = match std::env::var("LIVTET_HTTP_REWRITE_FROM") {
        Ok(v) if !v.is_empty() => v,
        _ => return url.to_string(),
    };
    let to = match std::env::var("LIVTET_HTTP_REWRITE_TO") {
        Ok(v) if !v.is_empty() => v,
        _ => return url.to_string(),
    };
    if let Some(rest) = url.strip_prefix(&from) {
        let rewritten = if rest.starts_with('/') {
            format!("{}{}", to, rest)
        } else {
            format!("{}/{}", to, rest)
        };
        rewritten
    } else {
        url.to_string()
    }
}

fn dispatch_get(
    rt: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
) -> Result<HostHttpResponse, HostError> {
    let url = rewrite_url(url);
    tracing::debug!(url, "embedded_http dispatch_get enter");
    let mut req = client.get(&url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = rt.block_on(req.send()).map_err(|e| {
        let chain = error_chain(&e);
        tracing::error!(url, chain, "embedded_http dispatch_get failed");
        HostError::Http(chain)
    })?;
    let status = resp.status().as_u16();
    tracing::debug!(status, url, "embedded_http dispatch_get sent");
    let body = rt
        .block_on(resp.text())
        .map_err(|e| {
            let chain = error_chain(&e);
            tracing::error!(url, chain, "embedded_http dispatch_get body read failed");
            HostError::Http(chain)
        })
        .ok();
    if let Some(ref b) = body {
        tracing::debug!(body_len = b.len(), url, "embedded_http dispatch_get body");
    }
    Ok(HostHttpResponse {
        status,
        body,
        headers: vec![],
    })
}

fn dispatch_post(
    rt: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    url: &str,
    body: Option<String>,
    headers: &[(String, String)],
) -> Result<HostHttpResponse, HostError> {
    let url = rewrite_url(url);
    let mut req = client.post(&url);
    if let Some(b) = body {
        req = req.body(b);
    }
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = rt.block_on(req.send()).map_err(|e| {
        let chain = error_chain(&e);
        tracing::error!(url, chain, "embedded_http dispatch_post failed");
        HostError::Http(chain)
    })?;
    let status = resp.status().as_u16();
    let resp_body = rt
        .block_on(resp.text())
        .map_err(|e| {
            let chain = error_chain(&e);
            tracing::error!(url, chain, "embedded_http dispatch_post body read failed");
            HostError::Http(chain)
        })
        .ok();
    Ok(HostHttpResponse {
        status,
        body: resp_body,
        headers: vec![],
    })
}

fn dispatch_with_method(
    rt: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    url: &str,
    method: reqwest::Method,
    body: Option<String>,
    headers: &[(String, String)],
) -> Result<HostHttpResponse, HostError> {
    let url = rewrite_url(url);
    let mut req = client.request(method, &url);
    if let Some(b) = body {
        req = req.body(b);
    }
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = rt
        .block_on(req.send())
        .map_err(|e| HostError::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = rt.block_on(resp.text()).ok();
    Ok(HostHttpResponse {
        status,
        body,
        headers: vec![],
    })
}

fn panic_msg(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Embedded host suitable for use in mobile FFI contexts.
/// Provides HTTP + logging + pure computation host functions.
/// Everything else returns `HostError::Unsupported`.
pub struct EmbeddedHost {
    system_secrets: std::collections::HashMap<PluginSystemSecret, String>,
}

impl EmbeddedHost {
    pub fn new() -> Self {
        Self {
            system_secrets: std::collections::HashMap::new(),
        }
    }

    pub fn with_system_secrets(
        system_secrets: std::collections::HashMap<PluginSystemSecret, String>,
    ) -> Self {
        Self { system_secrets }
    }
}

impl Default for EmbeddedHost {
    fn default() -> Self {
        Self::new()
    }
}

impl HostBase for EmbeddedHost {}

impl HostSystemSecrets for EmbeddedHost {
    fn get_system_secret(&self, name: PluginSystemSecret) -> Option<String> {
        self.system_secrets.get(&name).cloned()
    }
}

impl HostHttp for EmbeddedHost {
    fn http_get(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        tracing::debug!(url, "embedded_http http_get enter");
        let worker = HttpWorker::shared();
        let url = url.to_string();
        let headers = headers.to_vec();
        let result = worker.run(move |reply| HttpJob::Get {
            url,
            headers,
            reply,
        });
        tracing::debug!(ok = result.is_ok(), "embedded_http http_get return");
        result
    }

    fn http_post(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        let worker = HttpWorker::shared();
        let url = url.to_string();
        let body = body.map(str::to_string);
        let headers = headers.to_vec();
        worker.run(|reply| HttpJob::Post {
            url,
            body,
            headers,
            reply,
        })
    }

    fn http_put(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        let worker = HttpWorker::shared();
        let url = url.to_string();
        let body = body.map(str::to_string);
        let headers = headers.to_vec();
        worker.run(|reply| HttpJob::Put {
            url,
            body,
            headers,
            reply,
        })
    }

    fn http_patch(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        let worker = HttpWorker::shared();
        let url = url.to_string();
        let body = body.map(str::to_string);
        let headers = headers.to_vec();
        worker.run(|reply| HttpJob::Patch {
            url,
            body,
            headers,
            reply,
        })
    }

    fn http_delete(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        let worker = HttpWorker::shared();
        let url = url.to_string();
        let headers = headers.to_vec();
        worker.run(|reply| HttpJob::Delete {
            url,
            headers,
            reply,
        })
    }
}

impl HostLog for EmbeddedHost {
    fn log(&self, _plugin_id: &str, level: &str, message: &str) {
        match level {
            "error" => tracing::error!("[plugin] {message}"),
            "warn" => tracing::warn!("[plugin] {message}"),
            "info" => tracing::info!("[plugin] {message}"),
            "debug" => tracing::debug!("[plugin] {message}"),
            _ => tracing::trace!("[plugin] {message}"),
        }
    }
}

impl HostSecrets for EmbeddedHost {
    fn get_secret(&self, _plugin_id: &str, _name: &str) -> Result<Option<String>, HostError> {
        Err(HostError::Unsupported)
    }

    fn set_secret(&self, _plugin_id: &str, _name: &str, _value: &str) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
}

/// OAuth redemption is not yet wired on mobile. iOS / Android
/// implementations will use `ASWebAuthenticationSession` /
/// Chrome Custom Tabs respectively — those land in a follow-up.
impl HostOAuth for EmbeddedHost {
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

impl HostSettings for EmbeddedHost {
    fn get_setting(&self, _plugin_id: &str, _key: &str) -> Option<String> {
        None
    }

    fn set_setting(&self, _plugin_id: &str, _key: &str, _value: &str) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
}

impl HostDatabase for EmbeddedHost {
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
    ) -> Result<Option<crate::progress_entry::ProgressEntry>, HostError> {
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

impl HostEmbeddings for EmbeddedHost {
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

impl HostFiles for EmbeddedHost {
    fn read_file(&self, _path: &str) -> Result<Option<String>, HostError> {
        Err(HostError::Unsupported)
    }

    fn plugin_asset(&self, _plugin_dir: &str, _filename: &str) -> Result<Vec<u8>, HostError> {
        Err(HostError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::system_secrets::PluginSystemSecret;

    #[test]
    fn with_system_secrets_registers_values() {
        let mut map = HashMap::new();
        map.insert(
            PluginSystemSecret::GoogleBooksApiKey,
            "AIza-test".to_string(),
        );
        map.insert(
            PluginSystemSecret::PlatformUnauthenticatedAllowed,
            "true".to_string(),
        );
        let host = EmbeddedHost::with_system_secrets(map);
        assert_eq!(
            host.get_system_secret(PluginSystemSecret::GoogleBooksApiKey),
            Some("AIza-test".to_string()),
        );
        assert_eq!(
            host.get_system_secret(PluginSystemSecret::PlatformUnauthenticatedAllowed),
            Some("true".to_string()),
        );
    }

    #[test]
    fn missing_secret_returns_none_not_panic() {
        let host = EmbeddedHost::new();
        assert!(
            host.get_system_secret(PluginSystemSecret::GoogleBooksApiKey)
                .is_none()
        );
    }
}
