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

use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::{Mutex, OnceLock},
};

use livtet_lua_plugins::embedded_index;
use livtet_plugin::{
    capability::Capability,
    embedded_host::EmbeddedHost,
    host_lua::LuaHost,
    manifest::PluginManifest,
    mlua::{Table, Value},
    permissions::{GrantFormat, PluginGrant, default_grant_path, permissions_dir},
    protocol::HostToMain,
    system_secrets::PluginSystemSecret,
};

use crate::{MobileError, ProviderErrorCategory};

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

// LuaHost contains mlua::Lua which is !Send, but we ensure single-threaded
// access via the Mutex. The newtype makes the OnceLock<Mutex<...>> compile
// since we never send the Lua state across threads.
struct LuaHostWrapper(Mutex<LuaHost<EmbeddedHost>>);
unsafe impl Send for LuaHostWrapper {}
unsafe impl Sync for LuaHostWrapper {}

static PLUGIN_HOST: OnceLock<LuaHostWrapper> = OnceLock::new();

/// Compile-time system secrets supplied by the embedder (e.g. the
/// Android Kotlin layer reading `BuildConfig.GOOGLE_API_KEY`). Set
/// via [`set_system_secrets`] before the host is initialised; once
/// the `LuaHost` is constructed the map is consumed and the static
/// is cleared.
static PENDING_SYSTEM_SECRETS: std::sync::Mutex<Option<HashMap<String, String>>> =
    std::sync::Mutex::new(None);

fn set_system_secrets_inner(secrets: HashMap<String, String>) {
    let sanitized: HashMap<String, String> =
        secrets.into_iter().filter(|(_, v)| !v.is_empty()).collect();
    if let Ok(mut guard) = PENDING_SYSTEM_SECRETS.lock() {
        *guard = Some(sanitized);
    }
}

fn take_pending_system_secrets() -> HashMap<PluginSystemSecret, String> {
    if let Ok(mut guard) = PENDING_SYSTEM_SECRETS.lock() {
        guard
            .take()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| PluginSystemSecret::from_str(&k).ok().map(|s| (s, v)))
            .collect()
    } else {
        HashMap::new()
    }
}

fn get_or_init_host() -> Result<&'static Mutex<LuaHost<EmbeddedHost>>, MobileError> {
    let wrapper = PLUGIN_HOST.get_or_init(|| {
        let system_secrets = take_pending_system_secrets();
        let host_impl = std::sync::Arc::new(EmbeddedHost::with_system_secrets(system_secrets));
        let host = LuaHost::new(host_impl)
            .expect("LuaHost::new should succeed with default sandbox config");
        install_bundled_lua_require(&host)
            .expect("installing bundled-lua require resolver should not fail");
        LuaHostWrapper(Mutex::new(host))
    });
    Ok(&wrapper.0)
}

/// Override the `host.require` stub registered by
/// `LuaHost::setup_host_functions` with a closure that resolves Lua
/// module names against the in-process `livtet-lua-stdlib` index.
/// Results are cached per-Lua-state so repeated `require` calls don't
/// re-execute the chunk (the sandbox strips `package.loaded`).
fn install_bundled_lua_require(host: &LuaHost<EmbeddedHost>) -> livtet_plugin::mlua::Result<()> {
    let lua = host.lua();
    let globals = lua.globals();
    let host_table: Table = globals.get("host")?;
    let cache: std::rc::Rc<std::cell::RefCell<HashMap<String, Value>>> =
        std::rc::Rc::new(std::cell::RefCell::new(HashMap::new()));
    let cache_for_closure = std::rc::Rc::clone(&cache);
    let require_fn = lua.create_function(
        move |lua, (name,): (String,)| -> livtet_plugin::mlua::Result<Value> {
            if let Some(cached) = cache_for_closure.borrow().get(&name) {
                return Ok(cached.clone());
            }
            let bytes = livtet_lua_stdlib::embedded_index()
                .resolve(&name)
                .ok_or_else(|| {
                    livtet_plugin::mlua::Error::external(format!(
                        "no bundled-lua rock named {name:?}"
                    ))
                })?;
            let code = std::str::from_utf8(bytes).map_err(|e| {
                livtet_plugin::mlua::Error::external(format!(
                    "bundled-lua rock {name:?} not UTF-8: {e}"
                ))
            })?;
            let chunk_name = format!("bundled-lua:{name}");
            // Call the chunk as a function with no args; its return values are
            // handed back. For dkjson's `return json` last line, this returns
            // the `json` module table; for a chunk with bare assignments and
            // no trailing return, the result is an empty `MultiValue` (we
            // discard that and return Nil so callers always get a single
            // Value).
            let chunk = lua.load(code).set_name(&chunk_name);
            let returns: livtet_plugin::mlua::MultiValue = chunk.call(())?;
            let result = returns.into_iter().next().unwrap_or(Value::Nil);
            cache_for_closure.borrow_mut().insert(name, result.clone());
            Ok(result)
        },
    )?;
    host_table.set("require", require_fn)?;
    Ok(())
}

/// Acquire the plugin-host mutex, recovering from any prior panic
/// that may have poisoned the lock. The Lua state is robust enough to
/// tolerate a panic mid-call, and we'd rather return a fresh error
/// for the new call than refuse to serve the wizard entirely.
fn lock_host(
    mutex: &'static Mutex<LuaHost<EmbeddedHost>>,
) -> Result<std::sync::MutexGuard<'static, LuaHost<EmbeddedHost>>, MobileError> {
    match mutex.lock() {
        Ok(g) => Ok(g),
        Err(poisoned) => Ok(poisoned.into_inner()),
    }
}

/// Install compile-time system secrets before the Lua host is
/// initialised. The embedder (Kotlin `BuildConfig.*`, etc.) calls
/// this once at process startup; the values are consumed (and the
/// static cleared) by the first [`init_plugins`] call.
///
/// Keys are the snake_case variant names of
/// [`PluginSystemSecret`](livtet_plugin::system_secrets::PluginSystemSecret)
/// (e.g. `"google_books_api_key"`). Unknown keys are silently dropped
/// so a future enum variant added server-side does not break older
/// mobile clients. Empty values are also dropped.
#[uniffi::export]
pub fn set_system_secrets(secrets: HashMap<String, String>) {
    set_system_secrets_inner(secrets)
}

/// Create default permission-grant sidecars for bundled plugins that
/// declare `system_secrets = true` in their manifest. Without these
/// sidecars the host's two-gate check (manifest declaration + grant
/// allowlist) blocks `host.get_system_secret` and plugins return
/// "API key not configured" instead of the seeded `BuildConfig`
/// value. On desktop the Tauri UI calls `plugin_grant_paths` to
/// create them at runtime; on mobile there is no UI flow, so we
/// pre-write the grants during `init_plugins()` and let the user
/// revoke via the Settings screen in a follow-up.
///
/// Only writes when the sidecar is missing — never overwrites a
/// user-edited grant. Plugin manifests are the source of truth for
/// *which* secrets a plugin needs, so the allowlist is built
/// dynamically from the manifest's `requires.system_secrets = true`
/// gate and the canonical [`PluginSystemSecret`] enum.
fn ensure_default_grants() -> Result<(), MobileError> {
    let perms_dir = permissions_dir();
    tracing::info!(perms_dir = %perms_dir, "ensure_default_grants: resolving perms dir");
    if let Err(e) = fs_err::create_dir_all(&perms_dir) {
        tracing::warn!(
            perms_dir = %perms_dir,
            "could not create permissions directory; plugins will fall back to missing-sidecar errors: {e}"
        );
        return Ok(());
    }
    tracing::info!(perms_dir = %perms_dir, "ensure_default_grants: perms dir ensured");

    for plugin in embedded_index().iter() {
        let manifest_str = match std::str::from_utf8(plugin.manifest_bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let manifest: PluginManifest = match toml::from_str(manifest_str) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !manifest.plugin.has_capability_system_secrets() {
            continue;
        }

        let plugin_id = manifest.plugin.id.clone();
        let toml_path = default_grant_path(&perms_dir, &plugin_id, GrantFormat::Toml);
        let json_path = default_grant_path(&perms_dir, &plugin_id, GrantFormat::Json);
        if toml_path.exists() || json_path.exists() {
            tracing::debug!(
                plugin = %plugin_id,
                "grant sidecar already present; leaving user-provided grant untouched"
            );
            continue;
        }

        let allowlist: Vec<String> = match plugin_id.as_str() {
            "googlebooks" => vec![PluginSystemSecret::GoogleBooksApiKey.as_ref().to_string()],
            _ => Vec::new(),
        };
        let grant = PluginGrant {
            version: 1,
            read_paths: Vec::new(),
            sqlite_paths: Vec::new(),
            allow_writes: false,
            write_paths: Vec::new(),
            system_secrets: allowlist.clone(),
            embeddings: false,
            oauth_providers: Vec::new(),
            http_proxy_url: None,
        };
        match toml::to_string_pretty(&grant) {
            Ok(serialized) => {
                if let Err(e) = fs_err::write(&toml_path, serialized) {
                    tracing::warn!(
                        plugin = %plugin_id,
                        path = %toml_path,
                        "failed to write default grant sidecar: {e}"
                    );
                } else {
                    tracing::info!(
                        plugin = %plugin_id,
                        path = %toml_path,
                        secrets = ?allowlist,
                        "wrote default permission grant sidecar"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    plugin = %plugin_id,
                    "failed to serialise default grant: {e}"
                );
            }
        }
    }

    Ok(())
}

/// Safe to call multiple times — already-loaded plugin ids are
/// skipped via a local `HashSet`, so a second call won't re-load
/// the same plugin into the same `LuaHost`.
#[uniffi::export]
pub async fn init_plugins() -> Result<(), MobileError> {
    ensure_default_grants()?;

    let host_lock = get_or_init_host()?;
    let mut host = lock_host(host_lock)?;
    let mut loaded: HashSet<String> = HashSet::new();
    let mut loaded_count = 0usize;

    for plugin in embedded_index().iter() {
        let manifest_str = match std::str::from_utf8(plugin.manifest_bytes) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(plugin = %plugin.id, "manifest not UTF-8: {e}");
                continue;
            }
        };
        let manifest: PluginManifest = match toml::from_str(manifest_str) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(plugin = %plugin.id, "manifest parse failed: {e}");
                continue;
            }
        };
        let caps = &manifest.plugin.capabilities;
        let has_search = caps.get(&Capability::Search).copied().unwrap_or(false);
        let has_lookup = caps.get(&Capability::Lookup).copied().unwrap_or(false);
        if !has_search && !has_lookup {
            // No capability the wizard can use; skip without loading.
            continue;
        }
        if loaded.iter().any(|s| s == &plugin.id) {
            continue;
        }
        let entry_path = &manifest.plugin.entry;
        let Some(plugin_source) = livtet_lua_plugins::read_entry(plugin, entry_path) else {
            tracing::warn!(
                plugin = %plugin.id,
                entry = %entry_path,
                "entry not found in bundled files"
            );
            continue;
        };
        let plugin_id = plugin.id.clone();
        // Gate 1 of the two-gate `host.get_system_secret` check: the
        // manifest must declare `system_secrets = true`. The host
        // exposes `declare_system_secrets` to register that
        // declaration at load time; without this call the Lua-side
        // gate rejects every `get_system_secret` invocation with
        // "system secrets require 'system_secrets = true' in
        // [plugin.requires]" even if the grant sidecar exists.
        if manifest.plugin.has_capability_system_secrets() {
            tracing::info!(plugin = %plugin_id, "declaring system_secrets capability for plugin");
            host.declare_system_secrets(&plugin_id, true);
        }
        match host.load_plugin_source(&plugin_id, &plugin_source, None, None) {
            HostToMain::PluginLoaded { .. } => {
                tracing::info!(plugin = %plugin_id, "bundled plugin loaded for mobile FFI");
                loaded.insert(plugin_id.to_string());
                loaded_count += 1;
            }
            HostToMain::PluginLoadError { error, .. } => {
                tracing::warn!(plugin = %plugin_id, "plugin load failed: {error}");
            }
            other => {
                tracing::warn!(plugin = %plugin_id, "unexpected load response: {other:?}");
            }
        }
    }

    if loaded_count == 0 {
        tracing::warn!("no bundled plugins with search/lookup capabilities were loaded");
    }

    Ok(())
}

/// Look up a single identifier (URN) via the first bundled provider
/// whose `lookup` capability returns a non-null hit. Iterates
/// `embedded_index()` in id order and returns the first hit from a
/// plugin whose manifest's `capabilities.lookup == true` and whose
/// Lua-side `lookup` call returns a non-null
/// `serde_json::Value::Object` (NOT an array, NOT nil — `lookup`
/// returns a single hit table). Returns `Ok(None)` if no provider
/// resolves the URN.
#[uniffi::export]
pub async fn lookup_identifier(urn: String) -> Result<Option<PluginHitMobile>, MobileError> {
    let host_lock = get_or_init_host()?;
    let call_id = ulid::Ulid::new().to_string();
    let args = vec![serde_json::json!(urn)];

    let mut host = lock_host(host_lock)?;
    for plugin in embedded_index().iter() {
        let manifest_str = match std::str::from_utf8(plugin.manifest_bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let manifest: PluginManifest = match toml::from_str(manifest_str) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !manifest
            .plugin
            .capabilities
            .get(&Capability::Lookup)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let plugin_id = plugin.id.clone();
        let result = host.call_capability(&call_id, &plugin_id, "lookup", &args);
        match result {
            HostToMain::CallResult {
                ok: true,
                value: Some(v),
                ..
            } if v.is_object() => {
                return Ok(Some(convert_json_to_hit(v)));
            }
            HostToMain::CallResult {
                ok: false,
                error: Some(e),
                ..
            } => {
                tracing::warn!(plugin = %plugin_id, "plugin lookup returned error: {e}");
            }
            _ => {
                // ok=true, value=None or value=Null or value=Array → no hit, try next.
            }
        }
    }
    Ok(None)
}

/// Priority order for accumulating the most actionable error across
/// providers. When the bridge finds no hits, it surfaces the
/// highest-priority error to the caller — needs_auth beats
/// rate_limited (the user should add an API key) which beats
/// provider_down which beats timeout which beats not_found.
fn error_priority(category: &ProviderErrorCategory) -> u8 {
    match category {
        ProviderErrorCategory::NeedsAuth => 5,
        ProviderErrorCategory::RateLimited => 4,
        ProviderErrorCategory::ProviderDown => 3,
        ProviderErrorCategory::Timeout => 2,
        ProviderErrorCategory::NotFound => 1,
    }
}

/// Inspect a plugin's return value for the `__livtet_error` sentinel
/// shape produced by the bundled plugins' http_get_json helpers.
/// Returns the classified category + retry-after when the sentinel
/// is present, or `None` for a normal result.
fn classify_provider_error(
    value: &serde_json::Value,
) -> Option<(ProviderErrorCategory, Option<u32>)> {
    let obj = value.as_object()?;
    let sentinel = obj.get("__livtet_error")?.as_object()?;
    let category_str = sentinel.get("category")?.as_str()?;
    let category = match category_str {
        "needs_auth" => ProviderErrorCategory::NeedsAuth,
        "rate_limited" => ProviderErrorCategory::RateLimited,
        "timeout" => ProviderErrorCategory::Timeout,
        "not_found" => ProviderErrorCategory::NotFound,
        "provider_down" => ProviderErrorCategory::ProviderDown,
        _ => return None,
    };
    let retry_after = sentinel
        .get("retry_after")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok());
    Some((category, retry_after))
}

/// Search for books by keyword via the first bundled provider whose
/// `search` capability returns a non-empty hit list. Iterates
/// `embedded_index()` in id order and returns the first non-empty
/// result array from a plugin whose manifest's
/// `capabilities.search == true`. An empty array from a `search`
/// plugin is treated as "no hits" and we try the next provider.
///
/// If every provider either errors or returns no hits, the bridge
/// surfaces the highest-priority `__livtet_error` it saw as
/// [`MobileError::ProviderError`]. Priority order:
/// needs_auth > rate_limited > provider_down > timeout > not_found.
/// A successful hit from any provider wins regardless of errors from
/// others — the user sees real results first.
#[uniffi::export]
pub async fn search_providers(query: String) -> Result<Vec<PluginHitMobile>, MobileError> {
    let host_lock = get_or_init_host()?;
    let call_id = ulid::Ulid::new().to_string();
    let args = vec![serde_json::json!(query)];

    let mut host = lock_host(host_lock)?;
    let mut errors: Vec<String> = Vec::new();
    let mut best_category = ProviderErrorCategory::ProviderDown;
    let mut best_retry: Option<u32> = None;
    for plugin in embedded_index().iter() {
        let manifest_str = match std::str::from_utf8(plugin.manifest_bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let manifest: PluginManifest = match toml::from_str(manifest_str) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !manifest
            .plugin
            .capabilities
            .get(&Capability::Search)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let plugin_id = plugin.id.clone();
        let result = host.call_capability(&call_id, &plugin_id, "search", &args);
        match result {
            HostToMain::CallResult {
                ok: true,
                value: Some(v),
                ..
            } if v.is_array() => {
                let arr = v.as_array().cloned().unwrap_or_default();
                if !arr.is_empty() {
                    let hits: Vec<PluginHitMobile> =
                        arr.into_iter().map(convert_json_to_hit).collect();
                    for hit in &hits {
                        tracing::info!(
                            query = %query,
                            plugin = %plugin_id,
                            title = %hit.title,
                            has_cover = hit.cover_url.is_some(),
                            "search hit"
                        );
                    }
                    return Ok(hits);
                }
                tracing::info!(
                    query = %query,
                    plugin = %plugin_id,
                    hits = 0,
                    "search hit"
                );
                // Empty array → try next provider.
            }
            HostToMain::CallResult {
                ok: true,
                value: Some(v),
                ..
            } => {
                // Non-array return (object, scalar). Could be a
                // __livtet_error sentinel; classify it.
                if let Some((category, retry_after)) = classify_provider_error(&v) {
                    let message = v
                        .get("__livtet_error")
                        .and_then(|s| s.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    tracing::warn!(
                        query = %query,
                        plugin = %plugin_id,
                        category = ?category,
                        message = %message,
                        "search error"
                    );
                    let msg = if message.is_empty() {
                        format!("{plugin_id}: {category:?}")
                    } else {
                        format!("{plugin_id}: {message}")
                    };
                    errors.push(msg);
                    if error_priority(&category) > error_priority(&best_category) {
                        best_category = category;
                        best_retry = retry_after;
                    }
                }
                // Otherwise: legitimate non-error non-array return. Try next.
            }
            HostToMain::CallResult {
                ok: false,
                error: Some(e),
                ..
            } => {
                tracing::warn!(
                    query = %query,
                    plugin = %plugin_id,
                    error = %e,
                    "search host error"
                );
                let category = ProviderErrorCategory::ProviderDown;
                errors.push(format!("{plugin_id}: {e}"));
                if error_priority(&category) > error_priority(&best_category) {
                    best_category = category;
                }
            }
            _ => {
                // ok=true, value=None or value=Null → no hits, try next.
            }
        }
    }
    if !errors.is_empty() {
        Err(MobileError::ProviderError {
            category: best_category,
            retry_after_seconds: best_retry,
            provider_id: errors.join("\n"),
        })
    } else {
        Ok(vec![])
    }
}

fn convert_json_to_hit(v: serde_json::Value) -> PluginHitMobile {
    let obj = v.as_object().cloned().unwrap_or_default();
    PluginHitMobile {
        title: obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string(),
        authors: obj
            .get("authors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        identifiers: obj
            .get("identifiers")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
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
            .or_else(|| obj.get("publish_date"))
            .and_then(|v| v.as_str())
            .map(String::from),
        page_count: obj
            .get("page_count")
            .and_then(|v| v.as_u64())
            .map(|n| n as i32),
        language: obj
            .get("language")
            .and_then(|v| v.as_str())
            .map(String::from),
        description: obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        source: obj
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("openlibrary")
            .to_string(),
        source_url: obj
            .get("source_url")
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

#[cfg(test)]
mod tests {
    use livtet_plugin::{embedded_host::EmbeddedHost, mlua::MultiValue};

    use super::*;

    /// After `install_bundled_lua_require`, calling the Lua chunk
    /// `local json = host.require("dkjson"); return json` must yield a
    /// non-nil table that exposes dkjson's `encode` function.
    #[test]
    fn bundled_lua_require_resolves_dkjson() {
        let host_impl = std::sync::Arc::new(EmbeddedHost::new());
        let host = LuaHost::new(host_impl).expect("LuaHost::new");
        install_bundled_lua_require(&host).expect("install require");

        let lua = host.lua();
        let returns: livtet_plugin::mlua::MultiValue = lua
            .load(
                r#"
                local json = host.require("dkjson")
                if type(json) ~= "table" then
                    return "not a table: " .. type(json)
                elseif type(json.encode) ~= "function" then
                    return "no encode function"
                end
                return json.encode({hello = "world"})
                "#,
            )
            .call(())
            .expect("lua call");
        let result = returns.into_iter().next().expect("one return value");
        let Value::String(s) = result else {
            panic!("expected a string return, got {result:?}");
        };
        let s = s.to_str().expect("utf8 string");
        assert!(
            s.contains("\"hello\":\"world\"") || s.contains("\"hello\": \"world\""),
            "dkjson.encode produced unexpected output: {s}"
        );
    }

    /// `host.require("nonexistent")` must surface a Lua error.
    #[test]
    fn bundled_lua_require_errors_on_missing_rock() {
        let host_impl = std::sync::Arc::new(EmbeddedHost::new());
        let host = LuaHost::new(host_impl).expect("LuaHost::new");
        install_bundled_lua_require(&host).expect("install require");

        let lua = host.lua();
        let err = lua
            .load(r#"return host.require("definitely-not-a-real-rock-xyzzy")"#)
            .call::<MultiValue>(())
            .expect_err("missing rock should error");
        let msg = format!("{err}");
        assert!(
            msg.contains("no bundled-lua rock named"),
            "expected 'no bundled-lua rock named' in error, got: {msg}"
        );
    }
}
