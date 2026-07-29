//! Canonical on-disk locations for livtet binaries.
//!
//! Centralised so every crate (CLI, plugin host, Tauri parent, FFI
//! bridge) resolves the same paths for data, config, and logs.
//!
//! `BUNDLE_ID` is the OS keyring service identifier and the per-OS
//! subdirectory under the user's data/config roots. The
//! `*_with_migration` helpers also look at the v1 location
//! (`livtet`) so existing installs keep working after the rename.

use camino::Utf8PathBuf;

/// Reverse-DNS bundle id used as the OS keyring service name and as
/// the per-OS subdirectory under the user's data/config roots.
pub const BUNDLE_ID: &str = "net.olamaelcu.livtet";

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
    bundle_data_dir()
        .unwrap_or_else(|| Utf8PathBuf::from("logs"))
        .join("logs")
}

/// Migration-aware data dir: returns the v2 (`<BUNDLE_ID>`) location
/// when available, otherwise falls back to the v1 (`livtet`) location
/// so existing installs keep working.
pub fn data_dir_with_migration() -> Option<Utf8PathBuf> {
    bundle_data_dir().or_else(|| dirs::data_dir().and_then(|p| to_utf8(p.join("livtet"))))
}

/// Migration-aware config dir (v2 first, then v1 `livtet`).
pub fn config_dir_with_migration() -> Option<Utf8PathBuf> {
    bundle_config_dir().or_else(|| dirs::config_dir().and_then(|p| to_utf8(p.join("livtet"))))
}
