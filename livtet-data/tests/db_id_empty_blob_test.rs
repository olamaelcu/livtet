//! Zero-length id blobs decode as `NULL`, not as an undecodable id.
//!
//! Sync's change trigger serialises a `NULL` id as `hex(NULL)`, which SQLite
//! renders as `''`, and the receiving engine used to store that back as a
//! zero-length BLOB. No `DbId` can decode zero bytes, so one such value made
//! every query for the row fail — the desktop showed "Could not load this
//! book's details" and refused to open the book.
//!
//! Nullable columns must read back as `None`. Required columns still fail:
//! the empty blob is not a zero id.

use livtet_data::entities::{editions, works};
use livtet_data::orm::{ActiveModelTrait, DatabaseConnection, DbErr, EntityTrait, Set};
use livtet_data::sql::{self, AssertSqlSafe, SqlitePool};
use livtet_data::{Kind, TestDb};
use livtet_types::{CommonLanguages, DbId, KnownFormats};
use time::PrimitiveDateTime;

fn now() -> PrimitiveDateTime {
    PrimitiveDateTime::new(
        time::Date::from_calendar_date(2026, time::Month::January, 1).unwrap(),
        time::Time::MIDNIGHT,
    )
}

/// One edition whose format, language, and work are all real ids.
async fn seed_edition(db: &DatabaseConnection) -> editions::Model {
    let work = works::ActiveModel {
        id: Set(DbId::new()),
        title: Set("Test Work".into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();

    editions::ActiveModel {
        id: Set(DbId::new()),
        work_id: Set(work.id),
        group_id: Set(None),
        title: Set(Some("Test Edition".into())),
        published_date: Set(None),
        format_id: Set(Some(KnownFormats::Epub.into())),
        language_id: Set(Some(CommonLanguages::English.into())),
        notes: Set(None),
        description: Set(None),
        format_metadata: Set(None),
        cover_image_url: Set(None),
        created_at: Set(now()),
        updated_at: Set(None),
    }
    .insert(db)
    .await
    .unwrap()
}

/// `x''` is what the sync engine wrote for an id the sender lacked.
///
/// The write happened with enforcement off: a zero-length id cannot satisfy
/// the foreign key to `formats` / `languages`, so the sync daemon opens its
/// pool with `foreign_keys = OFF`. The PRAGMA is per connection, so issue it
/// on the same connection as the UPDATE.
async fn blank(pool: &SqlitePool, statement: String) {
    let mut conn = pool.acquire().await.unwrap();
    for sql_text in ["PRAGMA foreign_keys = OFF", statement.as_str()] {
        sql::query(AssertSqlSafe(sql_text.to_string()))
            .execute(&mut *conn)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn empty_blob_optional_ids_decode_as_none() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let edition = seed_edition(&db).await;

    blank(
        &test_db.pool,
        format!(
            "UPDATE editions SET format_id = x'', language_id = x'' WHERE id = x'{}'",
            edition.id.to_hex()
        ),
    )
    .await;

    let row = editions::Entity::find_by_id(edition.id)
        .one(&db)
        .await
        .expect("the row still decodes")
        .expect("the row exists");

    assert_eq!(row.format_id, None);
    assert_eq!(row.language_id, None);
    assert_eq!(row.title.as_deref(), Some("Test Edition"));
    assert_eq!(row.work_id, edition.work_id);
}

#[tokio::test]
async fn empty_blob_required_id_keeps_failing() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();
    let edition = seed_edition(&db).await;

    blank(
        &test_db.pool,
        format!(
            "UPDATE editions SET work_id = x'' WHERE id = x'{}'",
            edition.id.to_hex()
        ),
    )
    .await;

    let error = editions::Entity::find_by_id(edition.id)
        .one(&db)
        .await
        .expect_err("a required id cannot be empty");

    // The null surfaces from `DbId`'s decoder instead of becoming an
    // all-zero id.
    assert!(
        matches!(
            &error,
            DbErr::Type(message) if message.contains("work_id")
        ),
        "unexpected error: {error:?}"
    );
}
