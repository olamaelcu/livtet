use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0018-drop_edition_specific_covers"
    }
}

/// Collapse cover storage onto `digital_inventory`.
///
/// `edition_specific_covers` duplicated what `digital_inventory.cover_path`
/// already held for every edition that had a file, and the `edition_covers`
/// view existed only to coalesce the two. The table also allowed several rows
/// per edition with no unique key, so the view could emit an edition twice.
///
/// The view is rebuilt over the one remaining source and now also carries the
/// catalog URL, so a reader has a single place to ask for an edition's cover —
/// local file or remote reference.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP VIEW IF EXISTS edition_covers")
            .await?;
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS edition_specific_covers")
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE VIEW IF NOT EXISTS edition_covers AS
SELECT
    e.id AS edition_id,
    di.cover_path AS cover_path,
    CASE
        WHEN di.cover_path IS NOT NULL THEN 'inventory'
        ELSE NULL
    END AS cover_source,
    e.cover_image_url AS cover_url
FROM editions e
LEFT JOIN digital_inventory di ON di.edition_id = e.id",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The dropped rows cannot come back, so the table is recreated empty and
        // the old view is restored over it: a rollback restores the shape, not
        // the data.
        manager
            .get_connection()
            .execute_unprepared("DROP VIEW IF EXISTS edition_covers")
            .await?;
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS edition_specific_covers (
                    id BLOB PRIMARY KEY NOT NULL,
                    edition_id BLOB NOT NULL,
                    cover_path TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT,
                    CONSTRAINT \"fk_edition_specific_covers_edition\"
                        FOREIGN KEY (\"edition_id\") REFERENCES \"editions\" (\"id\")
                        ON DELETE CASCADE
                ) STRICT",
            )
            .await?;
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE VIEW IF NOT EXISTS edition_covers AS
SELECT
    e.id AS edition_id,
    COALESCE(di.cover_path, esc.cover_path) AS cover_path,
    CASE
        WHEN di.cover_path IS NOT NULL THEN 'inventory'
        WHEN esc.cover_path IS NOT NULL THEN 'manual'
        ELSE NULL
    END AS cover_source
FROM editions e
LEFT JOIN digital_inventory di ON di.edition_id = e.id
LEFT JOIN edition_specific_covers esc ON esc.edition_id = e.id",
            )
            .await?;
        Ok(())
    }
}
