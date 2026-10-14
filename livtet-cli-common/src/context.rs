//! Where a command reads and writes.
//!
//! Commands used to resolve the database and index straight from
//! [`livtet_core::paths`] at every call site, which made them untestable:
//! there was no way to point one at anywhere but the real install. A
//! [`Context`] is resolved once at the CLI boundary and threaded in, so a
//! test can hand a command a temp directory instead.

use camino::Utf8PathBuf;

use crate::Result;
use crate::path::{default_db_path, default_index_dir, profile_db_path, profile_index_dir};

/// The database file and search index a command operates on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    /// Path to the SQLite database file.
    pub db_path: Utf8PathBuf,
    /// Directory holding the Tantivy index.
    pub index_dir: Utf8PathBuf,
    /// Profile identifier for scoped paths (optional).
    pub profile_id: Option<String>,
}

impl Context {
    /// Resolve from the platform's data directory — what every binary front
    /// end uses.
    pub fn from_defaults(profile_id: Option<&str>) -> Result<Self> {
        let (db_path, index_dir) = match profile_id {
            Some(profile_id) => (profile_db_path(profile_id)?, profile_index_dir(profile_id)?),
            None => (default_db_path()?, default_index_dir()?),
        };
        Ok(Self {
            db_path,
            index_dir,
            profile_id: profile_id.map(|s| s.to_string()),
        })
    }

    /// Point at `dir`, laid out the way an install is: the index a sibling of
    /// the database file. Useful for tests and for any caller driving a
    /// throwaway catalog.
    pub fn in_dir(dir: impl Into<Utf8PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            db_path: dir.join("livtet.db"),
            index_dir: dir.join("search-index"),
            profile_id: None,
        }
    }

    /// Read-only connection URL. Query commands use this so a stray write
    /// fails loudly instead of mutating the catalog.
    pub fn db_url_ro(&self) -> String {
        format!("sqlite:{}?mode=ro", self.db_path)
    }

    /// Read-write connection URL, creating the file if absent.
    pub fn db_url_rwc(&self) -> String {
        format!("sqlite:{}?mode=rwc", self.db_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_dir_lays_the_index_beside_the_database() {
        let ctx = Context::in_dir("/tmp/livtet-example");
        assert_eq!(ctx.db_path, "/tmp/livtet-example/livtet.db");
        assert_eq!(ctx.index_dir, "/tmp/livtet-example/search-index");
        assert_eq!(
            ctx.db_path.parent(),
            ctx.index_dir.parent(),
            "index must be a sibling of the database file"
        );
    }

    #[test]
    fn connection_urls_carry_their_modes() {
        let ctx = Context::in_dir("/tmp/livtet-example");
        assert_eq!(
            ctx.db_url_ro(),
            "sqlite:/tmp/livtet-example/livtet.db?mode=ro"
        );
        assert_eq!(
            ctx.db_url_rwc(),
            "sqlite:/tmp/livtet-example/livtet.db?mode=rwc"
        );
    }
}