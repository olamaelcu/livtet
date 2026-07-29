//! Per-plugin permission grants.
//!
//! Each plugin gets its own sidecar file under
//! `<data-dir>/permissions/<plugin_id>.<toml|json>`, where `<data-dir>`
//! is the canonical bundle-ID-prefixed directory resolved by
//! [`livtet_core::paths::data_dir_with_migration`]. The host probes
//! both formats, prefers TOML when both exist, and logs a warning so
//! the user can clean up the loser. The grant declares glob sets
//! the plugin is allowed to read (`read_paths`) and SQLite files
//! it is allowed to query (`sqlite_paths`).
//!
//! The host loads the grant lazily on the first call to
//! `host.read_file` or `host.sqlite_query` for a given plugin id
//! and caches it for the lifetime of the host process. Edits to
//! the sidecar take effect on the next host (re)spawn, not
//! mid-session.
//!
//! See `docs/superpowers/specs/2026-06-10-plugin-permission-grants.md`
//! for the full contract.

use std::{collections::HashSet, sync::Arc};

use camino::{Utf8Path, Utf8PathBuf};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{
    error::{PluginError, PluginResult},
    system_secrets::PluginSystemSecret,
};

/// On-disk schema for the per-plugin grant sidecar. Serialised
/// as both TOML (the hand-edited format) and JSON (the
/// programmatically-written default). Unknown fields are
/// ignored so future versions can extend the schema without
/// breaking older hosts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginGrant {
    /// Schema version. v1 hosts accept only `version = 1`. Future
    /// versions will introduce a migration shim; older hosts
    /// ignore the field.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Glob globs the plugin is allowed to read with
    /// `host.read_file`.
    #[serde(default)]
    pub read_paths: Vec<String>,
    /// Glob globs the plugin is allowed to open with
    /// `host.sqlite_query`.
    #[serde(default)]
    pub sqlite_paths: Vec<String>,
    /// `allow_writes` is parsed and rejected in v1. The field
    /// exists in the schema so a future v2 can introduce write
    /// grants without breaking the format.
    #[serde(default)]
    pub allow_writes: bool,
    /// Glob globs the plugin is allowed to write to with
    /// `host.fs_copy` (dst) and `host.fs_symlink` (link_path).
    /// Empty or missing sidecar means no write access.
    #[serde(default)]
    pub write_paths: Vec<String>,
    /// Canonical `PluginSystemSecret` keys the user has explicitly
    /// allowed this plugin to read. Missing sidecar or empty list
    /// means no system-secret access. The list is matched exactly
    /// (no globs; secrets are a closed enum). Unknown values and
    /// `"*"` are rejected at sidecar-load time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_secrets: Vec<String>,
    #[serde(default)]
    pub embeddings: bool,
    /// OAuth provider allowlist. Each entry is a `provider_id` paired
    /// with the scopes the user has authorized. The plugin host uses
    /// this list to gate `host.oauth_*` calls: the plugin must have
    /// a matching allowlist entry for the (provider, scope-set) it
    /// requests, or the call returns `HostError::Message`.
    ///
    /// Multiple entries for the same `provider_id` are not allowed;
    /// later writers replace the earlier scopes. Use `oauth_use!` /
    /// `check_oauth!` in `host_lua.rs` to gate calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oauth_providers: Vec<OAuthGrantEntry>,
    /// HTTP proxy URL for plugins that need to bypass Cloudflare or
    /// other anti-bot protections (e.g., `goodreads_scrape`). The
    /// plugin host's HTTP client routes requests through this proxy
    /// when set. Example: `"http://flaresolverr:8191"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_proxy_url: Option<String>,
}

/// One row in `PluginGrant.oauth_providers`. `provider_id` is opaque
/// to the host (e.g. `livtet_cloud`, `atproto`, `openlibrary`); the
/// host's provider registry maps it to concrete authorization / token
/// endpoints. `scopes` is the set the user explicitly authorized;
/// the host's broker merges this with the manifest's declared scopes
/// at redemption time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct OAuthGrantEntry {
    pub provider_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

fn default_version() -> u32 {
    1
}

/// Resolved grant with glob sets pre-compiled for fast lookup.
/// GlobSet gives us O(log n) matching against any number of
/// globs; the per-call cost is dominated by the path canonicalisation
/// step, not the glob match itself.
#[derive(Debug)]
pub struct ResolvedGrant {
    pub raw: PluginGrant,
    pub read_paths: GlobSet,
    pub sqlite_paths: GlobSet,
    pub write_paths: GlobSet,
    /// Set of `PluginSystemSecret` variants explicitly allowed by
    /// the grant's `system_secrets` list. Compiled at sidecar-load
    /// time; unknown strings and `"*"` are rejected as parse errors.
    pub system_secrets: HashSet<PluginSystemSecret>,
    pub embeddings: bool,
    /// `provider_id` -> authorized scope set, derived from the
    /// sidecar's `oauth_providers` list. The broker uses this to
    /// gate `host.oauth_*` calls; `check_oauth` (below) wraps the
    /// common lookup pattern.
    pub oauth_providers: std::collections::HashMap<String, Vec<String>>,
    /// HTTP proxy URL, copied from the grant sidecar's `http_proxy_url`
    /// field. Used by the host to configure the HTTP client for plugins
    /// that need to bypass Cloudflare or other protections.
    pub http_proxy_url: Option<String>,
}

/// Format the loader should write when a new grant is created by
/// the UI or CLI. v1 picks JSON for parity with the rest of the
/// plugin system (`installed.json`, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantFormat {
    Toml,
    Json,
}

impl GrantFormat {
    pub fn extension(self) -> &'static str {
        match self {
            GrantFormat::Toml => "toml",
            GrantFormat::Json => "json",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "toml" => Some(GrantFormat::Toml),
            "json" => Some(GrantFormat::Json),
            _ => None,
        }
    }
}

/// Default location for the permissions directory. Uses the
/// canonical bundle-ID-prefixed data directory resolved by
/// [`livtet_core::paths::data_dir_with_migration`].
pub fn default_permissions_dir() -> Utf8PathBuf {
    livtet_core::paths::data_dir_with_migration()
        .map(|p| p.join(livtet_core::paths::subdirs::PERMISSIONS))
        .unwrap_or_else(|| Utf8PathBuf::from("/tmp/livtet/permissions"))
}

/// Return the path the loader will probe for a given plugin id
/// in the given format. `default_grant_path` lets the UI write
/// to the canonical path without re-deriving the directory.
pub fn default_grant_path(
    permissions_dir: &Utf8Path,
    plugin_id: &str,
    format: GrantFormat,
) -> Utf8PathBuf {
    permissions_dir.join(format!("{plugin_id}.{}", format.extension()))
}

/// Probe the permissions dir for `<plugin_id>.toml` first, then
/// `<plugin_id>.json`. If neither exists, returns `None` (the
/// host functions surface this as the "no grant sidecar" error).
/// If both exist, returns the TOML grant and logs a warning
/// naming the JSON file that was shadowed.
pub fn load_grant(
    plugin_id: &str,
    permissions_dir: &Utf8Path,
) -> PluginResult<Option<Arc<ResolvedGrant>>> {
    let toml_path = default_grant_path(permissions_dir, plugin_id, GrantFormat::Toml);
    let json_path = default_grant_path(permissions_dir, plugin_id, GrantFormat::Json);

    if toml_path.exists() {
        if json_path.exists() {
            warn!(
                plugin_id = plugin_id,
                toml = %toml_path,
                json = %json_path,
                "both .toml and .json grant sidecars exist; TOML wins. Delete the JSON file to silence this warning."
            );
        }
        let raw = parse_toml(&toml_path)?;
        let resolved = resolve_grant(raw)?;
        return Ok(Some(Arc::new(resolved)));
    }

    if json_path.exists() {
        let raw = parse_json(&json_path)?;
        let resolved = resolve_grant(raw)?;
        return Ok(Some(Arc::new(resolved)));
    }

    Ok(None)
}

fn parse_toml(path: &Utf8Path) -> PluginResult<PluginGrant> {
    let contents = fs_err::read_to_string(path)?;
    toml::from_str(&contents).map_err(PluginError::from)
}

fn parse_json(path: &Utf8Path) -> PluginResult<PluginGrant> {
    let contents = fs_err::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(|e| PluginError::Serialization(e.to_string()))
}

/// Build a `GlobSet` from a list of glob strings. Returns an
/// error if any glob fails to compile — the host refuses to
/// honour a grant it cannot evaluate, so a bad glob is treated
/// as no grant at all.
fn build_glob_set(globs: &[String]) -> PluginResult<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for g in globs {
        let glob =
            Glob::new(g).map_err(|e| PluginError::Discovery(format!("invalid glob '{g}': {e}")))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| PluginError::Discovery(format!("build globset: {e}")))
}

fn resolve_grant(raw: PluginGrant) -> PluginResult<ResolvedGrant> {
    if raw.allow_writes {
        return Err(PluginError::Discovery(
            "allow_writes=true is not supported in v1".to_string(),
        ));
    }
    let read_paths = build_glob_set(&raw.read_paths)?;
    let sqlite_paths = build_glob_set(&raw.sqlite_paths)?;
    let write_paths = build_glob_set(&raw.write_paths)?;
    let mut system_secrets: HashSet<PluginSystemSecret> = HashSet::new();
    for s in &raw.system_secrets {
        if s == "*" {
            return Err(PluginError::Discovery(
                "wildcard '*' is not allowed in system_secrets; list each secret by its canonical name".to_string(),
            ));
        }
        let parsed = s.parse::<PluginSystemSecret>().map_err(|e| {
            PluginError::Discovery(format!("invalid system secret '{s}' in grant sidecar: {e}"))
        })?;
        system_secrets.insert(parsed);
    }
    let embeddings = raw.embeddings;
    let mut oauth_providers: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for entry in &raw.oauth_providers {
        if entry.provider_id.is_empty() {
            return Err(PluginError::Discovery(
                "oauth_providers entry has empty provider_id".to_string(),
            ));
        }
        if oauth_providers
            .insert(entry.provider_id.clone(), entry.scopes.clone())
            .is_some()
        {
            return Err(PluginError::Discovery(format!(
                "duplicate oauth_providers entry for provider '{}'",
                entry.provider_id
            )));
        }
    }
    let http_proxy_url = raw.http_proxy_url.clone();
    Ok(ResolvedGrant {
        raw,
        read_paths,
        sqlite_paths,
        write_paths,
        system_secrets,
        embeddings,
        oauth_providers,
        http_proxy_url,
    })
}

/// Check whether `path` is covered by the grant's `read_paths`
/// glob set. Returns `true` on a match, `false` otherwise. The
/// caller is expected to surface a permission-denied error when
/// the match fails.
pub fn check_read(grant: &ResolvedGrant, path: &Utf8Path) -> bool {
    grant.read_paths.is_match(path.as_std_path())
}

/// Check whether `path` is covered by the grant's `sqlite_paths`
/// glob set.
pub fn check_sqlite(grant: &ResolvedGrant, path: &Utf8Path) -> bool {
    grant.sqlite_paths.is_match(path.as_std_path())
}

/// Check whether `path` matches any glob in `write_paths`. Used to
/// gate `host.fs_copy` (dst) and `host.fs_symlink` (link_path).
/// Canonicalizes the path before matching to defang `..` traversal
/// and symlink tricks; falls back to the raw path when canonicalize
/// fails (e.g. the destination does not exist yet for `fs_copy`).
pub fn check_write(grant: &ResolvedGrant, path: &Utf8Path) -> bool {
    match Utf8Path::canonicalize_utf8(path) {
        Ok(canonical) => grant.write_paths.is_match(canonical.as_std_path()),
        Err(e) => {
            warn!("cannot canonicalize {path:?} for write check: {e}");
            grant.write_paths.is_match(path.as_std_path())
        }
    }
}

/// Check whether `name` is in the grant's `system_secrets`
/// allowlist. Returns `true` on a match, `false` otherwise. The
/// caller is expected to surface a permission-denied error when
/// the match fails.
pub fn check_system_secret(grant: &ResolvedGrant, name: PluginSystemSecret) -> bool {
    grant.system_secrets.contains(&name)
}

pub fn check_embeddings(grant: &ResolvedGrant) -> bool {
    grant.embeddings
}

/// Check whether the plugin's grant sidecar allows OAuth access to
/// `provider_id`. Returns `Some(&scopes)` on a match (the scopes
/// the user has authorised) or `None` if the provider is not in
/// the allowlist. The caller decides whether a `None` should
/// surface as `HostError::Message` (the existing path) or as the
/// new structured `HostError::NeedsAuth { plugin_id, provider }`.
pub fn check_oauth<'a>(grant: &'a ResolvedGrant, provider_id: &str) -> Option<&'a [String]> {
    grant.oauth_providers.get(provider_id).map(Vec::as_slice)
}

/// Check whether the grant's `http_proxy_url` field is set.
/// Returns `Some(&url)` on a match, `None` otherwise. The caller
/// is expected to surface a permission-denied error when the check
/// fails.
pub fn check_http_proxy(grant: &ResolvedGrant) -> Option<&String> {
    grant.http_proxy_url.as_ref()
}

/// Canonical "HTTP proxy not in grant" error. The plugin sees
/// this as the `err` return from `host.http_get` when the proxy
/// is required but not granted.
pub fn http_proxy_denied_error(plugin_id: &str) -> String {
    format!(
        "permission denied: plugin '{plugin_id}' is not allowed to use an HTTP proxy — add http_proxy_url to the grant sidecar"
    )
}

/// Canonical "OAuth provider not in grant allowlist" error. The
/// plugin sees this as the `err` return from
/// `host.oauth_redeem_token` so it can render a user-facing
/// "grant access" prompt.
pub fn oauth_denied_error(plugin_id: &str, provider_id: &str) -> String {
    format!(
        "permission denied: plugin '{plugin_id}' is not allowed to call OAuth provider '{provider_id}' — \
         add it to the grant sidecar's oauth_providers list"
    )
}

/// Canonical "secret not in grant allowlist" error. Mirrors
/// `outside_glob_error` for the string-keyed path checks.
pub fn system_secret_denied_error(plugin_id: &str, name: PluginSystemSecret) -> String {
    format!(
        "permission denied: plugin '{plugin_id}' is not allowed to read system secret '{}' — \
         add it to the grant sidecar's system_secrets list",
        name.as_ref()
    )
}

/// Canonicalise the canonical error string for a missing
/// sidecar. The plugin sees this as the `err` return from
/// `host.read_file` / `host.sqlite_query` so it can render a
/// user-facing "grant access" prompt.
pub fn missing_sidecar_error(plugin_id: &str) -> String {
    format!(
        "permission denied: no grant sidecar at ~/.local/share/livtet/permissions/{plugin_id}.{{toml,json}}"
    )
}

/// Canonical "outside grant glob" error. The matcher reports
/// the *first* glob the path was tested against, which is
/// deterministic because `globset` preserves insertion order.
pub fn outside_glob_error(path: &Utf8Path, glob: &str) -> String {
    format!("permission denied: path '{path}' is outside grant glob '{glob}'")
}

/// The path the host should probe for a plugin's grant, taking
/// the `LIVTET_PLUGIN_PERMISSIONS_DIR` env var override into
/// account. Centralised so the host's `read_file` / `sqlite_query`
/// closure and the test harness agree.
pub fn permissions_dir() -> Utf8PathBuf {
    if let Some(p) =
        std::env::var_os("LIVTET_PLUGIN_PERMISSIONS_DIR").and_then(osstring_to_utf8_path)
    {
        return p;
    }
    default_permissions_dir()
}

fn osstring_to_utf8_string(s: std::ffi::OsString) -> Option<String> {
    s.into_string().ok()
}

fn osstring_to_utf8_path(s: std::ffi::OsString) -> Option<Utf8PathBuf> {
    osstring_to_utf8_string(s).map(Utf8PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_secret_allow_named_secret() {
        let raw = PluginGrant {
            version: 1,
            read_paths: vec![],
            sqlite_paths: vec![],
            allow_writes: false,
            write_paths: vec![],
            system_secrets: vec!["google_books_api_key".to_string()],
            embeddings: false,
            oauth_providers: vec![],
            http_proxy_url: None,
        };
        let resolved = super::resolve_grant(raw).expect("valid grant");
        assert!(
            super::check_system_secret(&resolved, PluginSystemSecret::GoogleBooksApiKey),
            "google_books_api_key should be allowed"
        );
        assert!(
            !super::check_system_secret(
                &resolved,
                PluginSystemSecret::PlatformUnauthenticatedAllowed
            ),
            "unlisted secret should be denied"
        );
    }

    #[test]
    fn system_secret_rejects_unlisted() {
        let raw = PluginGrant {
            version: 1,
            read_paths: vec![],
            sqlite_paths: vec![],
            allow_writes: false,
            write_paths: vec![],
            system_secrets: vec![],
            embeddings: false,
            oauth_providers: vec![],
            http_proxy_url: None,
        };
        let resolved = super::resolve_grant(raw).expect("valid grant");
        assert!(
            !super::check_system_secret(&resolved, PluginSystemSecret::GoogleBooksApiKey),
            "empty allowlist grants nothing"
        );
    }

    #[test]
    fn system_secret_rejects_unknown_string() {
        let raw = PluginGrant {
            version: 1,
            read_paths: vec![],
            sqlite_paths: vec![],
            allow_writes: false,
            write_paths: vec![],
            system_secrets: vec!["not_a_real_secret".to_string()],
            embeddings: false,
            oauth_providers: vec![],
            http_proxy_url: None,
        };
        let err = super::resolve_grant(raw).expect_err("unknown string must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("not_a_real_secret"),
            "error should name the bad key; got: {msg}"
        );
    }

    #[test]
    fn system_secret_rejects_wildcard() {
        let raw = PluginGrant {
            version: 1,
            read_paths: vec![],
            sqlite_paths: vec![],
            allow_writes: false,
            write_paths: vec![],
            system_secrets: vec!["*".to_string()],
            embeddings: false,
            oauth_providers: vec![],
            http_proxy_url: None,
        };
        let err = super::resolve_grant(raw).expect_err("wildcard must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("*"),
            "error should reject wildcard; got: {msg}"
        );
    }

    #[test]
    fn system_secret_denied_error_includes_plugin_and_secret() {
        let msg =
            super::system_secret_denied_error("googlebooks", PluginSystemSecret::GoogleBooksApiKey);
        assert!(msg.contains("googlebooks"));
        assert!(msg.contains("google_books_api_key"));
    }
}
