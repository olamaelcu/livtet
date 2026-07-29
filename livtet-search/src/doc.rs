//! Denormalized doc shapes for single-doc Tantivy writes.
//!
//! NAPI builds these from `BookHit` data and hands them to
//! [`crate::SearchIndex::upsert_edition`] / [`crate::SearchIndex::upsert_author`]
//! to skip the SQL reindex path. Tauri's full `reindex(db)` continues to
//! produce these internally via private helpers.

/// Denormalized data for a single edition Tantivy doc.
pub struct EditionDoc {
    pub edition_id: String,
    pub work_id: String,
    pub title: String,
    pub edition_title: Option<String>,
    pub work_description: Option<String>,
    pub edition_description: Option<String>,
    pub authors: Vec<String>,
    pub authors_ids: Vec<String>,
    pub tags: Vec<String>,
    pub genres: Vec<String>,
    pub subjects: Vec<String>,
    pub publishers: Vec<String>,
    pub identifier_kinds: Vec<String>,
    pub identifier_values: Vec<String>,
    pub notes: Option<String>,
    pub format: Option<String>,
    pub language: Option<String>,
    pub pub_date: Option<i64>,
    pub published_year: Option<i64>,
    pub title_sort: String,
    pub primary_author_sort: Option<String>,
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub popularity: i64,
}

/// Denormalized data for a single author Tantivy doc.
pub struct AuthorDoc {
    pub author_id: String,
    pub name: String,
    pub sort_name: String,
    pub birth_year: Option<i64>,
    pub death_year: Option<i64>,
    pub source: String,
}
