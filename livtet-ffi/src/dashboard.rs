//! Dashboard read API (methods on [`LivtetStore`]).
//!
//! Read-only aggregates over reading activity and search history. All
//! timestamps cross the boundary as the crate's RFC 3339 convention.

use livtet_core::data::entities::{
    authors, current_work_status, editions, reading_progress, reading_sessions, search_history,
    work_authors, works,
};
use livtet_core::data::orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use livtet_core::types::{DbId, WorkStatus};

use crate::dto::{DashboardStats, RecentSearch, RecentlyReadBook, ts};
use crate::error::LivtetError;
use crate::store::LivtetStore;

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// Aggregate library and reading-activity statistics.
    pub async fn get_dashboard_stats(&self) -> Result<DashboardStats, LivtetError> {
        let db = self.state.db_conn();

        let total_books = works::Entity::find().count(&db).await? as i64;

        let statuses = current_work_status::Entity::find().all(&db).await?;
        let books_in_progress = statuses
            .iter()
            .filter(|r| r.status == WorkStatus::Reading)
            .count() as i64;
        let finished_books = statuses
            .iter()
            .filter(|r| r.status == WorkStatus::Finished)
            .count() as i64;

        let progress = reading_progress::Entity::find().all(&db).await?;
        let total_reading_time_secs: i64 = progress.iter().map(|p| p.total_reading_time_secs).sum();
        let first_reading_at = progress.iter().map(|p| p.created_at).min();

        Ok(DashboardStats {
            total_books,
            books_in_progress,
            finished_books,
            total_reading_time_secs,
            first_reading_at: first_reading_at.as_ref().map(ts),
        })
    }

    /// Works with recorded reading progress, most recently read first.
    ///
    /// `last_read_at` is the start of the most recent reading session
    /// for the edition, falling back to when progress was first
    /// recorded. `limit` is clamped to at least one.
    pub async fn get_recently_read_books(
        &self,
        limit: u32,
    ) -> Result<Vec<RecentlyReadBook>, LivtetError> {
        let db = self.state.db_conn();

        let rows = reading_progress::Entity::find()
            .order_by(reading_progress::Column::CreatedAt, Order::Desc)
            .limit(Some(limit.max(1) as u64))
            .all(&db)
            .await?;

        let mut out = Vec::with_capacity(rows.len());
        for progress in rows {
            let Some(edition) = editions::Entity::find_by_id(progress.edition_id)
                .one(&db)
                .await?
            else {
                continue;
            };
            let Some(work) = works::Entity::find_by_id(edition.work_id).one(&db).await? else {
                continue;
            };

            let last_session = reading_sessions::Entity::find()
                .filter(reading_sessions::Column::EditionId.eq(progress.edition_id))
                .order_by(reading_sessions::Column::StartedAt, Order::Desc)
                .one(&db)
                .await?;
            let last_read_at = last_session
                .map(|s| s.started_at)
                .unwrap_or(progress.created_at);

            out.push(RecentlyReadBook {
                work_id: work.id,
                edition_id: progress.edition_id,
                title: work.title,
                author_name: first_work_author(&db, work.id).await?,
                progress: progress.progress,
                total_reading_time_secs: progress.total_reading_time_secs,
                last_read_at: ts(&last_read_at),
            });
        }
        Ok(out)
    }

    /// The most recent search-history entries, newest first.
    ///
    /// Nothing in the core writes `search_history` yet, so this returns
    /// whatever rows already exist (seeded or manually inserted).
    pub async fn get_recent_searches(&self, limit: u32) -> Result<Vec<RecentSearch>, LivtetError> {
        let db = self.state.db_conn();
        let rows = search_history::Entity::find()
            .order_by(search_history::Column::SearchedAt, Order::Desc)
            .limit(Some(limit.max(1) as u64))
            .all(&db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| RecentSearch {
                query: r.query,
                searched_at: ts(&r.searched_at),
            })
            .collect())
    }
}

/// First author name of a work, or `None` when it has none.
async fn first_work_author(
    db: &DatabaseConnection,
    work_id: DbId,
) -> Result<Option<String>, LivtetError> {
    let Some(link) = work_authors::Entity::find()
        .filter(work_authors::Column::WorkId.eq(work_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let author = authors::Entity::find_by_id(link.author_id).one(db).await?;
    Ok(author.map(|a| a.name))
}

#[cfg(all(test, feature = "seed"))]
mod tests {
    use std::sync::Arc;

    use livtet_core::data::seed::{SeedConfig, seed_database};
    use livtet_core::types::KnownFormats;

    use super::*;

    async fn seeded_store(num_works: u32) -> (camino_tempfile::Utf8TempDir, Arc<LivtetStore>) {
        let tmp = camino_tempfile::tempdir().unwrap();
        let store = LivtetStore::open(
            tmp.path().join("livtet.db").to_string(),
            tmp.path().join("index").to_string(),
        )
        .await
        .expect("open store");
        seed_database(
            &store.state.db_conn(),
            &SeedConfig {
                num_works,
                ..Default::default()
            },
        )
        .await
        .expect("seed");
        (tmp, store)
    }

    #[tokio::test]
    async fn dashboard_stats_reflect_seed() {
        let (_tmp, store) = seeded_store(3).await;
        let stats = store.get_dashboard_stats().await.unwrap();
        assert_eq!(stats.total_books, i64::from(3));
        assert!(stats.total_reading_time_secs >= 0);
        assert!(stats.books_in_progress <= stats.total_books);
        assert!(stats.finished_books <= stats.total_books);
    }

    #[tokio::test]
    async fn recently_read_tracks_progress_and_sessions() {
        let (_tmp, store) = seeded_store(2).await;
        let work = store.list_works(1, 0, None, None).await.unwrap()[0].clone();
        let edition = store.list_editions(work.id).await.unwrap()[0].id;
        let format = DbId::from(KnownFormats::Epub);

        store
            .record_reading_progress(edition, format, 0.5, None, None, 120)
            .await
            .unwrap();
        store
            .record_reading_session(crate::dto::ReadingSessionInput {
                edition_id: edition,
                format_id: format,
                duration_seconds: 120,
                progress_delta: 0.1,
                last_location: None,
                notes: None,
                started_at: None,
            })
            .await
            .unwrap();

        let recent = store.get_recently_read_books(10).await.unwrap();
        let hit = recent.iter().find(|r| r.work_id == work.id).unwrap();
        assert_eq!(hit.edition_id, edition);
        assert!(!hit.title.is_empty());
        assert_eq!(hit.progress, 0.5);
        assert!(hit.total_reading_time_secs >= 120);
        assert!(!hit.last_read_at.is_empty());
    }

    #[tokio::test]
    async fn recent_searches_reads_history_table() {
        let (_tmp, store) = seeded_store(1).await;
        // No write path exists yet; the contract is an empty-but-valid list.
        let searches = store.get_recent_searches(5).await.unwrap();
        assert!(searches.len() <= 5);
    }
}
