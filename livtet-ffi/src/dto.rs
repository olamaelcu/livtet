//! DTO records crossing the FFI boundary.
//!
//! These are deliberately flat: a foreign consumer gets everything it
//! needs in one record, with no entity/junction knowledge required.
//! Identifiers cross as `DbId` (ULID string on the foreign side),
//! timestamps as RFC 3339 strings, and enum-like values as the
//! `livtet-types` UniFFI enums.

use livtet_types::{DbId, DiskPath, ProgressUnit, PublishedDate};

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
