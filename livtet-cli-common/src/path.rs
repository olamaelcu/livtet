//! Default on-disk locations used by every livtet front-end.

use camino::Utf8PathBuf;

use crate::{CliError, Result};

/// Resolve the path of the default livtet SQLite database file.
pub fn default_db_path() -> Result<Utf8PathBuf> {
    let dir = livtet_core::paths::data_dir().ok_or_else(|| CliError::Operation {
        message: "Could not resolve the livtet data directory".to_string(),
    })?;
    Ok(dir.join("livtet.db"))
}

/// Resolve the on-disk search index directory (sibling of the DB).
pub fn default_index_dir() -> Result<Utf8PathBuf> {
    let dir = livtet_core::paths::data_dir().ok_or_else(|| CliError::Operation {
        message: "Could not resolve the livtet data directory".to_string(),
    })?;
    Ok(dir.join("search-index"))
}
