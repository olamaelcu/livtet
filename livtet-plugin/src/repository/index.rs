use std::collections::BTreeMap;

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use crate::{keys::signing::verify_bytes, repository::error::RepositoryError};

pub const SUPPORTED_INDEX_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexVersionEntry {
    pub entry: String,
    #[serde(default)]
    pub capabilities: BTreeMap<String, bool>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub archive: String,
    pub archive_size: u64,
    pub archive_sha256: String,
    pub min_app_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct IndexPlugin {
    #[serde(default)]
    pub versions: BTreeMap<String, IndexVersionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Index {
    pub format_version: u32,
    pub generated_at: String,
    pub plugins: BTreeMap<String, IndexPlugin>,
}

pub fn parse_index_json(text: &str) -> Result<Index, RepositoryError> {
    let index: Index = serde_json::from_str(text)
        .map_err(|e| RepositoryError::IndexParse(format!("index.json: {e}")))?;
    if index.format_version != SUPPORTED_INDEX_FORMAT_VERSION {
        return Err(RepositoryError::IndexParse(format!(
            "index.json: unsupported format_version {} (expected {})",
            index.format_version, SUPPORTED_INDEX_FORMAT_VERSION
        )));
    }
    if index.generated_at.is_empty() {
        return Err(RepositoryError::IndexParse(
            "index.json: generated_at is empty".to_string(),
        ));
    }
    Ok(index)
}

pub fn render_index_json(index: &Index) -> String {
    serde_json::to_string_pretty(index).expect("Index serializable")
}

pub fn verify_index_signature(
    _index: &Index,
    index_json_raw_bytes: &str,
    sig_bytes: &[u8],
    verifying_key: &VerifyingKey,
) -> Result<(), RepositoryError> {
    verify_bytes(verifying_key, index_json_raw_bytes.as_bytes(), sig_bytes)
        .map_err(|_| RepositoryError::BadIndexSignature)
}

pub fn find_version<'a>(
    index: &'a Index,
    id: &str,
    version: &str,
) -> Option<&'a IndexVersionEntry> {
    index.plugins.get(id)?.versions.get(version)
}
