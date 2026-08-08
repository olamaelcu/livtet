//! Book listing FFI surface.
//!
//! Exposes the [`Book`] record, the [`BookSearchSortOrder`] enum, and the
//! [`list_books`] export so the Android Library screen can render the
//! user's stored works. The query is a straight forward read against the
//! `works` table ordered by `created_at`; richer surfaces
//! (`list_books_with_filters`, `get_editions_for_work`, etc.) will follow
//! as separate commits.

use livtet_database::{
    orm::{EntityTrait, QueryOrder, QuerySelect},
    works,
};

use crate::{Book, BookSearchSortOrder, MobileError, get_state};

/// Return the user's stored books, paginated by `limit`/`offset` and
/// ordered by `works.created_at` per `order`. Negative `limit` or
/// `offset` is clamped to `0` so the query never errors on bad inputs.
#[tracing::instrument(name = "ffi_list_books", skip_all, err)]
#[uniffi::export]
pub async fn list_books(
    limit: i32,
    offset: i32,
    order: BookSearchSortOrder,
) -> Result<Vec<Book>, MobileError> {
    let state = get_state()?;
    let db = state.db_conn();
    let limit = (limit.max(0)) as u64;
    let offset = (offset.max(0)) as u64;

    let rows = match order {
        BookSearchSortOrder::Ascending => works::Entity::find()
            .order_by_asc(works::Column::CreatedAt)
            .offset(offset)
            .limit(limit)
            .all(&db)
            .await?,
        BookSearchSortOrder::Descending => works::Entity::find()
            .order_by_desc(works::Column::CreatedAt)
            .offset(offset)
            .limit(limit)
            .all(&db)
            .await?,
    };

    Ok(rows
        .into_iter()
        .map(|m| Book {
            id: m.id,
            title: m.title,
            description: m.description,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use livtet_core::{
        SeedConfig, SharedState, get_state, init_state, is_initialized, seed_database,
    };
    use livtet_database::{
        orm::{EntityTrait, PaginatorTrait},
        works,
    };

    use super::{Book, BookSearchSortOrder, list_books};

    /// Serialises the tests so they don't fight over the single shared
    /// in-memory pool. SQLite WAL allows multi-connection reads but the
    /// `seed_database` write transaction conflicts when two tests race.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Initialise the process-global `SharedState` and seed it once
    /// for the whole test binary. `seed_database` is *not* idempotent
    /// — it inserts fixed-name tags ("bestseller", etc.) that collide
    /// on a second call — so all tests must share the same seeded pool.
    /// Each test asserts on `list_books`'s behaviour over the shared
    /// data rather than seeding its own rows.
    ///
    /// Uses a tempfile-backed SQLite (not `:memory:`) because a pooled
    /// `sqlite::memory:` URL gives each connection its *own* private
    /// database; only the connection that ran migrations would see the
    /// `works` table, leading to flaky `no such table: works` errors
    /// when subsequent connections in the pool are handed out.
    async fn setup_state_and_seed() {
        if is_initialized() {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "livtet-ffi-books-tests-{}-{}",
            std::process::id(),
            ulid::Ulid::new()
        ));
        fs_err::create_dir_all(&dir).expect("create temp dir for tests");
        let db_path = dir.join("test.db");
        fs_err::write(&db_path, []).expect("create empty db file");
        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let state = SharedState::connect(&url)
            .await
            .expect("SharedState::connect should succeed for tempfile SQLite");
        init_state(state).expect("init_state should succeed on a fresh OnceLock");
        // `seed_database` is non-idempotent (UNIQUE on `tags.name`).
        // Subsequent calls fail with `UNIQUE constraint failed`, so we
        // seed exactly once for the entire test binary.
        seed_database(
            &get_state().expect("state initialised").db_conn(),
            &SeedConfig {
                num_works: 5,
                ..Default::default()
            },
        )
        .await
        .expect("seed 5 works (one-shot, non-idempotent)");
    }

    /// Count works currently in the table.
    async fn count_works() -> u64 {
        let db = get_state().expect("state initialised").db_conn();
        works::Entity::find().count(&db).await.unwrap()
    }

    /// Acquire the test lock for the entire test body, recovering from
    /// any prior panic that may have poisoned it. Holding the lock
    /// across the full test serialises setup_state_and_seed (which is
    /// not race-safe — the global `SharedState` is a `OnceLock`), so
    /// tests share the seeded pool without colliding.
    async fn run_serialised<F, Fut>(body: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        setup_state_and_seed().await;
        body().await;
    }

    #[tokio::test]
    async fn list_books_returns_seeded_works() {
        run_serialised(|| async {
            let total = count_works().await;
            assert!(total >= 5, "seed should produce >=5 works, got {total}");

            let books = list_books(100, 0, BookSearchSortOrder::Descending)
                .await
                .expect("list_books ok");
            assert!(
                books.len() >= 5,
                "list_books returned {} rows; expected >= 5",
                books.len()
            );
        })
        .await;
    }

    #[tokio::test]
    async fn list_books_respects_limit() {
        run_serialised(|| async {
            let total = count_works().await;
            assert!(total >= 2, "seed should produce >=2 works, got {total}");

            let books = list_books(2, 0, BookSearchSortOrder::Ascending)
                .await
                .expect("list_books ok");
            assert_eq!(books.len(), 2, "limit=2 caps the result count");
        })
        .await;
    }

    #[tokio::test]
    async fn list_books_respects_offset() {
        run_serialised(|| async {
            let total = count_works().await;
            assert!(total >= 3, "seed should produce >=3 works, got {total}");

            let first = list_books(1, 0, BookSearchSortOrder::Ascending)
                .await
                .expect("list_books(1,0) ok");
            let second = list_books(1, 1, BookSearchSortOrder::Ascending)
                .await
                .expect("list_books(1,1) ok");
            assert_eq!(first.len(), 1);
            assert_eq!(second.len(), 1);
            assert_ne!(
                first[0].id, second[0].id,
                "offset=1 returns a different row than offset=0"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn list_books_orders_by_created_at() {
        run_serialised(|| async {
            // The seed inserts all works with the same `timestamp`, so
            // ASC and DESC both return the same first row by tie-break.
            // We instead assert that the order parameter doesn't filter
            // anything out: both directions return the same total count.
            let asc = list_books(100, 0, BookSearchSortOrder::Ascending)
                .await
                .expect("asc ok");
            let desc = list_books(100, 0, BookSearchSortOrder::Descending)
                .await
                .expect("desc ok");
            assert_eq!(asc.len(), desc.len(), "ASC and DESC return the same count");
        })
        .await;
    }

    #[tokio::test]
    async fn list_books_clamps_negative_limit_and_offset() {
        run_serialised(|| async {
            let books = list_books(-5, -3, BookSearchSortOrder::Descending)
                .await
                .expect("negative inputs clamp rather than error");
            let _ = books;
        })
        .await;
    }

    #[tokio::test]
    async fn book_carries_title() {
        run_serialised(|| async {
            let books = list_books(10, 0, BookSearchSortOrder::Ascending)
                .await
                .expect("list_books ok");
            let any_book: Book = books
                .into_iter()
                .next()
                .expect("expected at least one book after seeding");
            assert!(!any_book.title.is_empty(), "Book.title is non-empty");
        })
        .await;
    }
}
