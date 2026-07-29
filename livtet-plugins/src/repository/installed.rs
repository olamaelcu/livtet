use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{
    archive::manifest::now_iso,
    repository::{
        error::RepositoryError,
        hmac::{HmacKey, read_protected, write_protected},
    },
};

/// One entry in `installed.json`. Mirrors the shape described in
/// the plugin signing/repositories design doc
/// (T19 in the implementation plan).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct InstalledEntry {
    pub id: String,
    pub version: String,
    /// Repository the plugin was installed from, if any. The
    /// `archive install` flow writes `None` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repo: Option<String>,
    /// Absolute path under `providers_dir` where the plugin lives.
    pub install_path: Utf8PathBuf,
    /// ISO-8601 timestamp recorded at install time.
    pub installed_at: String,
}

/// All installed plugin state for a single livtet user. Persisted
/// to `installed.json` under the config dir, HMAC-protected.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct InstalledFile {
    /// One entry per installed `(id, version)`. New versions of the
    /// same plugin appear as additional entries; the host manager
    /// decides which version is "active".
    pub entries: Vec<InstalledEntry>,
    /// Plugin ids the user has disabled. Applies to any plugin
    /// (bundled or disk-installed) that matches the id. Persisted
    /// so the user's choice survives across restarts and app
    /// upgrades that re-bundle the same plugin.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled: BTreeSet<String>,
}

impl InstalledFile {
    /// Load `installed.json` from disk. Returns `Self::default()`
    /// if the file does not yet exist. The HMAC sidecar is verified
    /// when the file exists; tampered data is rejected.
    pub fn load(path: &Utf8Path, key: &HmacKey) -> Result<Self, RepositoryError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = read_protected(path, key)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| RepositoryError::IndexParse(format!("installed.json: {e}")))?;
        serde_json::from_str(text)
            .map_err(|e| RepositoryError::IndexParse(format!("installed.json parse: {e}")))
    }

    /// Persist the file to disk, HMAC-signed via a sidecar.
    pub fn save(&self, path: &Utf8Path, key: &HmacKey) -> Result<(), RepositoryError> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| RepositoryError::IndexParse(format!("installed.json serialize: {e}")))?;
        write_protected(path, text.as_bytes(), key)
    }

    /// Add or replace the entry for `(entry.id, entry.version)`.
    /// Newer `installed_at` wins. Returns `true` if an existing
    /// entry was replaced.
    pub fn upsert(&mut self, entry: InstalledEntry) -> bool {
        for existing in &mut self.entries {
            if existing.id == entry.id && existing.version == entry.version {
                *existing = entry;
                return true;
            }
        }
        self.entries.push(entry);
        false
    }

    /// Mark a plugin id as disabled. No-op if already disabled.
    pub fn disable(&mut self, id: &str) {
        self.disabled.insert(id.to_string());
    }

    /// Mark a plugin id as enabled (removes from the disabled
    /// set). No-op if not currently disabled.
    pub fn enable(&mut self, id: &str) {
        self.disabled.remove(id);
    }

    /// True if the given plugin id is in the disabled set.
    pub fn is_disabled(&self, id: &str) -> bool {
        self.disabled.contains(id)
    }
}

/// Build an `InstalledEntry` from the inputs known at install
/// time. `installed_at` is the current wall-clock time; the other
/// fields are passed through.
pub fn entry_for(
    id: impl Into<String>,
    version: impl Into<String>,
    source_repo: Option<String>,
    install_path: Utf8PathBuf,
) -> InstalledEntry {
    InstalledEntry {
        id: id.into(),
        version: version.into(),
        source_repo,
        install_path,
        installed_at: now_iso(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_replaces_matching_id_version() {
        let mut f = InstalledFile::default();
        let e1 = entry_for(
            "openlibrary",
            "1.0.0",
            None,
            Utf8PathBuf::from("/p/openlibrary/1.0.0"),
        );
        let e2 = entry_for(
            "openlibrary",
            "1.0.0",
            Some("repo-a".to_string()),
            Utf8PathBuf::from("/p/openlibrary/1.0.0"),
        );
        assert!(!f.upsert(e1.clone()));
        assert!(f.upsert(e2.clone()));
        assert_eq!(f.entries.len(), 1);
        assert_eq!(f.entries[0].source_repo.as_deref(), Some("repo-a"));
    }

    #[test]
    fn upsert_adds_separate_versions() {
        let mut f = InstalledFile::default();
        let e1 = entry_for("x", "1.0.0", None, Utf8PathBuf::from("/p/x/1.0.0"));
        let e2 = entry_for("x", "2.0.0", None, Utf8PathBuf::from("/p/x/2.0.0"));
        assert!(!f.upsert(e1));
        assert!(!f.upsert(e2));
        assert_eq!(f.entries.len(), 2);
    }

    #[test]
    fn disable_enable_round_trip() {
        let mut f = InstalledFile::default();
        assert!(!f.is_disabled("openlibrary"));
        f.disable("openlibrary");
        assert!(f.is_disabled("openlibrary"));
        f.enable("openlibrary");
        assert!(!f.is_disabled("openlibrary"));
    }
}
