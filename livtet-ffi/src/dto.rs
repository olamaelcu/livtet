//! DTO records crossing the FFI boundary.
//!
//! These are deliberately flat: a foreign consumer gets everything it
//! needs in one record, with no entity/junction knowledge required.
//! Identifiers cross as `DbId` (ULID string on the foreign side),
//! timestamps as RFC 3339 strings, and enum-like values as the
//! `livtet-types` UniFFI enums.

use livtet_types::{DbId, DiskPath, PublishedDate};

/// RFC 3339 rendering shared by all DTO timestamps (UTC).
pub(crate) fn ts(dt: &time::PrimitiveDateTime) -> String {
    dt.assume_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("PrimitiveDateTime always formats as RFC 3339")
}

pub(crate) fn ts_opt(dt: Option<time::PrimitiveDateTime>) -> Option<String> {
    dt.as_ref().map(ts)
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
