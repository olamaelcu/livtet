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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_format_matches_doc_example() {
        assert_eq!(
            format_user_agent("livtet", "0.1.0", "desktop", "macos", false),
            "livtet/0.1.0 (desktop; macos) https://livtet.olamaelcu.net/kb/user-agent"
        );
    }

    #[test]
    fn debug_format_inserts_debug_flag_before_url() {
        assert_eq!(
            format_user_agent("livtet", "0.1.0", "mobile", "ios", true),
            "livtet/0.1.0 (mobile; ios) +debug https://livtet.olamaelcu.net/kb/user-agent"
        );
    }

    #[test]
    fn release_has_no_debug_flag() {
        let ua = format_user_agent("livtet", "0.1.0", "cli", "linux", false);
        assert!(!ua.contains("+debug"), "{ua}");
    }

    #[test]
    fn always_ends_with_kb_url() {
        for ua in [
            format_user_agent("livtet", "0.1.0", "desktop", "macos", false),
            format_user_agent("livtet", "0.1.0", "mobile", "ios", true),
            format_user_agent("other", "9.9.9", "cli", "windows", true),
        ] {
            assert!(ua.ends_with(KB_URL), "{ua}");
        }
    }

    #[test]
    fn passes_through_version_platform_and_os() {
        let ua = format_user_agent("livtet", "2.3.4", "cli", "linux", false);
        assert!(ua.starts_with("livtet/2.3.4 (cli; linux)"), "{ua}");
    }

    #[test]
    fn empty_fields_do_not_panic() {
        let ua = format_user_agent("", "", "", "", false);
        assert_eq!(ua, format!("/ (; ) {KB_URL}"));
    }
}
