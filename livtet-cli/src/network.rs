//! CLI HTTP agent configuration.
//!
//! The command-line build uses the same centralized User-Agent formatter as
//! the desktop and mobile builds, but identifies itself as `cli` so server
//! logs can distinguish beamplug/console traffic.

use std::{sync::OnceLock, time::Duration};

use reqwest::Client;

static USER_AGENT: OnceLock<String> = OnceLock::new();

fn build_user_agent() -> String {
    livtet_core::user_agent::format_user_agent(
        "livtet",
        env!("CARGO_PKG_VERSION"),
        "cli",
        std::env::consts::OS,
        cfg!(debug_assertions),
    )
}

/// The CLI User-Agent string.
pub fn user_agent() -> &'static str {
    USER_AGENT.get_or_init(build_user_agent)
}

/// Build a `reqwest::Client` with the CLI User-Agent and no explicit timeout.
pub fn agent() -> Client {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .build()
        .expect("CLI HTTP agent should build with default config")
}

/// Build a `reqwest::Client` with the CLI User-Agent and a custom timeout.
pub fn agent_with_timeout(timeout: Duration) -> Client {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(timeout)
        .build()
        .expect("CLI HTTP agent should build with default config")
}
