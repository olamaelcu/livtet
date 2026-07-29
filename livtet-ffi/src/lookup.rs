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

use livtet_plugins::{
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
        // TBD: the in-process bundled Lua stdlib (`livtet_lua_stdlib`)
        // and bundled Lua plugins (`livtet_lua_plugins`) were removed.
        // Their `host.require` resolver is gone; restore the lookup
        // once a replacement bundling pipeline lands.
        LuaHostWrapper(Mutex::new(host))
    });
    Ok(&wrapper.0)
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
/// [`PluginSystemSecret`](livtet_plugins::system_secrets::PluginSystemSecret)
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

    
    // TBD: bundled Lua plugin iteration removed alongside the
    // `livtet_lua_plugins` crate. Restore once the bundling pipeline lands.
    panic!("TBD: bundled plugin iteration removed; see livtet-ffi/src/lookup.rs");


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

    
    // TBD: bundled Lua plugin iteration removed alongside the
    // `livtet_lua_plugins` crate. Restore once the bundling pipeline lands.
    panic!("TBD: bundled plugin iteration removed; see livtet-ffi/src/lookup.rs");


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
    
    // TBD: bundled Lua plugin iteration removed alongside the
    // `livtet_lua_plugins` crate. Restore once the bundling pipeline lands.
    panic!("TBD: bundled plugin iteration removed; see livtet-ffi/src/lookup.rs");

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
    
    // TBD: bundled Lua plugin iteration removed alongside the
    // `livtet_lua_plugins` crate. Restore once the bundling pipeline lands.
    panic!("TBD: bundled plugin iteration removed; see livtet-ffi/src/lookup.rs");

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
    use livtet_plugins::embedded_host::EmbeddedHost;

    use super::*;

    // TBD: `install_bundled_lua_require` and the in-process bundled
    // Lua stdlib (`livtet_lua_stdlib`) were removed when those stub
    // crates were deleted. The two tests in this module that exercise
    // `host.require("dkjson")` and `host.require("nonexistent")` are
    // skipped at compile time. Restore once the bundling pipeline
    // returns.
}
