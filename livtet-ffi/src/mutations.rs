//! Edition mutation API (methods on [`LivtetStore`]).
//!
//! Every mutation writes through SQLite (the source of truth) and then
//! refreshes the edition's search-index document so reads via
//! `search_*` see the change immediately.

use livtet_core::data::entities::{digital_inventory, edition_identifiers, editions, identifiers};
use livtet_core::data::orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use livtet_core::types::{DbId, DiskPath, Identifier, PublishedDate, now_primitive};

use crate::dto::EditionDetail;
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

        let value = identifier.as_urn_string();
        let kind = identifier.kind.as_str().to_string();

        let identifier_id = match identifiers::Entity::find()
            .filter(identifiers::Column::Value.eq(&value))
            .one(&db)
            .await?
        {
            Some(row) => row.id,
            None => {
                let id = DbId::new();
                identifiers::Entity::insert(identifiers::ActiveModel {
                    id: Set(id),
                    value: Set(value.clone()),
                    kind: Set(kind),
                })
                .exec(&db)
                .await?;
                id
            }
        };

        let linked = edition_identifiers::Entity::find_by_id((edition_id, identifier_id))
            .one(&db)
            .await?;
        if linked.is_none() {
            edition_identifiers::Entity::insert(edition_identifiers::ActiveModel {
                edition_id: Set(edition_id),
                identifier_id: Set(identifier_id),
            })
            .exec(&db)
            .await?;
        }

        self.sync_edition_index(edition_id).await?;
        Ok(value)
    }

    /// Apply a [`EditionPatch`] and return the refreshed detail.
    pub async fn update_edition(
        &self,
        edition_id: DbId,
        patch: EditionPatch,
    ) -> Result<EditionDetail, LivtetError> {
        let db = self.state.db_conn();

        let published_date = match patch.published_date {
            None => None,
            Some(PublishedDate::YearMonthDay { year, month, day }) => {
                let month = time::Month::try_from(month)
                    .map_err(|e| LivtetError::InvalidInput(format!("invalid month: {e}")))?;
                Some(Some(
                    time::Date::from_calendar_date(year, month, day).map_err(|e| {
                        LivtetError::InvalidInput(format!("invalid calendar date: {e}"))
                    })?,
                ))
            }
            // Partial precision can't be stored: editions carries a
            // full `time::Date` column or NULL. Fail closed.
            Some(other) => {
                return Err(LivtetError::InvalidInput(format!(
                    "published_date must be a full y-m-d date, got {:?}",
                    other
                )));
            }
        };

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
    async fn sync_edition_index(&self, edition_id: DbId) -> Result<(), LivtetError> {
        self.index
            .add_edition(&self.state.db_conn(), edition_id)
            .await
            .map_err(|e| LivtetError::Search(e.to_string()))
    }
}

async fn require_edition(
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
    async fn update_edition_patches_and_rejects_partial_dates() {
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

        let bad = EditionPatch {
            title: None,
            description: None,
            notes: None,
            published_date: Some(PublishedDate::Year(1989)),
            format_id: None,
            language_id: None,
        };
        let err = store.update_edition(edition, bad).await.unwrap_err();
        assert!(matches!(err, LivtetError::InvalidInput(_)), "{err:?}");
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
}
