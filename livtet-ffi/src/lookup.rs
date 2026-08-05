//! Plugin-powered lookup and search via the embedded Lua host.
//!
//! Maintains a global `LuaHost<EmbeddedHost>` initialised once with
//! every bundled plugin whose manifest declares the `search` and/or
//! `lookup` capability. Exposes `init_plugins()`, `lookup_identifier()`,
//! and `search_providers()` as async UniFFI exports so the Android
//! bindings can take advantage of structured concurrency and coroutine
//! cancellation during keystroke debouncing in the add-book wizard.
//!
//! Lookup and search iterate `embedded_index()` in id order and
//! return the first non-empty hit set — a non-null
//! `serde_json::Value::Object` for `lookup`, a non-empty
//! `serde_json::Value::Array` for `search`. The answering plugin
//! tags each hit with its own `source` field (e.g. `"openlibrary"`,
//! `"googlebooks"`), so the wizard can surface which provider answered.
//!
//! The exported bodies are effectively synchronous — Lua calls into the
//! `reqwest::blocking` HTTP host on the calling (foreign) thread, which
//! is fine because uniffi-driven async-FFI handlers are polled by the
//! foreign runtime and never inside any Rust tokio runtime. The async
//! signature gives the foreign side a proper suspending function so
//! coroutine cancellation, structured concurrency, and thread-of-the-future
//! propagation work; cancellation between the cancellation itself and the
//! in-flight Lua / reqwest work is intentionally not implemented (would
//! require a dedicated Lua worker thread sending commands over channels;
//! tracked as a follow-up).
//!
//! The mobile host overrides the `host.require` slot registered by
//! `LuaHost::setup_host_functions` with a closure that resolves
//! Lua module names via the in-process `livtet-lua-stdlib` index
//! (dkjson, etc.). This is the only path on mobile: LuaRocks is not
//! available and there is no on-disk `.so` to dlopen. The override
//! is installed once when the host is first constructed, before any
//! plugin is loaded.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::MobileError;

// ── Globals ────────────────────────────────────────────────────────────

type LuaInner = livtet_plugins::host_lua::LuaHost<livtet_plugins::embedded_host::EmbeddedHost>;

struct PluginHost(Mutex<LuaInner>);

unsafe impl Sync for PluginHost {}
unsafe impl Send for PluginHost {}

#[allow(dead_code)]
impl PluginHost {
    fn lock(&self) -> std::sync::MutexGuard<'_, LuaInner> {
        self.0.lock().unwrap()
    }
}

static PLUGIN_HOST: std::sync::OnceLock<PluginHost> = std::sync::OnceLock::new();
static PENDING_SECRETS: Mutex<
    Option<HashMap<livtet_plugins::system_secrets::PluginSystemSecret, String>>,
> = Mutex::new(None);

// ── PluginHitMobile ────────────────────────────────────────────────────

#[derive(uniffi::Record)]
pub struct PluginHitMobile {
    pub title: String,
    pub authors: Vec<String>,
    pub identifiers: Vec<String>,
    pub cover_url: Option<String>,
    pub publisher: Option<String>,
    pub published_date: Option<String>,
    pub page_count: Option<i32>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub source: String,
    pub source_url: Option<String>,
}

#[allow(dead_code)]
impl PluginHitMobile {
    fn from_json(value: &serde_json::Value, source: &str) -> Option<Self> {
        let obj = value.as_object()?;
        Some(Self {
            title: obj
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            authors: obj
                .get("authors")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            identifiers: obj
                .get("identifiers")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            cover_url: obj
                .get("cover_url")
                .and_then(|v| v.as_str())
                .map(String::from),
            publisher: obj
                .get("publisher")
                .and_then(|v| v.as_str())
                .map(String::from),
            published_date: obj
                .get("published_date")
                .and_then(|v| v.as_str())
                .map(String::from),
            page_count: obj.get("page_count").and_then(|v| v.as_i64()).map(|n| n as i32),
            language: obj
                .get("language")
                .and_then(|v| v.as_str())
                .map(String::from),
            description: obj
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
            source: source.to_string(),
            source_url: obj
                .get("source_url")
                .and_then(|v| v.as_str())
                .map(String::from),
        })
    }
}

// ── System secrets ────────────────────────────────────────────────────

#[uniffi::export]
pub fn set_system_secrets(secrets: HashMap<String, String>) {
    use livtet_plugins::system_secrets::PluginSystemSecret;

    let mut parsed = HashMap::new();
    for (key, value) in secrets {
        if value.is_empty() {
            continue;
        }
        let variant = match key.as_str() {
            "google_books_api_key" => PluginSystemSecret::GoogleBooksApiKey,
            "platform_unauthenticated_allowed" => PluginSystemSecret::PlatformUnauthenticatedAllowed,
            _ => {
                tracing::debug!(%key, "unknown system secret key; skipping");
                continue;
            }
        };
        parsed.insert(variant, value);
    }
    if let Ok(mut guard) = PENDING_SECRETS.lock() {
        *guard = Some(parsed);
    }
}

// ── Grant sidecars ────────────────────────────────────────────────────

#[allow(dead_code)]
fn ensure_default_grants(
    manifests: &[livtet_plugins::manifest::PluginManifest],
) -> Result<(), MobileError> {
    use livtet_plugins::permissions;
    use livtet_plugins::plugin_requires::PluginRequires;

    let perms_dir = permissions::permissions_dir();
    fs_err::create_dir_all(&perms_dir)
        .map_err(|e| MobileError::Platform(format!("permissions dir: {e}")))?;

    for manifest in manifests {
        let plugin_id = &manifest.plugin.id;
        if permissions::load_grant(plugin_id, &perms_dir)
            .ok()
            .flatten()
            .is_some()
        {
            continue;
        }

        let requires_filesystem = manifest
            .plugin
            .requires
            .get(&PluginRequires::Filesystem)
            .copied()
            .unwrap_or(false);
        let requires_system_secrets = manifest
            .plugin
            .requires
            .get(&PluginRequires::SystemSecrets)
            .copied()
            .unwrap_or(false);

        let grant = permissions::PluginGrant {
            version: 1,
            read_paths: if requires_filesystem {
                vec!["**".into()]
            } else {
                vec![]
            },
            sqlite_paths: vec![],
            allow_writes: false,
            write_paths: vec![],
            system_secrets: if requires_system_secrets {
                vec![
                    "google_books_api_key".into(),
                    "platform_unauthenticated_allowed".into(),
                ]
            } else {
                vec![]
            },
            embeddings: false,
            oauth_providers: vec![],
            http_proxy_url: None,
        };

        let grant_path =
            permissions::default_grant_path(&perms_dir, plugin_id, permissions::GrantFormat::Toml);
        let toml_str = toml::to_string_pretty(&grant)
            .map_err(|e| MobileError::Platform(format!("grant TOML: {e}")))?;
        if let Some(parent) = grant_path.parent() {
            fs_err::create_dir_all(parent)?;
        }
        fs_err::write(&grant_path, toml_str)?;
        tracing::info!(%plugin_id, "wrote default grant sidecar");
    }
    Ok(())
}

// ── init_plugins ───────────────────────────────────────────────────────

#[uniffi::export]
pub async fn init_plugins() -> Result<(), MobileError> {
    use livtet_plugins::embedded_host::EmbeddedHost;
    use livtet_plugins::host_lua::LuaHost;

    let secrets = PENDING_SECRETS.lock().unwrap().take().unwrap_or_default();
    let host = EmbeddedHost::with_system_secrets(secrets);
    let lua_host = LuaHost::new(Arc::new(host))
        .map_err(|e| MobileError::Platform(format!("LuaHost::new: {e}")))?;

    PLUGIN_HOST
        .set(PluginHost(Mutex::new(lua_host)))
        .map_err(|_| MobileError::Init("plugins already initialized".into()))?;

    #[cfg(feature = "bundled")]
    {
        let bundled = livtet_plugins::bundled::bundled_index();
        tracing::info!(count = bundled.len(), "loading bundled plugins");

        if bundled.is_empty() {
            tracing::warn!("bundled feature enabled but no plugins found");
            return Ok(());
        }

        let mut manifests = Vec::with_capacity(bundled.len());
        let guard = PLUGIN_HOST.get().unwrap();

        for entry in &bundled {
            manifests.push(entry.manifest.clone());
            let source = String::from_utf8_lossy(entry.source_bytes);
            let mut host = guard.lock();
            let result = host.load_plugin_source(&entry.id, &source, None, None);
            match result {
                livtet_plugins::protocol::HostToMain::PluginLoaded { plugin_id, .. } => {
                    tracing::info!(%plugin_id, "bundled plugin loaded");
                }
                livtet_plugins::protocol::HostToMain::PluginLoadError {
                    plugin_id, error, ..
                } => {
                    tracing::error!(%plugin_id, %error, "failed to load bundled plugin");
                }
                other => {
                    tracing::warn!(
                        plugin_id = %entry.id,
                        message = ?other,
                        "unexpected load result"
                    );
                }
            }
        }

        ensure_default_grants(&manifests)?;
    }

    Ok(())
}

// ── lookup_identifier ──────────────────────────────────────────────────

#[uniffi::export]
pub async fn lookup_identifier(_urn: String) -> Result<Option<PluginHitMobile>, MobileError> {
    let _host = PLUGIN_HOST
        .get()
        .ok_or(MobileError::Init("plugins not initialized".into()))?;

    #[cfg(feature = "bundled")]
    {
        let urn = _urn;
        for entry in livtet_plugins::bundled::bundled_index() {
            let has_lookup = entry
                .manifest
                .plugin
                .capabilities
                .iter()
                .any(|(cap, enabled)| *enabled && cap.as_str() == "lookup");
            if !has_lookup {
                continue;
            }

            let mut guard = _host.lock();
            let result = guard.call_capability(
                &ulid::Ulid::new().to_string(),
                &entry.id,
                "lookup",
                &[serde_json::Value::String(urn.clone())],
            );

            if let livtet_plugins::protocol::HostToMain::CallResult { ok, value, .. } = result {
                if ok {
                    if let Some(ref val) = value {
                        if val.is_object() && !val.is_null() {
                            if let Some(hit) = PluginHitMobile::from_json(val, &entry.id) {
                                return Ok(Some(hit));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

// ── search_providers ───────────────────────────────────────────────────

#[uniffi::export]
pub async fn search_providers(_query: String) -> Result<Vec<PluginHitMobile>, MobileError> {
    let _host = PLUGIN_HOST
        .get()
        .ok_or(MobileError::Init("plugins not initialized".into()))?;

    #[cfg(feature = "bundled")]
    {
        let query = _query;
        for entry in livtet_plugins::bundled::bundled_index() {
            let has_search = entry
                .manifest
                .plugin
                .capabilities
                .iter()
                .any(|(cap, enabled)| *enabled && cap.as_str() == "search");
            if !has_search {
                continue;
            }

            let mut guard = _host.lock();
            let result = guard.call_capability(
                &ulid::Ulid::new().to_string(),
                &entry.id,
                "search",
                &[
                    serde_json::Value::String(query.clone()),
                    serde_json::Value::Object(serde_json::Map::new()),
                ],
            );

            if let livtet_plugins::protocol::HostToMain::CallResult { ok, value, .. } = result {
                if ok {
                    if let Some(serde_json::Value::Array(arr)) = value {
                        if !arr.is_empty() {
                            let hits: Vec<PluginHitMobile> = arr
                                .iter()
                                .filter_map(|v| PluginHitMobile::from_json(v, &entry.id))
                                .collect();
                            if !hits.is_empty() {
                                return Ok(hits);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(vec![])
}

#[cfg(test)]
mod tests {
    // TBD: `install_bundled_lua_require` and the in-process bundled
    // Lua stdlib (`livtet_lua_stdlib`) were removed when those stub
    // crates were deleted. The two tests in this module that exercise
    // `host.require("dkjson")` and `host.require("nonexistent")` are
    // skipped at compile time. Restore once the bundling pipeline
    // returns.
}
