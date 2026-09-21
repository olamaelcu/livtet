//! FRBR-aware full-text search built on Tantivy 0.26.
//!
//! Each Tantivy document represents one indexed entity. Today there
//! are two entity kinds:
//!
//! - `"edition"` (the default) — one per `editions` row, carrying
//!   all joined data the schema needs (work title, authors, tags,
//!   genres, subjects, publishers, format, language, identifiers).
//! - `"author"` — one per `authors` row, used for people-only
//!   search hits (`HitKind::Person`).
//!
//! The schema is declared once in [`build_schema`] and reused for
//! every index on disk. Reindexing always wipes and rebuilds (a
//! schema change is not safe to do incrementally on Tantivy), so
//! per-edition edits happen through [`SearchIndex::add_edition`]
//! and [`SearchIndex::delete_edition`] against an already-open index.
//!
//! Search APIs:
//!
//! - [`SearchIndex::search`] — edition-level text search.
//! - [`SearchIndex::search_works`] — over-fetches and collapses
//!   editions onto a work.
//! - [`SearchIndex::search_with_facets`] — edition-level + facet
//!   counts and a `pub_date`-desc tiebreak.
//!
//! Lookups for categorical IDs live alongside the index in
//! [`WorkLookup`], [`EditionLookup`], [`AuthorLookup`] and
//! [`ResourceLookup`]. The concrete SeaORM-backed implementation
//! is in [`crate::sea_orm_resource_lookup`].

#[cfg(feature = "uniffi")]
/// See `livtet-types`' `UniFfiTag` — UniFFI derives reference
/// `crate::UniFfiTag` at definition site.
#[doc(hidden)]
pub struct UniFfiTag;

pub mod doc;
pub mod index;
pub mod label_resolver;
pub mod lookups;
pub mod model;
pub mod schema;
pub mod sea_orm_resource_lookup;
pub mod search;
pub mod user_input_translator;
pub mod write;

pub use doc::{AuthorDoc, EditionDoc};
pub use index::{SearchError, SearchIndex};
pub use label_resolver::LabelResolver;
pub use lookups::{AuthorLookup, EditionLookup, ResourceKind, ResourceLookup, WorkLookup};
pub use model::{
    FacetCount, FacetedSearchResult, HighlightRange, HitKind, SearchHit, SearchOptions,
};
pub use schema::{
    DEFAULT_SNIPPET_CHARS, OPDS_WORK_ID_LIMIT, SCHEMA_VERSION, WORK_GROUP_OVERFETCH, build_schema,
    fields, kinds,
};
pub use search::{WorkFiltersQuery, WorkFiltersResolved};
pub use tantivy::query::{AllQuery, BooleanQuery, Query, TermQuery};
pub use user_input_translator::user_input_ast_to_query;
pub use write::ReindexEvent;
