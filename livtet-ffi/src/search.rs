//! Search API (methods on [`LivtetStore`]).
//!
//! Thin, fail-closed wrappers over [`livtet_core::search::SearchIndex`]. Hit,
//! facet, and options types come from `livtet-search` directly (built
//! with its `uniffi` feature), so there is one shape across Rust, TS
//! (specta), and the FFI bindings.

use livtet_core::search::model::{FacetedSearchResult, SearchHit, SearchOptions};

use crate::error::LivtetError;
use crate::store::LivtetStore;

fn opts(offset: u32, collapse_to_works: bool) -> SearchOptions {
    SearchOptions {
        offset: offset as i64,
        collapse_to_works,
        ..Default::default()
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// Full-text search over editions (and authors), BM25-ranked.
    /// An empty query string lists everything in index order.
    pub async fn search_editions(
        &self,
        query: String,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SearchHit>, LivtetError> {
        self.index
            .search_with_options(&query, limit as usize, &opts(offset, false))
            .await
            .map_err(|e| LivtetError::Search(e.to_string()))
    }

    /// Like [`Self::search_editions`] but collapses edition hits onto
    /// their work, one hit per work with `grouped_edition_ids`
    /// populated.
    pub async fn search_works(
        &self,
        query: String,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SearchHit>, LivtetError> {
        self.index
            .search_with_options(&query, limit as usize, &opts(offset, true))
            .await
            .map_err(|e| LivtetError::Search(e.to_string()))
    }

    /// Edition search with facet rollups (languages, publishers,
    /// subjects, genres) and a recently-added count. The underlying
    /// index API has no offset — it always returns the top page.
    pub async fn search_with_facets(
        &self,
        query: String,
        limit: u32,
    ) -> Result<FacetedSearchResult, LivtetError> {
        self.index
            .search_with_facets(&query, limit as usize)
            .await
            .map_err(|e| LivtetError::Search(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    async fn seeded_indexed_store(
        num_works: u32,
    ) -> (camino_tempfile::Utf8TempDir, Arc<LivtetStore>) {
        let tmp = camino_tempfile::tempdir().unwrap();
        let store = LivtetStore::open(
            tmp.path().join("livtet.db").to_string(),
            tmp.path().join("index").to_string(),
        )
        .await
        .expect("open store");

        livtet_core::data::seed::seed_database(
            &store.state.db_conn(),
            &livtet_core::data::seed::SeedConfig {
                num_works,
                ..Default::default()
            },
        )
        .await
        .expect("seed");
        store
            .index
            .reindex(&store.state.db_conn())
            .await
            .expect("reindex");
        (tmp, store)
    }

    #[tokio::test]
    async fn search_editions_lists_all_with_empty_query() {
        let (_tmp, store) = seeded_indexed_store(2).await;
        let hits = store.search_editions(String::new(), 100, 0).await.unwrap();
        assert!(!hits.is_empty(), "index is seeded and non-empty");
        assert!(hits.iter().all(|h| !h.work_id.is_empty()));
    }

    #[tokio::test]
    async fn search_works_collapses_to_unique_works() {
        let (_tmp, store) = seeded_indexed_store(2).await;
        let editions = store.search_editions(String::new(), 100, 0).await.unwrap();
        let works = store.search_works(String::new(), 100, 0).await.unwrap();
        let unique_works: std::collections::HashSet<_> =
            works.iter().map(|h| h.work_id.clone()).collect();
        assert_eq!(unique_works.len(), works.len(), "works are deduplicated");
        assert!(works.len() <= editions.len());
        assert_eq!(unique_works.len(), 2, "two seeded works");
    }

    #[tokio::test]
    async fn search_with_facets_returns_rollups() {
        let (_tmp, store) = seeded_indexed_store(2).await;
        let result = store.search_with_facets(String::new(), 100).await.unwrap();
        assert!(!result.hits.is_empty());
        // Facet buckets may be empty depending on seed content; the
        // contract is the shape, not specific counts.
        assert!(result.recently_added >= 0);
    }
}
