//! Client-owned sync audit triggers.
//!
//! Ports every `sync_*_changelog_*` audit trigger from the former
//! `livtet-sync` crate's `types::change_log` module. The client schema owns
//! these triggers so a future `livtet-sync` crate can carry no DDL at all.
//!
//! Trigger names and SQL bodies are byte-for-byte identical to that source.
//! The `change_log` / `conflicts` tables themselves are owned by
//! [`super::m0001_change_log`]; this migration only installs the triggers
//! that write to `change_log`.
//!
//! SQLite only creates a trigger for a table that already exists, so `up`
//! installs each trigger only when its business table is present. Client
//! migrations can therefore run standalone against a fresh database, and the
//! `CREATE TRIGGER IF NOT EXISTS` bodies keep re-runs idempotent.

use sea_orm_migration::prelude::*;

use super::schema::table_exists;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0005-sync_triggers"
    }
}

/// `(trigger_name, create_sql)` for every sync audit trigger, in the order
/// the source `setup_change_log` installed them. `down` derives its
/// `DROP TRIGGER IF EXISTS` statements from the same names, so the two
/// directions can never drift.
pub const TRIGGER_STATEMENTS: &[(&str, &str)] = &[
    (
        "sync_work_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_changelog_insert AFTER INSERT ON works BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work', lower(hex(new.id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'title', new.title,
            'description', new.description,
            'sort_title', new.sort_title,
            'series_type', new.series_type,
            'language_id', CASE WHEN new.language_id IS NOT NULL THEN lower(hex(new.language_id)) END,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_changelog_update AFTER UPDATE ON works BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work', lower(hex(new.id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'title', new.title,
            'description', new.description,
            'sort_title', new.sort_title,
            'series_type', new.series_type,
            'language_id', CASE WHEN new.language_id IS NOT NULL THEN lower(hex(new.language_id)) END,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_changelog_delete AFTER DELETE ON works BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work', lower(hex(old.id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(old.id)),
            'title', old.title,
            'description', old.description,
            'sort_title', old.sort_title,
            'series_type', old.series_type,
            'language_id', CASE WHEN old.language_id IS NOT NULL THEN lower(hex(old.language_id)) END,
            'created_at', old.created_at,
            'updated_at', old.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_changelog_insert AFTER INSERT ON editions BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition', lower(hex(new.id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'work_id', lower(hex(new.work_id)),
            'title', new.title,
            'published_date', new.published_date,
            'format_id', lower(hex(new.format_id)),
            'language_id', lower(hex(new.language_id)),
            'notes', new.notes,
            'description', new.description,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_changelog_update AFTER UPDATE ON editions BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition', lower(hex(new.id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'work_id', lower(hex(new.work_id)),
            'title', new.title,
            'published_date', new.published_date,
            'format_id', lower(hex(new.format_id)),
            'language_id', lower(hex(new.language_id)),
            'notes', new.notes,
            'description', new.description,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_changelog_delete AFTER DELETE ON editions BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition', lower(hex(old.id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(old.id)),
            'work_id', lower(hex(old.work_id)),
            'title', old.title,
            'published_date', old.published_date,
            'format_id', lower(hex(old.format_id)),
            'language_id', lower(hex(old.language_id)),
            'notes', old.notes,
            'description', old.description,
            'created_at', old.created_at,
            'updated_at', old.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_series_entry_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_series_entry_changelog_insert AFTER INSERT ON series_entries BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'series_entry', lower(hex(new.series_id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'series_id', lower(hex(new.series_id)),
            'edition_id', lower(hex(new.edition_id)),
            'position', new.position,
            'created_at', new.created_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_series_entry_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_series_entry_changelog_update AFTER UPDATE ON series_entries BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'series_entry', lower(hex(new.series_id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'series_id', lower(hex(new.series_id)),
            'edition_id', lower(hex(new.edition_id)),
            'position', new.position,
            'created_at', new.created_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_series_entry_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_series_entry_changelog_delete AFTER DELETE ON series_entries BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'series_entry', lower(hex(old.series_id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'series_id', lower(hex(old.series_id)),
            'edition_id', lower(hex(old.edition_id)),
            'position', old.position,
            'created_at', old.created_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_annotation_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_annotation_changelog_insert AFTER INSERT ON annotations BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'annotation', lower(hex(new.id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'user_id', lower(hex(new.user_id)),
            'content', new.content,
            'location', new.location,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_annotation_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_annotation_changelog_update AFTER UPDATE ON annotations BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'annotation', lower(hex(new.id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'user_id', lower(hex(new.user_id)),
            'content', new.content,
            'location', new.location,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_annotation_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_annotation_changelog_delete AFTER DELETE ON annotations BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'annotation', lower(hex(old.id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(old.id)),
            'edition_id', lower(hex(old.edition_id)),
            'user_id', lower(hex(old.user_id)),
            'content', old.content,
            'location', old.location,
            'created_at', old.created_at,
            'updated_at', old.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_reading_list_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_list_changelog_insert AFTER INSERT ON reading_lists BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_list', lower(hex(new.id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'name', new.name,
            'description', new.description,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_reading_list_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_list_changelog_update AFTER UPDATE ON reading_lists BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_list', lower(hex(new.id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'name', new.name,
            'description', new.description,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_reading_list_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_list_changelog_delete AFTER DELETE ON reading_lists BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_list', lower(hex(old.id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(old.id)),
            'name', old.name,
            'description', old.description,
            'created_at', old.created_at,
            'updated_at', old.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_reading_progress_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_progress_changelog_insert AFTER INSERT ON reading_progress BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_progress', lower(hex(new.id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'format_id', lower(hex(new.format_id)),
            'progress', new.progress,
            'last_location', new.last_location,
            'total_reading_time_secs', new.total_reading_time_secs,
            'created_at', new.created_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_reading_progress_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_progress_changelog_update AFTER UPDATE ON reading_progress BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_progress', lower(hex(new.id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'format_id', lower(hex(new.format_id)),
            'progress', new.progress,
            'last_location', new.last_location,
            'total_reading_time_secs', new.total_reading_time_secs,
            'created_at', new.created_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_reading_progress_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_progress_changelog_delete AFTER DELETE ON reading_progress BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_progress', lower(hex(old.id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(old.id)),
            'edition_id', lower(hex(old.edition_id)),
            'format_id', lower(hex(old.format_id)),
            'progress', old.progress,
            'last_location', old.last_location,
            'total_reading_time_secs', old.total_reading_time_secs,
            'created_at', old.created_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_digital_inventory_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_digital_inventory_changelog_insert AFTER INSERT ON digital_inventory BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'digital_inventory', lower(hex(new.id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'file_path', new.file_path,
            'cover_path', new.cover_path,
            'file_hash', new.file_hash,
            'file_size_bytes', new.file_size_bytes,
            'notes', new.notes,
            'added_at', new.added_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_digital_inventory_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_digital_inventory_changelog_update AFTER UPDATE ON digital_inventory BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'digital_inventory', lower(hex(new.id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'file_path', new.file_path,
            'cover_path', new.cover_path,
            'file_hash', new.file_hash,
            'file_size_bytes', new.file_size_bytes,
            'notes', new.notes,
            'added_at', new.added_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_digital_inventory_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_digital_inventory_changelog_delete AFTER DELETE ON digital_inventory BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'digital_inventory', lower(hex(old.id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(old.id)),
            'edition_id', lower(hex(old.edition_id)),
            'file_path', old.file_path,
            'cover_path', old.cover_path,
            'file_hash', old.file_hash,
            'file_size_bytes', old.file_size_bytes,
            'notes', old.notes,
            'added_at', old.added_at,
            'updated_at', old.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_owned_edition_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_owned_edition_changelog_insert AFTER INSERT ON owned_editions BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'owned_edition', lower(hex(new.id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'acquired_at', new.acquired_at,
            'condition_id', new.condition_id,
            'notes', new.notes,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_owned_edition_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_owned_edition_changelog_update AFTER UPDATE ON owned_editions BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'owned_edition', lower(hex(new.id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'acquired_at', new.acquired_at,
            'condition_id', new.condition_id,
            'notes', new.notes,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_owned_edition_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_owned_edition_changelog_delete AFTER DELETE ON owned_editions BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'owned_edition', lower(hex(old.id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(old.id)),
            'edition_id', lower(hex(old.edition_id)),
            'acquired_at', old.acquired_at,
            'condition_id', old.condition_id,
            'notes', old.notes,
            'created_at', old.created_at,
            'updated_at', old.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_loan_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_loan_changelog_insert AFTER INSERT ON editions_loans BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_loan', lower(hex(new.id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'loan_entity_id', lower(hex(new.loan_entity_id)),
            'owned_edition_id', lower(hex(new.owned_edition_id)),
            'loaned_date', new.loaned_date,
            'due_date', new.due_date,
            'returned_date', new.returned_date
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_loan_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_loan_changelog_update AFTER UPDATE ON editions_loans BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_loan', lower(hex(new.id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'edition_id', lower(hex(new.edition_id)),
            'loan_entity_id', lower(hex(new.loan_entity_id)),
            'owned_edition_id', lower(hex(new.owned_edition_id)),
            'loaned_date', new.loaned_date,
            'due_date', new.due_date,
            'returned_date', new.returned_date
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_loan_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_loan_changelog_delete AFTER DELETE ON editions_loans BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_loan', lower(hex(old.id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(old.id)),
            'edition_id', lower(hex(old.edition_id)),
            'loan_entity_id', lower(hex(old.loan_entity_id)),
            'owned_edition_id', lower(hex(old.owned_edition_id)),
            'loaned_date', old.loaned_date,
            'due_date', old.due_date,
            'returned_date', old.returned_date
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_author_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_author_changelog_insert AFTER INSERT ON work_authors BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_author', json_object('work_id', lower(hex(new.work_id)), 'author_id', lower(hex(new.author_id)), 'role', new.role), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'author_id', lower(hex(new.author_id)), 'role', new.role),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_author_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_author_changelog_delete AFTER DELETE ON work_authors BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_author', json_object('work_id', lower(hex(old.work_id)), 'author_id', lower(hex(old.author_id)), 'role', old.role), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'author_id', lower(hex(old.author_id)), 'role', old.role),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_tag_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_tag_changelog_insert AFTER INSERT ON work_tags BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_tag', json_object('work_id', lower(hex(new.work_id)), 'tag_id', lower(hex(new.tag_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'tag_id', lower(hex(new.tag_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_tag_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_tag_changelog_delete AFTER DELETE ON work_tags BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_tag', json_object('work_id', lower(hex(old.work_id)), 'tag_id', lower(hex(old.tag_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'tag_id', lower(hex(old.tag_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_genre_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_genre_changelog_insert AFTER INSERT ON work_genres BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_genre', json_object('work_id', lower(hex(new.work_id)), 'genre_id', lower(hex(new.genre_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'genre_id', lower(hex(new.genre_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_genre_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_genre_changelog_delete AFTER DELETE ON work_genres BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_genre', json_object('work_id', lower(hex(old.work_id)), 'genre_id', lower(hex(old.genre_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'genre_id', lower(hex(old.genre_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_subject_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_subject_changelog_insert AFTER INSERT ON work_subjects BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_subject', json_object('work_id', lower(hex(new.work_id)), 'subject_id', lower(hex(new.subject_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'subject_id', lower(hex(new.subject_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_subject_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_subject_changelog_delete AFTER DELETE ON work_subjects BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_subject', json_object('work_id', lower(hex(old.work_id)), 'subject_id', lower(hex(old.subject_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'subject_id', lower(hex(old.subject_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_publisher_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_publisher_changelog_insert AFTER INSERT ON work_publishers BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_publisher', json_object('work_id', lower(hex(new.work_id)), 'publisher_id', lower(hex(new.publisher_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'publisher_id', lower(hex(new.publisher_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_work_publisher_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_work_publisher_changelog_delete AFTER DELETE ON work_publishers BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_publisher', json_object('work_id', lower(hex(old.work_id)), 'publisher_id', lower(hex(old.publisher_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'publisher_id', lower(hex(old.publisher_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_author_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_author_changelog_insert AFTER INSERT ON edition_authors BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_author', json_object('edition_id', lower(hex(new.edition_id)), 'author_id', lower(hex(new.author_id)), 'role', new.role), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'author_id', lower(hex(new.author_id)), 'role', new.role),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_author_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_author_changelog_delete AFTER DELETE ON edition_authors BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_author', json_object('edition_id', lower(hex(old.edition_id)), 'author_id', lower(hex(old.author_id)), 'role', old.role), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'author_id', lower(hex(old.author_id)), 'role', old.role),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_tag_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_tag_changelog_insert AFTER INSERT ON edition_tags BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_tag', json_object('edition_id', lower(hex(new.edition_id)), 'tag_id', lower(hex(new.tag_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'tag_id', lower(hex(new.tag_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_tag_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_tag_changelog_delete AFTER DELETE ON edition_tags BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_tag', json_object('edition_id', lower(hex(old.edition_id)), 'tag_id', lower(hex(old.tag_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'tag_id', lower(hex(old.tag_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_genre_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_genre_changelog_insert AFTER INSERT ON edition_genres BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_genre', json_object('edition_id', lower(hex(new.edition_id)), 'genre_id', lower(hex(new.genre_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'genre_id', lower(hex(new.genre_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_genre_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_genre_changelog_delete AFTER DELETE ON edition_genres BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_genre', json_object('edition_id', lower(hex(old.edition_id)), 'genre_id', lower(hex(old.genre_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'genre_id', lower(hex(old.genre_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_subject_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_subject_changelog_insert AFTER INSERT ON edition_subjects BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_subject', json_object('edition_id', lower(hex(new.edition_id)), 'subject_id', lower(hex(new.subject_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'subject_id', lower(hex(new.subject_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_subject_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_subject_changelog_delete AFTER DELETE ON edition_subjects BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_subject', json_object('edition_id', lower(hex(old.edition_id)), 'subject_id', lower(hex(old.subject_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'subject_id', lower(hex(old.subject_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_publisher_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_publisher_changelog_insert AFTER INSERT ON edition_publishers BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_publisher', json_object('edition_id', lower(hex(new.edition_id)), 'publisher_id', lower(hex(new.publisher_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'publisher_id', lower(hex(new.publisher_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_publisher_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_publisher_changelog_delete AFTER DELETE ON edition_publishers BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_publisher', json_object('edition_id', lower(hex(old.edition_id)), 'publisher_id', lower(hex(old.publisher_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'publisher_id', lower(hex(old.publisher_id))),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_group_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_group_changelog_insert AFTER INSERT ON edition_groups BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_group', lower(hex(new.id)), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'label', new.label,
            'description', new.description,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_group_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_group_changelog_update AFTER UPDATE ON edition_groups BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_group', lower(hex(new.id)), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(new.id)),
            'label', new.label,
            'description', new.description,
            'created_at', new.created_at,
            'updated_at', new.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_edition_group_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_group_changelog_delete AFTER DELETE ON edition_groups BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_group', lower(hex(old.id)), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'id', lower(hex(old.id)),
            'label', old.label,
            'description', old.description,
            'created_at', old.created_at,
            'updated_at', old.updated_at
        ),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_reading_list_book_changelog_insert",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_list_book_changelog_insert AFTER INSERT ON reading_list_book BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_list_book', json_object('reading_list_id', lower(hex(new.reading_list_id)), 'edition_id', lower(hex(new.edition_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('reading_list_id', lower(hex(new.reading_list_id)), 'edition_id', lower(hex(new.edition_id)), 'position', new.position, 'added_at', new.added_at),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_reading_list_book_changelog_update",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_list_book_changelog_update AFTER UPDATE ON reading_list_book BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_list_book', json_object('reading_list_id', lower(hex(new.reading_list_id)), 'edition_id', lower(hex(new.edition_id))), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('reading_list_id', lower(hex(new.reading_list_id)), 'edition_id', lower(hex(new.edition_id)), 'position', new.position, 'added_at', new.added_at),
        datetime('now'), 'local'
    );
END
"#,
    ),
    (
        "sync_reading_list_book_changelog_delete",
        r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_list_book_changelog_delete AFTER DELETE ON reading_list_book BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_list_book', json_object('reading_list_id', lower(hex(old.reading_list_id)), 'edition_id', lower(hex(old.edition_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('reading_list_id', lower(hex(old.reading_list_id)), 'edition_id', lower(hex(old.edition_id)), 'position', old.position, 'added_at', old.added_at),
        datetime('now'), 'local'
    );
END
"#,
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for &(name, sql) in TRIGGER_STATEMENTS {
            let Some(table) = trigger_table(sql) else {
                return Err(DbErr::Custom(format!(
                    "could not determine target table for trigger `{name}`"
                )));
            };
            // SQLite refuses to create a trigger for a missing table, so
            // install each trigger only when its business table exists. This
            // keeps the client migrations runnable standalone while the
            // `IF NOT EXISTS` bodies keep re-runs idempotent.
            if table_exists(manager, table).await? {
                manager
                    .get_connection()
                    .execute_unprepared(sql)
                    .await
                    .map(|_| ())?;
            }
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for &(name, _sql) in TRIGGER_STATEMENTS {
            let drop = format!("DROP TRIGGER IF EXISTS {name}");
            manager
                .get_connection()
                .execute_unprepared(&drop)
                .await
                .map(|_| ())?;
        }
        Ok(())
    }
}

/// Extract the target table from a `CREATE TRIGGER ... ON <table> BEGIN ...`
/// statement. Our trigger bodies never contain the ` ON ` sequence, so the
/// first occurrence is always the trigger's `ON`.
fn trigger_table(sql: &str) -> Option<&str> {
    sql.split_once(" ON ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
}
