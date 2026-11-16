//! Canonical on-disk locations for livtet binaries.
//!
//! Centralised so every crate (CLI, plugin host, Tauri parent, FFI
//! bridge) resolves the same paths for data, config, and logs.
//!
//! `BUNDLE_ID` is the per-OS subdirectory under the user's data and
//! config roots. Debug builds select the `.dev` variant, so the
//! resolved path names the build kind (`.../net.olamaelcu.livtet.dev`
//! vs `.../net.olamaelcu.livtet`). The selection is
//! `cfg(debug_assertions)`: it follows the Cargo profile that
//! compiled this crate, not a runtime flag, so `cargo run`,
//! `cargo test`, and `tauri dev` are dev while `cargo build
//! --release` and packaged builds are production.

use camino::Utf8PathBuf;

/// Reverse-DNS bundle id used as the per-OS subdirectory under the
/// user's data and config roots. Debug builds use the `.dev` variant
/// so development data cannot collide with an installed release and
/// the path itself identifies the build kind.
#[cfg(not(debug_assertions))]
pub const BUNDLE_ID: &str = "net.olamaelcu.livtet";

/// Debug-build variant; see the release definition above.
#[cfg(debug_assertions)]
pub const BUNDLE_ID: &str = "net.olamaelcu.livtet.dev";

/// Subdirectory names appended under the data dir.
pub mod subdirs {
    /// Plugin repository cache (cloned `index.json`, downloaded
    /// plugin archives).
    pub const REPOS: &str = "repos";
    /// Installed plugin providers (the per-`<id>/<version>` tree).
    pub const PROVIDERS: &str = "providers";
    /// Resolved plugin permission grants.
    pub const PERMISSIONS: &str = "permissions";
}

/// Seam between `dirs` (which speaks `std::path::PathBuf`) and the
/// project's `camino` path vocabulary.
#[allow(clippy::disallowed_types)]
fn to_utf8(p: std::path::PathBuf) -> Option<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(p).ok()
}

fn bundle_data_dir() -> Option<Utf8PathBuf> {
    dirs::data_dir().and_then(|p| to_utf8(p.join(BUNDLE_ID)))
}

fn bundle_config_dir() -> Option<Utf8PathBuf> {
    dirs::config_dir().and_then(|p| to_utf8(p.join(BUNDLE_ID)))
}

/// Bundle-scoped data directory (`$XDG_DATA_HOME/<BUNDLE_ID>` on Linux,
/// `~/Library/Application Support/<BUNDLE_ID>` on macOS, `%APPDATA%\<BUNDLE_ID>`
/// on Windows). Returns `None` when the platform does not expose a
/// data-dir concept.
pub fn data_dir() -> Option<Utf8PathBuf> {
    bundle_data_dir()
}

/// Bundle-scoped config directory (`$XDG_CONFIG_HOME/<BUNDLE_ID>` on Linux,
/// `~/Library/Application Support/<BUNDLE_ID>` on macOS). Returns
/// `None` when the platform does not expose a config-dir concept.
pub fn config_dir() -> Option<Utf8PathBuf> {
    bundle_config_dir()
}

/// Per-bundle logs directory. Falls back to `<cwd>/logs` when the
/// platform does not expose a data dir.
pub fn logs_dir() -> Utf8PathBuf {
    bundle_data_dir().unwrap_or_default().join("logs")
}
