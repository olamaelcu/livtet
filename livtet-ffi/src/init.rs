use std::sync::Once;

use livtet_core::{
    SharedState, init_state,
    migrator::{self, Kind},
};

use crate::{MobileError, state};

static LOG_ONCE: Once = Once::new();
static PANIC_HOOK_ONCE: Once = Once::new();

/// Drop orphan indexes from `sqlite_master` on a database file before the
/// pool opens it. An index is "orphan" when its `tbl_name` no longer
/// points to an existing table.
///
/// Two flavors of orphan appeared in the wild:
///
/// 1. The autoindex `sqlite_autoindex_<table>_<N>`, created automatically
///    by SQLite for a UNIQUE constraint.
/// 2. Manually-named indexes like `idx_<table>_<columns>`, created by
///    sea-orm migrations.
///
/// Both are *not* dropped by SQLite's cascade when their table is removed
/// via `PRAGMA writable_schema = ON; DELETE FROM sqlite_master WHERE
/// type='table' ...` (see `wipe_user_tables` in `state.rs`). On the
/// next `init`, the `after_connect` callback's PRAGMA sequence triggers
/// SQLITE_CORRUPT with "malformed database schema (...) - no such table:
/// <tbl_name>" (autoindex variant) or "malformed database schema (...) -
/// orphan index" (manual index variant), and the pool times out with
/// "pool timed out while waiting for an open connection".
///
/// This helper opens a one-off connection with no `after_connect`
/// callback (so the schema check is deferred), then checks whether
/// the `works` table exists. If it doesn't, the migration tracking
/// (`core_migrations`, `client_migrations`) is stale: the migrations
/// think they're applied but the data tables are gone. Drop the
/// tracking tables *and* their autoindexes so the migrator re-runs
/// from scratch and re-creates the schema.
///
/// Note: an earlier iteration of this function also dropped *any*
/// indexes whose `tbl_name` was missing from `sqlite_master`, but
/// that turned out to be a one-time migration fluke (the orphan
/// indexes only appeared on a database that had been partially
/// wiped). The "works table missing" check is sufficient for the
/// case that actually leaves the database in a broken state.
async fn drop_orphan_autoindexes(db_path: &str) -> Result<(), MobileError> {
    use livtet_data::sql::sqlite::SqliteConnectOptions;
    use livtet_data::sql::Connection;

    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false);

    let mut conn = match livtet_data::sql::SqliteConnection::connect_with(&opts).await {
        Ok(c) => c,
        Err(e) => {
            // The raw connection refused to open the file itself
            // (the schema is so corrupted that sqlite3 refuses at
            // `sqlite3_open_v2`). Wipe the file and let the
            // migrator re-create it from scratch.
            tracing::warn!(
                e = %e,
                "orphan_cleanup: could not open raw connection; deleting database file and any -wal/-shm sidecars"
            );
            return reset_database_file(db_path);
        }
    };

    livtet_data::sql::query("PRAGMA writable_schema = ON")
        .execute(&mut conn)
        .await
        .map_err(|e| MobileError::Database(format!("orphan_cleanup: writable_schema=ON: {e}")))?;

    let works_exists = livtet_data::sql::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'works'",
    )
    .fetch_optional(&mut conn)
    .await
    .map_err(|e| MobileError::Database(format!("orphan_cleanup: check works: {e}")))?
    .is_some();

    if !works_exists {
        // Drop the migration tracking tables AND their autoindexes.
        // Using `writable_schema = ON; DELETE FROM sqlite_master` for
        // a table leaves the autoindexes (`sqlite_autoindex_<tbl>_<N>`)
        // orphaned, which then trips the next pool's after_connect
        // callback with exactly the same error we're trying to
        // recover from. Clean up both with a single DELETE.
        livtet_data::sql::query(
            "DELETE FROM sqlite_master \
             WHERE (type = 'table' AND name IN ('core_migrations', 'client_migrations')) \
                OR (type = 'index' \
                    AND (name IN ('core_migrations', 'client_migrations') \
                         OR tbl_name IN ('core_migrations', 'client_migrations')))",
        )
        .execute(&mut conn)
        .await
        .map_err(|e| MobileError::Database(format!("orphan_cleanup: DELETE migrations: {e}")))?;
    }

    livtet_data::sql::query("PRAGMA writable_schema = OFF")
        .execute(&mut conn)
        .await
        .map_err(|e| MobileError::Database(format!("orphan_cleanup: writable_schema=OFF: {e}")))?;

    conn.close().await.ok();

    if !works_exists {
        tracing::info!("Dropped stale migration tracking so migrations re-run");
    } else {
        tracing::info!("works table exists; migration tracking preserved");
    }
    Ok(())
}

/// Delete the on-disk database file and any WAL / SHM sidecars so
/// the migrator can re-create the schema from scratch. Used as a
/// last-resort fallback when the file is so corrupted that the
/// raw SQLite connection cannot even open it.
fn reset_database_file(db_path: &str) -> Result<(), MobileError> {
    use fs_err as fs;

    if let Err(e) = fs::remove_file(db_path) {
        return Err(MobileError::Database(format!("reset: delete {db_path}: {e}")));
    }
    for ext in ["-wal", "-shm", "-journal"] {
        let _ = fs::remove_file(format!("{db_path}{ext}"));
    }
    tracing::info!("Deleted database file (and any -wal/-shm sidecars)");
    Ok(())
}

fn install_panic_hook() {
    use std::panic::{self, PanicHookInfo};
    let prev = panic::take_hook();
    panic::set_hook(Box::new(move |info: &PanicHookInfo| {
        // Forward to logcat via the `log` crate so the panic message
        // is visible even with stderr disconnected on Android.
        // Falls through to the default hook for any default behavior
        // (backtrace, etc.).
        log::error!("PANIC [ffilt]: {}", info);
        prev(info);
    }));
}

fn init_logger() {
    PANIC_HOOK_ONCE.call_once(install_panic_hook);

    // FIXME: Extract LIVTET_RUST_LOG as LIVTET_LOG
    let log_level = std::env::var("LIVTET_RUST_LOG").unwrap_or_else(|_| "trace".to_string());
    let env_filter = tracing_subscriber::EnvFilter::new(&log_level);
    LOG_ONCE.call_once(|| {
        #[cfg(target_os = "android")]
        {
            // Wrap the builder init in catch_unwind so a logger init panic
            // (e.g. on Android emulators where /dev/pmsg0 is absent and the
            // pstore path panics) does not bring the app down. The panic is
            // forwarded to logcat by install_panic_hook above.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                use std::str::FromStr;
                let filter_level = log::LevelFilter::from_str(&log_level).unwrap();
                let _ = android_logd_logger::builder()
                    .filter_level(filter_level)
                    .tag("LivtetRust")
                    .prepend_module(true)
                    // android-logd-logger v0.5.0 enables pstore by default,
                    // which lazy-opens /dev/pmsg0 via `.expect()` and panics
                    // on emulators that don't expose that device. We don't
                    // need pstore for normal logcat output — logd is enough.
                    .pstore(false)
                    .init();
            }));
        }
        #[cfg(target_os = "ios")]
        {
            use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;

            let subscriber = tracing_subscriber::registry()
                .with(tracing_oslog::OsLogger::new(
                    "net.olamaelcu.livtet",
                    "rust-ffi",
                ))
                .with(env_filter);

            let _ = tracing::subscriber::set_global_default(subscriber);
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .try_init();
        }
    });
}

#[tracing::instrument(ret, err)]
pub async fn init_inner(db_path: &str) -> Result<(), MobileError> {
    init_logger();
    tracing::info!(db_path = %db_path, "Initializing database");

    // Recovery: drop orphan autoindexes left behind by a previous
    // `wipe_user_tables` call (or any other PRAGMA writable_schema
    // bypass). The first-ever call on a fresh database is a no-op
    // because every autoindex has a matching table.
    if let Err(e) = drop_orphan_autoindexes(db_path).await {
        tracing::warn!(e = %e, "orphan autoindex cleanup failed; continuing");
    }

    tracing::info!("File deletion complete, creating new pool");

    let pool = state::init_db_pool(db_path).await.map_err(|e| {
        tracing::error!(e = %e, "Failed to initialize database pool");
        MobileError::from(e)
    })?;

    tracing::info!("Running migrations...");
    livtet_data::sql::query("PRAGMA journal_mode = WAL")
        .execute(&*pool)
        .await
        .map_err(|e| MobileError::Database(format!("PRAGMA: {e}")))?;
    livtet_data::sql::query("PRAGMA synchronous = NORMAL")
        .execute(&*pool)
        .await
        .map_err(|e| MobileError::Database(format!("PRAGMA: {e}")))?;

    migrator::run_kinds(&*pool, [Kind::Business, Kind::Client])
        .await
        .map_err(|e| MobileError::Database(format!("migration: {e}")))?;

    tracing::info!("Initializing shared state...");
    let state = SharedState::from_pool((*pool).clone(), db_path.to_string());
    init_state(state).map_err(|e| {
        tracing::error!(e = %e, "State initialization failed");
        MobileError::from(e)
    })
}
