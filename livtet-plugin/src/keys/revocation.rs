use std::collections::HashSet;

use camino::Utf8Path;
use fs_err as fs;
use serde::{Deserialize, Serialize};

use crate::archive::error::ArchiveError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevocationEntry {
    pub fingerprint: String,
    pub reason: String,
    pub revoked_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RevocationList {
    pub entries: Vec<RevocationEntry>,
}

impl RevocationList {
    pub fn fingerprints(&self) -> HashSet<String> {
        self.entries.iter().map(|e| e.fingerprint.clone()).collect()
    }

    pub fn load_or_default(path: &Utf8Path) -> Result<Self, ArchiveError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(path).map_err(|e| {
            ArchiveError::Io(std::io::Error::new(e.kind(), format!("{}: {e}", path)))
        })?;
        toml::from_str(&contents)
            .map_err(|e| ArchiveError::InvalidArchive(format!("revocation-list.toml: {e}")))
    }

    pub fn save(&self, path: &Utf8Path) -> Result<(), ArchiveError> {
        let contents = toml::to_string_pretty(self).map_err(|e| {
            ArchiveError::InvalidArchive(format!("revocation-list.toml serialize: {e}"))
        })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(ArchiveError::Io)?;
        }
        fs::write(path, contents).map_err(ArchiveError::Io)?;
        Ok(())
    }

    pub fn revoke(&mut self, fingerprint: String, reason: String, now_iso: String) {
        if !self.entries.iter().any(|e| e.fingerprint == fingerprint) {
            self.entries.push(RevocationEntry {
                fingerprint,
                reason,
                revoked_at: now_iso,
            });
        }
    }
}
