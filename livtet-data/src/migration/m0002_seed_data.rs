use livtet_types::{
    BookCondition, CommonLanguages, DbId, KnownFormats, KnownGenres, KnownSubjects,
};
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0002-seed_data"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        seed_languages(manager).await?;
        seed_formats(manager).await?;
        seed_book_conditions(manager).await?;
        seed_genres(manager).await?;
        seed_subjects(manager).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Seed data is intentionally not deleted on down-migration.
        // Seeding uses INSERT OR IGNORE and rows are managed by cascading
        // drops from the schema migrations that own the tables.
        Ok(())
    }
}

async fn seed_languages(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for lang in CommonLanguages::all() {
        let ulid = lang.ulid();
        let stmt = sea_orm::Statement::from_sql_and_values(
            manager.get_database_backend(),
            r#"
            INSERT OR IGNORE INTO languages (id, name, code, flag_emoji, created_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
            [
                DbId(ulid).into(),
                lang.name().into(),
                lang.code().into(),
                lang.flag_emoji().into(),
                seed_now().into(),
            ],
        );
        manager.get_connection().execute_raw(stmt).await?;
    }
    Ok(())
}

async fn seed_formats(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for format in KnownFormats::all() {
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
    }
    Ok(())
}

async fn seed_book_conditions(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for cond in BookCondition::all() {
        let ulid = cond.ulid();
        let stmt = sea_orm::Statement::from_sql_and_values(
            manager.get_database_backend(),
            r#"
            INSERT OR IGNORE INTO book_conditions (id, name, value, created_at)
            VALUES ($1, $2, $3, $4)
            "#,
            [
                DbId(ulid).into(),
                cond.name().into(),
                (cond as i32).into(),
                seed_now().into(),
            ],
        );
        manager.get_connection().execute_raw(stmt).await?;
    }
    Ok(())
}

async fn seed_genres(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for genre in KnownGenres::all() {
        let ulid = genre.ulid();
        let stmt = sea_orm::Statement::from_sql_and_values(
            manager.get_database_backend(),
            r#"
            INSERT OR IGNORE INTO genres (id, name, created_at)
            VALUES ($1, $2, $3)
            "#,
            [DbId(ulid).into(), genre.name().into(), seed_now().into()],
        );
        manager.get_connection().execute_raw(stmt).await?;
    }
    Ok(())
}

async fn seed_subjects(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for subject in KnownSubjects::all() {
        let ulid = subject.ulid();
        let stmt = sea_orm::Statement::from_sql_and_values(
            manager.get_database_backend(),
            r#"
            INSERT OR IGNORE INTO subjects (id, name, created_at, updated_at)
            VALUES ($1, $2, $3, $4)
            "#,
            [
                DbId(ulid).into(),
                subject.name().into(),
                seed_now().into(),
                seed_now().into(),
            ],
        );
        manager.get_connection().execute_raw(stmt).await?;
    }
    Ok(())
}

fn seed_now() -> time::PrimitiveDateTime {
    {
        let n = time::OffsetDateTime::now_utc();
        time::PrimitiveDateTime::new(n.date(), n.time())
    }
}
