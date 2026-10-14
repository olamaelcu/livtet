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

/// Resolve the per-profile data directory: `{data_dir}/{profile_id}`.
pub fn profile_data_dir(profile_id: &str) -> Result<Utf8PathBuf> {
    let base = livtet_core::paths::data_dir().ok_or_else(|| CliError::Operation {
        message: "Could not resolve the livtet data directory".to_string(),
    })?;
    Ok(base.join(profile_id))
}

/// Resolve the per-profile database file: `{data_dir}/{profile_id}/livtet.db`.
pub fn profile_db_path(profile_id: &str) -> Result<Utf8PathBuf> {
    Ok(profile_data_dir(profile_id)?.join("livtet.db"))
}

/// Resolve the per-profile search index directory: `{data_dir}/{profile_id}/search-index`.
pub fn profile_index_dir(profile_id: &str) -> Result<Utf8PathBuf> {
    Ok(profile_data_dir(profile_id)?.join("search-index"))
}

#[cfg(test)]
mod tests {
    use super::{default_db_path, default_index_dir};

    /// Both defaults hang off the same platform data directory, and the
    /// index is a sibling of the database file -- `Context::in_dir` relies on
    /// that layout to stand in for a real install.
    #[test]
    fn the_defaults_are_siblings_with_stable_names() {
        let db = default_db_path().expect("data directory resolves");
        let index = default_index_dir().expect("data directory resolves");

        assert_eq!(db.file_name(), Some("livtet.db"));
        assert_eq!(index.file_name(), Some("search-index"));
        assert_eq!(
            db.parent(),
            index.parent(),
            "the index must sit beside the database"
        );
        assert!(db.is_absolute(), "got {db}");
    }
}