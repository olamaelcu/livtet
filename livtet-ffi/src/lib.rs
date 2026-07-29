mod init;
mod lookup;
mod runtime;
mod state;
pub mod sync;
pub mod sync_export;

use livtet_types::DbId;

// Register `DbId` as a uniffi custom type. The wire format is a `Vec<u8>`
// (16 bytes on the wire; uniffi has no fixed-size array FfiConverter). The
// Kotlin/Swift `uniffi.toml` then maps the bytes to `ULID` / `FastULID` for
// idiomatic use on the mobile side.
//
// `lower` runs on the Rust side when sending a `DbId` over FFI.
// `try_lift` runs on the Rust side when receiving bytes from FFI.
uniffi::custom_type!(DbId, Vec<u8>, {
    remote,
    lower: |id| id.to_bytes().to_vec(),
    try_lift: |bytes| {
        let arr: [u8; 16] = bytes
            .try_into()
            .map_err(|_| ::uniffi::deps::anyhow::anyhow!("DbId must be exactly 16 bytes"))?;
        Ok(DbId::from_bytes(arr))
    },
});

// ── Dashboard / stats records ──────────────────────────────────────────────

/// Aggregate dashboard statistics about the user's library and
/// reading activity. Returned by the `get_dashboard_stats` FFI
/// export.
#[derive(uniffi::Record)]
pub struct DashboardStats {
    pub total_books: i64,
    pub books_in_progress: i64,
    pub finished_books: i64,
    pub total_reading_time_secs: i64,
    pub first_reading_at_millis: Option<i64>,
}

/// A book the user is currently reading or has recently finished,
/// with its latest reading progress snapshot. Returned by
/// `get_recently_read_books`.
#[derive(uniffi::Record)]
pub struct RecentlyReadBook {
    pub work_id: DbId,
    pub edition_id: DbId,
    pub title: String,
    pub author_name: Option<String>,
    pub progress: f64,
    pub total_reading_time_secs: i64,
    pub last_read_at: String,
}

/// A single search-query entry surfaced on the search-history
/// autocomplete. Returned by `get_recent_searches`.
#[derive(uniffi::Record)]
pub struct RecentSearch {
    pub query: String,
    pub searched_at: String,
}

// ── Filter records (distinct values surfaced in the library filters UI) ──

/// A distinct book format actually present in the user's library.
#[derive(uniffi::Record)]
pub struct FormatInfo {
    pub id: DbId,
    pub name: String,
    /// JSON Schema document describing how reading progress is tracked for editions of this format.
    pub metadata_schema: String,
}

/// A distinct language actually present in the user's library
/// editions.
#[derive(uniffi::Record)]
pub struct LanguageInfo {
    pub id: DbId,
    pub name: String,
    pub flag_emoji: Option<String>,
}

/// A distinct work-status value actually present in the user's
/// library.
#[derive(uniffi::Record)]
pub struct WorkStatusInfo {
    pub id: DbId,
    pub name: String,
}

// ── Literary quotations ────────────────────────────────────────────────────
//
// These two records are the FFI-facing view of `livtet_core`'s
// `Greeting` and `EmptyMessage` types. The shape is preserved so the
// existing Android/iOS consumers of `Greeting` are unchanged, while
// the new `EmptyMessage` is intentionally narrow (no `label`, no
// `period`) so the UI can render it directly into any empty-state
// surface.

// ── Literary greeting ──────────────────────────────────────────────────────

/// A literary greeting drawn from African American and African
/// diaspora authors, chosen by the mobile UI for the time of day.
/// Returned by `get_greeting`.
#[derive(uniffi::Record)]
pub struct Greeting {
    pub label: String,
    pub text: String,
    pub author: String,
    pub material: String,
    pub period: String,
}

/// An empty-state filler: a literary quotation without a time-of-day
/// period or greeting label. Returned by `get_empty_state_quotation`.
#[derive(uniffi::Record)]
pub struct EmptyMessage {
    pub text: String,
    pub author: String,
    pub material: String,
}

// ── Pairing & plugin records ────────────────────────────────────────────

/// A device paired with this instance. Used by both Android and iOS
/// `Settings → Paired Devices` screens. The `device_type` field is the
/// display name resolved by `DeviceType::display_name_for` (e.g.
/// "E-Reader" for a canonical Ereader, "KOReader" for a custom Ereader
/// row written by `pair_device` with a non-canonical name).
#[derive(uniffi::Record)]
pub struct PairedDeviceMobile {
    pub device_id: DbId,
    pub name: String,
    pub listen_on: String,
    pub device_type: String,
    pub paired_at: String,
    pub last_sync_at: Option<String>,
}

/// One installed plugin row from the `installed_plugins` table.
/// The mobile `Settings → Plugins` section lists these and lets the
/// user toggle each one. `enabled = false` disables a plugin without
/// uninstalling it.
#[derive(uniffi::Record)]
pub struct InstalledPluginMobile {
    pub id: DbId,
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub source_path: String,
}

/// Non-loopback network addresses of the host. The mobile
/// pairing/settings screen uses this to show the user which addresses
/// to use when configuring manual pairing.
#[derive(uniffi::Record)]
pub struct NetworkAddressesMobile {
    pub addresses: Vec<String>,
}

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MobileError {
    #[error("Database: {0}")]
    Database(String),

    #[error("Failed to lock the database pool registry")]
    RegistryLocked,

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Init: {0}")]
    Init(String),

    #[error("Platform: {0}")]
    Platform(String),

    #[error("Network: {0}")]
    Network(String),

    /// A bundled provider surfaced a structured error sentinel
    /// (the `__livtet_error` table convention from the bundled
    /// plugin init.lua files) and the bridge classified it. The UI
    /// surfaces a tailored callout per category.
    #[error("Provider error ({category:?}): {provider_id}")]
    ProviderError {
        category: ProviderErrorCategory,
        retry_after_seconds: Option<u32>,
        provider_id: String,
    },

    #[error("ISBN conflict: isbn {conflicting_isbn} already on work {work_id}")]
    IsbnConflict {
        work_id: DbId,
        edition_id: DbId,
        conflicting_isbn: String,
    },

    #[error("Save rolled back: {detail}")]
    SaveRolledBack { detail: String },
}

/// Coarse category for a provider error. Matches the
/// `__livtet_error.category` strings emitted by every bundled
/// plugin's `http_get_json` helper. UI surfaces a distinct callout
/// per variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ProviderErrorCategory {
    /// 401/403 — provider requires authentication the user
    /// hasn't supplied (e.g. Google Books needs an API key).
    NeedsAuth,
    /// 429 — provider is rate-limiting. `retry_after_seconds` is
    /// populated when the upstream includes a `Retry-After` header.
    RateLimited,
    /// 408 — request timed out.
    Timeout,
    /// 404 — provider reports the resource doesn't exist. For
    /// search this is treated as silent (no results); for
    /// lookup this would surface as "not found" but the bridge
    /// currently routes it as "no hits" along with the others.
    NotFound,
    /// 5xx or connection failure — provider is unreachable or
    /// having problems. Transient.
    ProviderDown,
}

impl From<std::io::Error> for MobileError {
    fn from(e: std::io::Error) -> Self {
        Self::Platform(e.to_string())
    }
}

impl From<livtet_core::error::CoreError> for MobileError {
    fn from(e: livtet_core::error::CoreError) -> Self {
        match &e {
            livtet_core::error::CoreError::NotFound { entity, id } => {
                MobileError::NotFound(format!("{entity} with id {id}"))
            }
            livtet_core::error::CoreError::NotInitialized => {
                MobileError::Init("Not initialized".to_string())
            }
            _ => MobileError::Database(format!("Core error: {}", e)),
        }
    }
}

impl From<livtet_database::orm::DbErr> for MobileError {
    fn from(e: livtet_database::orm::DbErr) -> Self {
        MobileError::Database(format!("Database error: {}", e))
    }
}

impl From<livtet_database::sql::Error> for MobileError {
    fn from(e: livtet_database::sql::Error) -> Self {
        MobileError::Database(format!("SQLx error: {}", e))
    }
}

impl From<miette::Report> for MobileError {
    fn from(e: miette::Report) -> Self {
        MobileError::Database(format!("Error: {:?}", e))
    }
}

// ── Helper: get state or return error ───────────────────────────────────────

fn get_state() -> Result<livtet_core::SharedState, MobileError> {
    livtet_core::get_state().cloned().map_err(MobileError::from)
}

// ── FFI Exports ─────────────────────────────────────────────────────────────

/// Initialize the shared database pool. Must be called once at app startup.
/// The path should be a filesystem path (not a URL). The function builds
/// `sqlite:{path}?mode=rwc` internally. For tests, pass ":memory:" for in-memory SQLite.
/// Idempotent - subsequent calls with the same path succeed silently.
#[tracing::instrument(name = "ffi_init_db_pool")]
#[uniffi::export]
pub fn init_db_pool(database_path: String) -> Result<(), MobileError> {
    let _ = fs_err::remove_file(&database_path);
    runtime::block_on(crate::state::init_db_pool(&database_path)).map_err(MobileError::from)?;
    Ok(())
}

/// Initialize the library with a database path.
/// The path should be a filesystem path (not a URL). The function builds
/// `sqlite:{path}?mode=rwc` internally.
#[tracing::instrument(name = "ffi_init")]
#[uniffi::export]
pub fn init(database_path: String) -> Result<(), MobileError> {
    runtime::block_on(init::init_inner(&database_path))
}

/// Check if the library has been initialized.
#[uniffi::export]
pub fn is_initialized() -> bool {
    livtet_core::is_initialized()
}

/// Check if the sync database pool has been initialized.
#[uniffi::export]
pub fn is_sync_pool_initialized() -> bool {
    crate::state::is_initialized()
}

/// Return a literary greeting drawn from African American and African
/// diaspora authors, chosen at random from a set matching the current
/// time of day (Early Morning, Late Morning, Afternoon, Evening, Night,
/// or Late Night).
#[uniffi::export]
pub fn get_greeting() -> Greeting {
    Greeting {
        label: String::new(),
        text: String::new(),
        author: String::new(),
        material: String::new(),
        period: String::new(),
    }
}

/// Return an empty-state filler — a literary quotation without a
/// greeting label or time-of-day period. Use this when a list or view
/// would otherwise be empty. The quote is picked deterministically
/// from `livtet_core::data::quotes::empty.txt` per call.
#[uniffi::export]
pub fn get_empty_state_quotation() -> EmptyMessage {
    EmptyMessage {
        text: String::new(),
        author: String::new(),
        material: String::new(),
    }
}

// ── Seed (debug) ───────────────────────────────────────────────────────────

/// Summary of rows inserted by `seed_database`. Useful for showing a
/// "seeded N works / M editions" toast in the dev panel.
#[derive(uniffi::Record)]
pub struct SeedResultMobile {
    pub works_created: i32,
    pub editions_created: i32,
    pub authors_created: i32,
    pub publishers_created: i32,
    pub reading_status_count: i32,
    pub annotations_created: i32,
    pub digital_inventory_created: i32,
    pub loans_created: i32,
    pub reading_sessions_created: i32,
    pub saved_searches_created: i32,
    pub reading_lists_created: i32,
}

/// Populate the database with realistic demo data. Only available in
/// debug builds where `livtet-core` includes the `fake` feature.
/// Intended for use as a debug-only "seed sample library" button
/// in the mobile Settings UI.
#[uniffi::export]
pub async fn seed_database(works: i32) -> Result<SeedResultMobile, MobileError> {
    let state = get_state()?;
    let config = livtet_core::seed::SeedConfig {
        num_works: works.max(0) as u32,
        ..Default::default()
    };
    let result = runtime::block_on(livtet_core::seed::seed_database(&state.db_conn(), &config))
        .map_err(MobileError::from)?;
    Ok(SeedResultMobile {
        works_created: result.works_created as i32,
        editions_created: result.editions_created as i32,
        authors_created: result.authors_created as i32,
        publishers_created: result.publishers_created as i32,
        reading_status_count: result.reading_status_count as i32,
        annotations_created: result.annotations_created as i32,
        digital_inventory_created: result.digital_inventory_created as i32,
        loans_created: result.loans_created as i32,
        reading_sessions_created: result.reading_sessions_created as i32,
        saved_searches_created: result.saved_searches_created as i32,
        reading_lists_created: result.reading_lists_created as i32,
    })
}

uniffi::setup_scaffolding!();
