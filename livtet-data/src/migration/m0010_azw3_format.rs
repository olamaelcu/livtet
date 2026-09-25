use livtet_types::{DbId, KnownFormats};
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0010-azw3-format"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let format = KnownFormats::Azw3;
        let ulid = format.ulid();
        let schema = serde_json::to_string(&format.schema()).unwrap_or_default();
        let stmt = sea_orm::Statement::from_sql_and_values(
            manager.get_database_backend(),
            r#"
            INSERT OR IGNORE INTO formats (id, name, metadata_schema, created_at)
            VALUES ($1, $2, $3, $4)
            "#,
            [
                DbId(ulid).into(),
                format.name().into(),
                schema.into(),
                seed_now().into(),
            ],
        );
        manager.get_connection().execute_raw(stmt).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Seed data is intentionally not deleted on down-migration.
        // Seeding uses INSERT OR IGNORE and rows are managed by cascading
        // drops from the schema migrations that own the tables.
        Ok(())
    }
}

fn seed_now() -> time::PrimitiveDateTime {
    {
        let n = time::OffsetDateTime::now_utc();
        time::PrimitiveDateTime::new(n.date(), n.time())
    }
}
