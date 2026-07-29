//! Local sync engine: reads/writes the `change_log` and `conflicts`
//! SQLite tables directly via sea-orm.  Shared by the desktop server
//! and the FFI — the desktop uses it for the HTTP layer's local
//! queries, and the FFI uses it for the same on the mobile device.

use livtet_types::DbId;
use livtet_data::orm::{ConnectionTrait, TransactionTrait};

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

        for change in &changes {
            let table = entity_type_to_table(&change.entity_type).ok_or_else(|| {
                SyncError::UnknownEntityType {
                    type_name: change.entity_type.clone(),
                }
            })?;

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

                if local_version > change.version {
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

            match change.operation.as_str() {
                "INSERT" => {
                    txn.execute_raw(livtet_data::orm::Statement::from_sql_and_values(
                        livtet_data::orm::DatabaseBackend::Sqlite,
                        format!("INSERT OR IGNORE INTO {} (id) VALUES (?)", table),
                        [hex::decode(&change.entity_id).unwrap_or_default().into()],
                    ))
                    .await?;
                }
                "DELETE" => {
                    txn.execute_raw(livtet_data::orm::Statement::from_sql_and_values(
                        livtet_data::orm::DatabaseBackend::Sqlite,
                        format!("DELETE FROM {} WHERE id = ?", table),
                        [hex::decode(&change.entity_id).unwrap_or_default().into()],
                    ))
                    .await?;
                }
                _ => {
                    tracing::debug!(
                        operation = %change.operation,
                        entity_type = %change.entity_type,
                        "Unknown sync operation ignored during push"
                    );
                }
            }

            use livtet_data::client_entities::change_log;
            use livtet_data::orm::{ActiveModelTrait, Set};
            let now = time::OffsetDateTime::now_utc()
                .format(
                    &time::format_description::parse_borrowed::<2>(
                        "[year]-[month]-[day] [hour]:[minute]:[second]",
                    )
                    .unwrap(),
                )
                .unwrap();
            let log_entry = change_log::ActiveModel {
                entity_type: Set(change.entity_type.clone()),
                entity_id: Set(change.entity_id.clone()),
                operation: Set(change.operation.clone()),
                version: Set(change.version),
                payload: Set(change.payload.clone()),
                changed_at: Set(now.to_string()),
                device_id: Set(change.device_id.clone()),
                ..Default::default()
            };
            log_entry.insert(&txn).await?;
        }

        txn.commit().await?;

        let latest_version = self.get_latest_version().await.unwrap_or(0);
        Ok(PushResponse {
            accepted: conflicts.is_empty(),
            conflicts,
            latest_version,
        })
    }

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
        id: row.try_get::<DbId>("", "id").unwrap_or_default(),
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
