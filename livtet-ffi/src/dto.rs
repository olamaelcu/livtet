//! DTO records crossing the FFI boundary.
//!
//! These are deliberately flat: a foreign consumer gets everything it
//! needs in one record, with no entity/junction knowledge required.
//! Identifiers cross as `DbId` (ULID string on the foreign side),
//! timestamps as RFC 3339 strings, and enum-like values as the
//! `livtet-types` UniFFI enums.

use livtet_core::types::{DbId, DiskPath, ProgressUnit, PublishedDate};

/// RFC 3339 rendering shared by all DTO timestamps (UTC).
pub(crate) fn ts(dt: &time::PrimitiveDateTime) -> String {
    dt.assume_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("PrimitiveDateTime always formats as RFC 3339")
}

pub(crate) fn ts_opt(dt: Option<time::PrimitiveDateTime>) -> Option<String> {
    dt.as_ref().map(ts)
}

/// Parse an RFC 3339 timestamp from the foreign side
/// (fail-closed: invalid input is an error, not a silent `now()`).
pub(crate) fn ts_parse(s: &str) -> Result<time::PrimitiveDateTime, crate::error::LivtetError> {
    let odt = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .map_err(|e| {
            crate::error::LivtetError::InvalidInput(format!("invalid RFC 3339 timestamp: {e}"))
        })?;
    Ok(time::PrimitiveDateTime::new(odt.date(), odt.time()))
}

/// A work (platonic book) with its display-facing fields resolved.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WorkSummary {
    pub id: DbId,
    pub title: String,
    pub sort_title: Option<String>,
    pub description: Option<String>,
    /// Author/author-role display names in junction order.
    pub authors: Vec<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

/// One physical file of a digital edition (1:1 with `digital_inventory`).
#[derive(Debug, Clone, uniffi::Record)]
pub struct EditionFile {
    pub file_path: Option<DiskPath>,
    pub cover_path: Option<String>,
    pub blurhash: Option<String>,
    pub dominant_color: Option<String>,
    pub file_hash: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub file_format: Option<String>,
}

/// List-row view of an edition.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EditionSummary {
    pub id: DbId,
    pub work_id: DbId,
    pub title: Option<String>,
    pub format: Option<String>,
    pub language_code: Option<String>,
    /// Whether a digital file is attached (`digital_inventory.file_path`).
    pub has_file: bool,
    pub created_at: String,
    pub updated_at: Option<String>,
}

/// Full edition detail: metadata + contributors + identifiers + file.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EditionDetail {
    pub id: DbId,
    pub work_id: DbId,
    pub title: Option<String>,
    pub published_date: Option<PublishedDate>,
    pub format: Option<String>,
    pub language_code: Option<String>,
    pub notes: Option<String>,
    pub description: Option<String>,
    /// Author and other contributor display names in junction order.
    pub authors: Vec<String>,
    /// Publisher names in junction order.
    pub publishers: Vec<String>,
    /// Identifier URN strings (e.g. `urn:isbn:9780306406157`).
    pub identifiers: Vec<String>,
    pub file: Option<EditionFile>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

/// Reading progress for one edition.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ReadingProgress {
    pub id: DbId,
    pub edition_id: DbId,
    /// Progress value in the given unit.
    pub progress: f64,
    pub progress_unit: Option<ProgressUnit>,
    pub last_location: Option<String>,
    pub total_reading_time_secs: i64,
    pub created_at: String,
}

/// A user annotation pinned to an edition.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Annotation {
    pub id: DbId,
    pub edition_id: DbId,
    pub content: String,
    pub location: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

/// A named reading list with its members (edition ids, in position
/// order).
#[derive(Debug, Clone, uniffi::Record)]
pub struct ReadingList {
    pub id: DbId,
    pub name: String,
    pub description: Option<String>,
    pub edition_ids: Vec<DbId>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

/// One recorded reading session.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ReadingSessionInput {
    pub edition_id: DbId,
    pub format_id: DbId,
    pub duration_seconds: i64,
    pub progress_delta: f64,
    pub last_location: Option<String>,
    pub notes: Option<String>,
    /// RFC 3339 timestamp of session start; `None` means
    /// "`duration_seconds` before now".
    pub started_at: Option<String>,
}

/// One recorded reading session.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ReadingSession {
    pub id: DbId,
    pub edition_id: DbId,
    pub started_at: String,
    pub duration_seconds: i64,
    pub progress_delta: f64,
    pub last_location: Option<String>,
    pub notes: Option<String>,
}
// ── Dashboard ────────────────────────────────────────────────────────

/// Aggregate library and reading-activity statistics for the dashboard.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DashboardStats {
    pub total_books: i64,
    pub books_in_progress: i64,
    pub finished_books: i64,
    pub total_reading_time_secs: i64,
    /// RFC 3339 timestamp of the earliest recorded reading activity,
    /// or `None` when nothing has been read yet.
    pub first_reading_at: Option<String>,
}

/// A work the user is reading or has recently finished, with its
/// latest progress snapshot.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RecentlyReadBook {
    pub work_id: DbId,
    pub edition_id: DbId,
    pub title: String,
    pub author_name: Option<String>,
    pub progress: f64,
    pub total_reading_time_secs: i64,
    pub last_read_at: String,
}

/// One entry of the search-history autocomplete.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RecentSearch {
    pub query: String,
    pub searched_at: String,
}

// ── Library filters ──────────────────────────────────────────────────

/// A book format actually present in the library.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FormatInfo {
    pub id: DbId,
    pub name: String,
    /// JSON Schema document describing how reading progress is tracked
    /// for editions of this format.
    pub metadata_schema: String,
}

/// A language actually present in the library's editions. Named
/// `LibraryLanguage` (not `LanguageInfo`) because `livtet-types`
/// already exports a `LanguageInfo` record into the same bindings.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LibraryLanguage {
    pub id: DbId,
    pub name: String,
    pub flag_emoji: Option<String>,
}
