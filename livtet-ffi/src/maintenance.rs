//! Maintenance API: index rebuilds and the seed/reset helpers used by
//! mobile smoke tests (methods on [`LivtetStore`]).

use std::sync::Arc;

#[cfg(feature = "seed")]
use livtet_core::data::orm::EntityTrait;
use livtet_core::search::write::ReindexEvent;

use crate::error::LivtetError;
use crate::store::LivtetStore;

/// Progress event during [`LivtetStore::reindex`], mirroring
/// [`livtet_core::search::ReindexEvent`].
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum ReindexProgressEvent {
    /// Index loaded; DB rows are being read.
    Loading,
    /// A batch was committed; `done`/`total` are document counts.
    Indexing { done: u64, total: u64 },
}

impl From<ReindexEvent> for ReindexProgressEvent {
    fn from(ev: ReindexEvent) -> Self {
        match ev {
            ReindexEvent::Loading => Self::Loading,
            ReindexEvent::Indexing { done, total } => Self::Indexing { done, total },
        }
    }
}

/// Foreign-side progress listener for [`LivtetStore::reindex`].
/// Implementations live in Kotlin/Swift and are called on a worker
/// coroutine/thread — they must dispatch to the UI thread themselves.
#[uniffi::export(with_foreign)]
pub trait ReindexProgress: Send + Sync {
    fn on_event(&self, event: ReindexProgressEvent);
}

/// Counters returned by [`LivtetStore::seed_sample_data`] and
/// [`LivtetStore::reset_and_seed`]. Non-negative.
#[cfg(feature = "seed")]
#[derive(Debug, Clone, uniffi::Record)]
pub struct SeedStats {
    pub works_created: u32,
    pub editions_created: u32,
    pub authors_created: u32,
    pub publishers_created: u32,
    pub reading_status_count: u32,
    pub annotations_created: u32,
    pub digital_inventory_created: u32,
    pub loans_created: u32,
    pub reading_sessions_created: u32,
    pub saved_searches_created: u32,
    pub reading_lists_created: u32,
}

/// Content tables wiped by `reset_and_seed`, junction-first so foreign
/// keys never see a dangling parent. Reference dictionaries (formats,
/// languages, genres, subjects, tags) survive: seeds are idempotent.
#[cfg(feature = "seed")]
macro_rules! delete_all {
    ($db:expr, $($entity:path),+ $(,)?) => {
        $(
            <$entity>::delete_many().exec($db).await?;
        )+
    };
}

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// Rebuild the whole search index from the database. Pass a
    /// listener to receive [`ReindexProgressEvent`]s.
    pub async fn reindex(
        &self,
        progress: Option<Arc<dyn ReindexProgress>>,
    ) -> Result<(), LivtetError> {
        let db = self.state.db_conn();
        self.index
            .reindex_with_progress(&db, &mut |ev| {
                if let Some(cb) = &progress {
                    cb.on_event(ev.into());
                }
            })
            .await
            .map_err(|e| LivtetError::Search(e.to_string()))
    }
}

/// Seed/reset helpers used by mobile smoke tests. Gated behind the
/// `seed` feature so release builds can drop the `fake` code path.
#[cfg(feature = "seed")]
#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// Seed sample data (idempotent: a second call with the same
    /// arguments reports `works_created: 0`).
    pub async fn seed_sample_data(&self, num_works: u32) -> Result<SeedStats, LivtetError> {
        let result = livtet_core::data::seed::seed_database(
            &self.state.db_conn(),
            &livtet_core::data::seed::SeedConfig {
                num_works,
                ..Default::default()
            },
        )
        .await?;
        Ok(stats_from(&result))
    }

    /// Wipe the library content (keeping schema and reference
    /// dictionaries), seed `num_works` works, and rebuild the search
    /// index. Intended for smoke tests and demo data, not production
    /// flows.
    pub async fn reset_and_seed(&self, num_works: u32) -> Result<SeedStats, LivtetError> {
        use livtet_core::data::entities::*;
        let db = self.state.db_conn();

        delete_all!(
            &db,
            work_authors::Entity,
            work_tags::Entity,
            work_genres::Entity,
            work_subjects::Entity,
            work_publishers::Entity,
            work_identifiers::Entity,
            edition_authors::Entity,
            edition_tags::Entity,
            edition_genres::Entity,
            edition_subjects::Entity,
            edition_publishers::Entity,
            edition_identifiers::Entity,
            series_entries::Entity,
            reading_list_book::Entity,
            reading_progress::Entity,
            reading_sessions::Entity,
            annotations::Entity,
            editions_loans::Entity,
            owned_edition::Entity,
            digital_inventory::Entity,
            edition_specific_covers::Entity,
            current_work_status::Entity,
            saved_search::Entity,
            search_history::Entity,
            loan_entity_identifier::Entity,
            loan_entity::Entity,
            edition_groups::Entity,
            reading_lists::Entity,
            editions::Entity,
            works::Entity,
            authors::Entity,
            publishers::Entity,
            identifiers::Entity,
            series::Entity,
        );

        let stats = self.seed_sample_data(num_works).await?;
        self.reindex(None).await?;
        Ok(stats)
    }
}

#[cfg(feature = "seed")]
fn stats_from(r: &livtet_core::data::seed::SeedResult) -> SeedStats {
    SeedStats {
        works_created: r.works_created,
        editions_created: r.editions_created,
        authors_created: r.authors_created,
        publishers_created: r.publishers_created,
        reading_status_count: r.reading_status_count,
        annotations_created: r.annotations_created,
        digital_inventory_created: r.digital_inventory_created,
        loans_created: r.loans_created,
        reading_sessions_created: r.reading_sessions_created,
        saved_searches_created: r.saved_searches_created,
        reading_lists_created: r.reading_lists_created,
    }
}

#[cfg(all(test, feature = "seed"))]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct CollectEvents {
        events: Mutex<Vec<ReindexProgressEvent>>,
    }

    impl ReindexProgress for CollectEvents {
        fn on_event(&self, event: ReindexProgressEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn reindex_reports_progress_to_listener() {
        let tmp = camino_tempfile::tempdir().unwrap();
        let store = LivtetStore::open(
            tmp.path().join("livtet.db").to_string(),
            tmp.path().join("index").to_string(),
        )
        .await
        .unwrap();
        store.seed_sample_data(2).await.unwrap();

        let listener = Arc::new(CollectEvents {
            events: Mutex::new(Vec::new()),
        });
        store.reindex(Some(listener.clone())).await.unwrap();

        let events = listener.events.lock().unwrap();
        assert!(!events.is_empty(), "listener received progress events");
        let terminal = events
            .iter()
            .filter_map(|e| match e {
                ReindexProgressEvent::Indexing { done, total } => Some((done, total)),
                _ => None,
            })
            .next_back();
        assert!(terminal.is_some(), "indexing events were emitted");
        if let Some((done, total)) = terminal {
            assert_eq!(done, total, "last event reaches the total");
        }
    }

    #[tokio::test]
    async fn reset_and_seed_replaces_content() {
        let tmp = camino_tempfile::tempdir().unwrap();
        let store = LivtetStore::open(
            tmp.path().join("livtet.db").to_string(),
            tmp.path().join("index").to_string(),
        )
        .await
        .unwrap();

        store.seed_sample_data(2).await.unwrap();
        assert_eq!(store.count_works().await.unwrap(), 2);

        store.reset_and_seed(5).await.unwrap();
        assert_eq!(store.count_works().await.unwrap(), 5);

        // Index was rebuilt: all five works are searchable.
        let hits = store.search_works(String::new(), 100, 0).await.unwrap();
        let unique: std::collections::HashSet<_> = hits.iter().map(|h| &h.work_id).collect();
        assert_eq!(unique.len(), 5);
    }

    #[tokio::test]
    async fn seeding_twice_is_a_noop() {
        let tmp = camino_tempfile::tempdir().unwrap();
        let store = LivtetStore::open(
            tmp.path().join("livtet.db").to_string(),
            tmp.path().join("index").to_string(),
        )
        .await
        .unwrap();

        store.seed_sample_data(2).await.unwrap();
        let again = store.seed_sample_data(2).await.unwrap();
        assert_eq!(again.works_created, 0);
    }
}
