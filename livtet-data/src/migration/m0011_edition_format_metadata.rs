use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0011-edition-format-metadata"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Nullable JSON-as-TEXT: only some formats (audiobooks today) produce
        // per-edition metadata, and values are validated at the application
        // layer against the edition format's `FormatMetadataSchema`.
        // `TEXT` is STRICT-legal; existing rows read back as NULL.
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE editions ADD COLUMN format_metadata TEXT")
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite supports DROP COLUMN; the data is derived and re-importable.
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE editions DROP COLUMN format_metadata")
            .await
            .map(|_| ())
    }
}
