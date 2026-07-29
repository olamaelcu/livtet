//! Host-mock integration tests for the bundled `openlibrary` and
//! `googlebooks` providers.
//!
//! These tests drive the real bundled plugin sources (no IpcHost
//! sidecar, no reqwest) through an in-process `LuaHost<MockHost>`.
//! Every HTTP call the plugin makes is intercepted by `MockHost`,
//! captured into a `Vec<RecordedRequest>`, and answered from a
//! queued `VecDeque<HostHttpResponse>` so the test stays
//! deterministic.
//!
//! Two contracts are pinned:
//!
//! 1. **URL shape** — the URL the plugin builds matches the
//!    documented endpoint for that capability (e.g.
//!    `https://openlibrary.org/search.json?q=…&limit=…`).
//! 2. **Per-method User-Agent** — every outbound call sets
//!    `User-Agent: https://livtet.olamaelcu.net/kb/user-agent#<method>`,
//!    where `<method>` is the capability that initiated the call
//!    (`search`, `lookup`, `cover`, `enrich`, `catalog`,
//!    `series-detect`, `series-order`). The doc anchor doubles
//!    as a per-method behaviour log for the bundled providers.
//!
//! Plugin source is loaded via the Q8-decided path: the
//! `livtet-lua-plugins` crate's `embedded_index()` + `read_entry()`,
//! with the manifest's `plugin.entry` resolved at runtime so a
//! rename of the bundled directory is followed automatically.

use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use globset::GlobSetBuilder;
use livtet_plugins::{
    host_lua::LuaHost,
    host_trait::{
        GetEmbeddingResponse, HostBase, HostDatabase, HostEmbeddings, HostError, HostFiles,
        HostHttp, HostHttpResponse, HostLog, HostOAuth, HostSecrets, HostSettings,
        HostSystemSecrets, SimilarEdition, StoreEmbeddingResponse,
    },
    manifest::PluginManifest,
    permissions::{PluginGrant, ResolvedGrant},
    protocol::HostToMain,
    system_secrets::PluginSystemSecret,
};
use serde_json::Value;

/// Canonical User-Agent base the bundled plugins use. The
/// fragment suffix is the per-method capability that initiated
/// the call (search / lookup / cover / enrich / catalog /
/// series-detect / series-order). Tests pin the full string so
/// any regression that drops the fragment or shortens the base
/// surfaces here.
const USER_AGENT_BASE: &str = "https://livtet.olamaelcu.net/kb/user-agent";

/// A single intercepted HTTP request.
#[derive(Debug, Clone)]
#[allow(dead_code)] // `body` is captured for future per-method POST assertions.
struct RecordedRequest {
    method: HttpMethod,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// Test host that records every inbound call to `http_get` /
/// `http_post` and answers from a queued response list. The
/// non-HTTP traits are stubbed to `Ok(None)` / `Err(Unsupported)`
/// because the bundled `search` / `lookup` / `get_cover`
/// capabilities don't touch them. Mirrors the shape of
/// `EmbeddedHost` (see `crates/livtet-plugins/src/embedded_host.rs`).
struct MockHost {
    inner: Mutex<MockHostInner>,
}

struct MockHostInner {
    requests: Vec<RecordedRequest>,
    queued: VecDeque<HostHttpResponse>,
    system_secrets:
        std::collections::HashMap<livtet_plugins::system_secrets::PluginSystemSecret, String>,
}

impl MockHost {
    fn new() -> Self {
        Self {
            inner: Mutex::new(MockHostInner {
                requests: Vec::new(),
                queued: VecDeque::new(),
                system_secrets: std::collections::HashMap::new(),
            }),
        }
    }

    fn set_system_secret(&self, name: PluginSystemSecret, value: String) {
        let mut g = self.inner.lock().expect("mock host poisoned");
        g.system_secrets.insert(name, value);
    }

    /// Queue a plain response.
    fn push(&self, status: u16, body: &str, headers: Vec<(String, String)>) {
        let mut g = self.inner.lock().expect("mock host poisoned");
        g.queued.push_back(HostHttpResponse {
            status,
            body: Some(body.to_string()),
            headers,
        });
    }

    /// Queue a JSON response with `Content-Type: application/json`.
    fn push_json(&self, status: u16, body: &str) {
        self.push(
            status,
            body,
            vec![("Content-Type".to_string(), "application/json".to_string())],
        );
    }

    fn dequeue(&self) -> Option<HostHttpResponse> {
        let mut g = self.inner.lock().expect("mock host poisoned");
        g.queued.pop_front()
    }

    fn record(&self, req: RecordedRequest) {
        let mut g = self.inner.lock().expect("mock host poisoned");
        g.requests.push(req);
    }

    /// Snapshot of every recorded request. Clone-friendly so
    /// each assertion can read without holding the lock.
    fn requests(&self) -> Vec<RecordedRequest> {
        let g = self.inner.lock().expect("mock host poisoned");
        g.requests.clone()
    }
}

impl HostBase for MockHost {}
impl HostSystemSecrets for MockHost {
    fn get_system_secret(&self, name: PluginSystemSecret) -> Option<String> {
        let g = self.inner.lock().expect("mock host poisoned");
        g.system_secrets.get(&name).cloned()
    }
}

impl HostHttp for MockHost {
    fn http_get(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        self.record(RecordedRequest {
            method: HttpMethod::Get,
            url: url.to_string(),
            headers: headers.to_vec(),
            body: None,
        });
        self.dequeue()
            .ok_or_else(|| HostError::Message("MockHost: no queued GET response".to_string()))
    }

    fn http_post(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        self.record(RecordedRequest {
            method: HttpMethod::Post,
            url: url.to_string(),
            headers: headers.to_vec(),
            body: body.map(str::to_string),
        });
        self.dequeue()
            .ok_or_else(|| HostError::Message("MockHost: no queued POST response".to_string()))
    }

    fn http_put(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        self.record(RecordedRequest {
            method: HttpMethod::Put,
            url: url.to_string(),
            headers: headers.to_vec(),
            body: body.map(str::to_string),
        });
        self.dequeue()
            .ok_or_else(|| HostError::Message("MockHost: no queued PUT response".to_string()))
    }

    fn http_patch(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        self.record(RecordedRequest {
            method: HttpMethod::Patch,
            url: url.to_string(),
            headers: headers.to_vec(),
            body: body.map(str::to_string),
        });
        self.dequeue()
            .ok_or_else(|| HostError::Message("MockHost: no queued PATCH response".to_string()))
    }

    fn http_delete(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        self.record(RecordedRequest {
            method: HttpMethod::Delete,
            url: url.to_string(),
            headers: headers.to_vec(),
            body: None,
        });
        self.dequeue()
            .ok_or_else(|| HostError::Message("MockHost: no queued DELETE response".to_string()))
    }
}

impl HostLog for MockHost {
    fn log(&self, _plugin_id: &str, _level: &str, _message: &str) {
        // Drop plugin log lines on the floor. The tests don't
        // assert on them; surfacing them via `tracing::*!` would
        // add noise to `cargo test` output.
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

/// OAuth redemption is not exercised by these tests. Returning
/// `Unsupported` keeps `MockHost` plug-compatible with
/// `LuaHost<MockHost>` without forcing every test to wire a fake
/// OAuth provider.
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

/// Assert that the recorded request carried a `User-Agent` header
/// matching `https://livtet.olamaelcu.net/kb/user-agent#<method>`.
/// Lookups are case-insensitive on the header name (HTTP/1.1
/// headers are case-insensitive; reqwest normalises them and some
/// intermediaries pass them through verbatim).
fn assert_user_agent_for(req: &RecordedRequest, method: &str) {
    let expected = format!("{USER_AGENT_BASE}#{method}");
    let found = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("User-Agent"));
    let actual = found.map(|(_, v)| v.as_str()).unwrap_or_else(|| {
        panic!(
            "no User-Agent header in request to {}: headers={:?}",
            req.url, req.headers
        )
    });
    assert_eq!(
        actual, expected,
        "User-Agent for {} should be exactly {expected:?}, got {actual:?}",
        req.url
    );
}

/// TBD: bundled plugin source loading was removed when the
/// `livtet-lua-plugins` crate was deleted. The tests in this file
/// that depend on it are skipped at compile time until the
/// bundling pipeline lands. Restore `load_bundled_source` and the
/// `embedded_index` lookup once a replacement crate is in place.
fn load_bundled_source(_plugin_id: &str) -> String {
    panic!("TBD: bundled plugin source loader removed alongside livtet-lua-plugins");
}

fn make_host_typed() -> (Arc<MockHost>, LuaHost<MockHost>) {
    let mock = Arc::new(MockHost::new());
    let host = LuaHost::new(Arc::clone(&mock) as Arc<MockHost>).expect("LuaHost::new");
    (mock, host)
}

/// Pre-populate the plugin grant cache so `host.get_system_secret`
/// passes Gate 2 (the sidecar allowlist). Without this, real hosts
/// fall back to `missing_sidecar_error` and the plugin sees `nil`,
/// which would surface in our code as a `__livtet_error`.
fn grant_system_secret(host: &LuaHost<MockHost>, plugin_id: &str, secret: PluginSystemSecret) {
    let grant = std::sync::Arc::new(ResolvedGrant {
        raw: PluginGrant {
            version: 1,
            read_paths: Vec::new(),
            sqlite_paths: Vec::new(),
            allow_writes: false,
            write_paths: Vec::new(),
            system_secrets: vec![secret.as_ref().to_string()],
            embeddings: false,
            oauth_providers: Vec::new(),
            http_proxy_url: None,
        },
        read_paths: GlobSetBuilder::new().build().expect("empty globset"),
        sqlite_paths: GlobSetBuilder::new().build().expect("empty globset"),
        write_paths: GlobSetBuilder::new().build().expect("empty globset"),
        system_secrets: HashSet::from([secret]),
        embeddings: false,
        oauth_providers: std::collections::HashMap::new(),
        http_proxy_url: None,
    });
    host.grant_plugin(plugin_id, grant).expect("grant_plugin");
}

/// `openlibrary.search` hits `/search.json?q=...&limit=...&page=...&fields=...`,
/// decodes the response, and projects each `docs[]` entry through
/// `build_hit_from_search_doc`. We assert both the URL shape and
/// the per-method User-Agent fragment.
#[test]
fn openlibrary_search_uses_mocked_http() {
    let (mock, mut host) = make_host_typed();

    // The plugin's docs[] carries fields the projection needs:
    // key, title, edition_key, isbn, first_publish_year, cover_i.
    let response = r#"{
        "numFound": 1,
        "docs": [
            {
                "key": "/works/OL45804W",
                "title": "The Left Hand of Darkness",
                "author_name": ["Ursula K. Le Guin"],
                "edition_key": ["OL24250212M"],
                "isbn": ["9780441478125"],
                "first_publish_year": 1969,
                "cover_i": 8231856
            }
        ]
    }"#;
    mock.push_json(200, response);

    let source = load_bundled_source("openlibrary");
    match host.load_plugin_source("openlibrary", &source, None, None) {
        HostToMain::PluginLoaded { .. } => {}
        other => panic!("openlibrary load_plugin_source: {other:?}"),
    }

    let result = host.call_capability(
        "call-1",
        "openlibrary",
        "search",
        &[
            Value::String("foo".to_string()),
            Value::Object(Default::default()),
        ],
    );
    let call = match result {
        HostToMain::CallResult {
            ok: true,
            value: Some(v),
            ..
        } => v,
        other => panic!("openlibrary search: {other:?}"),
    };
    let hits = call
        .as_array()
        .unwrap_or_else(|| panic!("openlibrary search: expected array, got {call:?}"));
    assert_eq!(hits.len(), 1, "expected one hit; got {hits:?}");
    let hit = &hits[0];
    assert_eq!(hit["title"], "The Left Hand of Darkness");
    assert_eq!(hit["source"], "openlibrary");

    // The plugin set `provider.search`'s default limit to 20, so
    // we expect `limit=20` (not the `10` from the user-supplied
    // options table; the plugin caps at MAX_SEARCH_LIMIT = 100
    // but uses the default when the value is missing). Either
    // way the URL should be a well-formed /search.json with the
    // query string.
    let reqs = mock.requests();
    assert_eq!(
        reqs.len(),
        1,
        "expected exactly one HTTP call; got {reqs:?}"
    );
    let req = &reqs[0];
    assert_eq!(req.method, HttpMethod::Get, "search uses GET; got {req:?}");
    assert!(
        req.url.starts_with("https://openlibrary.org/search.json?"),
        "URL should hit /search.json; got {}",
        req.url
    );
    assert!(
        req.url.contains("q=foo"),
        "URL should encode query 'foo'; got {}",
        req.url
    );
    assert!(
        req.url.contains("limit="),
        "URL should include limit; got {}",
        req.url
    );
    assert_user_agent_for(req, "search");
}

/// `openlibrary.lookup` accepts `urn:isbn:...`, `urn:openlibrary:/books/...`,
/// and `urn:openlibrary:/works/...`. The ISBN branch hits
/// `/isbn/<isbn>.json`; the work OLID branch hits `/works/<id>.json`.
/// We exercise the ISBN branch and assert the per-method
/// User-Agent fragment.
#[test]
fn openlibrary_lookup_uses_mocked_http() {
    let (mock, mut host) = make_host_typed();
    let isbn = "9780000000000";

    // The plugin makes 1–2 calls on lookup: first the edition
    // fetch at `/isbn/<isbn>.json`, then optionally a work fetch
    // if the edition lacks authors. Provide one edition response
    // (with authors) so the test only sees one call.
    let edition = r#"{
        "key": "/books/OL24250212M",
        "title": "The Left Hand of Darkness",
        "publish_date": "1969",
        "publishers": ["Ace"],
        "isbn_10": ["0441478123"],
        "isbn_13": ["9780441478125"],
        "number_of_pages": 304,
        "authors": [{"key": "/authors/OL26320A", "name": "Ursula K. Le Guin"}],
        "works": [{"key": "/works/OL45804W"}]
    }"#;
    mock.push_json(200, edition);

    let source = load_bundled_source("openlibrary");
    match host.load_plugin_source("openlibrary", &source, None, None) {
        HostToMain::PluginLoaded { .. } => {}
        other => panic!("openlibrary load: {other:?}"),
    }

    let result = host.call_capability(
        "call-lookup-1",
        "openlibrary",
        "lookup",
        &[Value::String(format!("urn:isbn:{isbn}"))],
    );
    let value = match result {
        HostToMain::CallResult {
            ok: true,
            value: Some(v),
            ..
        } => v,
        other => panic!("openlibrary lookup: {other:?}"),
    };
    assert!(!value.is_null(), "expected a hit object, got null");
    let hit = value
        .as_object()
        .expect("openlibrary lookup should return an object");
    assert_eq!(hit["title"], "The Left Hand of Darkness");
    assert_eq!(hit["source"], "openlibrary");
    let identifiers = hit["identifiers"]
        .as_array()
        .expect("identifiers should be an array");
    assert!(
        identifiers
            .iter()
            .any(|v| v.as_str() == Some("urn:isbn:9780441478125")),
        "expected ISBN-13 URN in identifiers; got {identifiers:?}"
    );

    let reqs = mock.requests();
    assert_eq!(
        reqs.len(),
        1,
        "expected exactly one HTTP call; got {reqs:?}"
    );
    let req = &reqs[0];
    assert_eq!(req.method, HttpMethod::Get);
    assert!(
        req.url.contains(&format!("/isbn/{isbn}.json")),
        "URL should hit /isbn/{isbn}.json; got {}",
        req.url
    );
    assert_user_agent_for(req, "lookup");
}

/// `openlibrary.get_cover` takes edition-level ISBN and returns
/// the canonical `covers.openlibrary.org/b/isbn/<isbn>-L.jpg` URL.
/// We queue an edition response with no `covers[]`, which forces
/// the fallback path: the plugin returns the ISBN-keyed cover URL
/// directly. (When `covers[]` is present, the plugin prefers
/// `cover_url_for_id` over the ISBN-keyed form, which is the
/// other branch; we don't pin it here so the test stays robust
/// against future re-ordering of the `cover_url_for_*` helpers.)
#[test]
fn openlibrary_get_cover_uses_mocked_http() {
    let (mock, mut host) = make_host_typed();
    let isbn = "9780441478125";

    let edition = r#"{
        "key": "/books/OL24250212M",
        "title": "The Left Hand of Darkness",
        "isbn_13": ["9780441478125"]
    }"#;
    mock.push_json(200, edition);

    let source = load_bundled_source("openlibrary");
    match host.load_plugin_source("openlibrary", &source, None, None) {
        HostToMain::PluginLoaded { .. } => {}
        other => panic!("openlibrary load: {other:?}"),
    }

    // get_cover signature: provider.get_cover(work_info, edition_info).
    // The plugin reads `edition_info.isbn` first.
    let result = host.call_capability(
        "call-cover-1",
        "openlibrary",
        "get_cover",
        &[
            Value::Object(Default::default()),
            Value::Object({
                let mut m = serde_json::Map::new();
                m.insert("isbn".to_string(), Value::String(isbn.to_string()));
                m
            }),
        ],
    );
    let value = match result {
        HostToMain::CallResult {
            ok: true,
            value: Some(v),
            ..
        } => v,
        other => panic!("openlibrary get_cover: {other:?}"),
    };
    let url = value
        .as_object()
        .and_then(|o| o.get("url"))
        .and_then(|v| v.as_str())
        .expect("get_cover should return { url = '...' }");
    assert!(
        url.contains(&format!("/b/isbn/{isbn}-L.jpg")),
        "expected /b/isbn/{isbn}-L.jpg in {url}"
    );

    let reqs = mock.requests();
    assert_eq!(
        reqs.len(),
        1,
        "expected exactly one HTTP call; got {reqs:?}"
    );
    assert!(
        reqs[0].url.contains(&format!("/isbn/{isbn}.json")),
        "URL should hit /isbn/{isbn}.json; got {}",
        reqs[0].url
    );
    assert_user_agent_for(&reqs[0], "cover");
}

/// `googlebooks.search` hits `/volumes?q=...&maxResults=...&printType=books&projection=lite`
/// and projects each `items[]` entry through `build_hit`. We
/// assert the URL shape (q is URL-encoded, maxResults is 10, the
/// printType/projection flags are present) and the per-method
/// User-Agent fragment.
#[test]
fn googlebooks_search_uses_mocked_http() {
    let (mock, mut host) = make_host_typed();

    // The plugin reads `item.id`, `item.volumeInfo.{title,
    // subtitle, authors, publisher, publishedDate, imageLinks,
    // language, description, industryIdentifiers}`.
    let response = r#"{
        "totalItems": 1,
        "items": [
            {
                "id": "abc123",
                "volumeInfo": {
                    "title": "The Pragmatic Programmer",
                    "authors": ["Andrew Hunt", "David Thomas"],
                    "publisher": "Addison-Wesley",
                    "publishedDate": "1999-10-30",
                    "imageLinks": {
                        "thumbnail": "http://books.google.com/img/pragprog.jpg"
                    },
                    "language": "en",
                    "industryIdentifiers": [
                        {"type": "ISBN_10", "identifier": "020161622X"},
                        {"type": "ISBN_13", "identifier": "9780201616224"}
                    ]
                }
            }
        ]
    }"#;
    mock.push_json(200, response);

    // Google Books now REQUIRES an API key — the unauthenticated path
    // is rate-limited within minutes. Tests must inject one. The
    // secret AND grant allowlist must be set BEFORE `load_plugin_source`
    // because `init.lua` reads the key at module top-level, and the
    // host's two-gate check (Gate 1: declare, Gate 2: grant) runs
    // synchronously on each `host.get_system_secret` call.
    mock.set_system_secret(
        PluginSystemSecret::GoogleBooksApiKey,
        "AIzaSyTEST_KEY_FOR_MOCK".to_string(),
    );
    host.declare_system_secrets("googlebooks", true);
    grant_system_secret(&host, "googlebooks", PluginSystemSecret::GoogleBooksApiKey);

    let source = load_bundled_source("googlebooks");
    match host.load_plugin_source("googlebooks", &source, None, None) {
        HostToMain::PluginLoaded { .. } => {}
        other => panic!("googlebooks load: {other:?}"),
    }

    let result = host.call_capability(
        "call-gb-search-1",
        "googlebooks",
        "search",
        &[
            Value::String("hello world".to_string()),
            Value::Object(Default::default()),
        ],
    );
    let call = match result {
        HostToMain::CallResult {
            ok: true,
            value: Some(v),
            ..
        } => v,
        other => panic!("googlebooks search: {other:?}"),
    };
    let hits = call
        .as_array()
        .unwrap_or_else(|| panic!("googlebooks search: expected array, got {call:?}"));
    assert_eq!(hits.len(), 1, "expected one hit; got {hits:?}");
    let hit = &hits[0];
    assert_eq!(hit["title"], "The Pragmatic Programmer");
    assert_eq!(hit["source"], "googlebooks");
    assert_eq!(
        hit["cover_url"], "https://books.google.com/img/pragprog.jpg",
        "imageLinks.thumbnail should be http→https rewritten"
    );
    let identifiers = hit["identifiers"]
        .as_array()
        .expect("identifiers should be an array");
    assert!(
        identifiers
            .iter()
            .any(|v| v.as_str() == Some("urn:googlebooks:abc123")),
        "expected googlebooks volume-id URN; got {identifiers:?}"
    );
    assert!(
        identifiers
            .iter()
            .any(|v| v.as_str() == Some("urn:isbn:9780201616224")),
        "expected ISBN-13 URN; got {identifiers:?}"
    );

    let reqs = mock.requests();
    assert_eq!(
        reqs.len(),
        1,
        "expected exactly one HTTP call; got {reqs:?}"
    );
    let req = &reqs[0];
    assert_eq!(req.method, HttpMethod::Get);
    assert!(
        req.url
            .starts_with("https://www.googleapis.com/books/v1/volumes?"),
        "URL should hit /books/v1/volumes; got {}",
        req.url
    );
    // The plugin URL-encodes the query. Either form (`hello%20world`
    // or `hello+world`) is acceptable; mlua's `host.url_encode`
    // uses RFC 3986 percent-encoding so we get `hello%20world`.
    assert!(
        req.url.contains("q=hello%20world") || req.url.contains("q=hello+world"),
        "URL should encode query 'hello world'; got {}",
        req.url
    );
    assert!(
        req.url.contains("maxResults=10"),
        "maxResults=10 expected; got {}",
        req.url
    );
    assert!(
        req.url.contains("printType=books"),
        "printType=books expected; got {}",
        req.url
    );
    assert!(
        req.url.contains("projection=lite"),
        "projection=lite expected; got {}",
        req.url
    );
    assert_user_agent_for(req, "search");
}

/// `googlebooks.lookup` accepts `urn:googlebooks:<volumeId>` and
/// hits `/volumes/<volumeId>`. We assert the URL ends with the
/// volume id and the per-method User-Agent fragment.
#[test]
fn googlebooks_lookup_uses_mocked_http() {
    let (mock, mut host) = make_host_typed();

    let response = r#"{
        "id": "abc123",
        "volumeInfo": {
            "title": "The Pragmatic Programmer",
            "authors": ["Andrew Hunt", "David Thomas"],
            "publisher": "Addison-Wesley",
            "publishedDate": "1999-10-30",
            "imageLinks": {
                "thumbnail": "http://books.google.com/img/pragprog.jpg"
            },
            "language": "en"
        }
    }"#;
    mock.push_json(200, response);

    // Google Books now REQUIRES an API key — see search test.
    mock.set_system_secret(
        PluginSystemSecret::GoogleBooksApiKey,
        "AIzaSyTEST_KEY_FOR_MOCK".to_string(),
    );
    host.declare_system_secrets("googlebooks", true);
    grant_system_secret(&host, "googlebooks", PluginSystemSecret::GoogleBooksApiKey);

    let source = load_bundled_source("googlebooks");
    match host.load_plugin_source("googlebooks", &source, None, None) {
        HostToMain::PluginLoaded { .. } => {}
        other => panic!("googlebooks load: {other:?}"),
    }

    let result = host.call_capability(
        "call-gb-lookup-1",
        "googlebooks",
        "lookup",
        &[Value::String("urn:googlebooks:abc123".to_string())],
    );
    let value = match result {
        HostToMain::CallResult {
            ok: true,
            value: Some(v),
            ..
        } => v,
        other => panic!("googlebooks lookup: {other:?}"),
    };
    assert!(!value.is_null(), "expected a hit object, got null");
    let hit = value
        .as_object()
        .expect("googlebooks lookup should return an object");
    assert_eq!(hit["title"], "The Pragmatic Programmer");
    assert_eq!(hit["source"], "googlebooks");

    let reqs = mock.requests();
    assert_eq!(
        reqs.len(),
        1,
        "expected exactly one HTTP call; got {reqs:?}"
    );
    let req = &reqs[0];
    assert_eq!(req.method, HttpMethod::Get);
    // The URL now carries the API key — Google Books requires auth.
    assert!(
        req.url
            .starts_with("https://www.googleapis.com/books/v1/volumes/abc123?key="),
        "URL should hit /books/v1/volumes/abc123?key=<API_KEY>; got {}",
        req.url
    );
    assert_user_agent_for(req, "lookup");
}

/// `googlebooks.get_cover` requires a `urn:googlebooks:<id>` in
/// the work-level identifiers, hits `/volumes/<id>`, and returns
/// the (https-rewritten) `imageLinks.thumbnail` URL. We queue
/// the volume response and assert the URL.
#[test]
fn googlebooks_get_cover_uses_mocked_http() {
    let (mock, mut host) = make_host_typed();

    let response = r#"{
        "id": "abc123",
        "volumeInfo": {
            "title": "The Pragmatic Programmer",
            "imageLinks": {
                "thumbnail": "http://books.google.com/img/pragprog.jpg"
            }
        }
    }"#;
    mock.push_json(200, response);

    // Google Books now REQUIRES an API key — see search test.
    mock.set_system_secret(
        PluginSystemSecret::GoogleBooksApiKey,
        "AIzaSyTEST_KEY_FOR_MOCK".to_string(),
    );
    host.declare_system_secrets("googlebooks", true);
    grant_system_secret(&host, "googlebooks", PluginSystemSecret::GoogleBooksApiKey);

    let source = load_bundled_source("googlebooks");
    match host.load_plugin_source("googlebooks", &source, None, None) {
        HostToMain::PluginLoaded { .. } => {}
        other => panic!("googlebooks load: {other:?}"),
    }

    let result = host.call_capability(
        "call-gb-cover-1",
        "googlebooks",
        "get_cover",
        &[
            Value::Object({
                let mut m = serde_json::Map::new();
                m.insert(
                    "identifiers".to_string(),
                    Value::Array(vec![Value::String("urn:googlebooks:abc123".to_string())]),
                );
                m
            }),
            Value::Object(Default::default()),
        ],
    );
    let value = match result {
        HostToMain::CallResult {
            ok: true,
            value: Some(v),
            ..
        } => v,
        other => panic!("googlebooks get_cover: {other:?}"),
    };
    let url = value
        .as_object()
        .and_then(|o| o.get("url"))
        .and_then(|v| v.as_str())
        .expect("get_cover should return { url = '...' }");
    assert_eq!(url, "https://books.google.com/img/pragprog.jpg");

    let reqs = mock.requests();
    assert_eq!(
        reqs.len(),
        1,
        "expected exactly one HTTP call; got {reqs:?}"
    );
    assert!(
        reqs[0]
            .url
            .starts_with("https://www.googleapis.com/books/v1/volumes/abc123?key="),
        "URL should hit /books/v1/volumes/abc123?key=<API_KEY>; got {}",
        reqs[0].url
    );
    assert_user_agent_for(&reqs[0], "cover");
}
