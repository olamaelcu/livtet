use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::archive::error::ArchiveError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveMeta {
    pub format_version: u32,
    pub plugin_id: String,
    pub plugin_version: String,
    pub created_at: String,
    pub signed_by: String,
    pub tool: String,
}

pub const SUPPORTED_FORMAT_VERSION: u32 = 1;

pub fn parse_archive_toml(text: &str) -> Result<ArchiveMeta, ArchiveError> {
    let meta: ArchiveMeta = toml::from_str(text)
        .map_err(|e| ArchiveError::InvalidArchive(format!("archive.toml parse: {e}")))?;
    if meta.format_version != SUPPORTED_FORMAT_VERSION {
        return Err(ArchiveError::UnsupportedFormat {
            version: meta.format_version,
        });
    }
    OffsetDateTime::parse(&meta.created_at, &Rfc3339)
        .map_err(|e| ArchiveError::InvalidArchive(format!("archive.toml created_at: {e}")))?;
    Ok(meta)
}

pub fn render_archive_toml(meta: &ArchiveMeta) -> String {
    toml::to_string_pretty(meta).expect("ArchiveMeta serializable")
}

pub fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
