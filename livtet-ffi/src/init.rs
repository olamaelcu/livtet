use std::sync::Once;

use livtet_core::{
    SharedState, init_state,
    migrator::{self, Kind},
};

use crate::{MobileError, state};

static LOG_ONCE: Once = Once::new();

fn init_logger() {
    // FIXME: Extract LIVTET_RUST_LOG as LIVTET_LOG
    let log_level = std::env::var("LIVTET_RUST_LOG").unwrap_or_else(|_| "trace".to_string());
    let env_filter = tracing_subscriber::EnvFilter::new(&log_level);
    LOG_ONCE.call_once(|| {
        #[cfg(target_os = "android")]
        {
            use std::str::FromStr;

            let filter_level = log::LevelFilter::from_str(&log_level).unwrap();
            let _ = android_logd_logger::builder()
                .filter_level(filter_level)
                .tag("LivtetRust")
                .prepend_module(true)
                .init();
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

    let db_path_str = db_path.to_string();

    tracing::info!("File deletion complete, creating new pool");

    let pool = state::init_db_pool(db_path).await.map_err(|e| {
        tracing::error!(e = %e, "Failed to initialize database pool");
        MobileError::from(e)
    })?;

    tracing::info!("Running migrations...");
    livtet_database::sql::query("PRAGMA journal_mode = WAL")
        .execute(&*pool)
        .await
        .map_err(|e| MobileError::Database(format!("PRAGMA: {e}")))?;
    livtet_database::sql::query("PRAGMA synchronous = NORMAL")
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
