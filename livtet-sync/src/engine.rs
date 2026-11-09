//! Local sync engine: reads/writes the `change_log` and `conflicts`
//! SQLite tables directly via sea-orm.  Shared by the desktop server
//! and the FFI — the desktop uses it for the HTTP layer's local
//! queries, and the FFI uses it for the same on the mobile device.

use livtet_data::annotations as note_merge;
use livtet_data::orm::sea_query::Value as SqlValue;
use livtet_data::orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseBackend, Statement, TransactionTrait,
};
use livtet_types::DbId;
use serde_json::Value;

use crate::types::{Conflict, FullDump, PullResponse, PushResponse, SyncChange, SyncError};

/// All push/pull/full-dump logic against the local DB.
#[derive(Clone)]
pub struct SyncEngine {
    db: livtet_data::orm::DatabaseConnection,
    device_id: String,
}

impl SyncEngine {
    pub fn new(db: livtet_data::orm::DatabaseConnection, device_id: String) -> Self {
        Self { db, device_id }
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn db(&self) -> &livtet_data::orm::DatabaseConnection {
        &self.db
    }

    pub async fn get_latest_version(&self) -> Result<i64, SyncError> {
        let stmt = livtet_data::orm::Statement::from_string(
            livtet_data::orm::DatabaseBackend::Sqlite,
            "SELECT MAX(version) AS max_ver FROM change_log",
        );
        let row = self.db.query_one_raw(stmt).await?;
        Ok(row
            .and_then(|r| r.try_get::<i64>("", "max_ver").ok())
            .unwrap_or(0))
    }

    pub async fn pull_changes(
        &self,
        since_version: i64,
        limit: i64,
    ) -> Result<PullResponse, SyncError> {
        let stmt = livtet_data::orm::Statement::from_sql_and_values(
            livtet_data::orm::DatabaseBackend::Sqlite,
            "SELECT id, entity_type, entity_id, operation, version, payload, changed_at, device_id \
             FROM change_log WHERE version > ? ORDER BY version ASC LIMIT ?",
            [since_version.into(), limit.into()],
        );
        let rows = self.db.query_all_raw(stmt).await?;

        let mut changes: Vec<SyncChange> = rows
            .iter()
            .filter_map(|row| {
                // The change_log `id` column is `INTEGER PRIMARY KEY
                // AUTOINCREMENT` (see `change_log::CHANGE_LOG_TABLE`),
                // not a 16-byte ULID. Reading it as `DbId` would
                // fail and `filter_map` would drop every row;
                // reading as `i64` matches the schema and lets
                // the row survive.
                Some(SyncChange {
                    id: row.try_get::<i64>("", "id").ok()?,
                    entity_type: row.try_get::<String>("", "entity_type").ok()?,
                    entity_id: row.try_get::<String>("", "entity_id").ok()?,
                    operation: row.try_get::<String>("", "operation").ok()?,
                    version: row.try_get::<i64>("", "version").ok()?,
                    payload: row.try_get::<String>("", "payload").ok()?,
                    changed_at: row.try_get::<String>("", "changed_at").ok()?,
                    device_id: row.try_get::<String>("", "device_id").ok()?,
                })
            })
            .collect();

        // The page's bounds are read before the filter: an all-inventory page must still
        // advance the peer's cursor, or a device that logged nothing but its own file
        // rows would block every pull behind it.
        let latest_version = changes.last().map(|c| c.version).unwrap_or(since_version);
        let has_more = changes.len() as i64 == limit;
        // `digital_inventory` describes *this* device's files (absolute paths, local
        // hashes), so a peer is never told about them; accepting an offer writes the
        // receiver's own row.
        changes.retain(|change| change.entity_type != "digital_inventory");
        Ok(PullResponse {
            changes,
            has_more,
            latest_version,
        })
    }

    // Pulls a full dump of all syncable tables.
    #[allow(unused_variables)]
    pub async fn pull_full(&self) -> Result<FullDump, SyncError> {
        let version = self.get_latest_version().await.unwrap_or(0);

        async fn query_table(
            db: &livtet_data::orm::DatabaseConnection,
            table: &str,
            cols: &[&str],
        ) -> Result<Vec<serde_json::Value>, SyncError> {
            let col_list: Vec<String> = cols.iter().map(|s| (*s).to_string()).collect();
            let query = format!("SELECT {} FROM {}", col_list.join(", "), table);
            let stmt = livtet_data::orm::Statement::from_sql_and_values(
                livtet_data::orm::DatabaseBackend::Sqlite,
                &query,
                [],
            );
            let rows = db.query_all_raw(stmt).await?;
            let mut result = Vec::new();
            for row in rows {
                let mut obj = serde_json::Map::new();
                for col in cols {
                    if let Ok(v) = row.try_get::<i64>("", col) {
                        obj.insert(col.to_string(), serde_json::Value::Number(v.into()));
                    } else if let Ok(v) = row.try_get::<f64>("", col) {
                        if let Some(n) = serde_json::Number::from_f64(v) {
                            obj.insert(col.to_string(), serde_json::Value::Number(n));
                        }
                    } else if let Ok(v) = row.try_get::<String>("", col) {
                        obj.insert(col.to_string(), serde_json::Value::String(v));
                    } else if let Ok(v) = row.try_get::<Vec<u8>>("", col) {
                        // BINARY(16) / DbId column — convert to ULID string.
                        if let Ok(arr) = <[u8; 16]>::try_from(v.as_slice()) {
                            let id = livtet_types::DbId::from_bytes(arr);
                            obj.insert(col.to_string(), serde_json::Value::String(id.to_string()));
                        }
                    }
                }
                result.push(serde_json::Value::Object(obj));
            }
            Ok(result)
        }

        let works = query_table(
            &self.db,
            "works",
            &[
                "id",
                "title",
                "description",
                "sort_title",
                "series_type",
                "created_at",
                "updated_at",
            ],
        )
        .await?;
        let editions = query_table(
            &self.db,
            "editions",
            &[
                "id",
                "work_id",
                "title",
                "published_date",
                "format_id",
                "language_id",
                "notes",
                "description",
                "created_at",
                "updated_at",
            ],
        )
        .await?;
        let edition_groups = query_table(
            &self.db,
            "edition_groups",
            &["id", "label", "description", "created_at", "updated_at"],
        )
        .await?;
        let series_entries = query_table(
            &self.db,
            "series_entries",
            &["series_id", "edition_id", "position", "created_at"],
        )
        .await?;
        let owned_editions = query_table(
            &self.db,
            "owned_editions",
            &[
                "id",
                "edition_id",
                "acquired_at",
                "condition_id",
                "notes",
                "created_at",
                "updated_at",
            ],
        )
        .await?;
        let editions_loans = query_table(
            &self.db,
            "editions_loans",
            &[
                "id",
                "edition_id",
                "loan_entity_id",
                "owned_edition_id",
                "loaned_date",
                "due_date",
                "returned_date",
            ],
        )
        .await?;
        let reading_lists = query_table(
            &self.db,
            "reading_lists",
            &["id", "name", "description", "created_at", "updated_at"],
        )
        .await?;
        let reading_list_book = query_table(
            &self.db,
            "reading_list_book",
            &["reading_list_id", "edition_id", "position", "added_at"],
        )
        .await?;
        let reading_progress = query_table(
            &self.db,
            "reading_progress",
            &[
                "id",
                "edition_id",
                "format_id",
                "progress",
                "progress_unit",
                "last_location",
                "total_reading_time_secs",
                "annotations",
                "created_at",
            ],
        )
        .await?;

        Ok(FullDump {
            version,
            device_id: self.device_id.clone(),
            entities: crate::types::EntityDump {
                works,
                editions,
                edition_groups,
                series_entries,
                // A dump is the peer's library, not the peer's disk: the inventory rows
                // name paths that only exist there, so the field stays empty and the
                // receiver builds its own when an offer is accepted.
                digital_inventory: Vec::new(),
                owned_editions,
                editions_loans,
                annotations: Vec::new(),
                reading_lists,
                reading_list_book,
                reading_progress,
            },
        })
    }

    pub async fn push_changes(&self, changes: Vec<SyncChange>) -> Result<PushResponse, SyncError> {
        let mut conflicts = Vec::new();

        // Wrap the entire push loop in a single transaction so all
        // N changes commit in one fsync. Previously each change was
        // an implicit auto-commit per `execute_raw` call — N fsyncs
        // per push.
        let txn = self.db.begin().await?;

        // Set the apply flag so audit triggers are suppressed during remote apply.
        txn.execute_raw(livtet_data::orm::Statement::from_string(
            livtet_data::orm::DatabaseBackend::Sqlite,
            r#"INSERT INTO client_settings (key, value, updated_at) VALUES ("sync_apply","1", datetime("now")) ON CONFLICT(key) DO UPDATE SET value = "1", updated_at = datetime("now")"#,
        ))
        .await?;

        for change in &changes {
            if entity_type_to_table(&change.entity_type).is_none() {
                return Err(SyncError::UnknownEntityType {
                    type_name: change.entity_type.clone(),
                });
            }
            let operation = change.operation.to_ascii_uppercase();

            // A replayed change (same entity, version, device, operation)
            // is already in the log and must not be applied twice.
            let seen = livtet_data::orm::Statement::from_sql_and_values(
                livtet_data::orm::DatabaseBackend::Sqlite,
                "SELECT 1 FROM change_log WHERE entity_type = ? AND entity_id = ? AND version = ? AND device_id = ? AND operation = ? LIMIT 1",
                [
                    change.entity_type.clone().into(),
                    change.entity_id.clone().into(),
                    change.version.into(),
                    change.device_id.clone().into(),
                    operation.clone().into(),
                ],
            );
            if txn.query_one_raw(seen).await?.is_some() {
                continue;
            }

            let check = livtet_data::orm::Statement::from_sql_and_values(
                livtet_data::orm::DatabaseBackend::Sqlite,
                "SELECT version, payload, operation FROM change_log WHERE entity_type = ? AND entity_id = ? ORDER BY version DESC LIMIT 1",
                [
                    change.entity_type.clone().into(),
                    change.entity_id.clone().into(),
                ],
            );

            if let Some(local_row) = txn.query_one_raw(check).await? {
                let local_payload = local_row
                    .try_get::<String>("", "payload")
                    .unwrap_or_default();
                let local_operation = local_row
                    .try_get::<String>("", "operation")
                    .unwrap_or_default();

                // Versions are per-device counters, not a shared clock. Equal
                // canonical payloads and operations are the same state; any
                // other difference is a conflict.
                // A row's existence is the only operation distinction that
                // matters: INSERT and UPDATE carrying the same payload are the
                // same state. Versions are per-device counters, not a clock.
                let local_deleted = local_operation.eq_ignore_ascii_case("DELETE");
                let remote_deleted = operation.eq_ignore_ascii_case("DELETE");
                let same = canonical_payload(&local_payload) == canonical_payload(&change.payload)
                    && local_deleted == remote_deleted;
                if !same {
                    let insert_conflict = livtet_data::orm::Statement::from_sql_and_values(
                        livtet_data::orm::DatabaseBackend::Sqlite,
                        "INSERT OR IGNORE INTO conflicts (entity_type, entity_id, local_payload, remote_payload, detected_at) \
                         VALUES (?, ?, ?, ?, datetime('now'))",
                        [
                            change.entity_type.clone().into(),
                            change.entity_id.clone().into(),
                            canonical_payload(&local_payload).into(),
                            canonical_payload(&change.payload).into(),
                        ],
                    );
                    txn.execute_raw(insert_conflict).await?;

                    let fetch = livtet_data::orm::Statement::from_sql_and_values(
                        livtet_data::orm::DatabaseBackend::Sqlite,
                        "SELECT id, entity_type, entity_id, local_payload, remote_payload, \
                                resolved, resolution, merged_payload, detected_at \
                         FROM conflicts \
                         WHERE entity_type = ? AND entity_id = ? AND local_payload = ? AND remote_payload = ? \
                           AND resolved = 0 \
                         ORDER BY id DESC LIMIT 1",
                        [
                            change.entity_type.clone().into(),
                            change.entity_id.clone().into(),
                            canonical_payload(&local_payload).into(),
                            canonical_payload(&change.payload).into(),
                        ],
                    );
                    if let Some(row) = txn.query_one_raw(fetch).await? {
                        conflicts.push(row_to_conflict(&row));
                    }
                    continue;
                }
            }

            crate::apply::apply(&txn, change).await?;

            use livtet_data::client_entities::change_log;
            use livtet_data::orm::{ActiveModelTrait, Set};
            // `parse_borrowed` cannot fail for this literal format and
            // formatting an `OffsetDateTime` with it is likewise
            // infallible; fall back to an empty string rather than
            // panicking on the impossible error (no `unwrap` outside tests).
            let now = time::format_description::parse_borrowed::<2>(
                "[year]-[month]-[day] [hour]:[minute]:[second]",
            )
            .ok()
            .and_then(|fmt| time::OffsetDateTime::now_utc().format(&fmt).ok())
            .unwrap_or_default();
            let log_entry = change_log::ActiveModel {
                entity_type: Set(change.entity_type.clone()),
                entity_id: Set(change.entity_id.clone()),
                operation: Set(operation.clone()),
                version: Set(change.version),
                payload: Set(change.payload.clone()),
                changed_at: Set(now.to_string()),
                device_id: Set(change.device_id.clone()),
                ..Default::default()
            };
            log_entry.insert(&txn).await?;
        }

        // Remove the apply flag before commit; any error rolls back the whole txn.
        txn.execute_raw(livtet_data::orm::Statement::from_string(
            livtet_data::orm::DatabaseBackend::Sqlite,
            r#"DELETE FROM client_settings WHERE key = "sync_apply""#,
        ))
        .await?;

        txn.commit().await?;

        let latest_version = self.get_latest_version().await.unwrap_or(0);
        Ok(PushResponse {
            accepted: conflicts.is_empty(),
            conflicts,
            latest_version,
        })
    }
}

/// Record this device's copy of an offered file.
///
/// `digital_inventory` is device-specific and no longer synced — the sender's
/// audit triggers for it are dropped, and an incoming row is ignored — so the
/// row that makes a received book openable is written here. The offer carries
/// the bytes, the size, and the hash; only the receiver knows where it put them.
///
/// The row is matched by edition, because a library pull may already have
/// created one from a previous offer: a book has one file on one device.
pub async fn adopt_inventory(
    db: &livtet_data::orm::DatabaseConnection,
    inventory_id: livtet_types::DbId,
    edition_id: livtet_types::DbId,
    file_path: &str,
    cover_path: Option<&str>,
    file_hash: Option<&str>,
    file_size_bytes: Option<i64>,
) -> Result<(), SyncError> {
    ensure_edition(db, edition_id).await?;
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO digital_inventory \
         (id, edition_id, file_path, cover_path, file_hash, file_size_bytes, added_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, datetime('now'), datetime('now')) \
         ON CONFLICT(edition_id) DO UPDATE SET \
         file_path = excluded.file_path, \
         cover_path = COALESCE(excluded.cover_path, digital_inventory.cover_path), \
         file_hash = COALESCE(excluded.file_hash, digital_inventory.file_hash), \
         file_size_bytes = COALESCE(excluded.file_size_bytes, digital_inventory.file_size_bytes), \
         updated_at = datetime('now')",
        [
            inventory_id.to_bytes().to_vec().into(),
            edition_id.to_bytes().to_vec().into(),
            file_path.to_string().into(),
            cover_path.map(str::to_string).into(),
            file_hash.map(str::to_string).into(),
            file_size_bytes.into(),
        ],
    );
    db.execute_raw(statement).await?;
    Ok(())
}

/// Record where this device stored an edition's cover.
///
/// A cover can arrive for an edition this device has no *file* for, so the inventory row may
/// be created here with no `file_path` at all: the row is the device-local place a cover path
/// lives, and the `edition_covers` view reads it from there.
pub async fn adopt_cover(
    db: &livtet_data::orm::DatabaseConnection,
    edition_id: livtet_types::DbId,
    cover_path: &str,
) -> Result<(), SyncError> {
    // A cover for an edition this device does not have is not an error: the library
    // pull will bring the edition and the next sync will ask again.
    if !edition_exists(db, edition_id).await? {
        return Ok(());
    }
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO digital_inventory (id, edition_id, cover_path, added_at, updated_at) \
         VALUES (?, ?, ?, datetime('now'), datetime('now')) \
         ON CONFLICT(edition_id) DO UPDATE SET \
         cover_path = excluded.cover_path, updated_at = datetime('now')",
        [
            livtet_types::DbId::new().to_bytes().to_vec().into(),
            edition_id.to_bytes().to_vec().into(),
            cover_path.to_string().into(),
        ],
    );
    db.execute_raw(statement).await?;
    Ok(())
}

/// What an apply pass did.
#[derive(Debug, Default)]
pub struct ApplyOutcome {
    /// Changes written into the local tables.
    pub applied: usize,
    /// Rows ignored (offers, unknown entity types, unknown operations).
    pub skipped: usize,
    /// Version conflicts recorded instead of overwriting local state.
    pub conflicts: Vec<Conflict>,
}

impl SyncEngine {
    /// Apply changes pulled from a peer to the local database.
    ///
    /// Payloads are the audit triggers' canonical output: column names and
    /// values, with ids as hex. Applying a row fires the local triggers, which
    /// would echo the change back to the peer: rows written above the
    /// pre-apply version are deleted before the remote change row is recorded.
    ///
    /// Foreign-key checks are off for the daemon's pool (see
    /// `connect_with_migrations_foreign_keys`): library rows imported with FK
    /// enforcement off carry zero-length ids in optional columns
    /// (`editions.format_id`), and the matching parent row does not exist.
    pub async fn apply_changes(&self, changes: Vec<SyncChange>) -> Result<ApplyOutcome, SyncError> {
        self.apply(changes, false).await
    }

    /// Apply the rows an offer carries, as the reader asked for them.
    ///
    /// These rows are not a sync arriving in the background; they are the book the
    /// reader accepted. A local tombstone for the same edition (a delete the library
    /// pulled earlier) must not win over that decision, or the accepted book would be
    /// reported as a conflict and never appear.
    pub async fn apply_offer_changes(
        &self,
        changes: Vec<SyncChange>,
    ) -> Result<ApplyOutcome, SyncError> {
        self.apply(changes, true).await
    }

    async fn apply(
        &self,
        changes: Vec<SyncChange>,
        authoritative: bool,
    ) -> Result<ApplyOutcome, SyncError> {
        let txn = self.db.begin().await?;
        let mut outcome = ApplyOutcome::default();

        // Parent rows before children; SQLite may enforce foreign keys.
        let mut ordered = changes;
        ordered.sort_by_key(|change| apply_rank(&change.entity_type));

        for change in &ordered {
            if change.entity_type == "file_offer" {
                outcome.skipped += 1;
                continue;
            }
            // Device-specific: an older peer still logs these rows with its own paths.
            // Accepting an offer writes the local row instead.
            if change.entity_type == "digital_inventory" {
                outcome.skipped += 1;
                continue;
            }
            let Some(table) = entity_type_to_table(&change.entity_type) else {
                tracing::debug!(
                    entity_type = %change.entity_type,
                    "unknown sync entity ignored during apply"
                );
                outcome.skipped += 1;
                continue;
            };

            // Progress belongs to an edition this device must already hold. Writing it for an
            // edition that is not here fails the foreign key, and the pull's cursor was stored
            // before these rows were applied, so the failure would lose the whole page. Skip it
            // and count it: the next pull after the edition arrives brings the progress along.
            if change.entity_type == "reading_progress" {
                // Stage B converges notes and progress instead of reporting a
                // whole-row conflict. A row delete would drop every tombstone,
                // so a progress delete is a no-op; book deletion travels as an
                // edition delete and cascades locally.
                if change.operation.eq_ignore_ascii_case("DELETE") {
                    outcome.skipped += 1;
                    continue;
                }
                let edition = payload_edition_id(&change.payload);
                let present = match edition {
                    Some(edition_id) => edition_exists(&txn, edition_id).await?,
                    None => false,
                };
                if !present {
                    tracing::debug!(
                        entity_id = %change.entity_id,
                        parseable = edition.is_some(),
                        "reading progress skipped: its edition is not in this library"
                    );
                    outcome.skipped += 1;
                    continue;
                }

                let remote: Value = serde_json::from_str(&change.payload).map_err(|error| {
                    SyncError::Deserialization {
                        entity_type: change.entity_type.clone(),
                        message: error.to_string(),
                    }
                })?;
                let local_row = local_progress_row(&txn, &change.entity_id).await?;
                let merged = merge_progress_payload(local_row.as_ref(), &remote);
                let baseline = latest_version_in(&txn).await?;
                upsert_payload(&txn, table, &merged).await?;
                discard_echoes(&txn, change, baseline).await?;
                insert_change_row(&txn, change).await?;
                outcome.applied += 1;
                continue;
            }

            let local = latest_change(&txn, &change.entity_type, &change.entity_id).await?;
            if !authoritative && let Some((_, local_payload, local_operation)) = local {
                let local_deleted = local_operation.eq_ignore_ascii_case("DELETE");
                let remote_deleted = change.operation.eq_ignore_ascii_case("DELETE");
                let same = canonical_payload(&local_payload) == canonical_payload(&change.payload)
                    && local_deleted == remote_deleted;
                if same {
                    outcome.skipped += 1;
                    continue;
                }
                record_conflict(&txn, change, &local_payload).await?;
                let conflict = fetch_latest_conflict(&txn, change, &local_payload).await?;
                if let Some(conflict) = conflict {
                    outcome.conflicts.push(conflict);
                }
                continue;
            }

            if !matches!(change.operation.as_str(), "INSERT" | "UPDATE" | "DELETE") {
                outcome.skipped += 1;
                continue;
            }

            let baseline = latest_version_in(&txn).await?;
            if change.operation == "DELETE" {
                delete_entity(&txn, table, &change.entity_id).await?;
            } else {
                let payload: Value = serde_json::from_str(&change.payload).map_err(|error| {
                    SyncError::Deserialization {
                        entity_type: change.entity_type.clone(),
                        message: error.to_string(),
                    }
                })?;
                upsert_payload(&txn, table, &payload).await?;
            }

            // Remove this entity's trigger echoes, then record the remote row.
            discard_echoes(&txn, change, baseline).await?;
            insert_change_row(&txn, change).await?;
            outcome.applied += 1;
        }

        txn.commit().await?;
        Ok(outcome)
    }
}

/// Parent-first ordering for entities that carry foreign keys.
fn apply_rank(entity_type: &str) -> u8 {
    match entity_type {
        "work" | "edition_group" | "reading_list" => 0,
        "edition" => 1,
        "digital_inventory" | "owned_edition" | "work_author" | "work_genre" | "work_subject"
        | "work_publisher" | "work_tag" | "edition_author" | "edition_genre"
        | "edition_subject" | "edition_publisher" | "edition_tag" | "series_entry"
        | "reading_list_book" => 2,
        "edition_loan" | "annotation" => 3,
        "reading_progress" => 4,
        _ => 5,
    }
}

/// The edition a `reading_progress` change payload names.
///
/// The payload is the trigger's `json_object` output, so `edition_id` is 32-char lowercase
/// hex; either wire form is accepted. `None` means the payload does not name an edition, which
/// is a row this device has no place for.
fn payload_edition_id(payload: &str) -> Option<DbId> {
    let value: Value = serde_json::from_str(payload).ok()?;
    DbId::from_wire(value.get("edition_id")?.as_str()?)
}

/// Whether an edition row is present locally.
async fn edition_exists(db: &impl ConnectionTrait, edition_id: DbId) -> Result<bool, SyncError> {
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "SELECT 1 FROM editions WHERE id = ?",
        [edition_id.to_bytes().to_vec().into()],
    );
    Ok(db.query_one_raw(statement).await?.is_some())
}

/// Refuse to attach a file to an edition that is not here.
///
/// An offer's rows can arrive a page after the offer itself; writing the inventory row
/// before them would fail as a foreign-key error with no explanation. Say what is
/// missing instead: the reader's retry after the next sync will work.
async fn ensure_edition(
    db: &livtet_data::orm::DatabaseConnection,
    edition_id: livtet_types::DbId,
) -> Result<(), SyncError> {
    if edition_exists(db, edition_id).await? {
        return Ok(());
    }
    Err(SyncError::Db {
        message: "the book's catalog row has not arrived yet; sync again".to_string(),
        violation: None,
    })
}

async fn latest_version_in(db: &impl ConnectionTrait) -> Result<i64, SyncError> {
    let statement = Statement::from_string(
        DatabaseBackend::Sqlite,
        "SELECT COALESCE(MAX(version), 0) AS max_ver FROM change_log",
    );
    let row = db.query_one_raw(statement).await?;
    Ok(row
        .and_then(|row| row.try_get::<i64>("", "max_ver").ok())
        .unwrap_or(0))
}

async fn latest_change(
    db: &impl ConnectionTrait,
    entity_type: &str,
    entity_id: &str,
) -> Result<Option<(i64, String, String)>, SyncError> {
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "SELECT version, payload, operation FROM change_log WHERE entity_type = ? AND entity_id = ? \
         ORDER BY version DESC LIMIT 1",
        [entity_type.into(), entity_id.into()],
    );
    let row = db.query_one_raw(statement).await?;
    Ok(row.and_then(|row| {
        Some((
            row.try_get::<i64>("", "version").ok()?,
            row.try_get::<String>("", "payload").ok()?,
            row.try_get::<String>("", "operation").ok()?,
        ))
    }))
}

/// The local `reading_progress` row as the conflict UI decodes it.
struct LocalProgressRow {
    id: String,
    edition_id: String,
    format_id: String,
    progress: f64,
    progress_unit: Option<String>,
    last_location: Option<String>,
    total_reading_time_secs: i64,
    annotations: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    updated_by: Option<String>,
}

/// Load the local progress row without trusting the change log: a row written
/// before sync ran can hold real notes while having no local log entry.
async fn local_progress_row(
    db: &impl ConnectionTrait,
    entity_id: &str,
) -> Result<Option<LocalProgressRow>, SyncError> {
    let Ok(id) = DbId::from_hex(entity_id) else {
        return Ok(None);
    };
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "SELECT lower(hex(id)) AS id, lower(hex(edition_id)) AS edition_id, \
                lower(hex(format_id)) AS format_id, progress, progress_unit, last_location, \
                total_reading_time_secs, annotations, created_at \
                , updated_at, updated_by \
         FROM reading_progress WHERE id = ?",
        [id.to_bytes().to_vec().into()],
    );
    let Some(row) = db.query_one_raw(statement).await? else {
        return Ok(None);
    };
    Ok(Some(LocalProgressRow {
        id: row.try_get("", "id").unwrap_or_default(),
        edition_id: row.try_get("", "edition_id").unwrap_or_default(),
        format_id: row.try_get("", "format_id").unwrap_or_default(),
        progress: row.try_get("", "progress").unwrap_or_default(),
        progress_unit: row.try_get::<String>("", "progress_unit").ok(),
        last_location: row.try_get::<String>("", "last_location").ok(),
        total_reading_time_secs: row
            .try_get("", "total_reading_time_secs")
            .unwrap_or_default(),
        annotations: row.try_get::<String>("", "annotations").ok(),
        created_at: row.try_get::<String>("", "created_at").ok(),
        updated_at: row.try_get::<String>("", "updated_at").ok(),
        updated_by: row.try_get::<String>("", "updated_by").ok(),
    }))
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn notes(value: &Value) -> Vec<Value> {
    match value.get("annotations") {
        Some(Value::String(raw)) => note_merge::parse_notes(Some(raw)),
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// Converge a peer's progress row onto the local one (ADR 0010).
///
/// Notes union by id with tombstones final. Progress fields come from the
/// greatest `(updated_at, updated_by)`; accumulated reading time takes the
/// maximum and `created_at` stays local.
fn merge_progress_payload(local: Option<&LocalProgressRow>, remote: &Value) -> Value {
    let local_notes = local
        .and_then(|row| row.annotations.as_deref())
        .map(|raw| note_merge::parse_notes(Some(raw)))
        .unwrap_or_default();
    let merged_notes = note_merge::merge_notes(&local_notes, &notes(remote));

    let local_at = local
        .and_then(|row| row.updated_at.as_deref())
        .unwrap_or(note_merge::EPOCH);
    let local_by = local
        .and_then(|row| row.updated_by.as_deref())
        .unwrap_or("");
    let remote_at = text(remote, "updated_at").unwrap_or_else(|| note_merge::EPOCH.to_string());
    let remote_by = text(remote, "updated_by").unwrap_or_default();
    let remote_wins =
        local.is_none() || (remote_at.as_str(), remote_by.as_str()) > (local_at, local_by);

    let local_progress = local.map(|row| row.progress).unwrap_or(0.0);
    let remote_progress = remote
        .get("progress")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let progress = if remote_wins {
        remote_progress
    } else {
        local_progress
    };
    let unit = if remote_wins {
        text(remote, "progress_unit")
    } else {
        local.and_then(|row| row.progress_unit.clone())
    };
    let last_location = if remote_wins {
        text(remote, "last_location")
    } else {
        local.and_then(|row| row.last_location.clone())
    };
    let local_seconds = local.map(|row| row.total_reading_time_secs).unwrap_or(0);
    let remote_seconds = remote
        .get("total_reading_time_secs")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    serde_json::json!({
        "id": local
            .map(|row| row.id.clone())
            .or_else(|| text(remote, "id"))
            .unwrap_or_default(),
        "edition_id": local
            .map(|row| row.edition_id.clone())
            .or_else(|| text(remote, "edition_id"))
            .unwrap_or_default(),
        "format_id": local
            .map(|row| row.format_id.clone())
            .or_else(|| text(remote, "format_id"))
            .unwrap_or_default(),
        "progress": progress,
        "progress_unit": unit,
        "last_location": last_location,
        "total_reading_time_secs": local_seconds.max(remote_seconds),
        "annotations": note_merge::notes_json(&merged_notes),
        "created_at": local
            .and_then(|row| row.created_at.clone())
            .or_else(|| text(remote, "created_at")),
        "updated_at": if remote_wins { remote_at } else { local_at.to_string() },
        "updated_by": if remote_wins { remote_by } else { local_by.to_string() },
    })
}

async fn record_conflict(
    db: &impl ConnectionTrait,
    change: &SyncChange,
    local_payload: &str,
) -> Result<(), SyncError> {
    // Key order and whitespace are not state: canonicalise before comparing so
    // equal rows never conflict, and repeated conflicts dedupe on the text.
    let canonical = canonical_payload(local_payload);
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT OR IGNORE INTO conflicts (entity_type, entity_id, local_payload, remote_payload, detected_at) \
         VALUES (?, ?, ?, ?, datetime('now'))",
        [
            change.entity_type.clone().into(),
            change.entity_id.clone().into(),
            canonical.into(),
            canonical_payload(&change.payload).into(),
        ],
    );
    db.execute_raw(statement).await?;
    Ok(())
}

/// Stable text for equality: JSON object keys sort and whitespace disappears.
fn canonical_payload(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| raw.to_string())
}

async fn fetch_latest_conflict(
    db: &impl ConnectionTrait,
    change: &SyncChange,
    local_payload: &str,
) -> Result<Option<Conflict>, SyncError> {
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "SELECT id, entity_type, entity_id, local_payload, remote_payload, resolved, resolution, \
                merged_payload, detected_at FROM conflicts \
         WHERE entity_type = ? AND entity_id = ? AND local_payload = ? AND remote_payload = ? \
           AND resolved = 0 ORDER BY id DESC LIMIT 1",
        [
            change.entity_type.clone().into(),
            change.entity_id.clone().into(),
            canonical_payload(local_payload).into(),
            canonical_payload(&change.payload).into(),
        ],
    );
    let row = db.query_one_raw(statement).await?;
    Ok(row.map(|row| Conflict {
        id: row.try_get("", "id").unwrap_or_default(),
        entity_type: row.try_get("", "entity_type").unwrap_or_default(),
        entity_id: row.try_get("", "entity_id").unwrap_or_default(),
        local_payload: row.try_get("", "local_payload").unwrap_or_default(),
        remote_payload: row.try_get("", "remote_payload").unwrap_or_default(),
        resolved: row.try_get("", "resolved").unwrap_or(false),
        resolution: row.try_get("", "resolution").ok(),
        merged_payload: row.try_get("", "merged_payload").ok(),
        detected_at: row.try_get("", "detected_at").unwrap_or_default(),
    }))
}

async fn discard_echoes(
    db: &impl ConnectionTrait,
    change: &SyncChange,
    baseline: i64,
) -> Result<(), SyncError> {
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "DELETE FROM change_log WHERE entity_type = ? AND entity_id = ? AND version > ?",
        [
            change.entity_type.clone().into(),
            change.entity_id.clone().into(),
            baseline.into(),
        ],
    );
    db.execute_raw(statement).await?;
    Ok(())
}

async fn insert_change_row(
    db: &impl ConnectionTrait,
    change: &SyncChange,
) -> Result<(), SyncError> {
    use livtet_data::client_entities::change_log;
    use livtet_data::orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

    // A pull can re-offer a row after a cursor rewind (tests, resume). The
    // change_log identity index rejects the duplicate, but it is not state:
    // the row was already applied.
    let duplicate = change_log::Entity::find()
        .filter(change_log::Column::EntityType.eq(change.entity_type.clone()))
        .filter(change_log::Column::EntityId.eq(change.entity_id.clone()))
        .filter(change_log::Column::Operation.eq(change.operation.clone()))
        .filter(change_log::Column::Version.eq(change.version))
        .filter(change_log::Column::DeviceId.eq(change.device_id.clone()))
        .one(db)
        .await?;
    if duplicate.is_some() {
        return Ok(());
    }

    let now = time::format_description::parse_borrowed::<2>(
        "[year]-[month]-[day] [hour]:[minute]:[second]",
    )
    .ok()
    .and_then(|format| time::OffsetDateTime::now_utc().format(&format).ok())
    .unwrap_or_default();

    let entry = change_log::ActiveModel {
        entity_type: Set(change.entity_type.clone()),
        entity_id: Set(change.entity_id.clone()),
        operation: Set(change.operation.clone()),
        version: Set(change.version),
        payload: Set(change.payload.clone()),
        changed_at: Set(now),
        device_id: Set(change.device_id.clone()),
        ..Default::default()
    };
    entry.insert(db).await?;
    Ok(())
}

async fn delete_entity(
    db: &impl ConnectionTrait,
    table: &'static str,
    entity_id: &str,
) -> Result<(), SyncError> {
    if let Ok(Value::Object(key)) = serde_json::from_str::<Value>(entity_id) {
        let mut conditions = Vec::new();
        let mut values: Vec<SqlValue> = Vec::new();
        for (column, value) in key {
            if !is_identifier(&column) {
                return Err(SyncError::Deserialization {
                    entity_type: table.to_string(),
                    message: format!("unsafe key column in entity id: {column}"),
                });
            }
            conditions.push(format!("{column} = ?"));
            values.push(sql_value(&column, &value));
        }
        if conditions.is_empty() {
            return Err(SyncError::Deserialization {
                entity_type: table.to_string(),
                message: "empty compound entity id".to_string(),
            });
        }
        let statement = Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            format!("DELETE FROM {table} WHERE {}", conditions.join(" AND ")),
            values,
        );
        db.execute_raw(statement).await?;
        return Ok(());
    }

    let id = DbId::from_hex(entity_id).map_err(|error| SyncError::Deserialization {
        entity_type: table.to_string(),
        message: format!("invalid entity id {entity_id}: {error}"),
    })?;
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        format!("DELETE FROM {table} WHERE id = ?"),
        [SqlValue::Bytes(Some(id.to_bytes().to_vec()))],
    );
    db.execute_raw(statement).await?;
    Ok(())
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn sql_value(column: &str, value: &Value) -> SqlValue {
    if is_id_column(column) {
        match value {
            // An absent id reaches the wire as `hex(NULL)` -> `''`. Store it
            // as SQL NULL, not a zero-length blob: no `DbId` can decode zero
            // bytes, so a blob there poisons every read of the row.
            Value::String(text) if text.is_empty() => return SqlValue::Bytes(None),
            // Ids arrive as ULID text from a serialized model and as hex from a
            // change payload; the column is `BLOB`, so both forms must decode.
            Value::String(text) => {
                if let Some(id) = DbId::from_wire(text) {
                    return SqlValue::Bytes(Some(id.to_bytes().to_vec()));
                }
            }
            _ => {}
        }
    }
    match value {
        Value::String(text) => SqlValue::String(Some(text.clone())),
        Value::Bool(flag) => SqlValue::Bool(Some(*flag)),
        Value::Number(number) => number.as_i64().map_or_else(
            || SqlValue::Double(number.as_f64()),
            |value| SqlValue::BigInt(Some(value)),
        ),
        Value::Null => SqlValue::String(None),
        other => SqlValue::String(Some(other.to_string())),
    }
}

fn is_id_column(column: &str) -> bool {
    column == "id" || column.ends_with("_id")
}

/// Upsert one entity from its change payload.
///
/// The payload is the canonical trigger output: column names and values, with
/// ids as hex. Writing it straight into the table avoids re-encoding temporal
/// columns and leaves columns the trigger does not emit untouched on conflict.
async fn upsert_payload(
    db: &impl ConnectionTrait,
    table: &'static str,
    payload: &Value,
) -> Result<(), SyncError> {
    let Some(object) = payload.as_object() else {
        return Err(SyncError::Deserialization {
            entity_type: table.to_string(),
            message: "sync payload is not an object".to_string(),
        });
    };

    let mut columns = Vec::new();
    let mut placeholders = Vec::new();
    let mut updates = Vec::new();
    let mut values: Vec<SqlValue> = Vec::new();
    for (column, value) in object {
        if !is_identifier(column) {
            return Err(SyncError::Deserialization {
                entity_type: table.to_string(),
                message: format!("unsafe column name in payload: {column}"),
            });
        }
        columns.push(format!("\"{column}\""));
        placeholders.push("?".to_string());
        updates.push(format!("\"{column}\" = excluded.\"{column}\""));
        values.push(sql_value(column, value));
    }
    if columns.is_empty() {
        return Err(SyncError::Deserialization {
            entity_type: table.to_string(),
            message: "empty sync payload".to_string(),
        });
    }

    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        format!(
            "INSERT INTO {table} ({}) VALUES ({}) ON CONFLICT DO UPDATE SET {}",
            columns.join(", "),
            placeholders.join(", "),
            updates.join(", ")
        ),
        values,
    );
    db.execute_raw(statement).await?;
    Ok(())
}

impl SyncEngine {
    pub async fn list_conflicts(&self) -> Result<Vec<Conflict>, SyncError> {
        let stmt = livtet_data::orm::Statement::from_string(
            livtet_data::orm::DatabaseBackend::Sqlite,
            "SELECT id, entity_type, entity_id, local_payload, remote_payload, \
                    resolved, resolution, merged_payload, detected_at \
             FROM conflicts WHERE resolved = 0 ORDER BY id DESC",
        );
        let rows = self.db.query_all_raw(stmt).await?;
        Ok(rows.iter().map(row_to_conflict).collect())
    }

    pub async fn resolve_conflict(
        &self,
        conflict_id: i64,
        resolution: &str,
        merged_payload: Option<&str>,
    ) -> Result<bool, SyncError> {
        let stmt = match merged_payload {
            Some(merged) => livtet_data::orm::Statement::from_sql_and_values(
                livtet_data::orm::DatabaseBackend::Sqlite,
                "UPDATE conflicts SET resolved = 1, resolution = ?, merged_payload = ? \
                     WHERE id = ? AND resolved = 0",
                [resolution.into(), merged.into(), conflict_id.into()],
            ),
            None => livtet_data::orm::Statement::from_sql_and_values(
                livtet_data::orm::DatabaseBackend::Sqlite,
                "UPDATE conflicts SET resolved = 1, resolution = ?, merged_payload = NULL \
                     WHERE id = ? AND resolved = 0",
                [resolution.into(), conflict_id.into()],
            ),
        };
        let result = self.db.execute_raw(stmt).await?;
        Ok(result.rows_affected() > 0)
    }
}

pub fn entity_type_to_table(entity_type: &str) -> Option<&'static str> {
    crate::types::syncable_entity::entity_type_to_table(entity_type)
}

fn row_to_conflict(row: &livtet_data::orm::QueryResult) -> Conflict {
    Conflict {
        id: row.try_get::<i64>("", "id").unwrap_or_default(),
        entity_type: row.try_get::<String>("", "entity_type").unwrap_or_default(),
        entity_id: row.try_get::<String>("", "entity_id").unwrap_or_default(),
        local_payload: row
            .try_get::<String>("", "local_payload")
            .unwrap_or_default(),
        remote_payload: row
            .try_get::<String>("", "remote_payload")
            .unwrap_or_default(),
        resolved: row.try_get::<i32>("", "resolved").unwrap_or(0) == 1,
        resolution: row.try_get::<String>("", "resolution").ok(),
        merged_payload: row.try_get::<String>("", "merged_payload").ok(),
        detected_at: row.try_get::<String>("", "detected_at").unwrap_or_default(),
    }
}
