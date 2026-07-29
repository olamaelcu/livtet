use camino::Utf8Path;
use fs_err as fs;
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::repository::error::RepositoryError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoSection {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintainer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SigningSection {
    pub key_label: String,
    pub key_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RepoToml {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    pub repo: RepoSection,
    pub signing: SigningSection,
}

fn default_format_version() -> u32 {
    1
}

pub const SUPPORTED_FORMAT_VERSION: u32 = 1;

pub fn parse_repo_toml(text: &str) -> Result<RepoToml, RepositoryError> {
    let toml: RepoToml =
        toml::from_str(text).map_err(|e| RepositoryError::IndexParse(format!("repo.toml: {e}")))?;
    if toml.format_version != SUPPORTED_FORMAT_VERSION {
        return Err(RepositoryError::IndexParse(format!(
            "unsupported repo.toml format_version {} (expected {})",
            toml.format_version, SUPPORTED_FORMAT_VERSION
        )));
    }
    if toml.repo.name.is_empty() {
        return Err(RepositoryError::IndexParse(
            "repo.name is empty".to_string(),
        ));
    }
    if toml.repo.url.is_empty() {
        return Err(RepositoryError::IndexParse("repo.url is empty".to_string()));
    }
    if !toml.signing.key_fingerprint.starts_with("SHA256:") {
        return Err(RepositoryError::IndexParse(
            "repo.toml: key_fingerprint must start with 'SHA256:'".to_string(),
        ));
    }
    Ok(toml)
}

pub fn render_repo_toml(toml: &RepoToml) -> String {
    toml::to_string_pretty(toml).expect("RepoToml serializable")
}

pub fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn load_from_path(path: &Utf8Path) -> Result<RepoToml, RepositoryError> {
    let text = fs::read_to_string(path).map_err(RepositoryError::Io)?;
    parse_repo_toml(&text)
}
