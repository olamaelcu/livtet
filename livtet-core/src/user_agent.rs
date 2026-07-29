//! Shared User-Agent formatting for Rust HTTP clients.
//!
//! Format (mirrored by the Kotlin/Swift platform functions):
//! `<app>/<version> (<platform>; <os>) [+<mode>] <kb-url>`
//!
//! Example (release): `livtet/0.1.0 (desktop; macos) https://livtet.olamaelcu.net/kb/user-agent`
//! Example (debug):  `livtet/0.1.0 (mobile; ios) +debug https://livtet.olamaelcu.net/kb/user-agent`

/// Canonical knowledge-base URL emitted in every Livtet User-Agent.
pub const KB_URL: &str = "https://livtet.olamaelcu.net/kb/user-agent";

/// Format a Livtet User-Agent string.
///
/// `app_name` is the product name (usually `"livtet"`).
/// `version` should be the calling crate's `env!("CARGO_PKG_VERSION")`.
/// `platform` is a coarse platform identifier such as `"desktop"`, `"mobile"`, or `"cli"`.
/// `os` is the operating system identifier, typically `std::env::consts::OS`.
/// `is_debug` controls whether a `+debug` suffix is appended.
pub fn format_user_agent(
    app_name: &str,
    version: &str,
    platform: &str,
    os: &str,
    is_debug: bool,
) -> String {
    let mode = if is_debug { " +debug" } else { "" };
    format!("{app_name}/{version} ({platform}; {os}){mode} {KB_URL}")
}
