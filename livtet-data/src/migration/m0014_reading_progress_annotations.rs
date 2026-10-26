use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0014-reading-progress-annotations"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add annotations column to reading_progress table.
        // Stored as a JSON string: [{"id":"...","text":"...","locator":"..."}, …]
        manager
            .alter_table(
                Table::alter()
                    .table(ReadingProgress::Table)
                    .add_column(
                        ColumnDef::new(ReadingProgress::Annotations)
                            .text()
                            .null()
                            .to_owned(),
                    )
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove annotations column from reading_progress table
        manager
            .alter_table(
                Table::alter()
                    .table(ReadingProgress::Table)
                    .drop_column(ReadingProgress::Annotations)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}