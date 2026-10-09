//! Every migration's `down` is reversible.
//!
//! Each migration declares a `down` beside its `up`, but nothing exercised
//! them: a `down` that drops the wrong table, forgets a trigger, or fails on
//! a dependency it no longer owns would only be discovered by someone
//! actually rolling back a database. These tests roll both schemas all the
//! way down and all the way back up.
//!
//! Order matters. `run_kinds` migrates Business then Client, and the client
//! schema's triggers are defined on business tables, so rolling back has to
//! go Client first and then Business — the reverse of creation.

use livtet_data::orm::{DatabaseConnection, SqlxSqliteConnector};
use livtet_data::sql::SqlitePool;
use livtet_data::{Kind, TestDb, client_migration, migration, run_kinds};
use sea_orm_migration::MigratorTrait;

/// `Migrator::run` wraps the pool in a sea-ORM connection itself; `down` is
/// the raw trait method, so it needs the same wrapping.
fn conn(pool: &SqlitePool) -> DatabaseConnection {
    SqlxSqliteConnector::from_sqlx_sqlite_pool(pool.clone())
}

/// Roll both schemas back, client first: the client schema's triggers sit on
/// business tables, so they have to come off before those tables do.
async fn roll_all_the_way_down(pool: &SqlitePool) {
    let db = conn(pool);
    client_migration::Migrator::down(&db, None)
        .await
        .expect("client schema must roll back");
    migration::Migrator::down(&db, None)
        .await
        .expect("business schema must roll back");
}

/// Names of the tables SQLite currently holds, excluding its own bookkeeping
/// and the two migration ledgers.
async fn user_tables(pool: &SqlitePool) -> Vec<String> {
    livtet_data::sql::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' \
           AND name NOT LIKE 'sqlite_%' \
           AND name NOT IN ('core_migrations', 'client_migrations') \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("list tables")
}

async fn object_count(pool: &SqlitePool, kind: &str) -> i64 {
    livtet_data::sql::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = ? AND name NOT LIKE 'sqlite_%'",
    )
    .bind(kind)
    .fetch_one(pool)
    .await
    .expect("count objects")
}

#[tokio::test]
async fn both_schemas_roll_all_the_way_down() {
    let db = TestDb::new(&[Kind::Business, Kind::Client])
        .await
        .expect("migrate up");
    let pool = &db.pool;

    assert!(
        !user_tables(pool).await.is_empty(),
        "the fixture must start migrated"
    );
    assert!(
        object_count(pool, "trigger").await > 0,
        "the client schema installs change_log triggers"
    );

    roll_all_the_way_down(pool).await;

    let left = user_tables(pool).await;
    assert!(
        left.is_empty(),
        "every table should be gone after both rollbacks; {left:?} remain"
    );
    assert_eq!(
        object_count(pool, "trigger").await,
        0,
        "a trigger outliving its table would break the next migration up"
    );
    assert_eq!(
        object_count(pool, "view").await,
        0,
        "views are dropped with their migrations too"
    );
}

/// A `down` that leaves debris behind would make the next `up` fail on an
/// already-exists error. Rolling down and back up proves it does not.
#[tokio::test]
async fn a_rolled_back_database_migrates_up_again() {
    let db = TestDb::new(&[Kind::Business, Kind::Client])
        .await
        .expect("migrate up");
    let pool = &db.pool;
    let before = user_tables(pool).await;

    roll_all_the_way_down(pool).await;

    run_kinds(pool, &[Kind::Business, Kind::Client])
        .await
        .expect("a rolled-back database must migrate up again");

    assert_eq!(
        user_tables(pool).await,
        before,
        "the second migration up must reproduce the same schema"
    );
}

/// `down` is stepwise, so rolling back one migration at a time has to work
/// as well as rolling back the lot — that is what a partial rollback does.
#[tokio::test]
async fn the_business_schema_rolls_back_one_step_at_a_time() {
    let db = TestDb::new(&[Kind::Business, Kind::Client])
        .await
        .expect("migrate up");
    let pool = &db.pool;

    let db = conn(pool);
    client_migration::Migrator::down(&db, None)
        .await
        .expect("client down");

    let steps = migration::Migrator::migrations().len() as u32;
    assert!(steps > 1, "expected several business migrations");
    for step in 0..steps {
        migration::Migrator::down(&db, Some(1))
            .await
            .unwrap_or_else(|e| panic!("business step {step} failed to roll back: {e}"));
    }

    assert!(
        user_tables(pool).await.is_empty(),
        "stepping all the way down must land in the same place as one bulk down"
    );
}
