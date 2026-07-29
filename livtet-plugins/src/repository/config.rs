use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{
    archive::manifest::now_iso,
    repository::{
        error::RepositoryError,
        hmac::{HmacKey, read_protected, write_protected},
    },
    types::Repository,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepositoriesFile {
    pub repositories: Vec<Repository>,
}

impl RepositoriesFile {
    pub fn load(path: &Utf8Path, key: &HmacKey) -> Result<Self, RepositoryError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = read_protected(path, key)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| RepositoryError::IndexParse(format!("repositories.toml: {e}")))?;
        toml::from_str(text)
            .map_err(|e| RepositoryError::IndexParse(format!("repositories.toml parse: {e}")))
    }

    pub fn save(&self, path: &Utf8Path, key: &HmacKey) -> Result<(), RepositoryError> {
        let text = toml::to_string_pretty(self).map_err(|e| {
            RepositoryError::IndexParse(format!("repositories.toml serialize: {e}"))
        })?;
        write_protected(path, text.as_bytes(), key)
    }
}

impl Repository {
    #[allow(dead_code)]
    pub fn now_added_at() -> String {
        now_iso()
    }
}
