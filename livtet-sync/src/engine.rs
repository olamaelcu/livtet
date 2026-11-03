//! Local sync engine: reads/writes the `change_log` and `conflicts`
//! SQLite tables directly via sea-orm.  Shared by the desktop server
//! and the FFI — the desktop uses it for the HTTP layer's local
//! queries, and the FFI uses it for the same on the mobile device.

use std::collections::HashMap;

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

        let changes: Vec<SyncChange> = rows
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

        let latest_version = changes.last().map(|c| c.version).unwrap_or(since_version);
        let has_more = changes.len() as i64 == limit;
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
        let digital_inventory = query_table(
            &self.db,
            "digital_inventory",
            &[
                "id",
                "edition_id",
                "file_path",
                "cover_path",
                "file_hash",
                "file_size_bytes",
                "notes",
                "added_at",
                "updated_at",
            ],
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
        let annotations = query_table(
            &self.db,
            "annotations",
            &[
                "id",
                "edition_id",
                "user_id",
                "content",
                "location",
                "created_at",
                "updated_at",
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
                "last_location",
                "total_reading_time_secs",
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
                digital_inventory,
                owned_editions,
                editions_loans,
                annotations,
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
                "SELECT version, payload FROM change_log WHERE entity_type = ? AND entity_id = ? ORDER BY version DESC LIMIT 1",
                [
                    change.entity_type.clone().into(),
                    change.entity_id.clone().into(),
                ],
            );

            if let Some(local_row) = txn.query_one_raw(check).await? {
                let local_version = local_row.try_get::<i64>("", "version").unwrap_or(0);
                let local_payload = local_row
                    .try_get::<String>("", "payload")
                    .unwrap_or_default();

                if local_version > change.version
                    || (local_version == change.version && local_payload != change.payload)
                {
                    let insert_conflict = livtet_data::orm::Statement::from_sql_and_values(
                        livtet_data::orm::DatabaseBackend::Sqlite,
                        "INSERT INTO conflicts (entity_type, entity_id, local_payload, remote_payload, detected_at) \
                         VALUES (?, ?, ?, ?, datetime('now'))",
                        [
                            change.entity_type.clone().into(),
                            change.entity_id.clone().into(),
                            local_payload.into(),
                            change.payload.clone().into(),
                        ],
                    );
                    txn.execute_raw(insert_conflict).await?;

                    let fetch = livtet_data::orm::Statement::from_sql_and_values(
                        livtet_data::orm::DatabaseBackend::Sqlite,
                        "SELECT id, entity_type, entity_id, local_payload, remote_payload, \
                                resolved, resolution, merged_payload, detected_at \
                         FROM conflicts \
                         WHERE entity_type = ? AND entity_id = ? AND resolved = 0 \
                         ORDER BY id DESC LIMIT 1",
                        [
                            change.entity_type.clone().into(),
                            change.entity_id.clone().into(),
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

/// Filesystem paths the receiver assigned to one inventory row.
///
/// The sender's payload carries its own absolute paths; applying those
/// verbatim would point the receiver at another machine's disk.
#[derive(Debug, Clone, Default)]
pub struct InventoryPaths {
    pub file_path: Option<String>,
    pub cover_path: Option<String>,
}

/// Inputs for [`SyncEngine::apply_changes`].
#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// Inventory entity id (hex or ULID) → the receiver's local paths.
    pub inventory_paths: HashMap<String, InventoryPaths>,
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
    pub async fn apply_changes(
        &self,
        changes: Vec<SyncChange>,
        options: ApplyOptions,
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
            let Some(table) = entity_type_to_table(&change.entity_type) else {
                tracing::debug!(
                    entity_type = %change.entity_type,
                    "unknown sync entity ignored during apply"
                );
                outcome.skipped += 1;
                continue;
            };

            let local = latest_change(&txn, &change.entity_type, &change.entity_id).await?;
            if let Some((local_version, local_payload)) = local
                && local_version > change.version
            {
                record_conflict(&txn, change, &local_payload).await?;
                let conflict = fetch_latest_conflict(&txn, change).await?;
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
                let mut payload: Value =
                    serde_json::from_str(&change.payload).map_err(|error| {
                        SyncError::Deserialization {
                            entity_type: change.entity_type.clone(),
                            message: error.to_string(),
                        }
                    })?;
                if change.entity_type == "digital_inventory" {
                    apply_inventory_paths(&mut payload, change, &options);
                }
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

fn apply_inventory_paths(payload: &mut Value, change: &SyncChange, options: &ApplyOptions) {
    let normalized_id = payload.get("id").and_then(Value::as_str);
    let paths = options
        .inventory_paths
        .get(&change.entity_id)
        .or_else(|| normalized_id.and_then(|id| options.inventory_paths.get(id)));
    let Some(paths) = paths else {
        return;
    };
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    object.insert("file_path".to_string(), optional_string(&paths.file_path));
    object.insert("cover_path".to_string(), optional_string(&paths.cover_path));
}

fn optional_string(value: &Option<String>) -> Value {
    value
        .as_deref()
        .map_or(Value::Null, |text| Value::String(text.to_string()))
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
) -> Result<Option<(i64, String)>, SyncError> {
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "SELECT version, payload FROM change_log WHERE entity_type = ? AND entity_id = ? \
         ORDER BY version DESC LIMIT 1",
        [entity_type.into(), entity_id.into()],
    );
    let row = db.query_one_raw(statement).await?;
    Ok(row.and_then(|row| {
        Some((
            row.try_get::<i64>("", "version").ok()?,
            row.try_get::<String>("", "payload").ok()?,
        ))
    }))
}

async fn record_conflict(
    db: &impl ConnectionTrait,
    change: &SyncChange,
    local_payload: &str,
) -> Result<(), SyncError> {
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO conflicts (entity_type, entity_id, local_payload, remote_payload, detected_at) \
         VALUES (?, ?, ?, ?, datetime('now'))",
        [
            change.entity_type.clone().into(),
            change.entity_id.clone().into(),
            local_payload.to_string().into(),
            change.payload.clone().into(),
        ],
    );
    db.execute_raw(statement).await?;
    Ok(())
}

async fn fetch_latest_conflict(
    db: &impl ConnectionTrait,
    change: &SyncChange,
) -> Result<Option<Conflict>, SyncError> {
    let statement = Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "SELECT id, entity_type, entity_id, local_payload, remote_payload, resolved, resolution, \
                merged_payload, detected_at FROM conflicts \
         WHERE entity_type = ? AND entity_id = ? AND resolved = 0 ORDER BY id DESC LIMIT 1",
        [
            change.entity_type.clone().into(),
            change.entity_id.clone().into(),
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
    use livtet_data::orm::Set;

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
            Value::String(text)
                if text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
            {
                if let Ok(id) = DbId::from_hex(text) {
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
