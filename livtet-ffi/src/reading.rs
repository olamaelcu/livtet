//! Reading domain API: work status, progress, sessions, annotations,
//! reading lists (methods on [`LivtetStore`]).
//!
//! None of these tables participate in the search index, so no index
//! sync is needed here.

use livtet_core::data::entities::{
    current_work_status, reading_list_book, reading_lists, reading_progress, reading_sessions,
};
use livtet_core::data::orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use livtet_core::types::{DbId, ProgressUnit, WorkStatus, now_primitive};

use crate::dto::{
    Annotation, ReadingList, ReadingProgress, ReadingSession, ReadingSessionInput, ts, ts_opt,
    ts_parse,
};
use crate::error::LivtetError;
use crate::store::LivtetStore;

/// Local (non-synced) user marker for annotation ownership. Deterministic
/// and obviously sentinel, so a future users model can migrate cleanly.
const LOCAL_USER: [u8; 16] = [0; 16];

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    // ── Work status ──────────────────────────────────────────────

    /// Set the reading status of a work (insert-or-replace).
    pub async fn set_work_status(
        &self,
        work_id: DbId,
        status: WorkStatus,
    ) -> Result<(), LivtetError> {
        let db = self.state.db_conn();
        let existing = current_work_status::Entity::find_by_id(work_id)
            .one(&db)
            .await?;

        match existing {
            Some(model) => {
                let mut active: current_work_status::ActiveModel = model.into();
                active.status = Set(status);
                active.updated_at = Set(Some(now_primitive()));
                active.update(&db).await?;
            }
            None => {
                current_work_status::Entity::insert(current_work_status::ActiveModel {
                    work_id: Set(work_id),
                    status: Set(status),
                    created_at: Set(now_primitive()),
                    updated_at: Set(None),
                })
                .exec(&db)
                .await?;
            }
        }
        Ok(())
    }

    /// The reading status of a work, or `None` when never set.
    pub async fn get_work_status(&self, work_id: DbId) -> Result<Option<WorkStatus>, LivtetError> {
        let db = self.state.db_conn();
        let row = current_work_status::Entity::find_by_id(work_id)
            .one(&db)
            .await?;
        Ok(row.map(|r| r.status))
    }

    // ── Reading progress ─────────────────────────────────────────

    /// Record progress for an edition. Upserts the edition's progress
    /// row: `additional_seconds` accumulates into the total reading
    /// time, the rest replaces.
    pub async fn record_reading_progress(
        &self,
        edition_id: DbId,
        format_id: DbId,
        progress: f64,
        unit: Option<ProgressUnit>,
        last_location: Option<String>,
        additional_seconds: i64,
    ) -> Result<ReadingProgress, LivtetError> {
        let db = self.state.db_conn();
        let existing = reading_progress::Entity::find()
            .filter(reading_progress::Column::EditionId.eq(edition_id))
            .one(&db)
            .await?;

        let model = match existing {
            Some(prev) => {
                let mut active: reading_progress::ActiveModel = prev.into();
                active.progress = Set(progress);
                active.progress_unit = Set(unit.map(|u| u.as_str().to_string()));
                active.last_location = Set(last_location);
                active.total_reading_time_secs =
                    Set(active.total_reading_time_secs.unwrap() + additional_seconds);
                active.update(&db).await?
            }
            None => {
                let active = reading_progress::ActiveModel {
                    id: Set(DbId::new()),
                    edition_id: Set(edition_id),
                    format_id: Set(format_id),
                    progress: Set(progress),
                    progress_unit: Set(unit.map(|u| u.as_str().to_string())),
                    last_location: Set(last_location),
                    total_reading_time_secs: Set(additional_seconds.max(0)),
                    created_at: Set(now_primitive()),
                };
                reading_progress::Entity::insert(active)
                    .exec_with_returning(&db)
                    .await?
            }
        };

        Ok(ReadingProgress {
            id: model.id,
            edition_id: model.edition_id,
            progress: model.progress,
            progress_unit: model.progress_unit.as_deref().and_then(unit_from_str),
            last_location: model.last_location,
            total_reading_time_secs: model.total_reading_time_secs,
            created_at: ts(&model.created_at),
        })
    }

    /// The progress row for an edition, if any.
    pub async fn get_reading_progress(
        &self,
        edition_id: DbId,
    ) -> Result<Option<ReadingProgress>, LivtetError> {
        let db = self.state.db_conn();
        let row = reading_progress::Entity::find()
            .filter(reading_progress::Column::EditionId.eq(edition_id))
            .one(&db)
            .await?;
        Ok(row.map(|m| ReadingProgress {
            id: m.id,
            edition_id: m.edition_id,
            progress: m.progress,
            progress_unit: m.progress_unit.as_deref().and_then(unit_from_str),
            last_location: m.last_location,
            total_reading_time_secs: m.total_reading_time_secs,
            created_at: ts(&m.created_at),
        }))
    }

    /// Record one finished reading session.
    pub async fn record_reading_session(
        &self,
        input: ReadingSessionInput,
    ) -> Result<ReadingSession, LivtetError> {
        let db = self.state.db_conn();

        let started = match input.started_at {
            Some(s) => ts_parse(&s)?,
            None => now_primitive()
                .checked_sub(time::Duration::seconds(input.duration_seconds))
                .unwrap_or_else(now_primitive),
        };
        let ended = started.checked_add(time::Duration::seconds(input.duration_seconds));

        let active = reading_sessions::ActiveModel {
            id: Set(DbId::new()),
            edition_id: Set(input.edition_id),
            format_id: Set(input.format_id),
            source_id: Set(None),
            started_at: Set(started),
            ended_at: Set(ended),
            duration_seconds: Set(Some(input.duration_seconds.max(0))),
            raw_progression: Set(None),
            progress_delta: Set(input.progress_delta),
            last_location: Set(input.last_location),
            notes: Set(input.notes),
            created_at: Set(now_primitive()),
            updated_at: Set(None),
        };
        let model = reading_sessions::Entity::insert(active)
            .exec_with_returning(&db)
            .await?;

        Ok(ReadingSession {
            id: model.id,
            edition_id: model.edition_id,
            started_at: ts(&model.started_at),
            duration_seconds: model.duration_seconds.unwrap_or(0),
            progress_delta: model.progress_delta,
            last_location: model.last_location,
            notes: model.notes,
        })
    }

    // ── Annotations ──────────────────────────────────────────────

    /// Add an annotation (note/highlight) to an edition.
    pub async fn add_annotation(
        &self,
        edition_id: DbId,
        content: String,
        location: Option<String>,
    ) -> Result<Annotation, LivtetError> {
        let db = self.state.db_conn();
        let active = livtet_core::data::entities::annotations::ActiveModel {
            id: Set(DbId::new()),
            edition_id: Set(edition_id),
            user_id: Set(DbId::from_bytes(LOCAL_USER)),
            content: Set(content),
            location: Set(location),
            created_at: Set(now_primitive()),
            updated_at: Set(None),
        };
        let model = livtet_core::data::entities::annotations::Entity::insert(active)
            .exec_with_returning(&db)
            .await?;
        Ok(annotation_from(model))
    }

    /// All annotations for an edition, oldest first.
    pub async fn list_annotations(&self, edition_id: DbId) -> Result<Vec<Annotation>, LivtetError> {
        let db = self.state.db_conn();
        let rows = livtet_core::data::entities::annotations::Entity::find()
            .filter(livtet_core::data::entities::annotations::Column::EditionId.eq(edition_id))
            .order_by_asc(livtet_core::data::entities::annotations::Column::CreatedAt)
            .all(&db)
            .await?;
        Ok(rows.into_iter().map(annotation_from).collect())
    }

    /// Delete one annotation. `false` when it did not exist.
    pub async fn delete_annotation(&self, id: DbId) -> Result<bool, LivtetError> {
        let db = self.state.db_conn();
        Ok(
            livtet_core::data::entities::annotations::Entity::delete_by_id(id)
                .exec(&db)
                .await?
                .rows_affected
                > 0,
        )
    }

    // ── Reading lists ────────────────────────────────────────────

    /// Create an empty reading list.
    pub async fn create_reading_list(
        &self,
        name: String,
        description: Option<String>,
    ) -> Result<ReadingList, LivtetError> {
        let db = self.state.db_conn();
        let active = reading_lists::ActiveModel {
            id: Set(DbId::new()),
            name: Set(name),
            description: Set(description),
            created_at: Set(now_primitive()),
            updated_at: Set(None),
        };
        let model = reading_lists::Entity::insert(active)
            .exec_with_returning(&db)
            .await?;
        Ok(ReadingList {
            id: model.id,
            name: model.name,
            description: model.description,
            edition_ids: Vec::new(),
            created_at: ts(&model.created_at),
            updated_at: ts_opt(model.updated_at),
        })
    }

    /// All reading lists, each with its edition ids in position order.
    pub async fn list_reading_lists(&self) -> Result<Vec<ReadingList>, LivtetError> {
        let db = self.state.db_conn();
        let lists = reading_lists::Entity::find()
            .order_by_asc(reading_lists::Column::CreatedAt)
            .all(&db)
            .await?;
        let mut out = Vec::with_capacity(lists.len());
        for list in lists {
            let edition_ids = list.edition_ids(&db).await?;
            out.push(ReadingList {
                id: list.id,
                name: list.name,
                description: list.description,
                edition_ids,
                created_at: ts(&list.created_at),
                updated_at: ts_opt(list.updated_at),
            });
        }
        Ok(out)
    }

    /// Append an edition to a reading list (position = end). No-op for
    /// a duplicate membership; fails `NotFound` for a missing list.
    pub async fn add_edition_to_list(
        &self,
        list_id: DbId,
        edition_id: DbId,
    ) -> Result<(), LivtetError> {
        let db = self.state.db_conn();

        // Fail-closed: the list must exist.
        reading_lists::Entity::find_by_id(list_id)
            .one(&db)
            .await?
            .ok_or_else(|| LivtetError::NotFound {
                entity: "reading_lists".to_string(),
                id: list_id.to_string(),
            })?;

        let members = reading_list_book::Entity::find()
            .filter(reading_list_book::Column::ReadingListId.eq(list_id))
            .all(&db)
            .await?;
        if members.iter().any(|m| m.edition_id == edition_id) {
            return Ok(());
        }
        let position = members
            .iter()
            .map(|m| m.position)
            .max()
            .map_or(0, |p| p + 1);

        reading_list_book::Entity::insert(reading_list_book::ActiveModel {
            reading_list_id: Set(list_id),
            edition_id: Set(edition_id),
            position: Set(position),
            added_at: Set(now_primitive()),
        })
        .exec(&db)
        .await?;
        Ok(())
    }

    /// Remove an edition from a reading list. `false` when not a member.
    pub async fn remove_edition_from_list(
        &self,
        list_id: DbId,
        edition_id: DbId,
    ) -> Result<bool, LivtetError> {
        let db = self.state.db_conn();
        Ok(reading_list_book::Entity::delete_many()
            .filter(reading_list_book::Column::ReadingListId.eq(list_id))
            .filter(reading_list_book::Column::EditionId.eq(edition_id))
            .exec(&db)
            .await?
            .rows_affected
            > 0)
    }

    /// Delete a reading list and its memberships. `false` when absent.
    pub async fn delete_reading_list(&self, list_id: DbId) -> Result<bool, LivtetError> {
        let db = self.state.db_conn();
        reading_list_book::Entity::delete_many()
            .filter(reading_list_book::Column::ReadingListId.eq(list_id))
            .exec(&db)
            .await?;
        Ok(reading_lists::Entity::delete_by_id(list_id)
            .exec(&db)
            .await?
            .rows_affected
            > 0)
    }
}

trait ListMemberships {
    async fn edition_ids(
        &self,
        db: &livtet_core::data::orm::DatabaseConnection,
    ) -> Result<Vec<DbId>, LivtetError>;
}

impl ListMemberships for reading_lists::Model {
    async fn edition_ids(
        &self,
        db: &livtet_core::data::orm::DatabaseConnection,
    ) -> Result<Vec<DbId>, LivtetError> {
        let members = reading_list_book::Entity::find()
            .filter(reading_list_book::Column::ReadingListId.eq(self.id))
            .order_by_asc(reading_list_book::Column::Position)
            .all(db)
            .await?;
        Ok(members.into_iter().map(|m| m.edition_id).collect())
    }
}

/// Parse the stored `progress_unit` string; unknown stored values
/// degrade to `None` (raw value stays in the DB untouched).
fn unit_from_str(s: &str) -> Option<ProgressUnit> {
    match s {
        "percentage" => Some(ProgressUnit::Percentage),
        "page" => Some(ProgressUnit::Page),
        "virtual_page" => Some(ProgressUnit::VirtualPage),
        "timestamp" => Some(ProgressUnit::Timestamp),
        "chapter" => Some(ProgressUnit::Chapter),
        "cfi" => Some(ProgressUnit::Cfi),
        _ => None,
    }
}

fn annotation_from(m: livtet_core::data::entities::annotations::Model) -> Annotation {
    Annotation {
        id: m.id,
        edition_id: m.edition_id,
        content: m.content,
        location: m.location,
        created_at: ts(&m.created_at),
        updated_at: ts_opt(m.updated_at),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use livtet_core::types::KnownFormats;

    async fn seeded_store_with_edition() -> (
        camino_tempfile::Utf8TempDir,
        Arc<LivtetStore>,
        DbId, // work
        DbId, // edition
    ) {
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

        let work = store
            .list_works(1, 0, None, None)
            .await
            .unwrap()
            .swap_remove(0);
        let edition = store.list_editions(work.id).await.unwrap()[0].id;
        (tmp, store, work.id, edition)
    }

    #[tokio::test]
    async fn work_status_roundtrip() {
        let (_tmp, store, work, _e) = seeded_store_with_edition().await;
        store
            .set_work_status(work, WorkStatus::Reading)
            .await
            .unwrap();
        assert_eq!(
            store.get_work_status(work).await.unwrap(),
            Some(WorkStatus::Reading)
        );
    }

    #[tokio::test]
    async fn progress_upsert_accumulates_time() {
        let (_tmp, store, _w, edition) = seeded_store_with_edition().await;
        let fmt = DbId::from(KnownFormats::Epub);

        // Seed may have already created a progress row for this
        // edition; account for either case (create vs upsert).
        let existing = store.get_reading_progress(edition).await.unwrap();

        let first = store
            .record_reading_progress(
                edition,
                fmt,
                0.25,
                Some(ProgressUnit::Percentage),
                None,
                300,
            )
            .await
            .unwrap();
        match &existing {
            Some(prev) => {
                assert_eq!(first.id, prev.id, "existing row is updated in place");
                assert_eq!(
                    first.total_reading_time_secs,
                    prev.total_reading_time_secs + 300
                );
            }
            None => assert_eq!(first.total_reading_time_secs, 300),
        }

        let second = store
            .record_reading_progress(edition, fmt, 0.5, Some(ProgressUnit::Percentage), None, 180)
            .await
            .unwrap();
        assert_eq!(second.id, first.id, "same row updated");
        assert_eq!(
            second.total_reading_time_secs,
            first.total_reading_time_secs + 180
        );
        assert_eq!(second.progress, 0.5);

        let fetched = store.get_reading_progress(edition).await.unwrap().unwrap();
        assert_eq!(fetched.id, first.id);
    }

    #[tokio::test]
    async fn session_is_recorded() {
        let (_tmp, store, _w, edition) = seeded_store_with_edition().await;
        let fmt = DbId::from(KnownFormats::Epub);
        let session = store
            .record_reading_session(ReadingSessionInput {
                edition_id: edition,
                format_id: fmt,
                duration_seconds: 600,
                progress_delta: 0.03,
                last_location: None,
                notes: None,
                started_at: None,
            })
            .await
            .unwrap();
        assert_eq!(session.edition_id, edition);
        assert_eq!(session.duration_seconds, 600);
        assert_eq!(session.progress_delta, 0.03);
    }

    #[tokio::test]
    async fn annotations_crud() {
        let (_tmp, store, _w, edition) = seeded_store_with_edition().await;
        let ann = store
            .add_annotation(edition, "Nice passage".to_string(), Some("ch1".to_string()))
            .await
            .unwrap();

        let listed = store.list_annotations(edition).await.unwrap();
        assert!(listed.iter().any(|a| a.id == ann.id));

        assert!(store.delete_annotation(ann.id).await.unwrap());
        assert!(!store.delete_annotation(ann.id).await.unwrap());
    }

    #[tokio::test]
    async fn reading_list_flow() {
        let (_tmp, store, _w, edition) = seeded_store_with_edition().await;

        let list = store
            .create_reading_list("Summer".to_string(), None)
            .await
            .unwrap();
        store.add_edition_to_list(list.id, edition).await.unwrap();
        // Duplicate add is a no-op.
        store.add_edition_to_list(list.id, edition).await.unwrap();

        let lists = store.list_reading_lists().await.unwrap();
        let mine = lists.iter().find(|l| l.id == list.id).unwrap();
        assert_eq!(mine.edition_ids, vec![edition]);

        assert!(
            store
                .remove_edition_from_list(list.id, edition)
                .await
                .unwrap()
        );
        assert!(
            !store
                .remove_edition_from_list(list.id, edition)
                .await
                .unwrap()
        );

        assert!(store.delete_reading_list(list.id).await.unwrap());
        assert!(!store.delete_reading_list(list.id).await.unwrap());
    }
}
