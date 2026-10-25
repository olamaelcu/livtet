//! Shelf entity and mutation/query helpers.
//!
//! A *shelf* is a named reading-status bucket (`to-read`, `reading`, `read`,
//! `dnf`) and a *book shelf* is one edition placed on one shelf by one
//! account. Both mirror the AT Protocol records the desktop publishes, so the
//! record URIs are the primary keys.
use crate::data::orm::ActiveValue::{NotSet, Set};
use crate::data::orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
};

use crate::data::book_shelf;
use crate::data::book_shelf::Entity as BookShelfEntity;
use crate::data::shelf;
use crate::data::shelf::Entity as ShelfEntity;

pub use crate::data::book_shelf::Model as BookShelf;
pub use crate::data::shelf::Model as Shelf;

/// The four shelves every account has, in the order the UI shows them.
#[derive(Clone, Debug, PartialEq, Eq, Copy)]
pub enum ReadingStatus {
    Reading,
    Read,
    ToRead,
    Dnf,
}

impl ReadingStatus {
    /// Shelf collection rkey — also the shelving row's `status`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReadingStatus::Reading => "reading",
            ReadingStatus::Read => "read",
            ReadingStatus::ToRead => "to-read",
            ReadingStatus::Dnf => "dnf",
        }
    }

    /// Display name carried by the shelf record (`shelf.name`).
    pub fn shelf_name(&self) -> &'static str {
        match self {
            ReadingStatus::Reading => "Currently Reading",
            ReadingStatus::Read => "Finished",
            ReadingStatus::ToRead => "Want to Read",
            ReadingStatus::Dnf => "Abandoned",
        }
    }

    /// The status named by a shelf rkey / `book_shelf.status` value.
    pub fn from_key(value: &str) -> Option<Self> {
        match value {
            "reading" => Some(ReadingStatus::Reading),
            "read" => Some(ReadingStatus::Read),
            "to-read" => Some(ReadingStatus::ToRead),
            "dnf" => Some(ReadingStatus::Dnf),
            _ => None,
        }
    }
}

/// Current wall-clock time as the primitive timestamp the entity columns use.
fn now() -> time::PrimitiveDateTime {
    let utc = time::OffsetDateTime::now_utc();
    time::PrimitiveDateTime::new(utc.date(), utc.time())
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
    if shelf_uris.is_empty() {
        return Ok(Vec::new());
    }
    shelf::Entity::find()
        .filter(shelf::Column::Uri.is_in(shelf_uris))
        .all(conn)
        .await
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

pub async fn delete_shelf(conn: &DatabaseConnection, shelf_uri: &str) -> Result<u64, DbErr> {
    ShelfEntity::delete_by_id(shelf_uri)
        .exec(conn)
        .await
        .map(|r| r.rows_affected)
}

pub async fn query_book_shelf(
    conn: &DatabaseConnection,
    shelf_uri: &str,
    book_uri: &str,
) -> Result<Option<BookShelf>, DbErr> {
    use book_shelf::Column::*;
    BookShelfEntity::find()
        .filter(ShelfUri.eq(shelf_uri))
        .filter(BookUri.eq(book_uri))
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
        .filter(ShelfUri.eq(shelf_uri))
        .filter(BookUri.eq(book_uri))
        .exec(conn)
        .await
        .map(|r| r.rows_affected)
}

/// Place `book_uri` on `shelf_uri` for `did`.
///
/// One book has one shelving row per account, so moving a book between
/// shelves retargets the existing row instead of adding a second one; rows
/// left behind for the same book are dropped. Returns the surviving row.
pub async fn upsert_book_shelf(
    conn: &DatabaseConnection,
    did: &str,
    shelf_uri: &str,
    book_uri: &str,
    status: ReadingStatus,
) -> Result<BookShelf, DbErr> {
    let rows = BookShelfEntity::find()
        .filter(book_shelf::Column::BookUri.eq(book_uri))
        .order_by_desc(book_shelf::Column::CreatedAt)
        .all(conn)
        .await?;

    let mut surviving: Option<BookShelf> = None;
    for row in rows {
        if surviving.is_none() {
            surviving = Some(row);
        } else {
            BookShelfEntity::delete_by_id(row.uri).exec(conn).await?;
        }
    }

    match surviving {
        Some(row) => {
            let mut active: book_shelf::ActiveModel = row.into();
            active.shelf_uri = Set(shelf_uri.to_string());
            active.status = Set(status.as_str().to_string());
            active.did = Set(did.to_string());
            active.updated_at = Set(Some(now()));
            update_book_shelf(conn, active).await
        }
        None => {
            let entry = book_shelf::ActiveModel {
                uri: Set(format!("{book_uri}:{shelf_uri}")),
                shelf_uri: Set(shelf_uri.to_string()),
                book_uri: Set(book_uri.to_string()),
                status: Set(status.as_str().to_string()),
                position: NotSet,
                notes: NotSet,
                emoji: NotSet,
                created_at: Set(now()),
                updated_at: NotSet,
                did: Set(did.to_string()),
            };
            insert_book_shelf(conn, entry).await
        }
    }
}

/// The shelf a book currently sits on (newest row wins).
pub async fn shelf_of_book(
    conn: &DatabaseConnection,
    book_uri: &str,
) -> Result<Option<BookShelf>, DbErr> {
    BookShelfEntity::find()
        .filter(book_shelf::Column::BookUri.eq(book_uri))
        .order_by_desc(book_shelf::Column::CreatedAt)
        .one(conn)
        .await
}
