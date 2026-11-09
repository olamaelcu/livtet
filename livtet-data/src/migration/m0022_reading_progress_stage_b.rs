//! Stage B note convergence: note metadata, progress metadata, tombstones.
//!
//! Every existing note is backfilled with `updatedAt` at the epoch and a
//! deterministic `updatedBy`, then sorted by id so two devices that held the
//! same notes serialize them identically. `reading_progress` gains
//! `updated_at`/`updated_by` for progress arbitration.
//!
//! The `reading_annotations` view hides tombstones from readers.

use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0022-reading-progress-stage-b"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("ALTER TABLE reading_progress ADD COLUMN updated_at TEXT")
            .await?;
        db.execute_unprepared("ALTER TABLE reading_progress ADD COLUMN updated_by TEXT")
            .await?;

        backfill(db).await?;

        db.execute_unprepared("DROP VIEW IF EXISTS reading_annotations")
            .await?;
        db.execute_unprepared(
            r#"
CREATE VIEW IF NOT EXISTS reading_annotations AS
SELECT
    rp.edition_id AS edition_id,
    rp.format_id AS format_id,
    json_extract(a.value, '$.id') AS id,
    COALESCE(json_extract(a.value, '$.text'), '') AS content,
    json_extract(a.value, '$.locator') AS location,
    json_extract(a.value, '$.quote') AS quote,
    json_extract(a.value, '$.pageNumber') AS page_number,
    json_extract(a.value, '$.color') AS color,
    COALESCE(json_extract(a.value, '$.createdAt'), rp.created_at) AS created_at,
    json_extract(a.value, '$.updatedAt') AS updated_at
FROM reading_progress rp
JOIN json_each(
    CASE WHEN json_valid(rp.annotations) THEN rp.annotations ELSE '[]' END
) a
WHERE rp.annotations IS NOT NULL
  AND json_extract(a.value, '$.id') IS NOT NULL
  AND json_extract(a.value, '$.deletedAt') IS NULL
"#,
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP VIEW IF EXISTS reading_annotations")
            .await?;
        db.execute_unprepared(
            r#"
CREATE VIEW IF NOT EXISTS reading_annotations AS
SELECT
    rp.edition_id AS edition_id,
    rp.format_id AS format_id,
    json_extract(a.value, '$.id') AS id,
    COALESCE(json_extract(a.value, '$.text'), '') AS content,
    json_extract(a.value, '$.locator') AS location,
    json_extract(a.value, '$.quote') AS quote,
    json_extract(a.value, '$.pageNumber') AS page_number,
    json_extract(a.value, '$.color') AS color,
    COALESCE(json_extract(a.value, '$.createdAt'), rp.created_at) AS created_at
FROM reading_progress rp
JOIN json_each(
    CASE WHEN json_valid(rp.annotations) THEN rp.annotations ELSE '[]' END
) a
WHERE rp.annotations IS NOT NULL
  AND json_extract(a.value, '$.id') IS NOT NULL
"#,
        )
        .await?;
        db.execute_unprepared("ALTER TABLE reading_progress DROP COLUMN updated_by")
            .await?;
        db.execute_unprepared("ALTER TABLE reading_progress DROP COLUMN updated_at")
            .await?;
        Ok(())
    }
}

/// Backfill note and progress metadata for rows written before Stage B.
///
/// Public so tests can exercise the SQL against legacy rows.
pub async fn backfill<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    db.execute_unprepared(
        r#"
UPDATE reading_progress
SET updated_at = COALESCE(
        strftime('%Y-%m-%dT%H:%M:%fZ', created_at),
        '1970-01-01T00:00:00Z'
    ),
    updated_by = COALESCE(updated_by, 'legacy')
WHERE updated_at IS NULL OR updated_by IS NULL
"#,
    )
    .await?;

    // Backfill note metadata and canonicalise the array order. The nested
    // ORDER BY is what makes the json_group_array output stable.
    db.execute_unprepared(
        r#"
UPDATE reading_progress
SET annotations = (
    SELECT json_group_array(json(note))
    FROM (
        SELECT json_set(
                   value,
                   '$.updatedAt',
                   COALESCE(json_extract(value, '$.updatedAt'), '1970-01-01T00:00:00Z'),
                   '$.updatedBy',
                   COALESCE(json_extract(value, '$.updatedBy'), 'legacy')
               ) AS note
        FROM json_each(reading_progress.annotations)
        ORDER BY json_extract(value, '$.id')
    )
)
WHERE annotations IS NOT NULL
  AND json_valid(annotations)
  AND json_type(annotations) = 'array'
"#,
    )
    .await?;
    Ok(())
}
