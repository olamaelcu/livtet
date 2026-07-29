//! System secrets the host recognizes.
//!
//! System secrets are compile-time-injected credentials that the host
//! owns and only exposes to plugins read-only. They are sourced from
//! the project's SOPS bundle (see ADR 0032 for the build-time wiring)
//! and registered at host boot under the canonical reverse-DNS key
//! below. Plugins access them via `host.get_system_secret(name)`,
//! which goes through `PluginSystemSecret::from_str` so any unknown
//! key string is rejected at the bridge.

use strum::{AsRefStr, Display, EnumString, IntoStaticStr};

/// Canonical, statically-known system secrets. The string form is
/// snake-case (`strum(serialize_all = "snake_case")`) so it is stable
/// across the Lua boundary, FFI HashMap keys, and the future addition
/// of a Kotlin/Swift-generated constants module.
///
/// `EnumString` provides the `FromStr` impl for free; we do not write
/// a custom one to avoid colliding with the derived `FromStr`. Unknown
/// names surface to callers as `strum::ParseError`, which the Lua
/// bridge maps to the user-facing
/// `"unknown system secret: <name>"` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, AsRefStr, Display, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum PluginSystemSecret {
    /// `google_books_api_key` — Google Books API key, baked in from
    /// SOPS at compile time on desktop (Tauri) and from the
    /// per-platform SOPS-routed `BuildConfig.*` value on Android
    /// and iOS. The Lua plugin `bundled/googlebooks` consumes this.
    GoogleBooksApiKey,

    /// `platform_unauthenticated_allowed` — `"true"` only on Android
    /// `fdroid` and `generic` flavors, indicating the plugin may
    /// fall back to per-IP unauthenticated requests when the API
    /// key is absent. Never registered on desktop or iOS.
    PlatformUnauthenticatedAllowed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_books_api_key_round_trips_through_strings() {
        let v = PluginSystemSecret::GoogleBooksApiKey;
        assert_eq!(v.as_ref(), "google_books_api_key");
        assert_eq!(v.to_string(), "google_books_api_key");
        let parsed: PluginSystemSecret = "google_books_api_key".parse().unwrap();
        assert_eq!(parsed, PluginSystemSecret::GoogleBooksApiKey);
    }

    #[test]
    fn platform_unauthenticated_allowed_round_trips() {
        let v = PluginSystemSecret::PlatformUnauthenticatedAllowed;
        assert_eq!(v.as_ref(), "platform_unauthenticated_allowed");
        assert_eq!(v.to_string(), "platform_unauthenticated_allowed");
        let parsed: PluginSystemSecret = "platform_unauthenticated_allowed".parse().unwrap();
        assert_eq!(parsed, PluginSystemSecret::PlatformUnauthenticatedAllowed);
    }

    #[test]
    fn unknown_string_rejects_at_parse_time() {
        let result: Result<PluginSystemSecret, _> = "not_a_real_secret".parse();
        assert!(result.is_err());
    }
}
