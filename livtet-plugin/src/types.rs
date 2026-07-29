use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RepositoryAddResult {
    NeedsTofuConfirmation {
        name: String,
        url: String,
        fingerprint: String,
    },
    Ok {
        name: String,
        plugin_count: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RepositoryUpdateResult {
    Ok {
        plugin_count: usize,
    },
    KeyChanged {
        name: String,
        old_fingerprint: String,
        new_fingerprint: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InstallReport {
    pub id: String,
    pub version: String,
    pub signer_label: String,
    pub signer_fingerprint: String,
    pub trusted: bool,
    pub replaced_versions: Vec<String>,
    pub warnings: Vec<String>,
    pub install_path: Utf8PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VerifyReport {
    pub valid: bool,
    pub plugin_id: Option<String>,
    pub version: Option<String>,
    pub signer_key_id: Option<String>,
    pub signer_label: Option<String>,
    pub trusted: Option<bool>,
    pub file_count: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct KeygenReport {
    pub label: String,
    pub key_path: Utf8PathBuf,
    pub pubkey_path: Utf8PathBuf,
    pub fingerprint: String,
    pub encrypted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TrustReport {
    pub label: String,
    pub fingerprint: String,
    pub key_path: Utf8PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TrustedKey {
    pub label: String,
    pub fingerprint: String,
    pub source: TrustedKeySource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum TrustedKeySource {
    Builtin,
    User,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Repository {
    pub name: String,
    pub url: String,
    pub description: Option<String>,
    pub maintainer: Option<String>,
    pub added_at: String,
    pub last_index_update: Option<String>,
    pub key_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RepoSearchResult {
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub repository: String,
    pub relevance_score: f64,
}
