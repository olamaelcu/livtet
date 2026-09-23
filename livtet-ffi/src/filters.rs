//! Library filter API (methods on [`LivtetStore`]).
//!
//! Read-only: the option lists for the filter UI plus a filtered,
//! paginated work listing. Filtering runs against SQLite (the source of
//! truth) rather than the search index, because format/language are
//! relational facts rather than FRBR text.

use livtet_core::data::entities::{editions, formats, languages, works};
use livtet_core::data::orm::{
    ColumnTrait, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    QueryTrait,
};
use livtet_core::types::WorkFilters;

use crate::dto::{FormatInfo, LibraryLanguage, WorkSummary};
use crate::error::LivtetError;
use crate::store::LivtetStore;
use crate::works::{apply_work_ordering, summarize_works};

/// Page size used by [`LivtetStore::list_works_filtered`] when the
/// filter set carries no explicit limit.
const DEFAULT_PAGE_SIZE: u64 = 50;

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// Formats attached to at least one edition, ordered by name.
    pub async fn list_formats(&self) -> Result<Vec<FormatInfo>, LivtetError> {
        let db = self.state.db_conn();
        let rows = formats::Entity::find()
            .filter(
                formats::Column::Id.in_subquery(
                    editions::Entity::find()
                        .select_only()
                        .column(editions::Column::FormatId)
                        .filter(editions::Column::FormatId.is_not_null())
                        .into_query(),
                ),
            )
            .order_by_asc(formats::Column::Name)
            .all(&db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|f| FormatInfo {
                id: f.id,
                name: f.name,
                metadata_schema: f.metadata_schema.to_string(),
            })
            .collect())
    }

    /// Languages used by at least one edition, ordered by name.
    pub async fn list_languages(&self) -> Result<Vec<LibraryLanguage>, LivtetError> {
        let db = self.state.db_conn();
        let rows = languages::Entity::find()
            .filter(
                languages::Column::Id.in_subquery(
                    editions::Entity::find()
                        .select_only()
                        .column(editions::Column::LanguageId)
                        .filter(editions::Column::LanguageId.is_not_null())
                        .into_query(),
                ),
            )
            .order_by_asc(languages::Column::Name)
            .all(&db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|l| LibraryLanguage {
                id: l.id,
                name: l.name,
                flag_emoji: l.flag_emoji,
            })
            .collect())
    }

    /// Works matching `filters`, ordered and paginated.
    ///
    /// Only `format_ids`, `language_ids`, `sort_by`, `sort_direction`,
    /// and `limit` are honoured; any other filter dimension fails closed
    /// with `InvalidInput` rather than silently returning unfiltered
    /// results.
    pub async fn list_works_filtered(
        &self,
        filters: WorkFilters,
        offset: u32,
    ) -> Result<Vec<WorkSummary>, LivtetError> {
        reject_unsupported(&filters)?;
        let db = self.state.db_conn();

        let ordered = apply_work_ordering(
            filtered_works_query(&filters),
            filters.sort_by,
            filters.sort_direction,
        );
        let models = ordered
            .order_by(works::Column::Id, Order::Asc)
            .limit(Some(filters.limit.unwrap_or(DEFAULT_PAGE_SIZE).max(1)))
            .offset(Some(u64::from(offset)))
            .all(&db)
            .await?;

        summarize_works(&db, models).await
    }

    /// Number of works matching `filters` (ignores `limit`/`offset`).
    pub async fn count_works_filtered(&self, filters: WorkFilters) -> Result<u64, LivtetError> {
        reject_unsupported(&filters)?;
        let db = self.state.db_conn();
        Ok(filtered_works_query(&filters).count(&db).await?)
    }
}

/// Build the `works` query for the supported filter dimensions.
fn filtered_works_query(filters: &WorkFilters) -> livtet_core::data::orm::Select<works::Entity> {
    let mut query = works::Entity::find();
    if !filters.format_ids.is_empty() {
        query = query.filter(
            works::Column::Id.in_subquery(
                editions::Entity::find()
                    .select_only()
                    .column(editions::Column::WorkId)
                    .filter(editions::Column::FormatId.is_in(filters.format_ids.clone()))
                    .into_query(),
            ),
        );
    }
    if !filters.language_ids.is_empty() {
        query = query.filter(
            works::Column::Id.in_subquery(
                editions::Entity::find()
                    .select_only()
                    .column(editions::Column::WorkId)
                    .filter(editions::Column::LanguageId.is_in(filters.language_ids.clone()))
                    .into_query(),
            ),
        );
    }
    query
}

/// Reject filter dimensions this API does not implement yet.
fn reject_unsupported(filters: &WorkFilters) -> Result<(), LivtetError> {
    let dimensions = [
        ("tag_ids", &filters.tag_ids),
        ("genre_ids", &filters.genre_ids),
        ("subject_ids", &filters.subject_ids),
        ("publisher_ids", &filters.publisher_ids),
        ("author_ids", &filters.author_ids),
    ];
    for (name, ids) in dimensions {
        if !ids.is_empty() {
            return Err(LivtetError::InvalidInput(format!(
                "filter dimension `{name}` is not supported yet"
            )));
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "seed"))]
mod tests {
    use std::sync::Arc;

    use livtet_core::data::seed::{SeedConfig, seed_database};

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
    async fn option_lists_reflect_seeded_editions() {
        let (_tmp, store) = seeded_store(4).await;
        let formats = store.list_formats().await.unwrap();
        let languages = store.list_languages().await.unwrap();
        assert!(!formats.is_empty(), "seed attaches at least one format");
        assert!(!languages.is_empty(), "seed attaches at least one language");
        assert!(formats.iter().all(|f| !f.name.is_empty()));
    }

    #[tokio::test]
    async fn filtering_by_format_narrows_the_listing() {
        let (_tmp, store) = seeded_store(5).await;
        let all = store.list_works(100, 0, None, None).await.unwrap();
        let format = store.list_formats().await.unwrap()[0].clone();

        let filtered = store
            .list_works_filtered(
                WorkFilters {
                    format_ids: vec![format.id],
                    ..Default::default()
                },
                0,
            )
            .await
            .unwrap();
        assert!(!filtered.is_empty());
        assert!(filtered.len() <= all.len());
        assert_eq!(
            store
                .count_works_filtered(WorkFilters {
                    format_ids: vec![format.id],
                    ..Default::default()
                })
                .await
                .unwrap(),
            filtered.len() as u64,
        );
    }

    #[tokio::test]
    async fn unsupported_dimension_fails_closed() {
        let (_tmp, store) = seeded_store(1).await;
        let err = store
            .list_works_filtered(
                WorkFilters {
                    tag_ids: vec![livtet_core::types::DbId::new()],
                    ..Default::default()
                },
                0,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::InvalidInput(_)), "{err:?}");
    }
}
