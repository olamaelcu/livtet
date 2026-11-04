//! Structured author name parts (`m0017`).
//!
//! `authors.name` is a joined string; citation styles need it as separate
//! family and given parts. `m0017` adds those columns and derives them with one
//! documented rule: split on the first comma when there is one, otherwise the
//! final whitespace token is the family name. These tests pin the rule from
//! both ends — the pure split, and the backfill writing it into rows without
//! overwriting a part an import already supplied.

use livtet_data::entities::authors;
use livtet_data::migration::m0019_author_name_parts::{apply, split_name};
use livtet_data::orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use livtet_data::{Kind, TestDb};
use livtet_types::DbId;

async fn insert_author(db: &DatabaseConnection, name: &str) -> DbId {
    authors::ActiveModel {
        id: Set(DbId::new()),
        name: Set(name.into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("author inserts")
    .id
}

/// `(family, given)` as stored.
async fn parts(db: &DatabaseConnection, id: DbId) -> (Option<String>, Option<String>) {
    let model = authors::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("author fetches")
        .expect("author is present");
    (model.family, model.given)
}

#[test]
fn the_split_rule_keeps_the_parts_separate() {
    assert_eq!(
        split_name("Rowling, J. K."),
        (Some("Rowling".to_string()), Some("J. K.".to_string())),
        "a comma form splits at the comma, not at the last token"
    );
    assert_eq!(
        split_name("Ursula K. Le Guin"),
        (Some("Le Guin".to_string()), Some("Ursula K.".to_string())),
        "a western form puts only the final token in `family`"
    );
    assert_eq!(
        split_name("Homer"),
        (Some("Homer".to_string()), None),
        "a mononym has no given part"
    );

    assert_eq!(
        split_name("Vincent van der Berg"),
        (
            Some("van der Berg".to_string()),
            Some("Vincent".to_string())
        ),
        "a multi-word particle belongs to the family name"
    );
    assert_eq!(
        split_name("Van Morrison"),
        (Some("Morrison".to_string()), Some("Van".to_string())),
        "a particle-looking first token stays given when nothing else would"
    );
}

#[tokio::test]
async fn the_backfill_splits_the_recorded_forms() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();

    let comma = insert_author(&db, "Rowling, J. K.").await;
    let western = insert_author(&db, "Ursula K. Le Guin").await;
    let mononym = insert_author(&db, "Homer").await;

    apply(&db).await.expect("backfill runs");

    assert_eq!(
        parts(&db, comma).await,
        (Some("Rowling".to_string()), Some("J. K.".to_string())),
        "the first comma separates a family-first name"
    );
    assert_eq!(
        parts(&db, western).await,
        (Some("Le Guin".to_string()), Some("Ursula K.".to_string())),
        "an unpunctuated name takes its last token as the family name"
    );
    assert_eq!(
        parts(&db, mononym).await,
        (Some("Homer".to_string()), None),
        "a single-token name leaves `given` NULL"
    );
}

#[tokio::test]
async fn a_rerun_does_not_clobber_parts_an_import_supplied() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();

    let id = insert_author(&db, "Mieville, China").await;
    apply(&db).await.expect("backfill runs");
    assert_eq!(
        parts(&db, id).await,
        (Some("Mieville".to_string()), Some("China".to_string()))
    );

    // An importer that knows the real spelling replaces the derived family
    // part and leaves `given` empty because it has no given names.
    let model = authors::Entity::find_by_id(id)
        .one(&db)
        .await
        .expect("author fetches")
        .expect("author is present");
    let mut active: authors::ActiveModel = model.into();
    active.family = Set(Some("Miéville".to_string()));
    active.given = Set(None);
    active.update(&db).await.expect("import update applies");

    apply(&db).await.expect("a second backfill runs");

    assert_eq!(
        parts(&db, id).await,
        (Some("Miéville".to_string()), Some("China".to_string())),
        "the import-supplied family part survives while the NULL given part is filled"
    );
}

#[tokio::test]
async fn a_row_with_both_parts_set_is_left_alone() {
    let test_db = TestDb::new(&[Kind::Business]).await.unwrap();
    let db = test_db.state().db_conn();

    // A single-token name would derive no `given` part at all; the row below
    // already carries both, which is what an import with real structure writes.
    let id = insert_author(&db, "Cher").await;
    let mut active: authors::ActiveModel = authors::Entity::find_by_id(id)
        .one(&db)
        .await
        .expect("author fetches")
        .expect("author is present")
        .into();
    active.family = Set(Some("Sarkisian".to_string()));
    active.given = Set(Some("Cherilyn".to_string()));
    active.update(&db).await.expect("structured row stores");

    apply(&db).await.expect("backfill runs");

    assert_eq!(
        parts(&db, id).await,
        (Some("Sarkisian".to_string()), Some("Cherilyn".to_string())),
        "the backfill only fills NULL parts"
    );
}
