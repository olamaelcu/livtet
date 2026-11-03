//! Works and editions read API (methods on [`LivtetStore`]).

use std::collections::HashMap;

use livtet_core::data::entities::{
    authors, current_work_status, digital_inventory, edition_authors, edition_identifiers,
    edition_publishers, editions, formats, identifiers, languages, publishers, work_authors, works,
};
use livtet_core::data::orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Select,
};
use livtet_core::types::{
    DbId, Identifier, IdentifierKind, SortDirection, Urn, WorkSortBy, WorkStatus,
};

use crate::dto::{EditionDetail, EditionFile, EditionSummary, WorkSummary, ts, ts_opt};
use crate::error::LivtetError;
use crate::store::LivtetStore;

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// List works, with optional sort: `sort_by`/`sort_direction`
    /// select the field and direction (defaults: title ascending).
    /// `WorkSortBy::NewestCap` always means `created_at DESC`
    /// (the "newly added" view).
    pub async fn list_works(
        &self,
        limit: u32,
        offset: u32,
        sort_by: Option<WorkSortBy>,
        sort_direction: Option<SortDirection>,
    ) -> Result<Vec<WorkSummary>, LivtetError> {
        let db = self.state.db_conn();

        let ordered = apply_work_ordering(works::Entity::find(), sort_by, sort_direction);

        let models = ordered
            .order_by(works::Column::Id, Order::Asc)
            .limit(Some(limit.max(1) as u64))
            .offset(Some(offset as u64))
            .all(&db)
            .await?;

        summarize_works(&db, models).await
    }

    /// List the editions of one work, newest first.
    pub async fn list_editions(&self, work_id: DbId) -> Result<Vec<EditionSummary>, LivtetError> {
        let db = self.state.db_conn();

        let models = editions::Entity::find()
            .filter(editions::Column::WorkId.eq(work_id))
            .order_by(editions::Column::CreatedAt, Order::Desc)
            .order_by(editions::Column::Id, Order::Asc)
            .all(&db)
            .await?;

        let summaries = summarize_editions(&db, models).await?;
        Ok(summaries)
    }

    /// Fetch one edition with all of its related data. `Ok(None)` when
    /// no edition with this id exists.
    pub async fn get_edition(&self, id: DbId) -> Result<Option<EditionDetail>, LivtetError> {
        let db = self.state.db_conn();

        let Some(model) = editions::Entity::find_by_id(id).one(&db).await? else {
            return Ok(None);
        };

        Ok(Some(edition_detail(&db, model).await?))
    }

    /// Identifiers attached to one edition, ordered by URN.
    ///
    /// `identifiers` rows are shared library-wide, so the same URN may
    /// appear on several editions.
    pub async fn list_edition_identifiers(
        &self,
        edition_id: DbId,
    ) -> Result<Vec<Identifier>, LivtetError> {
        let db = self.state.db_conn();

        let links = edition_identifiers::Entity::find()
            .filter(edition_identifiers::Column::EditionId.eq(edition_id))
            .all(&db)
            .await?;
        let ids: Vec<DbId> = links.iter().map(|l| l.identifier_id).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = identifiers::Entity::find()
            .filter(identifiers::Column::Id.is_in(ids))
            .order_by(identifiers::Column::Value, Order::Asc)
            .all(&db)
            .await?;
        Ok(rows.iter().map(to_identifier).collect())
    }

    /// Editions carrying a given identifier — the dedupe hook for
    /// imports: one ISBN that is already in the library yields the
    /// editions that hold it.
    pub async fn find_editions_by_identifier(
        &self,
        identifier: Identifier,
    ) -> Result<Vec<EditionSummary>, LivtetError> {
        let db = self.state.db_conn();

        let Some(row) = identifiers::Entity::find()
            .filter(identifiers::Column::Value.eq(identifier.as_urn_string()))
            .one(&db)
            .await?
        else {
            return Ok(Vec::new());
        };

        let links = edition_identifiers::Entity::find()
            .filter(edition_identifiers::Column::IdentifierId.eq(row.id))
            .all(&db)
            .await?;
        let ids: Vec<DbId> = links.iter().map(|l| l.edition_id).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let models = editions::Entity::find()
            .filter(editions::Column::Id.is_in(ids))
            .order_by(editions::Column::CreatedAt, Order::Desc)
            .all(&db)
            .await?;
        summarize_editions(&db, models).await
    }

    /// Total number of works.
    pub async fn count_works(&self) -> Result<u64, LivtetError> {
        let db = self.state.db_conn();
        Ok(works::Entity::find().count(&db).await?)
    }
}

/// Apply the shared work-list ordering: title ascending by default,
/// with `WorkSortBy::NewestCap` always meaning `created_at DESC`.
pub(crate) fn apply_work_ordering(
    query: Select<works::Entity>,
    sort_by: Option<WorkSortBy>,
    sort_direction: Option<SortDirection>,
) -> Select<works::Entity> {
    match sort_by {
        // NewestCap's semantic is fixed: newest first, direction ignored
        // (see WorkFilters::effective_limit docs).
        Some(WorkSortBy::NewestCap) => query.order_by(works::Column::CreatedAt, Order::Desc),
        other => {
            let column = match other.unwrap_or(WorkSortBy::Title) {
                WorkSortBy::CreatedAt | WorkSortBy::NewestCap => works::Column::CreatedAt,
                WorkSortBy::Title => works::Column::Title,
                WorkSortBy::UpdatedAt => works::Column::UpdatedAt,
            };
            let direction = match sort_direction.unwrap_or_default() {
                SortDirection::Asc => Order::Asc,
                SortDirection::Desc => Order::Desc,
            };
            query.order_by(column, direction)
        }
    }
}

/// Convert work models into [`WorkSummary`]s, batching author lookups
/// over the whole page.
pub(crate) async fn summarize_works(
    db: &DatabaseConnection,
    models: Vec<works::Model>,
) -> Result<Vec<WorkSummary>, LivtetError> {
    let ids: Vec<DbId> = models.iter().map(|w| w.id).collect();
    let authors_by_work = author_names_for_works(db, &ids).await?;
    let covers_by_work = cover_urls_for_works(db, &ids).await?;
    let status_by_work = work_statuses(db, &ids).await?;

    Ok(models
        .into_iter()
        .map(|w| {
            let authors = authors_by_work.get(&w.id).cloned().unwrap_or_default();
            WorkSummary {
                id: w.id,
                title: w.title,
                sort_title: w.sort_title,
                description: w.description,
                authors,
                cover_url: covers_by_work.get(&w.id).cloned(),
                status: status_by_work.get(&w.id).copied(),
                created_at: ts(&w.created_at),
                updated_at: ts_opt(w.updated_at),
            }
        })
        .collect())
}

/// Cover URL per work.
///
/// A catalog cover wins over a local one: it is the image the edition was
/// imported with, and imported editions have no file to take a cover from.
/// Local covers come from `digital_inventory.cover_path`, the same column the
/// `edition_covers` view unions.
async fn cover_urls_for_works(
    db: &DatabaseConnection,
    work_ids: &[DbId],
) -> Result<HashMap<DbId, String>, LivtetError> {
    let mut out: HashMap<DbId, String> = HashMap::new();
    if work_ids.is_empty() {
        return Ok(out);
    }

    let editions_of_work = editions::Entity::find()
        .filter(editions::Column::WorkId.is_in(work_ids.iter().copied()))
        .order_by_asc(editions::Column::CreatedAt)
        .all(db)
        .await?;

    for edition in &editions_of_work {
        if let Some(url) = edition.cover_image_url.as_deref().filter(|u| !u.is_empty()) {
            out.entry(edition.work_id)
                .or_insert_with(|| url.to_string());
        }
    }

    let without_remote: HashMap<DbId, DbId> = editions_of_work
        .iter()
        .filter(|e| !out.contains_key(&e.work_id))
        .map(|e| (e.id, e.work_id))
        .collect();
    if without_remote.is_empty() {
        return Ok(out);
    }

    let local = digital_inventory::Entity::find()
        .filter(digital_inventory::Column::EditionId.is_in(without_remote.keys().copied()))
        .all(db)
        .await?;
    for row in local {
        let Some(work_id) = without_remote.get(&row.edition_id) else {
            continue;
        };
        if let Some(path) = row.cover_path.filter(|p| !p.is_empty()) {
            out.entry(*work_id).or_insert(path);
        }
    }

    Ok(out)
}

/// Current reading status per work; works never shelved are simply absent.
async fn work_statuses(
    db: &DatabaseConnection,
    work_ids: &[DbId],
) -> Result<HashMap<DbId, WorkStatus>, LivtetError> {
    if work_ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(current_work_status::Entity::find()
        .filter(current_work_status::Column::WorkId.is_in(work_ids.iter().copied()))
        .all(db)
        .await?
        .into_iter()
        .map(|m| (m.work_id, m.status))
        .collect())
}

/// Author names for a set of work ids, batched in two queries.
async fn author_names_for_works(
    db: &livtet_core::data::orm::DatabaseConnection,
    work_ids: &[DbId],
) -> Result<HashMap<DbId, Vec<String>>, LivtetError> {
    let mut out: HashMap<DbId, Vec<String>> = HashMap::new();
    if work_ids.is_empty() {
        return Ok(out);
    }

    let links = work_authors::Entity::find()
        .filter(work_authors::Column::WorkId.is_in(work_ids.iter().copied()))
        .all(db)
        .await?;

    let author_ids: Vec<DbId> = links.iter().map(|l| l.author_id).collect();
    let author_names: HashMap<DbId, String> = authors::Entity::find()
        .filter(authors::Column::Id.is_in(author_ids))
        .all(db)
        .await?
        .into_iter()
        .map(|a| (a.id, a.name))
        .collect();

    for link in links {
        if let Some(name) = author_names.get(&link.author_id) {
            out.entry(link.work_id).or_default().push(name.clone());
        }
    }
    Ok(out)
}

/// Author names for one edition (via `edition_authors`).
async fn author_names_for_edition(
    db: &livtet_core::data::orm::DatabaseConnection,
    edition_id: DbId,
) -> Result<Vec<String>, LivtetError> {
    let links = edition_authors::Entity::find()
        .filter(edition_authors::Column::EditionId.eq(edition_id))
        .all(db)
        .await?;

    let ids: Vec<DbId> = links.iter().map(|l| l.author_id).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let authors = authors::Entity::find()
        .filter(authors::Column::Id.is_in(ids))
        .all(db)
        .await?;
    Ok(authors.into_iter().map(|a| a.name).collect())
}

/// Rebuild an [`Identifier`] from an `identifiers` row: `kind` holds the
/// stored scheme string and `value` the canonical URN.
fn to_identifier(row: &identifiers::Model) -> Identifier {
    let kind = IdentifierKind::parse(&row.kind)
        .unwrap_or_else(|| IdentifierKind::Custom(row.kind.clone()));
    let urn =
        Urn::parse(&row.value).unwrap_or_else(|_| Urn::new(row.kind.clone(), row.value.clone()));
    Identifier { kind, urn }
}

/// Convert edition models into summaries, batching format/language/file
/// lookups over the whole page.
async fn summarize_editions(
    db: &livtet_core::data::orm::DatabaseConnection,
    models: Vec<editions::Model>,
) -> Result<Vec<EditionSummary>, LivtetError> {
    if models.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<DbId> = models.iter().map(|e| e.id).collect();
    let format_ids: Vec<DbId> = models.iter().filter_map(|e| e.format_id).collect();
    let language_ids: Vec<DbId> = models.iter().filter_map(|e| e.language_id).collect();

    let formats: HashMap<DbId, String> = formats::Entity::find()
        .filter(formats::Column::Id.is_in(format_ids))
        .all(db)
        .await?
        .into_iter()
        .map(|f| (f.id, f.name))
        .collect();

    let languages: HashMap<DbId, String> = languages::Entity::find()
        .filter(languages::Column::Id.is_in(language_ids))
        .all(db)
        .await?
        .into_iter()
        .map(|l| (l.id, l.code))
        .collect();

    let with_files: std::collections::HashSet<DbId> = digital_inventory::Entity::find()
        .filter(digital_inventory::Column::EditionId.is_in(ids))
        .filter(digital_inventory::Column::FilePath.is_not_null())
        .all(db)
        .await?
        .into_iter()
        .map(|d| d.edition_id)
        .collect();

    Ok(models
        .into_iter()
        .map(|e| EditionSummary {
            has_file: with_files.contains(&e.id),
            format: e.format_id.and_then(|id| formats.get(&id).cloned()),
            language_code: e.language_id.and_then(|id| languages.get(&id).cloned()),
            id: e.id,
            work_id: e.work_id,
            title: e.title,
            created_at: ts(&e.created_at),
            updated_at: ts_opt(e.updated_at),
        })
        .collect())
}

/// Assemble a full [`EditionDetail`] from the edition row plus its
/// batched relations.
async fn edition_detail(
    db: &livtet_core::data::orm::DatabaseConnection,
    model: editions::Model,
) -> Result<EditionDetail, LivtetError> {
    let mut summaries = summarize_editions(db, vec![model.clone()]).await?;
    let summary = summaries.pop().expect("one edition in, one summary out");

    let publishers: Vec<String> = {
        let links = edition_publishers::Entity::find()
            .filter(edition_publishers::Column::EditionId.eq(model.id))
            .all(db)
            .await?;
        let ids: Vec<DbId> = links.iter().map(|l| l.publisher_id).collect();
        if ids.is_empty() {
            Vec::new()
        } else {
            publishers::Entity::find()
                .filter(publishers::Column::Id.is_in(ids))
                .all(db)
                .await?
                .into_iter()
                .map(|p| p.name)
                .collect()
        }
    };

    let identifiers: Vec<String> = {
        let links = edition_identifiers::Entity::find()
            .filter(edition_identifiers::Column::EditionId.eq(model.id))
            .all(db)
            .await?;
        let ids: Vec<DbId> = links.iter().map(|l| l.identifier_id).collect();
        if ids.is_empty() {
            Vec::new()
        } else {
            identifiers::Entity::find()
                .filter(identifiers::Column::Id.is_in(ids))
                .all(db)
                .await?
                .into_iter()
                .map(|i| i.value)
                .collect()
        }
    };

    let file = digital_inventory::Entity::find()
        .filter(digital_inventory::Column::EditionId.eq(model.id))
        .one(db)
        .await?
        .map(|d| EditionFile {
            file_path: d
                .file_path
                .map(|p| livtet_core::types::DiskPath::from_path(camino::Utf8Path::new(&p))),
            cover_path: d.cover_path,
            blurhash: d.blurhash,
            dominant_color: d.dominant_color,
            file_hash: d.file_hash,
            file_size_bytes: d.file_size_bytes,
            file_format: d.file_format,
        });

    Ok(EditionDetail {
        id: summary.id,
        work_id: summary.work_id,
        title: summary.title,
        published_date: model.published_date.map(|d| {
            livtet_core::types::PublishedDate::YearMonthDay {
                year: d.year(),
                month: d.month() as u8,
                day: d.day(),
            }
        }),
        format: summary.format,
        language_code: summary.language_code,
        notes: model.notes,
        description: model.description,
        authors: author_names_for_edition(db, model.id).await?,
        publishers,
        identifiers,
        file,
        created_at: summary.created_at,
        updated_at: summary.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seeded_store(
        num_works: u32,
    ) -> (camino_tempfile::Utf8TempDir, std::sync::Arc<LivtetStore>) {
        let tmp = camino_tempfile::tempdir().unwrap();
        let store = LivtetStore::open(
            tmp.path().join("livtet.db").to_string(),
            tmp.path().join("index").to_string(),
        )
        .await
        .expect("open store");

        let db = store.state.db_conn();
        livtet_core::data::seed::seed_database(
            &db,
            &livtet_core::data::seed::SeedConfig {
                num_works,
                ..Default::default()
            },
        )
        .await
        .expect("seed");
        (tmp, store)
    }

    #[tokio::test]
    async fn list_works_returns_seeded_works() {
        let (_tmp, store) = seeded_store(3).await;
        let works = store.list_works(100, 0, None, None).await.expect("list");
        assert_eq!(works.len(), 3);
        assert!(works.iter().all(|w| !w.title.is_empty()));
        assert_eq!(store.count_works().await.unwrap(), 3);
    }

    #[tokio::test]
    async fn list_works_paginates() {
        let (_tmp, store) = seeded_store(5).await;
        let all = store.list_works(100, 0, None, None).await.unwrap();
        let page = store.list_works(2, 0, None, None).await.unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id, all[0].id);
        let rest = store.list_works(2, 2, None, None).await.unwrap();
        assert_eq!(rest[0].id, all[2].id);
        assert_eq!(rest[1].id, all[3].id);
    }

    #[tokio::test]
    async fn list_editions_and_detail_roundtrip() {
        let (_tmp, store) = seeded_store(2).await;
        let works = store.list_works(100, 0, None, None).await.unwrap();
        let work = &works[0];

        let editions = store.list_editions(work.id).await.unwrap();
        assert!(!editions.is_empty(), "seeded work has editions");

        let detail = store
            .get_edition(editions[0].id)
            .await
            .unwrap()
            .expect("edition exists");
        assert_eq!(detail.id, editions[0].id);
        assert_eq!(detail.work_id, work.id);
    }

    #[tokio::test]
    async fn get_edition_unknown_id_is_none() {
        let (_tmp, store) = seeded_store(1).await;
        assert!(store.get_edition(DbId::new()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_works_newest_cap_orders_by_created_desc() {
        let (_tmp, store) = seeded_store(5).await;

        // Give each work a distinct created_at, then verify ordering
        // through the FFI method rather than raw rows.
        let by_title = store.list_works(100, 0, None, None).await.unwrap();
        let db = store.state.db_conn();
        for (i, work) in by_title.iter().enumerate() {
            use livtet_core::data::orm::{ActiveModelTrait, EntityTrait, Set};
            let mut active: livtet_core::data::entities::works::ActiveModel =
                livtet_core::data::entities::works::Entity::find_by_id(work.id)
                    .one(&db)
                    .await
                    .unwrap()
                    .unwrap()
                    .into();
            active.created_at = Set(time::PrimitiveDateTime::new(
                time::Date::from_calendar_date(2020, time::Month::January, (i + 1) as u8).unwrap(),
                time::Time::MIDNIGHT,
            ));
            active.update(&db).await.unwrap();
        }

        let newest = store
            .list_works(
                100,
                0,
                Some(livtet_core::types::WorkSortBy::NewestCap),
                None,
            )
            .await
            .unwrap();
        // ordering: index 4 (latest day) first, descending.
        assert_eq!(newest[0].id, by_title[4].id);
        assert_eq!(newest[4].id, by_title[0].id);
    }
}
