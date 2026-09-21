//! `LivtetStore` — the FFI facade object.
//!
//! One object owns both the SQLite pool (via [`SharedState`]) and the
//! Tantivy [`SearchIndex`]. The object is the *only* way to reach the
//! library from foreign code: there is no global state on this side of
//! the boundary, so no call can happen before [`LivtetStore::open`]
//! succeeds. `close` (and `Drop`, as a backstop) release the pool.

use std::sync::Arc;

use camino::Utf8Path;
use livtet_data::SharedState;
use livtet_data::migrator::Kind;
use livtet_search::index::SearchIndex;

use crate::error::LivtetError;

/// Handle to an open livtet library (SQLite database + search index).
#[derive(uniffi::Object)]
pub struct LivtetStore {
    pub(crate) state: SharedState,
    pub(crate) index: SearchIndex,
}

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// Open (creating and migrating if needed) the SQLite database at
    /// `db_path` and the search index at `index_dir`, and return a
    /// ready store handle. Both paths are created if missing.
    ///
    /// NOTE: this refers to opening *different* paths. Tantivy holds an
    /// exclusive writer lock on the index dir, so two handles onto the
    /// *same* paths cannot coexist — see the `same_paths_...` test.
    #[uniffi::constructor]
    pub async fn open(db_path: String, index_dir: String) -> Result<Arc<Self>, LivtetError> {
        let state = SharedState::connect(&db_path, &[Kind::Business]).await?;
        let index = SearchIndex::open(Utf8Path::new(&index_dir))?;
        Ok(Arc::new(Self { state, index }))
    }

    /// Flush SQLite (`PRAGMA optimize`) and close the connection pool.
    /// Subsequent use of this handle will fail at the database layer.
    pub async fn close(&self) -> Result<(), LivtetError> {
        self.state.optimize_and_close().await?;
        Ok(())
    }
}

impl Drop for LivtetStore {
    fn drop(&mut self) {
        // Best-effort pool shutdown when the foreign side drops its
        // last handle without calling `close` first.
        let pool = self.state.pool.clone();
        drop(tokio::task::spawn(async move { pool.close().await }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_db_and_index() {
        let tmp = camino_tempfile::tempdir().unwrap();
        let db = tmp.path().join("livtet.db");
        let index_dir = tmp.path().join("search-index");

        let store = LivtetStore::open(db.to_string(), index_dir.to_string())
            .await
            .expect("open");

        assert!(db.exists(), "database file should be created");
        assert!(index_dir.exists(), "index dir should be created");

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn two_stores_are_independent() {
        let tmp_a = camino_tempfile::tempdir().unwrap();
        let tmp_b = camino_tempfile::tempdir().unwrap();

        let store_a = LivtetStore::open(
            tmp_a.path().join("a.db").to_string(),
            tmp_a.path().join("index").to_string(),
        )
        .await
        .expect("open a");
        let store_b = LivtetStore::open(
            tmp_b.path().join("b.db").to_string(),
            tmp_b.path().join("index").to_string(),
        )
        .await
        .expect("open b");

        assert_eq!(store_a.state.db_path, tmp_a.path().join("a.db"));
        assert_eq!(store_b.state.db_path, tmp_b.path().join("b.db"));

        store_a.close().await.unwrap();
        store_b.close().await.unwrap();
    }

    #[tokio::test]
    async fn same_paths_conflict_on_the_index_lock() {
        let tmp = camino_tempfile::tempdir().unwrap();
        let db = tmp.path().join("livtet.db").to_string();
        let index = tmp.path().join("index").to_string();

        let first = LivtetStore::open(db.clone(), index.clone()).await.unwrap();
        let err = match LivtetStore::open(db, index).await {
            Err(e) => e,
            Ok(_) => panic!("second open on the same index must fail (tantivy lock)"),
        };
        assert!(matches!(err, LivtetError::Search(_)), "{err:?}");

        first.close().await.unwrap();
    }
}
