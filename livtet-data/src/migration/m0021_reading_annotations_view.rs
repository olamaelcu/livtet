use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0021-reading-annotations-view"
    }
}

/// Notes move into the desktop's storage model: a JSON array on the
/// `reading_progress` row (`[{"id","text","locator","createdAt"}]`).
/// `reading_annotations` is the read side that plucks them back out; the old
/// table is folded in and dropped.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Fold existing rows into the latest progress row of their edition.
        db.execute_unprepared(
            r#"
UPDATE reading_progress AS rp
SET annotations = (
    SELECT json_group_array(
        json_object(
            'id', lower(hex(a.id)),
            'text', a.content,
            'locator', a.location,
            'createdAt', a.created_at
        )
    )
    FROM annotations a
    WHERE a.edition_id = rp.edition_id
)
WHERE rp.id = (
    SELECT latest.id FROM reading_progress latest
    WHERE latest.edition_id = rp.edition_id
    ORDER BY latest.created_at DESC, latest.id DESC
    LIMIT 1
)
AND EXISTS (SELECT 1 FROM annotations a WHERE a.edition_id = rp.edition_id)
"#,
        )
        .await?;

        // Annotated editions without a progress row get one, so no note is lost.
        db.execute_unprepared(
            r#"
INSERT INTO reading_progress (
    id, edition_id, format_id, progress, progress_unit, last_location,
    total_reading_time_secs, annotations, created_at
)
SELECT
    randomblob(16),
    notes.edition_id,
    COALESCE(e.format_id, (SELECT f.id FROM formats f ORDER BY f.name LIMIT 1)),
    0.0,
    NULL,
    NULL,
    0,
    (
        SELECT json_group_array(
            json_object(
                'id', lower(hex(a.id)),
                'text', a.content,
                'locator', a.location,
                'createdAt', a.created_at
            )
        )
        FROM annotations a
        WHERE a.edition_id = notes.edition_id
    ),
    CURRENT_TIMESTAMP
FROM (SELECT DISTINCT edition_id FROM annotations) AS notes
JOIN editions e ON e.id = notes.edition_id
WHERE NOT EXISTS (
    SELECT 1 FROM reading_progress rp WHERE rp.edition_id = notes.edition_id
)
AND COALESCE(e.format_id, (SELECT f.id FROM formats f ORDER BY f.name LIMIT 1)) IS NOT NULL
"#,
        )
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

        db.execute_unprepared("DROP TABLE IF EXISTS annotations")
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The JSON notes cannot be turned back into rows without inventing
        // BLOB ids, so a rollback restores the shape, not the data.
        let db = manager.get_connection();
        db.execute_unprepared("DROP VIEW IF EXISTS reading_annotations")
            .await?;
        db.execute_unprepared(
            r#"
CREATE TABLE IF NOT EXISTS annotations (
    id BLOB PRIMARY KEY NOT NULL,
    edition_id BLOB NOT NULL,
    user_id BLOB NOT NULL,
    content TEXT NULL,
    location TEXT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT,
    CONSTRAINT "fk_annotations_edition"
        FOREIGN KEY ("edition_id") REFERENCES "editions" ("id")
        ON DELETE CASCADE
) STRICT
"#,
        )
        .await?;
        Ok(())
    }
}
