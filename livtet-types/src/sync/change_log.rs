use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};

pub const CHANGE_LOG_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS change_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type  TEXT    NOT NULL,
    entity_id    TEXT    NOT NULL,
    operation    TEXT    NOT NULL,
    version      INTEGER NOT NULL,
    payload      TEXT    NOT NULL,
    changed_at   TEXT    NOT NULL,
    device_id    TEXT    NOT NULL
)
"#;

pub const CONFLICTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS conflicts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type    TEXT    NOT NULL,
    entity_id      TEXT    NOT NULL,
    local_payload  TEXT    NOT NULL,
    remote_payload TEXT    NOT NULL,
    resolved       INTEGER NOT NULL DEFAULT 0,
    resolution     TEXT,
    merged_payload TEXT,
    detected_at    TEXT    NOT NULL
)
"#;

// ─── work ───────────────────────────────────────────────────────────────

pub const SYNC_WORK_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_WORK_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_WORK_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── edition ────────────────────────────────────────────────────────────

pub const SYNC_EDITION_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_EDITION_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_EDITION_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── series entry ───────────────────────────────────────────────────────

pub const SYNC_SERIES_ENTRY_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_SERIES_ENTRY_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_SERIES_ENTRY_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── annotation ─────────────────────────────────────────────────────────

pub const SYNC_ANNOTATION_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_ANNOTATION_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_ANNOTATION_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── reading list ───────────────────────────────────────────────────────

pub const SYNC_READING_LIST_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_READING_LIST_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_READING_LIST_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── reading progress ───────────────────────────────────────────────────

pub const SYNC_READING_PROGRESS_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_READING_PROGRESS_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_READING_PROGRESS_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── digital inventory ──────────────────────────────────────────────────

pub const SYNC_DIGITAL_INVENTORY_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_DIGITAL_INVENTORY_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_DIGITAL_INVENTORY_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── owned edition ──────────────────────────────────────────────────────

pub const SYNC_OWNED_EDITION_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_OWNED_EDITION_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_OWNED_EDITION_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── edition loan ───────────────────────────────────────────────────────

pub const SYNC_EDITION_LOAN_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_EDITION_LOAN_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_EDITION_LOAN_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── work author ────────────────────────────────────────────────────────

pub const SYNC_WORK_AUTHOR_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_author_changelog_insert AFTER INSERT ON work_authors BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_author', json_object('work_id', lower(hex(new.work_id)), 'author_id', lower(hex(new.author_id)), 'role', new.role), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'author_id', lower(hex(new.author_id)), 'role', new.role),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_WORK_AUTHOR_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_author_changelog_delete AFTER DELETE ON work_authors BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_author', json_object('work_id', lower(hex(old.work_id)), 'author_id', lower(hex(old.author_id)), 'role', old.role), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'author_id', lower(hex(old.author_id)), 'role', old.role),
        datetime('now'), 'local'
    );
END
"#;

// ─── work tag ───────────────────────────────────────────────────────────

pub const SYNC_WORK_TAG_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_tag_changelog_insert AFTER INSERT ON work_tags BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_tag', json_object('work_id', lower(hex(new.work_id)), 'tag_id', lower(hex(new.tag_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'tag_id', lower(hex(new.tag_id))),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_WORK_TAG_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_tag_changelog_delete AFTER DELETE ON work_tags BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_tag', json_object('work_id', lower(hex(old.work_id)), 'tag_id', lower(hex(old.tag_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'tag_id', lower(hex(old.tag_id))),
        datetime('now'), 'local'
    );
END
"#;

// ─── work genre ─────────────────────────────────────────────────────────

pub const SYNC_WORK_GENRE_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_genre_changelog_insert AFTER INSERT ON work_genres BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_genre', json_object('work_id', lower(hex(new.work_id)), 'genre_id', lower(hex(new.genre_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'genre_id', lower(hex(new.genre_id))),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_WORK_GENRE_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_genre_changelog_delete AFTER DELETE ON work_genres BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_genre', json_object('work_id', lower(hex(old.work_id)), 'genre_id', lower(hex(old.genre_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'genre_id', lower(hex(old.genre_id))),
        datetime('now'), 'local'
    );
END
"#;

// ─── work subject ───────────────────────────────────────────────────────

pub const SYNC_WORK_SUBJECT_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_subject_changelog_insert AFTER INSERT ON work_subjects BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_subject', json_object('work_id', lower(hex(new.work_id)), 'subject_id', lower(hex(new.subject_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'subject_id', lower(hex(new.subject_id))),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_WORK_SUBJECT_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_subject_changelog_delete AFTER DELETE ON work_subjects BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_subject', json_object('work_id', lower(hex(old.work_id)), 'subject_id', lower(hex(old.subject_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'subject_id', lower(hex(old.subject_id))),
        datetime('now'), 'local'
    );
END
"#;

// ─── work publisher ─────────────────────────────────────────────────────

pub const SYNC_WORK_PUBLISHER_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_publisher_changelog_insert AFTER INSERT ON work_publishers BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_publisher', json_object('work_id', lower(hex(new.work_id)), 'publisher_id', lower(hex(new.publisher_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(new.work_id)), 'publisher_id', lower(hex(new.publisher_id))),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_WORK_PUBLISHER_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_work_publisher_changelog_delete AFTER DELETE ON work_publishers BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'work_publisher', json_object('work_id', lower(hex(old.work_id)), 'publisher_id', lower(hex(old.publisher_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('work_id', lower(hex(old.work_id)), 'publisher_id', lower(hex(old.publisher_id))),
        datetime('now'), 'local'
    );
END
"#;

// ─── edition author ─────────────────────────────────────────────────────

pub const SYNC_EDITION_AUTHOR_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_author_changelog_insert AFTER INSERT ON edition_authors BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_author', json_object('edition_id', lower(hex(new.edition_id)), 'author_id', lower(hex(new.author_id)), 'role', new.role), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'author_id', lower(hex(new.author_id)), 'role', new.role),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_EDITION_AUTHOR_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_author_changelog_delete AFTER DELETE ON edition_authors BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_author', json_object('edition_id', lower(hex(old.edition_id)), 'author_id', lower(hex(old.author_id)), 'role', old.role), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'author_id', lower(hex(old.author_id)), 'role', old.role),
        datetime('now'), 'local'
    );
END
"#;

// ─── edition tag ────────────────────────────────────────────────────────

pub const SYNC_EDITION_TAG_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_tag_changelog_insert AFTER INSERT ON edition_tags BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_tag', json_object('edition_id', lower(hex(new.edition_id)), 'tag_id', lower(hex(new.tag_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'tag_id', lower(hex(new.tag_id))),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_EDITION_TAG_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_tag_changelog_delete AFTER DELETE ON edition_tags BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_tag', json_object('edition_id', lower(hex(old.edition_id)), 'tag_id', lower(hex(old.tag_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'tag_id', lower(hex(old.tag_id))),
        datetime('now'), 'local'
    );
END
"#;

// ─── edition genre ──────────────────────────────────────────────────────

pub const SYNC_EDITION_GENRE_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_genre_changelog_insert AFTER INSERT ON edition_genres BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_genre', json_object('edition_id', lower(hex(new.edition_id)), 'genre_id', lower(hex(new.genre_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'genre_id', lower(hex(new.genre_id))),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_EDITION_GENRE_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_genre_changelog_delete AFTER DELETE ON edition_genres BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_genre', json_object('edition_id', lower(hex(old.edition_id)), 'genre_id', lower(hex(old.genre_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'genre_id', lower(hex(old.genre_id))),
        datetime('now'), 'local'
    );
END
"#;

// ─── edition subject ────────────────────────────────────────────────────

pub const SYNC_EDITION_SUBJECT_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_subject_changelog_insert AFTER INSERT ON edition_subjects BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_subject', json_object('edition_id', lower(hex(new.edition_id)), 'subject_id', lower(hex(new.subject_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'subject_id', lower(hex(new.subject_id))),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_EDITION_SUBJECT_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_subject_changelog_delete AFTER DELETE ON edition_subjects BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_subject', json_object('edition_id', lower(hex(old.edition_id)), 'subject_id', lower(hex(old.subject_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'subject_id', lower(hex(old.subject_id))),
        datetime('now'), 'local'
    );
END
"#;

// ─── edition publisher ──────────────────────────────────────────────────

pub const SYNC_EDITION_PUBLISHER_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_publisher_changelog_insert AFTER INSERT ON edition_publishers BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_publisher', json_object('edition_id', lower(hex(new.edition_id)), 'publisher_id', lower(hex(new.publisher_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(new.edition_id)), 'publisher_id', lower(hex(new.publisher_id))),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_EDITION_PUBLISHER_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_publisher_changelog_delete AFTER DELETE ON edition_publishers BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_publisher', json_object('edition_id', lower(hex(old.edition_id)), 'publisher_id', lower(hex(old.publisher_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('edition_id', lower(hex(old.edition_id)), 'publisher_id', lower(hex(old.publisher_id))),
        datetime('now'), 'local'
    );
END
"#;

// ─── edition group ────────────────────────────────────────────────────

pub const SYNC_EDITION_GROUP_CHANGELOG_INSERT: &str = r#"
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
"#;

pub const SYNC_EDITION_GROUP_CHANGELOG_UPDATE: &str = r#"
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
"#;

pub const SYNC_EDITION_GROUP_CHANGELOG_DELETE: &str = r#"
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
"#;

// ─── reading list book ──────────────────────────────────────────────────

pub const SYNC_READING_LIST_BOOK_CHANGELOG_INSERT: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_list_book_changelog_insert AFTER INSERT ON reading_list_book BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_list_book', json_object('reading_list_id', lower(hex(new.reading_list_id)), 'edition_id', lower(hex(new.edition_id))), 'INSERT',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('reading_list_id', lower(hex(new.reading_list_id)), 'edition_id', lower(hex(new.edition_id)), 'position', new.position, 'added_at', new.added_at),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_READING_LIST_BOOK_CHANGELOG_UPDATE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_list_book_changelog_update AFTER UPDATE ON reading_list_book BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_list_book', json_object('reading_list_id', lower(hex(new.reading_list_id)), 'edition_id', lower(hex(new.edition_id))), 'UPDATE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('reading_list_id', lower(hex(new.reading_list_id)), 'edition_id', lower(hex(new.edition_id)), 'position', new.position, 'added_at', new.added_at),
        datetime('now'), 'local'
    );
END
"#;

pub const SYNC_READING_LIST_BOOK_CHANGELOG_DELETE: &str = r#"
CREATE TRIGGER IF NOT EXISTS sync_reading_list_book_changelog_delete AFTER DELETE ON reading_list_book BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'reading_list_book', json_object('reading_list_id', lower(hex(old.reading_list_id)), 'edition_id', lower(hex(old.edition_id))), 'DELETE',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object('reading_list_id', lower(hex(old.reading_list_id)), 'edition_id', lower(hex(old.edition_id)), 'position', old.position, 'added_at', old.added_at),
        datetime('now'), 'local'
    );
END
"#;

// ─── setup ──────────────────────────────────────────────────────────────

pub async fn setup_change_log(db: &DatabaseConnection) -> Result<(), DbErr> {
    let builder = db.get_database_backend();
    let stmts = [
        CHANGE_LOG_TABLE,
        CONFLICTS_TABLE,
        SYNC_WORK_CHANGELOG_INSERT,
        SYNC_WORK_CHANGELOG_UPDATE,
        SYNC_WORK_CHANGELOG_DELETE,
        SYNC_EDITION_CHANGELOG_INSERT,
        SYNC_EDITION_CHANGELOG_UPDATE,
        SYNC_EDITION_CHANGELOG_DELETE,
        SYNC_SERIES_ENTRY_CHANGELOG_INSERT,
        SYNC_SERIES_ENTRY_CHANGELOG_UPDATE,
        SYNC_SERIES_ENTRY_CHANGELOG_DELETE,
        SYNC_ANNOTATION_CHANGELOG_INSERT,
        SYNC_ANNOTATION_CHANGELOG_UPDATE,
        SYNC_ANNOTATION_CHANGELOG_DELETE,
        SYNC_READING_LIST_CHANGELOG_INSERT,
        SYNC_READING_LIST_CHANGELOG_UPDATE,
        SYNC_READING_LIST_CHANGELOG_DELETE,
        SYNC_READING_PROGRESS_CHANGELOG_INSERT,
        SYNC_READING_PROGRESS_CHANGELOG_UPDATE,
        SYNC_READING_PROGRESS_CHANGELOG_DELETE,
        SYNC_DIGITAL_INVENTORY_CHANGELOG_INSERT,
        SYNC_DIGITAL_INVENTORY_CHANGELOG_UPDATE,
        SYNC_DIGITAL_INVENTORY_CHANGELOG_DELETE,
        SYNC_OWNED_EDITION_CHANGELOG_INSERT,
        SYNC_OWNED_EDITION_CHANGELOG_UPDATE,
        SYNC_OWNED_EDITION_CHANGELOG_DELETE,
        SYNC_EDITION_LOAN_CHANGELOG_INSERT,
        SYNC_EDITION_LOAN_CHANGELOG_UPDATE,
        SYNC_EDITION_LOAN_CHANGELOG_DELETE,
        SYNC_WORK_AUTHOR_CHANGELOG_INSERT,
        SYNC_WORK_AUTHOR_CHANGELOG_DELETE,
        SYNC_WORK_TAG_CHANGELOG_INSERT,
        SYNC_WORK_TAG_CHANGELOG_DELETE,
        SYNC_WORK_GENRE_CHANGELOG_INSERT,
        SYNC_WORK_GENRE_CHANGELOG_DELETE,
        SYNC_WORK_SUBJECT_CHANGELOG_INSERT,
        SYNC_WORK_SUBJECT_CHANGELOG_DELETE,
        SYNC_WORK_PUBLISHER_CHANGELOG_INSERT,
        SYNC_WORK_PUBLISHER_CHANGELOG_DELETE,
        SYNC_EDITION_AUTHOR_CHANGELOG_INSERT,
        SYNC_EDITION_AUTHOR_CHANGELOG_DELETE,
        SYNC_EDITION_TAG_CHANGELOG_INSERT,
        SYNC_EDITION_TAG_CHANGELOG_DELETE,
        SYNC_EDITION_GENRE_CHANGELOG_INSERT,
        SYNC_EDITION_GENRE_CHANGELOG_DELETE,
        SYNC_EDITION_SUBJECT_CHANGELOG_INSERT,
        SYNC_EDITION_SUBJECT_CHANGELOG_DELETE,
        SYNC_EDITION_PUBLISHER_CHANGELOG_INSERT,
        SYNC_EDITION_PUBLISHER_CHANGELOG_DELETE,
        SYNC_EDITION_GROUP_CHANGELOG_INSERT,
        SYNC_EDITION_GROUP_CHANGELOG_UPDATE,
        SYNC_EDITION_GROUP_CHANGELOG_DELETE,
        SYNC_READING_LIST_BOOK_CHANGELOG_INSERT,
        SYNC_READING_LIST_BOOK_CHANGELOG_UPDATE,
        SYNC_READING_LIST_BOOK_CHANGELOG_DELETE,
    ];
    for sql in stmts {
        db.execute_raw(Statement::from_string(builder, sql.to_string()))
            .await?;
    }
    Ok(())
}
