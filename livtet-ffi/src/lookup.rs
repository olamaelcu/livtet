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

use livtet_plugins::permissions::permissions_dir;

use crate::MobileError;

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
    let sanitized: HashMap<String, String> =
        secrets.into_iter().filter(|(_, v)| !v.is_empty()).collect();
    let _ = sanitized;
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
    Ok(())
}

/// Safe to call multiple times — already-loaded plugin ids are
/// skipped via a local `HashSet`, so a second call won't re-load
/// the same plugin into the same `LuaHost`.
#[uniffi::export]
pub async fn init_plugins() -> Result<(), MobileError> {
    ensure_default_grants()?;

    // TBD: bundled Lua plugin iteration removed alongside the
    // `livtet_lua_plugins` crate. Restore once the bundling pipeline lands.
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
    let _urn = urn;

    // TBD: bundled Lua plugin iteration removed alongside the
    // `livtet_lua_plugins` crate. Restore once the bundling pipeline lands.
    Ok(None)
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
    let _query = query;

    // TBD: bundled Lua plugin iteration removed alongside the
    // `livtet_lua_plugins` crate. Restore once the bundling pipeline lands.
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
