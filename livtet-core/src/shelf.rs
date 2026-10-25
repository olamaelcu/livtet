//! Shelf entity and mutation/query helpers.
use crate::data::sql::{SqlitePool as DatabaseConnection};
use crate::data::orm::{
    prelude::*, DbErr, EntityTrait, QueryFilter, QueryOrder,
};

use crate::data::{shelf, shelf::Entity as ShelfEntity};
use crate::data::{book_shelf, book_shelf::Entity as BookShelfEntity};
use crate::data::shelf::ActiveModel as ShelfActiveModel;

pub use crate::data::shelf::Model as Shelf;
pub use crate::data::book_shelf::Model as BookShelf;

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
pub enum ReadingStatus {
    Reading,
    Read,
    ToRead,
    Dnf,
}

impl ReadingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReadingStatus::Reading => "reading",
            ReadingStatus::Read => "read",
            ReadingStatus::ToRead => "to-read",
            ReadingStatus::Dnf => "dnf",
        }
    }
}

pub struct ShelfEvent {
    pub event: &'static str,
    pub shelf_uri: String,
    pub did: String,
}

pub async fn query_shelf(
    conn: &DatabaseConnection,
    shelf_uri: &str,
) -> Result<Option<Shelf>, DbErr> {
    ShelfEntity::find_by_id(shelf_uri).one(conn).await
}

pub async fn list_shelves_for_did(
    conn: &DatabaseConnection,
    did: &str,
) -> Result<Vec<Shelf>, DbErr> {
    // Aggregate shelf_uris via book_shelf rows for this did
    let entries = book_shelf::Entity::find()
        .filter(book_shelf::Column::Did.eq(did))
        .all(conn)
        .await?;
    let mut shelf_uris: Vec<String> = Vec::new();
    for e in entries {
        if !shelf_uris.contains(&e.shelf_uri) {
            shelf_uris.push(e.shelf_uri);
        }
    }
    let results = shelf::Entity::find()
        .filter(shelf::Column::Uri.is_in(shelf_uris))
        .all(conn)
        .await?;
    Ok(results)
}

pub async fn insert_shelf(
    conn: &DatabaseConnection,
    shelf: shelf::ActiveModel,
) -> Result<Shelf, DbErr> {
    ShelfEntity::insert(shelf).exec_with_returning(conn).await
}

pub async fn update_shelf(
    conn: &DatabaseConnection,
    shelf: shelf::ActiveModel,
) -> Result<Shelf, DbErr> {
    shelf.update(conn).await
}

pub async fn delete_shelf(
    conn: &DatabaseConnection,
    shelf_uri: &str,
) -> Result<u64, DbErr> {
    ShelfEntity::delete_by_id(shelf_uri).exec(conn).await.map(|r| r.rows_affected)
}

pub async fn query_book_shelf(
    conn: &DatabaseConnection,
    shelf_uri: &str,
    book_uri: &str,
) -> Result<Option<BookShelf>, DbErr> {
    use book_shelf::Column::*;
    BookShelfEntity::find()
        .filter(ShelfUri.eq(shelf_uri).and(BookUri.eq(book_uri)))
        .one(conn)
        .await
}

pub async fn list_book_shelves(
    conn: &DatabaseConnection,
    shelf_uri: Option<&str>,
    did: Option<&str>,
    book_uri: Option<&str>,
) -> Result<Vec<BookShelf>, DbErr> {
    let mut q = BookShelfEntity::find();
    use book_shelf::Column::*;
    if let Some(uri) = shelf_uri {
        q = q.filter(ShelfUri.eq(uri));
    }
    if let Some(id) = did {
        q = q.filter(Did.eq(id));
    }
    if let Some(b) = book_uri {
        q = q.filter(BookUri.eq(b));
    }
    q.all(conn).await
}

pub async fn insert_book_shelf(
    conn: &DatabaseConnection,
    entry: book_shelf::ActiveModel,
) -> Result<BookShelf, DbErr> {
    BookShelfEntity::insert(entry).exec_with_returning(conn).await
}

pub async fn update_book_shelf(
    conn: &DatabaseConnection,
    entry: book_shelf::ActiveModel,
) -> Result<BookShelf, DbErr> {
    entry.update(conn).await
}

pub async fn delete_book_shelf(
    conn: &DatabaseConnection,
    shelf_uri: &str,
    book_uri: &str,
) -> Result<u64, DbErr> {
    use book_shelf::Column::*;
    BookShelfEntity::delete_many()
        .filter(ShelfUri.eq(shelf_uri).and(BookUri.eq(book_uri)))
        .exec(conn)
        .await
        .map(|r| r.rows_affected)
}
