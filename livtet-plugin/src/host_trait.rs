//! Trait definitions for the plugin host function surface.
//!
//! Each host function that a Lua plugin can call via `host.*` is
//! represented as a method on one of the traits below. The traits are
//! intentionally fine-grained so that different platforms can implement
//! only the capabilities they support:
//!
//! - **Desktop** (`IpcHost`): all traits via the sidecar IPC protocol.
//! - **Mobile** (`EmbeddedHost`): `HostBase` + `HostHttp` + `HostLog`.
//!   Everything else returns `HostError::Unsupported`.

use thiserror::Error;

use crate::system_secrets::PluginSystemSecret;

/// Error type returned by host operations.
#[derive(Debug, Error)]
pub enum HostError {
    #[error("{0}")]
    Message(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("operation not supported on this platform")]
    Unsupported,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    /// The plugin called a `host.oauth_*` function for a provider it
    /// has not been granted access to. The frontend detects this
    /// variant and surfaces a "Grant access" prompt to the user.
    /// The Tauri main process's `OAuthBroker` resolves it by running
    /// the PKCE flow; the Lua host retries the call once the user
    /// has authorized.
    #[error(
        "plugin {plugin_id} has not authorized OAuth provider {provider}; prompt the user via /settings/plugins/{plugin_id}"
    )]
    NeedsAuth { plugin_id: String, provider: String },
}

/// HTTP response returned by `HostHttp` methods.
pub struct HostHttpResponse {
    pub status: u16,
    pub body: Option<String>,
    pub headers: Vec<(String, String)>,
}

// ── Traits ─────────────────────────────────────────────────────────────────

/// Pure-computation host functions that any implementation can provide.
/// These have no I/O and no platform-specific behavior.
pub trait HostBase: Send + Sync {
    /// Build a canonical URN string: `urn:{ns}:{value}`.
    /// Returns an error if `ns` is empty or contains characters
    /// outside `[%w_-]`.
    fn build_urn(&self, ns: &str, value: &str) -> Result<String, HostError> {
        validate_urn_scheme(ns)?;
        if value.is_empty() {
            return Err(HostError::Message(
                "host.urn: value must not be empty".to_string(),
            ));
        }
        Ok(format!("urn:{ns}:{value}"))
    }

    /// Percent-encode a string per RFC 3986.
    fn url_encode(&self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.as_bytes() {
            match *b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(*b as char);
                }
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    /// Percent-decode a string.
    fn url_decode(&self, s: &str) -> Result<String, HostError> {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .map_err(|e| HostError::Message(format!("url_decode utf8: {e}")))?;
                let v = u8::from_str_radix(hex, 16)
                    .map_err(|e| HostError::Message(format!("url_decode hex: {e}")))?;
                out.push(v);
                i += 3;
            } else if bytes[i] == b'+' {
                out.push(b' ');
                i += 1;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out)
            .map_err(|e| HostError::Message(format!("url_decode utf8 result: {e}")))
    }

    /// Decode a JSON string into a `serde_json::Value`.
    fn json_decode(&self, s: &str) -> Result<serde_json::Value, HostError> {
        Ok(serde_json::from_str(s)?)
    }

    /// Encode a `serde_json::Value` to a JSON string.
    fn json_encode(&self, v: &serde_json::Value) -> Result<String, HostError> {
        Ok(serde_json::to_string(v)?)
    }

    /// Strip HTML tags, decode common entities, and collapse whitespace.
    fn html_strip(&self, html: &str) -> String {
        strip_html_to_text(html)
    }
}

/// HTTP transport: `host.http_get` and `host.http_post`.
pub trait HostHttp: HostBase {
    fn http_get(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError>;
    fn http_post(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError>;
    /// PUT with optional body. Same shape as `http_post`.
    fn http_put(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError>;
    /// PATCH with optional body. Same shape as `http_post`.
    fn http_patch(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError>;
    /// DELETE. No body.
    fn http_delete(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError>;
}

/// Diagnostics / logging: `host.log`.
pub trait HostLog: HostBase {
    fn log(&self, plugin_id: &str, level: &str, message: &str);
}

/// Secret store: `host.get_secret` and `host.set_secret`.
pub trait HostSecrets: HostBase {
    fn get_secret(&self, plugin_id: &str, name: &str) -> Result<Option<String>, HostError>;
    fn set_secret(&self, plugin_id: &str, name: &str, value: &str) -> Result<(), HostError>;
}

/// Per-plugin settings: `host.get_setting` and `host.set_setting`.
pub trait HostSettings: HostBase {
    fn get_setting(&self, plugin_id: &str, key: &str) -> Option<String>;
    fn set_setting(&self, plugin_id: &str, key: &str, value: &str) -> Result<(), HostError>;
}

/// Compile-time, host-owned secrets.
///
/// The host holds the values for these names; the plugin never writes
/// them. They are populated at boot from a project-managed SOPS
/// bundle (see ADR 0032) and surfaced here as `Option<String>`. A
/// `None` is the canonical "no value registered" — plugins should
/// treat that as a missing-credential signal and surface a
/// `needs_auth` error rather than silently falling back.
///
/// Default returns `None` so tests and new host implementations can
/// be wired up incrementally.
pub trait HostSystemSecrets: HostBase {
    fn get_system_secret(&self, name: PluginSystemSecret) -> Option<String> {
        let _ = name;
        None
    }
}

/// Database access: identifier resolution, edition metadata, SQLite queries, progress.
pub trait HostDatabase: HostBase {
    fn resolve_identifier(&self, urn: &str) -> Result<Option<String>, HostError>;
    fn resolve_identifiers(&self, urns: &[String]) -> Result<Vec<Option<String>>, HostError>;
    fn get_edition_info(&self, edition_id: &str) -> Result<Option<serde_json::Value>, HostError>;
    fn get_edition_identifiers(&self, edition_id: &str) -> Result<Vec<String>, HostError>;
    fn fetch_progress(
        &self,
        urn: &str,
    ) -> Result<Option<crate::progress_entry::ProgressEntry>, HostError>;
    fn upsert_progress(
        &self,
        urn: &str,
        progress: f64,
        last_location: Option<String>,
        total_reading_time_secs: i64,
    ) -> Result<serde_json::Value, HostError>;
}

/// Response from a successful embedding store operation.
#[derive(Debug, Clone)]
pub struct StoreEmbeddingResponse {
    pub row_id: String,
    pub dimensions: usize,
}

/// Response from a successful embedding retrieval.
#[derive(Debug, Clone)]
pub struct GetEmbeddingResponse {
    pub vector: Vec<u8>,
    pub model: String,
}

/// A single similarity result — an edition ID paired with its cosine-similarity score.
#[derive(Debug, Clone)]
pub struct SimilarEdition {
    pub edition_id: String,
    pub score: f32,
}

/// Vector embedding storage, retrieval, and similarity search.
///
/// Plugins use these to store embeddings computed from edition metadata,
/// retrieve previously stored vectors, and find similar editions using
/// cosine similarity on the device-local SQLite store.
pub trait HostEmbeddings: HostBase {
    /// Store an embedding vector for an edition.
    ///
    /// `vector_bytes` must be a multiple of 4 (packed f32 elements).
    fn store_embedding(
        &self,
        edition_id: &str,
        model: &str,
        vector_bytes: &[u8],
    ) -> Result<StoreEmbeddingResponse, HostError>;

    /// Retrieve a stored embedding vector for an edition.
    fn get_embedding(
        &self,
        edition_id: &str,
        model: &str,
    ) -> Result<Option<GetEmbeddingResponse>, HostError>;

    /// Find editions with embedding vectors similar to `query_vector`.
    ///
    /// Returns at most `limit` results sorted by descending cosine similarity.
    fn find_similar_editions(
        &self,
        query_vector: &[u8],
        model: &str,
        limit: usize,
    ) -> Result<Vec<SimilarEdition>, HostError>;
}

/// Filesystem access (grant-gated).
pub trait HostFiles: HostBase {
    fn read_file(&self, path: &str) -> Result<Option<String>, HostError>;
    fn plugin_asset(&self, plugin_dir: &str, filename: &str) -> Result<Vec<u8>, HostError>;
}

/// OAuth redemption flow.
///
/// The plugin calls `redeem_token` to obtain a long-lived token for
/// a third-party provider (e.g. the user's `livtet.olamaelcu.net`
/// account). The host runs the PKCE flow, stores the resulting
/// grant + refresh token in its secure storage, and returns an
/// access token to the plugin.
///
/// `get_valid_token` returns a currently valid access token,
/// transparently refreshing when the stored one is within 60s of
/// expiry. The plugin never sees the refresh token — only the
/// access token leaves the host.
///
/// `revoke_token` deletes the stored grant and clears any cached
/// access token.
///
/// Implementations:
/// - Desktop (`IpcHost`): runs the full PKCE flow against
///   `livtet.olamaelcu.net` via `tauri-plugin-oauth`, stores tokens
///   in the OS keychain under `net.olamaelcu.livtet` /
///   `<plugin_id>:oauth:<provider>`.
/// - Mobile (`EmbeddedHost`): returns `HostError::Unsupported` for
///   v1; future iOS / Android implementations will use
///   `ASWebAuthenticationSession` / Chrome Custom Tabs.
///
/// Provider IDs are namespaced strings like `livtet_cloud` or
/// `openlibrary`. They are opaque to the host — the host only
/// uses them to key its storage and to construct the
/// authorization request URL.
pub trait HostOAuth: Send + Sync {
    /// Run the PKCE flow (or look up a cached grant) and return
    /// a fresh access token. Implementations block until the user
    /// completes the consent UI or denies.
    fn redeem_token(&self, plugin_id: &str, provider: &str) -> Result<String, HostError>;

    /// Return a currently valid access token, refreshing if needed.
    /// If no grant exists, the host MUST run the full flow
    /// (equivalent to `redeem_token`).
    fn get_valid_token(&self, plugin_id: &str, provider: &str) -> Result<String, HostError>;

    /// Delete the stored grant and any cached access token. Returns
    /// `Ok` even if no grant existed.
    fn revoke_token(&self, plugin_id: &str, provider: &str) -> Result<(), HostError>;

    /// Fire-and-forget OAuth authorisation. Opens the system browser
    /// and registers the PKCE pending consent, but returns
    /// immediately without waiting for the user to complete the
    /// flow. The plugin subsequently calls `redeem_token` to obtain
    /// the access token. Returns `Ok(())` if the browser was
    /// launched and the pending consent registered.
    fn authorize(&self, plugin_id: &str, provider: &str) -> Result<(), HostError>;
}

/// Sandbox configuration for the Lua VM.
pub trait SandboxConfig {
    fn memory_limit(&self) -> usize {
        64 * 1024 * 1024
    }
    fn instruction_limit(&self) -> i64 {
        10_000_000
    }
    fn hook_interval(&self) -> u32 {
        10_000
    }
}

// ── HTML stripping helpers (shared by all platforms) ────────────────────────

fn strip_html_to_text(html: &str) -> String {
    let mut out = strip_cdata(html);
    out = strip_comments_and_special_blocks(&out);
    out = strip_tags(&out);
    out = decode_html_entities(&out);
    collapse_whitespace(&out)
}

fn strip_cdata(s: &str) -> String {
    if let Some(start) = s.find("<![CDATA[")
        && let Some(end) = s[start + 9..].find("]]>")
    {
        let inner = &s[start + 9..start + 9 + end];
        let mut out = String::with_capacity(s.len());
        out.push_str(&s[..start]);
        out.push_str(inner);
        out.push_str(&s[start + 9 + end + 3..]);
        return out;
    }
    s.to_string()
}

fn strip_comments_and_special_blocks(s: &str) -> String {
    fn strip_range(s: &str, open_pattern: &str, close_pattern: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(start) = rest.find(open_pattern) {
            out.push_str(&rest[..start]);
            let after_open = &rest[start + open_pattern.len()..];
            match after_open.find('>') {
                Some(tag_end) => {
                    let after_tag = &after_open[tag_end + 1..];
                    match after_tag.find(close_pattern) {
                        Some(block_end) => {
                            rest = &after_tag[block_end + close_pattern.len()..];
                        }
                        None => return out,
                    }
                }
                None => {
                    out.push_str(&rest[start..]);
                    return out;
                }
            }
        }
        out.push_str(rest);
        out
    }

    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        match after.find("-->") {
            Some(end) => rest = &after[end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out = strip_range(&out, "<script", "</script>");
    out = strip_range(&out, "<style", "</style>");
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('>') {
            Some(end) => rest = &after[end + 1..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .pipe(|s| decode_numeric_entities(&s))
}

fn decode_numeric_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' && i + 2 < bytes.len() && bytes[i + 1] == b'#' {
            let mut j = i + 2;
            let is_hex = j < bytes.len() && (bytes[j] == b'x' || bytes[j] == b'X');
            if is_hex {
                j += 1;
            }
            let start = j;
            while j < bytes.len() && j - start < 8 && bytes[j] != b';' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b';' && j > start {
                let body = &s[start..j];
                let code = if is_hex {
                    u32::from_str_radix(body, 16).ok()
                } else {
                    body.parse::<u32>().ok()
                };
                if let Some(code) = code
                    && let Some(ch) = char::from_u32(code)
                {
                    out.push(ch);
                    i = j + 1;
                    continue;
                }
            }
        }
        let ch = match s[i..].chars().next() {
            Some(c) => c,
            None => continue,
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

trait Pipe: Sized {
    fn pipe<U, F: FnOnce(Self) -> U>(self, f: F) -> U {
        f(self)
    }
}
impl Pipe for String {}

pub(crate) fn validate_urn_scheme(ns: &str) -> Result<(), HostError> {
    if ns.is_empty() {
        return Err(HostError::Message(
            "host.urn: namespace must not be empty".to_string(),
        ));
    }
    if !ns
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(HostError::Message(format!(
            "host.urn: namespace must match [%w_-]+, got: {ns:?}"
        )));
    }
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{HostSystemSecrets, PluginSystemSecret, validate_urn_scheme};

    #[test]
    fn default_system_secret_impl_returns_none() {
        struct Dummy;
        impl super::HostBase for Dummy {}
        impl super::HostSystemSecrets for Dummy {}
        let d = Dummy;
        assert!(
            d.get_system_secret(PluginSystemSecret::GoogleBooksApiKey)
                .is_none()
        );
        assert!(
            d.get_system_secret(PluginSystemSecret::PlatformUnauthenticatedAllowed)
                .is_none()
        );
    }

    #[test]
    fn validate_urn_scheme_accepts_canonical_namespaces() {
        for ok in [
            "isbn",
            "openlibrary",
            "wikidata",
            "oclc",
            "lccn",
            "koreader",
            "a-b_c",
        ] {
            assert!(
                validate_urn_scheme(ok).is_ok(),
                "expected {ok:?} to be a valid namespace"
            );
        }
    }

    #[test]
    fn validate_urn_scheme_rejects_empty() {
        let err = validate_urn_scheme("").unwrap_err();
        assert!(err.to_string().contains("must not be empty"), "got: {err}");
    }

    #[test]
    fn validate_urn_scheme_rejects_path_separator() {
        let err = validate_urn_scheme("openlibrary/books").unwrap_err();
        assert!(err.to_string().contains("[%w_-]+"), "got: {err}");
        assert!(err.to_string().contains("openlibrary/books"), "got: {err}");
    }

    #[test]
    fn validate_urn_scheme_rejects_colon_and_whitespace() {
        for bad in ["isbn:", "isbn:foo", "open library", "isbn.13"] {
            assert!(
                validate_urn_scheme(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }
}

// ── OAuth Dispatch Trait ────────────────────────────────────────────────────

/// Trait for handling OAuth IPC messages from the plugin host to the
/// Tauri main process. Each method corresponds to a `HostOAuth` trait
/// method on the Lua side, but the IPC-level handling is async because
/// the PKCE flow involves opening the browser and waiting for the
/// deep-link callback.
///
/// The Tauri main process provides a concrete implementation via
/// `OAuthBroker` (in `crates/livtet-tauri/src/oauth/`). The plugin
/// host side (this crate) only knows about this trait.
#[async_trait::async_trait]
pub trait OAuthDispatchHandler: Send + Sync {
    /// Start the PKCE authorization-code flow. Returns the
    /// `OAuthRedeemResult` callback to send back to the Lua host.
    async fn handle_redeem_token(
        &self,
        id: String,
        plugin_id: String,
        provider: String,
    ) -> crate::protocol::MainToHostCallback;

    /// Return a currently valid access token, refreshing if needed.
    async fn handle_get_valid_token(
        &self,
        id: String,
        plugin_id: String,
        provider: String,
    ) -> crate::protocol::MainToHostCallback;

    /// Delete the stored grant and any cached access token.
    async fn handle_revoke_token(
        &self,
        id: String,
        plugin_id: String,
        provider: String,
    ) -> crate::protocol::MainToHostCallback;

    /// Fire-and-forget authorisation. Opens the system browser and
    /// registers the pending consent entry, but returns `ok: true`
    /// immediately rather than blocking on the user's completion.
    async fn handle_authorize(
        &self,
        id: String,
        plugin_id: String,
        provider: String,
    ) -> crate::protocol::MainToHostCallback;
}
