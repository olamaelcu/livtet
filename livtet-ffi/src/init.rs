use std::sync::Once;

use livtet_core::{
    SharedState, init_state,
    migrator::{self, Kind},
};

use crate::{MobileError, state};

static LOG_ONCE: Once = Once::new();
static PANIC_HOOK_ONCE: Once = Once::new();

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
