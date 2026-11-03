//! Edition mutation API (methods on [`LivtetStore`]).
//!
//! Every mutation writes through SQLite (the source of truth) and then
//! refreshes the edition's search-index document so reads via
//! `search_*` see the change immediately.

use livtet_core::data::entities::{
    digital_inventory, edition_identifiers, editions, identifiers, works,
};
use livtet_core::data::orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use livtet_core::types::{DbId, DiskPath, Identifier, PublishedDate, now_primitive};

use crate::dto::{EditionDetail, WorkSummary};
use crate::error::LivtetError;
use crate::store::LivtetStore;

/// `set-if-Some` patch for edition fields. Fields left as `None` are
/// not modified. (Clearing a field is not exposed yet; set an empty
/// string where the UI needs that.)
#[derive(Debug, Clone, uniffi::Record)]
pub struct EditionPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    /// Only the full year-month-day variant can be stored; other
    /// precisions fail `InvalidInput`.
    pub published_date: Option<PublishedDate>,
    pub format_id: Option<DbId>,
    pub language_id: Option<DbId>,
}

/// Fields accepted when creating an edition. Everything but the owning
/// work is optional, mirroring the `editions` table.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NewEdition {
    pub work_id: DbId,
    pub title: Option<String>,
    pub description: Option<String>,
    pub published_date: Option<PublishedDate>,
    pub format_id: Option<DbId>,
    pub language_id: Option<DbId>,
    pub notes: Option<String>,
    /// Catalog cover image URL; local covers live in `digital_inventory`.
    pub cover_image_url: Option<String>,
}

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// Attach (or replace) the digital file of an edition. Creates the
    /// 1:1 `digital_inventory` row on first use. Fails `NotFound`
    /// when the edition does not exist.
    pub async fn set_edition_file(
        &self,
        edition_id: DbId,
        file_path: DiskPath,
        file_format: Option<String>,
        file_size_bytes: Option<i64>,
    ) -> Result<(), LivtetError> {
        let db = self.state.db_conn();
        require_edition(&db, edition_id).await?;

        let existing = digital_inventory::Entity::find()
            .filter(digital_inventory::Column::EditionId.eq(edition_id))
            .one(&db)
            .await?;

        match existing {
            Some(model) => {
                let mut active: digital_inventory::ActiveModel = model.into();
                active.file_path = Set(Some(file_path.to_string()));
                if file_format.is_some() {
                    active.file_format = Set(file_format);
                }
                if file_size_bytes.is_some() {
                    active.file_size_bytes = Set(file_size_bytes);
                }
                active.updated_at = Set(Some(now_primitive()));
                active.update(&db).await?;
            }
            None => {
                let active = digital_inventory::ActiveModel {
                    id: Set(DbId::new()),
                    edition_id: Set(edition_id),
                    file_path: Set(Some(file_path.to_string())),
                    file_format: Set(file_format),
                    file_size_bytes: Set(file_size_bytes),
                    cover_path: Set(None),
                    blurhash: Set(None),
                    dominant_color: Set(None),
                    file_hash: Set(None),
                    notes: Set(None),
                    added_at: Set(now_primitive()),
                    updated_at: Set(None),
                };
                digital_inventory::Entity::insert(active).exec(&db).await?;
            }
        }

        self.sync_edition_index(edition_id).await
    }

    /// Detach the digital file by deleting the edition's
    /// `digital_inventory` row. Returns `false` when the edition had
    /// no file (or does not exist).
    pub async fn remove_edition_file(&self, edition_id: DbId) -> Result<bool, LivtetError> {
        let db = self.state.db_conn();

        let removed = digital_inventory::Entity::delete_many()
            .filter(digital_inventory::Column::EditionId.eq(edition_id))
            .exec(&db)
            .await?
            .rows_affected
            > 0;

        if removed {
            self.sync_edition_index(edition_id).await?;
        }
        Ok(removed)
    }

    /// Attach an identifier to an edition. Idempotent per (edition,
    /// URN): existing rows are reused, so duplicate calls do not
    /// create duplicate junctions. Fails `NotFound` for a missing
    /// edition, `InvalidInput` bubbles up from the lifted
    /// [`Identifier`] URN at the FFI boundary.
    pub async fn add_edition_identifier(
        &self,
        edition_id: DbId,
        identifier: Identifier,
    ) -> Result<String, LivtetError> {
        let db = self.state.db_conn();
        require_edition(&db, edition_id).await?;

        attach_identifier(&db, edition_id, &identifier).await?;
        self.sync_edition_index(edition_id).await?;
        Ok(identifier.as_urn_string())
    }

    /// Create a work. `works.title` is `NOT NULL`, so a blank title
    /// fails `InvalidInput` rather than inserting an unnamed row.
    pub async fn create_work(
        &self,
        title: String,
        sort_title: Option<String>,
        description: Option<String>,
    ) -> Result<WorkSummary, LivtetError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(LivtetError::InvalidInput(
                "work title must not be empty".to_string(),
            ));
        }

        let db = self.state.db_conn();
        let model = works::ActiveModel {
            id: Set(DbId::new()),
            title: Set(title),
            description: Set(description.filter(|d| !d.trim().is_empty())),
            sort_title: Set(sort_title.filter(|s| !s.trim().is_empty())),
            series_type: Set(None),
            language_id: Set(None),
            preferred_edition_id: Set(None),
            created_at: Set(now_primitive()),
            updated_at: Set(None),
        }
        .insert(&db)
        .await?;

        let mut summaries = crate::works::summarize_works(&db, vec![model]).await?;
        summaries
            .pop()
            .ok_or_else(|| LivtetError::InvalidInput("work insert produced no row".to_string()))
    }

    /// Create an edition under an existing work. Fails `NotFound` for a
    /// missing work; every other field is optional, like the table.
    pub async fn create_edition(&self, input: NewEdition) -> Result<EditionDetail, LivtetError> {
        let db = self.state.db_conn();
        require_work(&db, input.work_id).await?;

        let published_date = input.published_date.map(full_date).transpose()?;

        let id = DbId::new();
        editions::ActiveModel {
            id: Set(id),
            work_id: Set(input.work_id),
            group_id: Set(None),
            title: Set(input.title.filter(|t| !t.trim().is_empty())),
            published_date: Set(published_date),
            format_id: Set(input.format_id),
            language_id: Set(input.language_id),
            notes: Set(input.notes.filter(|n| !n.trim().is_empty())),
            description: Set(input.description.filter(|d| !d.trim().is_empty())),
            format_metadata: Set(None),
            cover_image_url: Set(input.cover_image_url.filter(|u| !u.trim().is_empty())),
            created_at: Set(now_primitive()),
            updated_at: Set(None),
        }
        .insert(&db)
        .await?;

        self.get_edition(id)
            .await?
            .ok_or_else(|| LivtetError::NotFound {
                entity: "editions".to_string(),
                id: id.to_string(),
            })
    }

    /// Replace one identifier of an edition with another.
    ///
    /// `from` is only detached from *this* edition, so a URN shared with
    /// other editions stays attached to them. When `to` already exists
    /// library-wide the existing `identifiers` row is reused.
    pub async fn update_edition_identifier(
        &self,
        edition_id: DbId,
        from: Identifier,
        to: Identifier,
    ) -> Result<Identifier, LivtetError> {
        let db = self.state.db_conn();
        require_edition(&db, edition_id).await?;

        detach_identifier(&db, edition_id, &from.as_urn_string()).await?;
        attach_identifier(&db, edition_id, &to).await?;
        self.sync_edition_index(edition_id).await?;
        Ok(to)
    }

    /// Detach an identifier from an edition, returning whether a link
    /// existed. The shared `identifiers` row is left in place: other
    /// editions carrying the same URN still reference it.
    pub async fn remove_edition_identifier(
        &self,
        edition_id: DbId,
        identifier: Identifier,
    ) -> Result<bool, LivtetError> {
        let db = self.state.db_conn();
        let removed = detach_identifier(&db, edition_id, &identifier.as_urn_string()).await?;
        if removed {
            self.sync_edition_index(edition_id).await?;
        }
        Ok(removed)
    }

    /// Apply a [`EditionPatch`] and return the refreshed detail.
    pub async fn update_edition(
        &self,
        edition_id: DbId,
        patch: EditionPatch,
    ) -> Result<EditionDetail, LivtetError> {
        let db = self.state.db_conn();

        // Partial precision can't be stored: the column is a full
        // `time::Date` or NULL. Fail closed.
        let published_date = patch.published_date.map(full_date).transpose()?.map(Some);

        let model = require_edition(&db, edition_id).await?;
        let mut active: editions::ActiveModel = model.into();
        if let Some(title) = patch.title {
            active.title = Set(if title.is_empty() { None } else { Some(title) });
        }
        if let Some(description) = patch.description {
            active.description = Set(if description.is_empty() {
                None
            } else {
                Some(description)
            });
        }
        if let Some(notes) = patch.notes {
            active.notes = Set(if notes.is_empty() { None } else { Some(notes) });
        }
        if let Some(date) = published_date {
            active.published_date = Set(date);
        }
        if let Some(format_id) = patch.format_id {
            active.format_id = Set(Some(format_id));
        }
        if let Some(language_id) = patch.language_id {
            active.language_id = Set(Some(language_id));
        }
        active.updated_at = Set(Some(now_primitive()));
        active.update(&db).await?;

        self.sync_edition_index(edition_id).await?;

        self.get_edition(edition_id)
            .await?
            .map(Ok)
            .unwrap_or_else(|| {
                Err(LivtetError::NotFound {
                    entity: "editions".to_string(),
                    id: edition_id.to_string(),
                })
            })
    }
}

/// Shared index refresh after a mutation to one edition.
impl LivtetStore {
    pub(crate) async fn sync_edition_index(&self, edition_id: DbId) -> Result<(), LivtetError> {
        self.index
            .add_edition(&self.state.db_conn(), edition_id)
            .await
            .map_err(|e| LivtetError::Search(e.to_string()))
    }
}

pub(crate) async fn require_edition(
    db: &livtet_core::data::orm::DatabaseConnection,
    edition_id: DbId,
) -> Result<editions::Model, LivtetError> {
    editions::Entity::find_by_id(edition_id)
        .one(db)
        .await?
        .ok_or_else(|| LivtetError::NotFound {
            entity: "editions".to_string(),
            id: edition_id.to_string(),
        })
}

async fn require_work(
    db: &livtet_core::data::orm::DatabaseConnection,
    work_id: DbId,
) -> Result<works::Model, LivtetError> {
    works::Entity::find_by_id(work_id)
        .one(db)
        .await?
        .ok_or_else(|| LivtetError::NotFound {
            entity: "works".to_string(),
            id: work_id.to_string(),
        })
}

/// Attach `identifier` to `edition_id`, reusing the shared
/// `identifiers` row for the URN when one already exists and skipping
/// the junction when it is already linked. Returns the identifier id.
async fn attach_identifier(
    db: &livtet_core::data::orm::DatabaseConnection,
    edition_id: DbId,
    identifier: &Identifier,
) -> Result<DbId, LivtetError> {
    let value = identifier.as_urn_string();
    let kind = identifier.kind.as_str().to_string();

    let identifier_id = match identifiers::Entity::find()
        .filter(identifiers::Column::Value.eq(&value))
        .one(db)
        .await?
    {
        Some(row) => row.id,
        None => {
            let id = DbId::new();
            identifiers::Entity::insert(identifiers::ActiveModel {
                id: Set(id),
                value: Set(value),
                kind: Set(kind),
            })
            .exec(db)
            .await?;
            id
        }
    };

    let linked = edition_identifiers::Entity::find_by_id((edition_id, identifier_id))
        .one(db)
        .await?;
    if linked.is_none() {
        edition_identifiers::Entity::insert(edition_identifiers::ActiveModel {
            edition_id: Set(edition_id),
            identifier_id: Set(identifier_id),
        })
        .exec(db)
        .await?;
    }

    Ok(identifier_id)
}

/// Remove the link between `edition_id` and the identifier URN, leaving
/// the shared `identifiers` row (and every other edition's link) alone.
async fn detach_identifier(
    db: &livtet_core::data::orm::DatabaseConnection,
    edition_id: DbId,
    urn: &str,
) -> Result<bool, LivtetError> {
    let Some(row) = identifiers::Entity::find()
        .filter(identifiers::Column::Value.eq(urn))
        .one(db)
        .await?
    else {
        return Ok(false);
    };

    let affected = edition_identifiers::Entity::delete_many()
        .filter(edition_identifiers::Column::EditionId.eq(edition_id))
        .filter(edition_identifiers::Column::IdentifierId.eq(row.id))
        .exec(db)
        .await?
        .rows_affected;
    Ok(affected > 0)
}

/// Convert a [`PublishedDate`] into the full date the `editions` column
/// stores, filling in the first day of whatever precision is missing.
fn full_date(published_date: PublishedDate) -> Result<time::Date, LivtetError> {
    // The column holds a full `time::Date`, so precision the caller did not
    // supply degrades to the first day of the period they did: a bare year
    // becomes January 1 of that year, a year-month the first of that month.
    let (year, month, day) = match published_date {
        PublishedDate::YearMonthDay { year, month, day } => (year, month, day),
        PublishedDate::YearMonth { year, month } => (year, month, 1),
        PublishedDate::Year(year) => (year, 1, 1),
    };
    let month = time::Month::try_from(month)
        .map_err(|e| LivtetError::InvalidInput(format!("invalid month: {e}")))?;
    time::Date::from_calendar_date(year, month, day)
        .map_err(|e| LivtetError::InvalidInput(format!("invalid calendar date: {e}")))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    async fn seeded_store_with_edition() -> (camino_tempfile::Utf8TempDir, Arc<LivtetStore>, DbId) {
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
                num_works: 2,
                ..Default::default()
            },
        )
        .await
        .expect("seed");

        let work = &store.list_works(1, 0, None, None).await.unwrap()[0];
        let edition = store.list_editions(work.id).await.unwrap()[0].id;
        (tmp, store, edition)
    }

    #[tokio::test]
    async fn set_and_remove_edition_file() {
        let (_tmp, store, edition) = seeded_store_with_edition().await;

        store
            .set_edition_file(
                edition,
                DiskPath::from_path(camino::Utf8Path::new("/books/a.epub")),
                Some("epub".to_string()),
                Some(1024),
            )
            .await
            .unwrap();

        let detail = store.get_edition(edition).await.unwrap().unwrap();
        let file = detail.file.expect("file attached");
        assert_eq!(file.file_format.as_deref(), Some("epub"));
        assert_eq!(file.file_size_bytes, Some(1024));
        assert_eq!(
            file.file_path.unwrap().to_string(),
            "/books/a.epub".to_string()
        );

        assert!(store.remove_edition_file(edition).await.unwrap());
        let detail = store.get_edition(edition).await.unwrap().unwrap();
        assert!(detail.file.is_none());
        // Second removal reports no-op.
        assert!(!store.remove_edition_file(edition).await.unwrap());
    }

    #[tokio::test]
    async fn add_identifier_is_idempotent() {
        let (_tmp, store, edition) = seeded_store_with_edition().await;
        let urn = "urn:isbn:9780306406157";
        let id1 = Identifier::parse(urn).unwrap();

        assert_eq!(
            store.add_edition_identifier(edition, id1).await.unwrap(),
            urn
        );
        let again = Identifier::parse(urn).unwrap();
        store.add_edition_identifier(edition, again).await.unwrap();

        let detail = store.get_edition(edition).await.unwrap().unwrap();
        let matches = detail.identifiers.iter().filter(|i| *i == urn).count();
        assert_eq!(matches, 1, "identifier appears exactly once");
    }

    #[tokio::test]
    async fn update_edition_patches_and_fills_partial_dates() {
        let (_tmp, store, edition) = seeded_store_with_edition().await;

        let patch = EditionPatch {
            title: Some("Patched Title".to_string()),
            description: None,
            notes: None,
            published_date: Some(PublishedDate::YearMonthDay {
                year: 1989,
                month: 9,
                day: 3,
            }),
            format_id: None,
            language_id: None,
        };
        let detail = store.update_edition(edition, patch).await.unwrap();
        assert_eq!(detail.title.as_deref(), Some("Patched Title"));
        assert_eq!(
            detail.published_date,
            Some(PublishedDate::YearMonthDay {
                year: 1989,
                month: 9,
                day: 3
            })
        );

        // Partial precision degrades to the first day of the known period.
        let year_only = EditionPatch {
            title: None,
            description: None,
            notes: None,
            published_date: Some(PublishedDate::Year(1989)),
            format_id: None,
            language_id: None,
        };
        let detail = store.update_edition(edition, year_only).await.unwrap();
        assert_eq!(
            detail.published_date,
            Some(PublishedDate::YearMonthDay {
                year: 1989,
                month: 1,
                day: 1
            })
        );

        let month_only = EditionPatch {
            title: None,
            description: None,
            notes: None,
            published_date: Some(PublishedDate::YearMonth {
                year: 2001,
                month: 7,
            }),
            format_id: None,
            language_id: None,
        };
        let detail = store.update_edition(edition, month_only).await.unwrap();
        assert_eq!(
            detail.published_date,
            Some(PublishedDate::YearMonthDay {
                year: 2001,
                month: 7,
                day: 1
            })
        );
    }

    #[tokio::test]
    async fn mutations_fail_not_found_on_missing_edition() {
        let (_tmp, store, _edition) = seeded_store_with_edition().await;
        let missing = DbId::new();
        let err = store
            .set_edition_file(
                missing,
                DiskPath::from_path(camino::Utf8Path::new("/x.epub")),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::NotFound { .. }));
    }

    async fn new_edition_under(store: &LivtetStore, title: &str) -> EditionDetail {
        let work = store
            .create_work(title.to_string(), None, None)
            .await
            .unwrap();
        store
            .create_edition(NewEdition {
                work_id: work.id,
                title: Some(title.to_string()),
                description: None,
                published_date: None,
                format_id: None,
                language_id: None,
                notes: None,
                cover_image_url: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_work_and_edition_roundtrip() {
        let (_tmp, store, _edition) = seeded_store_with_edition().await;

        let work = store
            .create_work("A New Work".to_string(), None, Some("desc".to_string()))
            .await
            .unwrap();
        assert_eq!(work.title, "A New Work");
        assert_eq!(work.description.as_deref(), Some("desc"));

        let edition = store
            .create_edition(NewEdition {
                work_id: work.id,
                title: Some("First Edition".to_string()),
                description: None,
                published_date: Some(PublishedDate::YearMonthDay {
                    year: 1999,
                    month: 12,
                    day: 31,
                }),
                format_id: None,
                language_id: None,
                notes: None,
                cover_image_url: None,
            })
            .await
            .unwrap();
        assert_eq!(edition.work_id, work.id);
        assert_eq!(edition.title.as_deref(), Some("First Edition"));
        assert_eq!(
            edition.published_date,
            Some(PublishedDate::YearMonthDay {
                year: 1999,
                month: 12,
                day: 31
            })
        );

        let editions = store.list_editions(work.id).await.unwrap();
        assert_eq!(editions.len(), 1);
        assert_eq!(editions[0].id, edition.id);
    }

    #[tokio::test]
    async fn created_edition_cover_surfaces_on_the_work() {
        let (_tmp, store, _edition) = seeded_store_with_edition().await;

        let work = store
            .create_work("Covered Work".to_string(), None, None)
            .await
            .unwrap();
        store
            .create_edition(NewEdition {
                work_id: work.id,
                title: Some("Covered Edition".to_string()),
                description: None,
                published_date: None,
                format_id: None,
                language_id: None,
                notes: None,
                cover_image_url: Some("https://covers.example/covered.jpg".to_string()),
            })
            .await
            .unwrap();

        let listed = store.list_works(100, 0, None, None).await.unwrap();
        let row = listed
            .iter()
            .find(|w| w.id == work.id)
            .expect("work listed");
        // The library draws covers from the work row, so the edition's catalog
        // URL has to reach it; status stays None until the work is shelved.
        assert_eq!(
            row.cover_url.as_deref(),
            Some("https://covers.example/covered.jpg")
        );
        assert!(row.status.is_none());
    }

    #[tokio::test]
    async fn create_work_rejects_blank_title() {
        let (_tmp, store, _edition) = seeded_store_with_edition().await;
        let err = store
            .create_work("   ".to_string(), None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::InvalidInput(_)), "{err:?}");
    }

    #[tokio::test]
    async fn create_edition_requires_existing_work() {
        let (_tmp, store, _edition) = seeded_store_with_edition().await;
        let err = store
            .create_edition(NewEdition {
                work_id: DbId::new(),
                title: None,
                description: None,
                published_date: None,
                format_id: None,
                language_id: None,
                notes: None,
                cover_image_url: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::NotFound { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn identifiers_list_update_and_remove() {
        let (_tmp, store, _edition) = seeded_store_with_edition().await;
        let edition = new_edition_under(&store, "Identifier Host").await;

        let isbn = Identifier::parse("urn:isbn:9780306406157").unwrap();
        let oclc = Identifier::parse("urn:oclc:424242").unwrap();
        let doi = Identifier::parse("urn:doi:10.1000/xyz").unwrap();

        store
            .add_edition_identifier(edition.id, isbn.clone())
            .await
            .unwrap();
        store
            .add_edition_identifier(edition.id, oclc.clone())
            .await
            .unwrap();

        let listed = store.list_edition_identifiers(edition.id).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&isbn) && listed.contains(&oclc));

        let replaced = store
            .update_edition_identifier(edition.id, oclc.clone(), doi.clone())
            .await
            .unwrap();
        assert_eq!(replaced, doi);
        let listed = store.list_edition_identifiers(edition.id).await.unwrap();
        assert!(listed.contains(&doi));
        assert!(!listed.contains(&oclc), "old identifier detached");

        assert!(
            store
                .remove_edition_identifier(edition.id, isbn.clone())
                .await
                .unwrap()
        );
        assert!(
            !store
                .remove_edition_identifier(edition.id, isbn)
                .await
                .unwrap()
        );
        let listed = store.list_edition_identifiers(edition.id).await.unwrap();
        assert_eq!(listed, vec![doi]);
    }

    #[tokio::test]
    async fn identifier_is_shared_across_editions() {
        let (_tmp, store, _edition) = seeded_store_with_edition().await;
        let a = new_edition_under(&store, "Share A").await;
        let b = new_edition_under(&store, "Share B").await;

        let isbn = Identifier::parse("urn:isbn:9780140328721").unwrap();
        store
            .add_edition_identifier(a.id, isbn.clone())
            .await
            .unwrap();
        store
            .add_edition_identifier(b.id, isbn.clone())
            .await
            .unwrap();

        let hits = store
            .find_editions_by_identifier(isbn.clone())
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);

        // Unlinking one edition leaves the other's link intact.
        assert!(
            store
                .remove_edition_identifier(a.id, isbn.clone())
                .await
                .unwrap()
        );
        assert!(
            store
                .list_edition_identifiers(a.id)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.list_edition_identifiers(b.id).await.unwrap(),
            vec![isbn.clone()]
        );

        let hits = store.find_editions_by_identifier(isbn).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, b.id);
    }
}
