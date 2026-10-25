use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0012-shelf-tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        conn.execute_unprepared(
            "CREATE TABLE shelf (
                uri TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT
            )",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE TABLE book_shelf (
                uri TEXT PRIMARY KEY NOT NULL,
                shelf_uri TEXT NOT NULL REFERENCES shelf(uri),
                book_uri TEXT NOT NULL,
                status TEXT NOT NULL,
                position INTEGER,
                notes TEXT,
                emoji TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT,
                did TEXT NOT NULL
            )",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE INDEX idx_book_shelf_shelf ON book_shelf(shelf_uri)",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE INDEX idx_book_shelf_book ON book_shelf(book_uri)",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        conn.execute_unprepared("DROP INDEX IF EXISTS idx_book_shelf_book")
            .await
            .map(|_| ())?;
        conn.execute_unprepared("DROP INDEX IF EXISTS idx_book_shelf_shelf")
            .await
            .map(|_| ())?;
        conn.execute_unprepared("DROP TABLE IF EXISTS book_shelf")
            .await
            .map(|_| ())?;
        conn.execute_unprepared("DROP TABLE IF EXISTS shelf")
            .await
            .map(|_| ())?;
        Ok(())
    }
}
