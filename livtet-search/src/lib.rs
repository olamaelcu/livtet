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

use std::{collections::HashMap, sync::Arc};

use camino::Utf8Path;
use fs_err as fs;
use livtet_data::orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use specta::Type;
use tantivy::{
    Index, IndexWriter, Order, ReloadPolicy, Term,
    collector::{Count, FacetCollector, TopDocs},
    directory::MmapDirectory,
    doc,
    query::{Occur, QueryParser, TermSetQuery},
    schema::*,
    snippet::SnippetGenerator,
};
use thiserror::Error;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// All field names used in the schema. Centralised so reindexers,
/// query parsers, snippet generators, and lookups all agree on the
/// string literals.
pub mod fields {
    pub const EDITION_ID: &str = "edition_id";
    pub const WORK_ID: &str = "work_id";
    pub const WORK_ID_HASH: &str = "work_id_hash";
    pub const AUTHOR_ID: &str = "author_id";
    pub const KIND: &str = "kind";

    pub const TITLE: &str = "title";
    pub const EDITION_TITLE: &str = "edition_title";
    pub const WORK_DESCRIPTION: &str = "work_description";
    pub const EDITION_DESCRIPTION: &str = "edition_description";
    pub const AUTHORS: &str = "authors";
    pub const TAGS: &str = "tags";
    pub const GENRES: &str = "genres";
    pub const SUBJECTS: &str = "subjects";
    pub const PUBLISHERS: &str = "publishers";
    pub const IDENTIFIER_KINDS: &str = "identifier_kinds";
    pub const IDENTIFIER_VALUES: &str = "identifier_values";
    pub const NOTES: &str = "notes";

    pub const FORMAT: &str = "format";
    pub const LANGUAGE: &str = "language";

    pub const LANGUAGE_FACET: &str = "language_facet";
    pub const PUBLISHER_FACET: &str = "publisher_facet";
    pub const SUBJECT_FACET: &str = "subject_facet";
    pub const GENRE_FACET: &str = "genre_facet";

    pub const PUB_DATE: &str = "pub_date";
    pub const PUBLISHED_YEAR: &str = "published_year";
    pub const TITLE_SORT: &str = "title_sort";
    pub const PRIMARY_AUTHOR_SORT: &str = "primary_author_sort";
    pub const CREATED_AT: &str = "created_at";
    pub const UPDATED_AT: &str = "updated_at";
    pub const POPULARITY: &str = "popularity";

    pub const SOURCE: &str = "source";

    pub const TAG_ID: &str = "tag_id";
    pub const GENRE_ID: &str = "genre_id";
    pub const SUBJECT_ID: &str = "subject_id";
    pub const SERIES_ID: &str = "series_id";
    pub const PUBLISHER_ID: &str = "publisher_id";
}

/// Default snippet budget for [`SearchHit::snippet_text`].
pub const DEFAULT_SNIPPET_CHARS: usize = 180;

/// Default over-fetch multiplier for [`SearchIndex::search_works`].
pub const WORK_GROUP_OVERFETCH: usize = 8;

/// Maximum number of work IDs returned by
/// [`SearchIndex::matching_work_ids`]. The OPDS server caps the
/// per-page response at this magnitude; the search backend should
/// not enumerate further.
pub const OPDS_WORK_ID_LIMIT: usize = 1_000;

/// Current schema version. Stored in `search_schema_version.json` next to the
/// tantivy index dir. Bumped when `build_schema()` changes.
pub const SCHEMA_VERSION: u32 = 2;

/// Build the Tantivy schema.
///
/// Important constraints from the design plan:
/// - **No** `isbn` field. ISBNs are canonicalised via
///   [`livtet_types::Isbn::parse`] and indexed under
///   `identifier_values` only.
/// - All categorical ID fields (`author_id`, `tag_id`, `genre_id`,
///   `subject_id`, `series_id`, `publisher_id`) are added as
///   text fast fields. Tantivy's text fast fields are inherently
///   multi-valued — calling `add_text` repeatedly on the same
///   field inside one document stores multiple values, which is
///   exactly what we need for an edition that has many authors /
///   tags / genres / subjects / publishers.
/// - The `kind` discriminator uses `STRING | INDEXED | STORED`.
pub fn build_schema() -> Schema {
    let mut b = Schema::builder();

    // ----- IDs (single-valued strings) -----
    b.add_text_field(fields::EDITION_ID, STRING | STORED);
    b.add_text_field(fields::WORK_ID, STRING | STORED);
    // u64 hash of `work_id` so the `search_works` group-by can use a
    // fast field instead of comparing 26-char ULIDs.
    b.add_u64_field(fields::WORK_ID_HASH, INDEXED | FAST);

    // ----- Doc-kind discriminator -----
    // STRING is already an indexed, untokenized text type in
    // tantivy 0.26 (the underlying TextFieldIndexing is set with a
    // basic record option). The plan's `STRING | INDEXED | STORED`
    // therefore collapses to `STRING | STORED` here — STRING
    // implies INDEXED. Tying the discriminator to a string keeps
    // the per-`kind` parser fast path (TermQuery against `kinds::EDITION`)
    // straightforward.
    b.add_text_field(fields::KIND, STRING | STORED);

    // ----- Categorical IDs (STRING | INDEXED | FAST) -----
    //
    // Tantivy text fast fields are inherently multi-valued: each
    // `d.add_text(field, value)` call adds another value to the
    // same column. That's exactly the semantic we need for an
    // edition that carries many authors / tags / genres / subjects
    // / publishers / series entries. Author documents carry a
    // single id per doc; edition documents carry many.
    let id_text = || {
        TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("raw")
                    .set_index_option(IndexRecordOption::Basic),
            )
            .set_stored()
            .set_fast(None)
    };
    b.add_text_field(fields::AUTHOR_ID, id_text());
    b.add_text_field(fields::TAG_ID, id_text());
    b.add_text_field(fields::GENRE_ID, id_text());
    b.add_text_field(fields::SUBJECT_ID, id_text());
    b.add_text_field(fields::SERIES_ID, id_text());
    b.add_text_field(fields::PUBLISHER_ID, id_text());

    // ----- Full-text -----
    let title_text = TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("en_stem")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    b.add_text_field(fields::TITLE, title_text);
    let stored_text = || {
        TextOptions::default().set_stored().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("en_stem")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
    };
    b.add_text_field(fields::EDITION_TITLE, stored_text());
    b.add_text_field(fields::WORK_DESCRIPTION, stored_text());
    b.add_text_field(fields::EDITION_DESCRIPTION, stored_text());
    b.add_text_field(fields::AUTHORS, stored_text());
    b.add_text_field(fields::TAGS, stored_text());
    b.add_text_field(fields::GENRES, stored_text());
    b.add_text_field(fields::SUBJECTS, stored_text());
    b.add_text_field(fields::PUBLISHERS, stored_text());

    // Identifiers are paired: `identifier_kinds[i]` and
    // `identifier_values[i]` belong to the same logical identifier.
    // ISBNs are canonicalised to ISBN-13 before storage; the
    // `isbn` schema field is intentionally omitted.
    let id_stored_indexed = || {
        TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("default")
                    .set_index_option(IndexRecordOption::Basic),
            )
            .set_stored()
    };
    b.add_text_field(fields::IDENTIFIER_KINDS, id_stored_indexed());
    b.add_text_field(fields::IDENTIFIER_VALUES, id_stored_indexed());

    b.add_text_field(fields::NOTES, TEXT | STORED);

    // ----- Filters / sort / facet -----
    b.add_text_field(fields::FORMAT, stored_text());
    b.add_text_field(fields::LANGUAGE, stored_text());

    b.add_facet_field(fields::LANGUAGE_FACET, FacetOptions::default().set_stored());
    b.add_facet_field(
        fields::PUBLISHER_FACET,
        FacetOptions::default().set_stored(),
    );
    b.add_facet_field(fields::SUBJECT_FACET, FacetOptions::default().set_stored());
    b.add_facet_field(fields::GENRE_FACET, FacetOptions::default().set_stored());

    b.add_date_field(fields::PUB_DATE, INDEXED | STORED | FAST);
    b.add_u64_field(fields::PUBLISHED_YEAR, INDEXED | FAST);
    b.add_text_field(fields::TITLE_SORT, STRING | FAST);
    b.add_text_field(fields::PRIMARY_AUTHOR_SORT, STRING | FAST);
    b.add_date_field(fields::CREATED_AT, INDEXED | STORED | FAST);
    b.add_date_field(fields::UPDATED_AT, INDEXED | STORED | FAST);
    b.add_u64_field(fields::POPULARITY, FAST);
    b.add_text_field(fields::SOURCE, TEXT | STORED);

    b.build()
}

// ---------------------------------------------------------------------------
// IndexKind discriminator (used internally for `kind` field).
// ---------------------------------------------------------------------------

/// Values the `kind` field can take. Mirrors the spec's "edition" |
/// "author" discriminator. We keep these as `&str`s on the wire so a
/// saved query can match them without depending on this enum.
pub mod kinds {
    pub const EDITION: &str = "edition";
    pub const AUTHOR: &str = "author";
}

// ---------------------------------------------------------------------------
// SearchHit / HitKind
// ---------------------------------------------------------------------------

/// What kind of document a [`SearchHit`] came from.
///
/// Serialised as `"edition"`, `"work"`, or `"person"` (snake_case)
/// so the specta-generated TS type stays narrow.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HitKind {
    Edition,
    Work,
    Person,
}

/// A single search result.
///
/// Replaces the legacy 3-field shape from before the FRBR rework.
/// The fields are intentionally wide: the frontend can show an
/// edition-level hit (`HitKind::Edition`, `edition_id` populated), a
/// work-level collapse (`HitKind::Work`, `grouped_edition_ids`
/// populated), or a person hit (`HitKind::Person`, `author_id`
/// populated, `kind = "author"` documents).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SearchHit {
    pub kind: HitKind,
    /// Set for `HitKind::Edition`.
    pub edition_id: Option<String>,
    /// Always populated for edition/work hits. ULID string.
    pub work_id: String,
    /// Set for `HitKind::Person`. ULID string.
    pub author_id: Option<String>,

    /// Best-effort title for the hit (edition title → work title
    /// → author name for people hits).
    pub title: String,
    /// Underlying work title (edition hits may differ from work).
    pub work_title: Option<String>,
    /// Underlying edition title if distinct from work title.
    pub edition_title: Option<String>,
    /// Author names. Empty for `HitKind::Person` (use `title`).
    pub authors: Vec<String>,

    /// Canonical ISBN-13 if this edition has any ISBN rows.
    pub isbn: Option<String>,
    /// Format name (e.g. "EPUB", "PDF").
    pub format: Option<String>,
    /// Language name (from `languages.name`).
    pub language: Option<String>,
    /// Publication date as an ISO-8601 string, or `None`.
    pub published_date: Option<String>,

    /// BM25 score from Tantivy.
    pub score: f32,
    /// Pretty-printed Tantivy explanation tree, or `None` when
    /// [`SearchOptions::explain`] is false.
    pub explanation: Option<String>,

    /// Plain text snippet (no HTML). `None` when the query didn't
    /// match any snippetable field for this document.
    pub snippet_text: Option<String>,
    /// Byte ranges into `snippet_text` that should be highlighted.
    /// Empty when the snippet was generated without highlights.
    /// Each entry is `[start, end]` — byte offsets into `snippet_text`.
    pub snippet_highlighted: Vec<[u32; 2]>,

    /// When `HitKind::Work`, the edition IDs collapsed into this
    /// work hit. Empty for edition/person hits.
    pub grouped_edition_ids: Vec<String>,

    /// Provenance of this hit ("catalog" for site-owned rows,
    /// plugin id prefix for imported data). See the `source`
    /// field documentation.
    pub source: String,
}

// ---------------------------------------------------------------------------
// SearchOptions
// ---------------------------------------------------------------------------

/// Options that control one search call.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// When true, every hit carries the Tantivy explanation tree
    /// serialised to JSON.
    pub explain: bool,
    /// When true, a snippet is generated from the best-matching
    /// description field for each hit. The default snippet budget
    /// is [`DEFAULT_SNIPPET_CHARS`].
    pub with_snippet: bool,
    /// Snippet budget in characters. Ignored when `with_snippet`
    /// is false.
    pub snippet_chars: usize,
    /// When true, edition hits are collapsed onto works. Equivalent
    /// to calling [`SearchIndex::search_works`].
    pub collapse_to_works: bool,
    /// Over-fetch multiplier for the work-collapse path. The default
    /// is 8 (`WORK_GROUP_OVERFETCH`).
    pub work_overfetch: usize,
    /// Optional explicit sort. When `Some`,
    /// [`SearchIndex::search_with_options`] sorts the top-N result
    /// by the corresponding fast field (`Title` / `CreatedAt` /
    /// `UpdatedAt`) in the requested direction; `Score` is a no-op
    /// since score-ordering is the default. When `None`, the
    /// legacy BM25 score ordering is preserved.
    ///
    /// Implementation note: tantivy's `TopDocs::order_by_fast_field`
    /// is parameterised over `FastValue`-implementing types
    /// (`u64`, `i64`, `f64`, `DateTime`, `IpAddr`) and therefore
    /// cannot sort a text fast field like `title_sort` directly. To
    /// keep the API uniform across all four [`livtet_types::SortField`]
    /// variants we always collect a score-ordered top-N and then
    /// post-sort by reading each document's stored value of the
    /// relevant field. The over-fetch is bumped to
    /// `max(limit * 2, limit + 64)` so post-sort truncation to
    /// `limit` doesn't bias toward the score-best slice.
    pub sort: Option<livtet_types::SortSpec>,
    /// In-memory offset for pagination. When non-zero, the search
    /// fetches `limit + offset` hits from Tantivy and then drops
    /// the first `offset` results. This is necessary because
    /// Tantivy's `TopDocs` collector does not natively support
    /// offset — the offset is applied post-hoc on the
    /// score-ordered (or post-sorted) result slice.
    /// ...
    pub offset: usize,
    /// When `Some`, only hits whose stored `source` field matches
    /// this string are returned. For `range = "catalog"` the
    /// filter is `"catalog"`; for `range = "provider"` it is
    /// `None` (all sources pass through). Calls that need
    /// a negative-match-on-source should compose the term query
    /// externally.
    pub source_filter: Option<String>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            explain: false,
            with_snippet: true,
            snippet_chars: DEFAULT_SNIPPET_CHARS,
            collapse_to_works: false,
            work_overfetch: WORK_GROUP_OVERFETCH,
            sort: None,
            offset: 0,
            source_filter: None,
        }
    }
}

// ---------------------------------------------------------------------------
// WorkFiltersQuery — translates WorkFilters to tantivy BooleanQuery
// ---------------------------------------------------------------------------

/// Resolved `WorkFilters` with `format_ids` / `language_ids` converted
/// to text labels. Produced by the Tauri command or FFI handler before
/// constructing a [`WorkFiltersQuery`].
#[derive(Debug, Clone)]
pub struct WorkFiltersResolved {
    /// The original filters with DbId-based fields.
    pub filters: livtet_types::WorkFilters,
    /// Resolved format labels (e.g. `["EPUB", "PDF"]`).
    pub format_labels: Vec<String>,
    /// Resolved language labels (e.g. `["English", "French"]`).
    pub language_labels: Vec<String>,
}

impl WorkFiltersResolved {
    /// Create from a `WorkFilters` with resolved labels.
    pub fn from_filters(
        filters: livtet_types::WorkFilters,
        format_labels: Vec<String>,
        language_labels: Vec<String>,
    ) -> Self {
        Self {
            filters,
            format_labels,
            language_labels,
        }
    }
}

/// A query built from a user-supplied free-text string and
/// [`WorkFilters`], lowered to a tantivy [`Box<dyn Query>`].
///
/// Construction requires format/language IDs to already be resolved
/// to text labels (see [`WorkFiltersResolved`]).
pub struct WorkFiltersQuery {
    resolved: WorkFiltersResolved,
    query: String,
}

impl WorkFiltersQuery {
    /// Production constructor — call after the Tauri command or FFI
    /// handler has resolved `format_ids` / `language_ids` to text
    /// labels via the `formats` / `languages` tables.
    pub fn new(resolved: WorkFiltersResolved, query: String) -> Self {
        Self { resolved, query }
    }

    /// Test convenience — no label resolution. Equivalent to calling
    /// `new` with empty format/language labels.
    pub fn from_filters(filters: livtet_types::WorkFilters, query: String) -> Self {
        Self {
            resolved: WorkFiltersResolved {
                filters,
                format_labels: Vec::new(),
                language_labels: Vec::new(),
            },
            query,
        }
    }

    /// Build the Tantivy `Box<dyn Query>` for this filter+query combo.
    /// The `index` argument is needed to construct a [`QueryParser`] for
    /// the free-text portion of the query.
    pub fn build_query(&self, index: &Index) -> tantivy::Result<Box<dyn Query>> {
        let schema = index.schema();
        let filters = &self.resolved.filters;
        let has_query = !self.query.trim().is_empty();
        let has_filters = !filters.tag_ids.is_empty()
            || !filters.genre_ids.is_empty()
            || !filters.subject_ids.is_empty()
            || !filters.publisher_ids.is_empty()
            || !filters.author_ids.is_empty()
            || !self.resolved.format_labels.is_empty()
            || !self.resolved.language_labels.is_empty();

        // Empty query AND empty filters → AllQuery (MatchAllDocs).
        // An empty-armed BooleanQuery returns zero hits, so this
        // guard is essential.
        if !has_query && !has_filters {
            return Ok(Box::new(AllQuery));
        }

        let mut must_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        // Free-text query via QueryParser (preserves field boosts,
        // fuzzy-on-title, conjunction-by-default).
        if has_query {
            let parser = knn_query_parser(index);
            let q = parser.parse_query(&self.query)?;
            must_clauses.push((Occur::Must, q));
        }

        // ---- Categorical ID filters (TermSetQuery) ----
        // Each DbId becomes a Term::from_field_text(id_field, id.to_string()).
        // Multiple ids within one axis are OR'd by TermSetQuery.
        // Filters across axes are AND'd via BooleanQuery Must.

        let tag_id_field = schema.get_field(fields::TAG_ID).ok();
        if let Some(field) = tag_id_field {
            map_id_filter(&filters.tag_ids, field, &mut must_clauses);
        }

        let genre_id_field = schema.get_field(fields::GENRE_ID).ok();
        if let Some(field) = genre_id_field {
            map_id_filter(&filters.genre_ids, field, &mut must_clauses);
        }

        let subject_id_field = schema.get_field(fields::SUBJECT_ID).ok();
        if let Some(field) = subject_id_field {
            map_id_filter(&filters.subject_ids, field, &mut must_clauses);
        }

        let publisher_id_field = schema.get_field(fields::PUBLISHER_ID).ok();
        if let Some(field) = publisher_id_field {
            map_id_filter(&filters.publisher_ids, field, &mut must_clauses);
        }

        let author_id_field = schema.get_field(fields::AUTHOR_ID).ok();
        if let Some(field) = author_id_field {
            map_id_filter(&filters.author_ids, field, &mut must_clauses);
        }

        // ---- Format / Language text-label filters (TermSetQuery) ----
        let format_field = schema.get_field(fields::FORMAT).ok();
        if let Some(field) = format_field {
            map_text_filter(&self.resolved.format_labels, field, &mut must_clauses);
        }

        let language_field = schema.get_field(fields::LANGUAGE).ok();
        if let Some(field) = language_field {
            map_text_filter(&self.resolved.language_labels, field, &mut must_clauses);
        }

        Ok(Box::new(tantivy::query::BooleanQuery::new(must_clauses)))
    }

    /// Build the sort specification for this filter+query combo.
    pub fn build_sort(&self) -> livtet_types::SortSpec {
        use livtet_types::{SortDirection, SortField};
        let filters = &self.resolved.filters;
        let direction = filters.sort_direction.unwrap_or(SortDirection::Desc);
        let limit = filters.effective_limit();
        match filters.sort_by {
            Some(livtet_types::WorkSortBy::Title) => livtet_types::SortSpec {
                field: SortField::Title,
                direction,
                limit,
            },
            Some(livtet_types::WorkSortBy::UpdatedAt) => livtet_types::SortSpec {
                field: SortField::UpdatedAt,
                direction,
                limit,
            },
            Some(livtet_types::WorkSortBy::NewestCap) => livtet_types::SortSpec {
                field: SortField::CreatedAt,
                direction: SortDirection::Desc,
                limit: limit.or(Some(100)),
            },
            Some(livtet_types::WorkSortBy::CreatedAt) | None => livtet_types::SortSpec {
                field: SortField::CreatedAt,
                direction,
                limit,
            },
        }
    }
}

/// Build a tantivy QueryParser for free-text with the schema's field
/// boosts and fuzzy-on-title. Mirrors the parser in SearchIndex but
/// uses the index's tokenizers and schema.
pub(crate) fn knn_query_parser(index: &Index) -> QueryParser {
    let schema = index.schema();
    let title = schema.get_field(fields::TITLE).expect("title");
    let authors = schema.get_field(fields::AUTHORS).expect("authors");
    let tags = schema.get_field(fields::TAGS).expect("tags");
    let genres = schema.get_field(fields::GENRES).expect("genres");
    let subjects = schema.get_field(fields::SUBJECTS).expect("subjects");
    let publishers = schema.get_field(fields::PUBLISHERS).expect("publishers");
    let edition_description = schema
        .get_field(fields::EDITION_DESCRIPTION)
        .expect("edition_description");
    let work_description = schema
        .get_field(fields::WORK_DESCRIPTION)
        .expect("work_description");
    let edition_title = schema
        .get_field(fields::EDITION_TITLE)
        .expect("edition_title");
    let notes = schema.get_field(fields::NOTES).expect("notes");
    let identifier_values = schema
        .get_field(fields::IDENTIFIER_VALUES)
        .expect("identifier_values");
    let format = schema.get_field(fields::FORMAT).expect("format");
    let language = schema.get_field(fields::LANGUAGE).expect("language");

    let mut parser = QueryParser::for_index(
        index,
        vec![
            title,
            edition_title,
            authors,
            tags,
            genres,
            subjects,
            publishers,
            identifier_values,
            edition_description,
            work_description,
            notes,
            format,
            language,
        ],
    );
    parser.set_field_boost(title, 4.0);
    parser.set_field_boost(authors, 2.0);
    parser.set_field_boost(edition_description, 1.5);
    parser.set_field_boost(work_description, 1.0);
    parser.set_field_boost(identifier_values, 1.0);
    parser.set_conjunction_by_default();
    parser.set_field_fuzzy(title, false, 2, false);
    parser
}

/// Helper: add a TermSetQuery for a list of DbIds to a BooleanQuery's
/// must clauses. If the list is empty, this is a no-op.
fn map_id_filter(
    ids: &[livtet_types::DbId],
    field: tantivy::schema::Field,
    clauses: &mut Vec<(Occur, Box<dyn Query>)>,
) {
    if ids.is_empty() {
        return;
    }
    let terms: Vec<Term> = ids
        .iter()
        .map(|id| Term::from_field_text(field, &id.to_string()))
        .collect();
    clauses.push((Occur::Must, Box::new(TermSetQuery::new(terms))));
}

/// Helper: add a TermSetQuery for a list of text labels.
fn map_text_filter(
    labels: &[String],
    field: tantivy::schema::Field,
    clauses: &mut Vec<(Occur, Box<dyn Query>)>,
) {
    if labels.is_empty() {
        return;
    }
    let terms: Vec<Term> = labels
        .iter()
        .map(|label| Term::from_field_text(field, label))
        .collect();
    clauses.push((Occur::Must, Box::new(TermSetQuery::new(terms))));
}

// ---------------------------------------------------------------------------
// SearchIndex
// ---------------------------------------------------------------------------

/// Tantivy-backed search index.
pub struct SearchIndex {
    index: Index,
    reader: tantivy::IndexReader,
    writer: Arc<RwLock<IndexWriter>>,
    schema: Schema,
    /// Pre-resolver for format and language IDs → labels.
    pub label_resolver: LabelResolver,
}

/// Things that can go wrong inside `SearchIndex`. We collapse
/// tantivy + sea-orm errors into a single error type so callers
/// only have to handle one enum.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
    #[error("database error: {0}")]
    Db(#[from] livtet_data::orm::DbErr),
    #[error("snippet generation failed: {0}")]
    Snippet(String),
    #[error(
        "index schema version {found} does not match build version {expected}; rebuild index with the original build or run migrate_to first"
    )]
    SchemaVersionMismatch { found: u32, expected: u32 },
}

impl SearchIndex {
    /// Open (or create) an index at `index_dir`. Reads the sidecar
    /// `search_schema_version.json` for the on-disk schema version. If the version
    /// doesn't match `SCHEMA_VERSION`, this is a hard error —
    /// callers MUST call `migrate_to` first.
    pub fn open(index_dir: &Utf8Path) -> Result<Self, SearchError> {
        let schema = build_schema();
        fs::create_dir_all(index_dir).ok();
        let meta_path = index_dir.join("search_schema_version.json");
        let on_disk_version = if meta_path.is_file() {
            let raw = fs::read_to_string(&meta_path).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "cannot read search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "malformed search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            parsed
                .get("schema_version")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(1)
        } else {
            // Fresh index dir with no sidecar: assume the current schema.
            // Callers bootstrapping a brand-new index shouldn't need to
            // run `migrate_to` first. The version file is written below.
            SCHEMA_VERSION
        };
        if on_disk_version > SCHEMA_VERSION {
            return Err(SearchError::SchemaVersionMismatch {
                found: on_disk_version,
                expected: SCHEMA_VERSION,
            });
        }
        if on_disk_version < SCHEMA_VERSION {
            return Err(SearchError::SchemaVersionMismatch {
                found: on_disk_version,
                expected: SCHEMA_VERSION,
            });
        }
        let dir = MmapDirectory::open(index_dir)
            .map_err(|e| tantivy::TantivyError::InvalidArgument(e.to_string()))?;
        let index = Index::open_or_create(dir, schema.clone())?;
        let writer = index.writer(50_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        // If we just opened a fresh dir (no sidecar yet), write the
        // current schema version so subsequent opens see a matching
        // version and don't trip the mismatch guard. Idempotent: if
        // the sidecar already existed above, this is a no-op write of
        // the same value.
        if !meta_path.is_file() {
            let payload = serde_json::json!({ "schema_version": SCHEMA_VERSION });
            fs::write(&meta_path, payload.to_string()).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "cannot write search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
        }

        Ok(Self {
            index,
            reader,
            writer: Arc::new(RwLock::new(writer)),
            schema,
            label_resolver: LabelResolver::new(),
        })
    }

    /// Idempotent migration. Reads the stored `search_schema_version.json`,
    /// compares to `SCHEMA_VERSION`, and:
    /// - If the on-disk version is older: drops the index dir and
    ///   calls `reindex(db)` from scratch, then writes the new
    ///   `search_schema_version.json`.
    /// - If equal: no-op.
    /// - If newer: returns `SearchError::SchemaVersionMismatch`.
    /// Returns the previous schema version.
    #[tracing::instrument(level = "info", name = "search.migrate", skip_all)]
    pub async fn migrate_to(
        index_dir: &Utf8Path,
        db: &DatabaseConnection,
    ) -> Result<u32, SearchError> {
        fs::create_dir_all(index_dir).ok();
        let meta_path = index_dir.join("search_schema_version.json");
        let prev_version = if meta_path.is_file() {
            let raw = fs::read_to_string(&meta_path).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "cannot read search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "malformed search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            parsed
                .get("schema_version")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(1)
        } else {
            1
        };
        if prev_version > SCHEMA_VERSION {
            return Err(SearchError::SchemaVersionMismatch {
                found: prev_version,
                expected: SCHEMA_VERSION,
            });
        }
        if prev_version == SCHEMA_VERSION {
            tracing::debug!(
                prev_version,
                "search index schema is current, nothing to migrate"
            );
            return Ok(prev_version);
        }

        tracing::info!(
            prev_version,
            target = SCHEMA_VERSION,
            "migrating search index schema"
        );

        let schema = build_schema();
        let dir = MmapDirectory::open(index_dir)
            .map_err(|e| tantivy::TantivyError::InvalidArgument(e.to_string()))?;
        let index = Index::open_or_create(dir, schema.clone())?;
        let writer = index.writer(50_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let label_resolver = LabelResolver::new();

        // Rebuild + commit; `reindex` is on the SearchIndex struct
        // so we build a temporary one and call through to its method.
        let temp = Self {
            index,
            reader,
            writer: Arc::new(RwLock::new(writer)),
            schema,
            label_resolver,
        };
        temp.reindex(db).await?;

        let meta = serde_json::json!({ "schema_version": SCHEMA_VERSION });
        fs::write(&meta_path, meta.to_string() + "\n").map_err(|e| {
            SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                "cannot write search_schema_version.json at {meta_path}: {e}"
            )))
        })?;

        tracing::info!(
            prev_version,
            target = SCHEMA_VERSION,
            "search index schema migration complete"
        );
        Ok(prev_version)
    }

    /// A reference to the underlying Tantivy [`Index`]. Exposed for
    /// building parsers and queries outside of [`SearchIndex`].
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// The Tantivy [`Schema`] backing this index. Exposed so the
    /// lookup traits and snippet generators can resolve field
    /// handles without rebuilding the schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Rebuild the index from scratch by streaming every edition
    /// (and every author) from the DB through the schema. This is
    /// the only safe way to apply a schema change, so the design
    /// plan keeps it as the migration path.
    ///
    /// The pre-existing edition documents are deleted at the start
    /// via `delete_all_documents()`. Author docs are added on top.
    #[tracing::instrument(level = "info", name = "search.reindex", skip_all)]
    pub async fn reindex(&self, db: &DatabaseConnection) -> Result<(), SearchError> {
        let start = std::time::Instant::now();
        use livtet_data::entities::{
            authors::Entity as Authors, edition_authors::Entity as EditionAuthors,
            edition_genres::Entity as EditionGenres, edition_identifiers::Entity as EditionIds,
            edition_publishers::Entity as EditionPublishers,
            edition_subjects::Entity as EditionSubjects, edition_tags::Entity as EditionTags,
            editions::Entity as Editions, formats::Entity as Formats, genres::Entity as Genres,
            identifiers::Entity as Identifiers, languages::Entity as Languages,
            publishers::Entity as Publishers, series_entries::Entity as SeriesEntries,
            subjects::Entity as Subjects, tags::Entity as Tags,
            work_authors::Entity as WorkAuthors, works::Entity as Works,
        };

        // ---- Phase 1: clear existing documents and load everything
        //              we need from the DB into HashMaps so we can
        //              resolve joins in O(1).
        {
            let mut writer = self.writer.write().await;
            writer.delete_all_documents()?;
            writer.commit()?;
        }

        let all_editions = Editions::find().all(db).await?;
        let all_works = Works::find().all(db).await?;
        let all_authors = Authors::find().all(db).await?;
        let all_tags = Tags::find().all(db).await?;
        let all_genres = Genres::find().all(db).await?;
        let all_subjects = Subjects::find().all(db).await?;
        let all_publishers = Publishers::find().all(db).await?;
        let all_formats = Formats::find().all(db).await?;
        let all_languages = Languages::find().all(db).await?;

        let edition_ids: Vec<livtet_types::DbId> = all_editions.iter().map(|e| e.id).collect();

        // edition_authors: edition_id -> [author_id]
        let edition_author_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionAuthors::find()
                .filter(
                    livtet_data::entities::edition_authors::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let mut edition_to_authors: HashMap<livtet_types::DbId, Vec<(livtet_types::DbId, String)>> =
            HashMap::new();
        // author_id -> name
        let authors_by_id: HashMap<livtet_types::DbId, &str> = all_authors
            .iter()
            .map(|a| (a.id, a.name.as_str()))
            .collect();
        for ea in &edition_author_rows {
            if let Some(name) = authors_by_id.get(&ea.author_id) {
                edition_to_authors
                    .entry(ea.edition_id)
                    .or_default()
                    .push((ea.author_id, (*name).to_string()));
            }
        }
        // work_authors: work_id -> [author_id, author_id, ...] for
        // the work-level fallback when an edition has no
        // edition_authors row.
        let work_ids: Vec<livtet_types::DbId> = all_editions.iter().map(|e| e.work_id).collect();
        let work_author_rows = if work_ids.is_empty() {
            Vec::new()
        } else {
            WorkAuthors::find()
                .filter(livtet_data::entities::work_authors::Column::WorkId.is_in(work_ids.clone()))
                .all(db)
                .await?
        };
        let mut work_to_authors: HashMap<livtet_types::DbId, Vec<(livtet_types::DbId, String)>> =
            HashMap::new();
        for wa in &work_author_rows {
            if let Some(name) = authors_by_id.get(&wa.author_id) {
                work_to_authors
                    .entry(wa.work_id)
                    .or_default()
                    .push((wa.author_id, (*name).to_string()));
            }
        }

        // Tags / genres / subjects / publishers junction tables.
        let edition_tag_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionTags::find()
                .filter(
                    livtet_data::entities::edition_tags::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let tags_by_id: HashMap<livtet_types::DbId, &str> =
            all_tags.iter().map(|t| (t.id, t.name.as_str())).collect();
        let mut edition_to_tag_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &edition_tag_rows {
            edition_to_tag_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.tag_id);
        }

        let edition_genre_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionGenres::find()
                .filter(
                    livtet_data::entities::edition_genres::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let genres_by_id: HashMap<livtet_types::DbId, &str> =
            all_genres.iter().map(|g| (g.id, g.name.as_str())).collect();
        let mut edition_to_genre_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &edition_genre_rows {
            edition_to_genre_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.genre_id);
        }

        let edition_subject_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionSubjects::find()
                .filter(
                    livtet_data::entities::edition_subjects::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let subjects_by_id: HashMap<livtet_types::DbId, &str> = all_subjects
            .iter()
            .map(|s| (s.id, s.name.as_str()))
            .collect();
        let mut edition_to_subject_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &edition_subject_rows {
            edition_to_subject_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.subject_id);
        }

        let edition_publisher_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionPublishers::find()
                .filter(
                    livtet_data::entities::edition_publishers::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let publishers_by_id: HashMap<livtet_types::DbId, &str> = all_publishers
            .iter()
            .map(|p| (p.id, p.name.as_str()))
            .collect();
        let mut edition_to_publisher_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &edition_publisher_rows {
            edition_to_publisher_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.publisher_id);
        }

        // Series entries (series_id per edition).
        let series_entry_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            SeriesEntries::find()
                .filter(
                    livtet_data::entities::series_entries::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let mut edition_to_series_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &series_entry_rows {
            edition_to_series_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.series_id);
        }

        // Identifiers via edition_identifier → identifiers. Filter to
        // both `isbn` and the non-isbn kinds so we can canonicalise
        // ISBNs via `livtet_types::Isbn::parse`.
        let edition_id_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionIds::find()
                .filter(
                    livtet_data::entities::edition_identifiers::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let ident_pk_ids: Vec<livtet_types::DbId> =
            edition_id_rows.iter().map(|r| r.identifier_id).collect();
        let identifiers: Vec<livtet_data::entities::identifiers::Model> = if ident_pk_ids.is_empty()
        {
            Vec::new()
        } else {
            Identifiers::find()
                .filter(livtet_data::entities::identifiers::Column::Id.is_in(ident_pk_ids))
                .all(db)
                .await?
        };
        let ident_by_id: HashMap<livtet_types::DbId, &livtet_data::entities::identifiers::Model> =
            identifiers.iter().map(|i| (i.id, i)).collect();

        let mut edition_isbns: HashMap<livtet_types::DbId, Vec<String>> = HashMap::new();
        let mut edition_ident_kinds: HashMap<livtet_types::DbId, Vec<String>> = HashMap::new();
        let mut edition_ident_values: HashMap<livtet_types::DbId, Vec<String>> = HashMap::new();
        for row in &edition_id_rows {
            let Some(ident) = ident_by_id.get(&row.identifier_id) else {
                continue;
            };
            // Store every kind as-is, but canonicalise ISBNs to
            // ISBN-13 via Isbn::parse. Unparseable ISBN rows still
            // surface as a `kind = isbn, value = raw` pair so they
            // remain searchable; we just don't claim canonical form.
            if ident.kind == "isbn" {
                match livtet_types::Isbn::parse(&ident.value) {
                    Ok(canonical) => {
                        edition_isbns
                            .entry(row.edition_id)
                            .or_default()
                            .push(canonical.to_string());
                        edition_ident_kinds
                            .entry(row.edition_id)
                            .or_default()
                            .push(ident.kind.clone());
                        edition_ident_values
                            .entry(row.edition_id)
                            .or_default()
                            .push(canonical.to_string());
                    }
                    Err(_) => {
                        edition_ident_kinds
                            .entry(row.edition_id)
                            .or_default()
                            .push(ident.kind.clone());
                        edition_ident_values
                            .entry(row.edition_id)
                            .or_default()
                            .push(ident.value.clone());
                    }
                }
            } else {
                edition_ident_kinds
                    .entry(row.edition_id)
                    .or_default()
                    .push(ident.kind.clone());
                edition_ident_values
                    .entry(row.edition_id)
                    .or_default()
                    .push(ident.value.clone());
            }
        }

        // Source provenance: the identifier entity no longer carries
        // `source` / `fetched_at` columns, so every edition defaults
        // to the `"catalog"` provenance for indexing purposes.
        let edition_to_source: HashMap<livtet_types::DbId, String> = HashMap::new();

        let formats_by_id: HashMap<livtet_types::DbId, &str> = all_formats
            .iter()
            .map(|f| (f.id, f.name.as_str()))
            .collect();
        let languages_by_id: HashMap<livtet_types::DbId, &str> = all_languages
            .iter()
            .map(|l| (l.id, l.name.as_str()))
            .collect();
        let works_by_id: HashMap<livtet_types::DbId, &livtet_data::entities::works::Model> =
            all_works.iter().map(|w| (w.id, w)).collect();

        // ---- Phase 2: write documents.
        let mut writer = self.writer.write().await;
        let edition_id_field = self
            .schema
            .get_field(fields::EDITION_ID)
            .expect("edition_id");
        let work_id_field = self.schema.get_field(fields::WORK_ID).expect("work_id");
        let work_id_hash_field = self
            .schema
            .get_field(fields::WORK_ID_HASH)
            .expect("work_id_hash");
        let author_id_field = self.schema.get_field(fields::AUTHOR_ID).expect("author_id");
        let kind_field = self.schema.get_field(fields::KIND).expect("kind");
        let title_field = self.schema.get_field(fields::TITLE).expect("title");
        let edition_title_field = self
            .schema
            .get_field(fields::EDITION_TITLE)
            .expect("edition_title");
        let work_description_field = self
            .schema
            .get_field(fields::WORK_DESCRIPTION)
            .expect("work_description");
        let edition_description_field = self
            .schema
            .get_field(fields::EDITION_DESCRIPTION)
            .expect("edition_description");
        let authors_field = self.schema.get_field(fields::AUTHORS).expect("authors");
        let tags_field = self.schema.get_field(fields::TAGS).expect("tags");
        let genres_field = self.schema.get_field(fields::GENRES).expect("genres");
        let subjects_field = self.schema.get_field(fields::SUBJECTS).expect("subjects");
        let publishers_field = self
            .schema
            .get_field(fields::PUBLISHERS)
            .expect("publishers");
        let identifier_kinds_field = self
            .schema
            .get_field(fields::IDENTIFIER_KINDS)
            .expect("identifier_kinds");
        let identifier_values_field = self
            .schema
            .get_field(fields::IDENTIFIER_VALUES)
            .expect("identifier_values");
        let notes_field = self.schema.get_field(fields::NOTES).expect("notes");
        let format_field = self.schema.get_field(fields::FORMAT).expect("format");
        let language_field = self.schema.get_field(fields::LANGUAGE).expect("language");
        let language_facet_field = self
            .schema
            .get_field(fields::LANGUAGE_FACET)
            .expect("language_facet");
        let publisher_facet_field = self
            .schema
            .get_field(fields::PUBLISHER_FACET)
            .expect("publisher_facet");
        let subject_facet_field = self
            .schema
            .get_field(fields::SUBJECT_FACET)
            .expect("subject_facet");
        let genre_facet_field = self
            .schema
            .get_field(fields::GENRE_FACET)
            .expect("genre_facet");
        let pub_date_field = self.schema.get_field(fields::PUB_DATE).expect("pub_date");
        let published_year_field = self
            .schema
            .get_field(fields::PUBLISHED_YEAR)
            .expect("published_year");
        let title_sort_field = self
            .schema
            .get_field(fields::TITLE_SORT)
            .expect("title_sort");
        let primary_author_sort_field = self
            .schema
            .get_field(fields::PRIMARY_AUTHOR_SORT)
            .expect("primary_author_sort");
        let created_at_field = self
            .schema
            .get_field(fields::CREATED_AT)
            .expect("created_at");
        let updated_at_field = self
            .schema
            .get_field(fields::UPDATED_AT)
            .expect("updated_at");
        let popularity_field = self
            .schema
            .get_field(fields::POPULARITY)
            .expect("popularity");
        let source_field = self.schema.get_field(fields::SOURCE).expect("source");
        let tag_id_field = self.schema.get_field(fields::TAG_ID).expect("tag_id");
        let genre_id_field = self.schema.get_field(fields::GENRE_ID).expect("genre_id");
        let subject_id_field = self
            .schema
            .get_field(fields::SUBJECT_ID)
            .expect("subject_id");
        let series_id_field = self.schema.get_field(fields::SERIES_ID).expect("series_id");
        let publisher_id_field = self
            .schema
            .get_field(fields::PUBLISHER_ID)
            .expect("publisher_id");

        // Build all edition documents in parallel. Document
        // construction only reads the lookup maps and copies data
        // into a fresh `TantivyDocument` per edition; the index
        // writer is touched only in the sequential commit loop
        // below. This is the pre-build-Vec pattern: rayon handles
        // the CPU-bound work, and we keep the writer single-threaded
        // so we don't rely on `IndexWriter`'s Send-ness (tantivy's
        // public API doesn't guarantee it across all configurations).
        let edition_docs: Vec<tantivy::TantivyDocument> = all_editions
            .par_iter()
            .map(|edition| {
                let work = works_by_id.get(&edition.work_id);
                let edition_title = edition.title.clone().unwrap_or_default();
                let work_title = work.map(|w| w.title.clone()).unwrap_or_default();
                let resolved_title = if edition_title.is_empty() {
                    work_title.clone()
                } else {
                    edition_title.clone()
                };
                // Author resolution: edition_authors → work_authors →
                // empty.
                let authors = edition_to_authors
                    .get(&edition.id)
                    .cloned()
                    .or_else(|| work_to_authors.get(&edition.work_id).cloned())
                    .unwrap_or_default();
                let author_names: Vec<String> = authors.iter().map(|(_, n)| n.clone()).collect();
                // Categorical IDs are stored as text strings — tantivy's
                // text-fast fields accept any value, and the text form
                // lets us query `author_id:01HX...` and `author_id:01HY...`
                // against the same multi-valued column.
                let author_id_strs: Vec<String> =
                    authors.iter().map(|(id, _)| id.to_string()).collect();

                // Identifier kind/value vectors are aligned.
                let ident_kinds = edition_ident_kinds
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default();
                let ident_values = edition_ident_values
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default();

                // Tags / genres / subjects / publishers by name (for
                // text) and by id (for fast fields).
                let tag_names: Vec<String> = edition_to_tag_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|tid| tags_by_id.get(&tid).map(|n| (*n).to_string()))
                    .collect();
                let tag_id_strs: Vec<String> = edition_to_tag_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let genre_names: Vec<String> = edition_to_genre_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|gid| genres_by_id.get(&gid).map(|n| (*n).to_string()))
                    .collect();
                let genre_id_strs: Vec<String> = edition_to_genre_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let subject_names: Vec<String> = edition_to_subject_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|sid| subjects_by_id.get(&sid).map(|n| (*n).to_string()))
                    .collect();
                let subject_id_strs: Vec<String> = edition_to_subject_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let publisher_names: Vec<String> = edition_to_publisher_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|pid| publishers_by_id.get(&pid).map(|n| (*n).to_string()))
                    .collect();
                let publisher_id_strs: Vec<String> = edition_to_publisher_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let series_id_strs: Vec<String> = edition_to_series_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();

                let format_name = edition
                    .format_id
                    .and_then(|fid| formats_by_id.get(&fid).map(|n| (*n).to_string()));
                let language_name = edition
                    .language_id
                    .and_then(|lid| languages_by_id.get(&lid).map(|n| (*n).to_string()));
                let work_language_name = work
                    .and_then(|w| w.language_id)
                    .and_then(|lid| languages_by_id.get(&lid).map(|n| (*n).to_string()));
                let language_name = language_name.or(work_language_name);

                let work_id_hash = hash_work_id(&edition.work_id.to_string());

                let pub_date_value = edition.published_date.map(|d| {
                    let nanos = d.midnight().assume_utc().unix_timestamp_nanos();
                    tantivy::DateTime::from_timestamp_millis((nanos / 1_000_000) as i64)
                });
                let published_year = edition.published_date.map(|d| d.year() as u64).unwrap_or(0);
                let created_at_value = Some({
                    let nanos = edition.created_at.assume_utc().unix_timestamp_nanos();
                    tantivy::DateTime::from_timestamp_millis((nanos / 1_000_000) as i64)
                });

                let mut d = doc!();
                // IDs and kind discriminator
                d.add_text(edition_id_field, edition.id.to_string());
                d.add_text(work_id_field, edition.work_id.to_string());
                d.add_u64(work_id_hash_field, work_id_hash);
                for aid in &author_id_strs {
                    d.add_text(author_id_field, aid);
                }
                d.add_text(kind_field, kinds::EDITION);

                // Full-text + multi-valued
                d.add_text(title_field, &resolved_title);
                if !edition_title.is_empty() {
                    d.add_text(edition_title_field, &edition_title);
                }
                if let Some(w) = work
                    && let Some(desc) = &w.description
                {
                    d.add_text(work_description_field, desc);
                }
                if let Some(desc) = &edition.description {
                    d.add_text(edition_description_field, desc);
                }
                for n in &author_names {
                    d.add_text(authors_field, n);
                }
                for n in &tag_names {
                    d.add_text(tags_field, n);
                }
                for n in &genre_names {
                    d.add_text(genres_field, n);
                }
                for n in &subject_names {
                    d.add_text(subjects_field, n);
                }
                for n in &publisher_names {
                    d.add_text(publishers_field, n);
                }
                for k in &ident_kinds {
                    d.add_text(identifier_kinds_field, k);
                }
                for v in &ident_values {
                    d.add_text(identifier_values_field, v);
                }
                d.add_text(notes_field, edition.notes.clone().unwrap_or_default());

                // Filters / sort / facet
                if let Some(f) = &format_name {
                    d.add_text(format_field, f);
                }
                if let Some(l) = &language_name {
                    d.add_text(language_field, l);
                    d.add_facet(
                        language_facet_field,
                        Facet::from_text(&format!("/{}", l))
                            .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
                    );
                }
                for p in &publisher_names {
                    d.add_facet(
                        publisher_facet_field,
                        Facet::from_text(&format!("/{}", p))
                            .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
                    );
                }
                for s in &subject_names {
                    d.add_facet(
                        subject_facet_field,
                        Facet::from_text(&format!("/{}", s))
                            .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
                    );
                }
                for g in &genre_names {
                    d.add_facet(
                        genre_facet_field,
                        Facet::from_text(&format!("/{}", g))
                            .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
                    );
                }
                if let Some(pd) = pub_date_value {
                    d.add_date(pub_date_field, pd);
                }
                if published_year > 0 {
                    d.add_u64(published_year_field, published_year);
                }
                d.add_text(title_sort_field, resolved_title.to_lowercase());
                if let Some((_, primary_author)) = authors.first() {
                    d.add_text(primary_author_sort_field, primary_author.to_lowercase());
                }
                if let Some(ts) = created_at_value {
                    d.add_date(created_at_field, ts);
                }
                // updated_at — same treatment as created_at, using the
                // edition's updated_at when available.
                if let Some(ua) = edition.updated_at {
                    let nanos = ua.assume_utc().unix_timestamp_nanos();
                    d.add_date(
                        updated_at_field,
                        tantivy::DateTime::from_timestamp_millis((nanos / 1_000_000) as i64),
                    );
                }
                // popularity is currently unset; we still add the field
                // so the fast column exists.
                d.add_u64(popularity_field, 0u64);

                // Source provenance
                let edition_source = edition_to_source
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_else(|| "catalog".to_string());
                d.add_text(source_field, &edition_source);

                // Categorical IDs.
                for t in &tag_id_strs {
                    d.add_text(tag_id_field, t);
                }
                for g in &genre_id_strs {
                    d.add_text(genre_id_field, g);
                }
                for s in &subject_id_strs {
                    d.add_text(subject_id_field, s);
                }
                for s in &series_id_strs {
                    d.add_text(series_id_field, s);
                }
                for p in &publisher_id_strs {
                    d.add_text(publisher_id_field, p);
                }

                d
            })
            .collect();

        // Commit the pre-built documents to the index in a single
        // thread. Tantivy's `IndexWriter` Send-ness is not part of
        // its public API contract, so we keep writer access strictly
        // serial — the rayon work above is purely CPU-bound document
        // construction, with no shared mutable state.
        //
        // `add_document` takes the `Document` by value, so we move
        // each one out of the `Vec`. `edition_docs` is no longer
        // needed after this loop and is dropped at the end of the
        // scope.
        for d in edition_docs {
            writer.add_document(d)?;
        }

        // Authors get their own documents, indexed with `kind = "author"`.
        for author in &all_authors {
            let mut d = doc!();
            d.add_text(author_id_field, author.id.to_string());
            d.add_text(kind_field, kinds::AUTHOR);
            d.add_text(title_field, &author.name);
            d.add_text(authors_field, &author.name);
            d.add_text(title_sort_field, author.name.to_lowercase());
            d.add_text(primary_author_sort_field, author.name.to_lowercase());
            writer.add_document(d)?;
        }

        writer.commit()?;
        self.reader.reload()?;
        tracing::debug!(
            target: "livtet.search.perf",
            elapsed_ms = start.elapsed().as_millis(),
            "search reindex complete"
        );
        Ok(())
    }

    /// Add or update one edition in the index.
    ///
    /// For now this delegates to a full reindex because Tantivy
    /// can't rewrite a single document cheaply. The seam exists so
    /// callers (the Tauri command) can swap in a true upsert path
    /// later without changing call sites.
    pub async fn add_edition(
        &self,
        db: &DatabaseConnection,
        edition_id: livtet_types::DbId,
    ) -> Result<(), SearchError> {
        let _ = (db, edition_id);
        self.reindex(db).await
    }

    /// Single-doc upsert from a denormalized [`EditionDoc`].
    ///
    /// Unlike [`add_edition`], this never touches the database; the
    /// caller (typically the NAPI binding) assembles the document
    /// shape and writes it directly. Used to keep the NAPI hot path
    /// off the SQL reindex seam while still surfacing edits in
    /// search results.
    ///
    /// Semantics: any existing document whose `edition_id` matches
    /// `doc.edition_id` is deleted before the new one is added, so a
    /// second call with the same id overwrites rather than
    /// duplicating.
    pub async fn upsert_edition(&self, doc: EditionDoc) -> Result<(), SearchError> {
        let edition_id_field = self
            .schema
            .get_field(fields::EDITION_ID)
            .expect("edition_id");
        let work_id_field = self.schema.get_field(fields::WORK_ID).expect("work_id");
        let work_id_hash_field = self
            .schema
            .get_field(fields::WORK_ID_HASH)
            .expect("work_id_hash");
        let author_id_field = self.schema.get_field(fields::AUTHOR_ID).expect("author_id");
        let kind_field = self.schema.get_field(fields::KIND).expect("kind");
        let title_field = self.schema.get_field(fields::TITLE).expect("title");
        let edition_title_field = self
            .schema
            .get_field(fields::EDITION_TITLE)
            .expect("edition_title");
        let work_description_field = self
            .schema
            .get_field(fields::WORK_DESCRIPTION)
            .expect("work_description");
        let edition_description_field = self
            .schema
            .get_field(fields::EDITION_DESCRIPTION)
            .expect("edition_description");
        let authors_field = self.schema.get_field(fields::AUTHORS).expect("authors");
        let tags_field = self.schema.get_field(fields::TAGS).expect("tags");
        let genres_field = self.schema.get_field(fields::GENRES).expect("genres");
        let subjects_field = self.schema.get_field(fields::SUBJECTS).expect("subjects");
        let publishers_field = self
            .schema
            .get_field(fields::PUBLISHERS)
            .expect("publishers");
        let identifier_kinds_field = self
            .schema
            .get_field(fields::IDENTIFIER_KINDS)
            .expect("identifier_kinds");
        let identifier_values_field = self
            .schema
            .get_field(fields::IDENTIFIER_VALUES)
            .expect("identifier_values");
        let notes_field = self.schema.get_field(fields::NOTES).expect("notes");
        let format_field = self.schema.get_field(fields::FORMAT).expect("format");
        let language_field = self.schema.get_field(fields::LANGUAGE).expect("language");
        let language_facet_field = self
            .schema
            .get_field(fields::LANGUAGE_FACET)
            .expect("language_facet");
        let publisher_facet_field = self
            .schema
            .get_field(fields::PUBLISHER_FACET)
            .expect("publisher_facet");
        let subject_facet_field = self
            .schema
            .get_field(fields::SUBJECT_FACET)
            .expect("subject_facet");
        let genre_facet_field = self
            .schema
            .get_field(fields::GENRE_FACET)
            .expect("genre_facet");
        let pub_date_field = self.schema.get_field(fields::PUB_DATE).expect("pub_date");
        let published_year_field = self
            .schema
            .get_field(fields::PUBLISHED_YEAR)
            .expect("published_year");
        let title_sort_field = self
            .schema
            .get_field(fields::TITLE_SORT)
            .expect("title_sort");
        let primary_author_sort_field = self
            .schema
            .get_field(fields::PRIMARY_AUTHOR_SORT)
            .expect("primary_author_sort");
        let created_at_field = self
            .schema
            .get_field(fields::CREATED_AT)
            .expect("created_at");
        let updated_at_field = self
            .schema
            .get_field(fields::UPDATED_AT)
            .expect("updated_at");
        let popularity_field = self
            .schema
            .get_field(fields::POPULARITY)
            .expect("popularity");
        let source_field = self.schema.get_field(fields::SOURCE).expect("source");

        let work_id_hash = hash_work_id(&doc.work_id);
        let pub_date_value = doc
            .pub_date
            .map(|secs| tantivy::DateTime::from_timestamp_millis(secs.saturating_mul(1_000)));
        let published_year = doc.published_year.map(|y| y.max(0) as u64).unwrap_or(0);
        let created_at_value =
            tantivy::DateTime::from_timestamp_millis(doc.created_at.saturating_mul(1_000));

        let mut d = doc!();
        d.add_text(edition_id_field, &doc.edition_id);
        d.add_text(work_id_field, &doc.work_id);
        d.add_u64(work_id_hash_field, work_id_hash);
        for aid in &doc.authors_ids {
            d.add_text(author_id_field, aid);
        }
        d.add_text(kind_field, kinds::EDITION);
        d.add_text(title_field, &doc.title);
        if let Some(et) = &doc.edition_title {
            d.add_text(edition_title_field, et);
        }
        if let Some(desc) = &doc.work_description {
            d.add_text(work_description_field, desc);
        }
        if let Some(desc) = &doc.edition_description {
            d.add_text(edition_description_field, desc);
        }
        for n in &doc.authors {
            d.add_text(authors_field, n);
        }
        for n in &doc.tags {
            d.add_text(tags_field, n);
        }
        for n in &doc.genres {
            d.add_text(genres_field, n);
        }
        for n in &doc.subjects {
            d.add_text(subjects_field, n);
        }
        for n in &doc.publishers {
            d.add_text(publishers_field, n);
        }
        for k in &doc.identifier_kinds {
            d.add_text(identifier_kinds_field, k);
        }
        for v in &doc.identifier_values {
            d.add_text(identifier_values_field, v);
        }
        d.add_text(notes_field, doc.notes.clone().unwrap_or_default());
        if let Some(f) = &doc.format {
            d.add_text(format_field, f);
        }
        if let Some(l) = &doc.language {
            d.add_text(language_field, l);
            d.add_facet(
                language_facet_field,
                Facet::from_text(&format!("/{}", l))
                    .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
            );
        }
        for p in &doc.publishers {
            d.add_facet(
                publisher_facet_field,
                Facet::from_text(&format!("/{}", p))
                    .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
            );
        }
        for s in &doc.subjects {
            d.add_facet(
                subject_facet_field,
                Facet::from_text(&format!("/{}", s))
                    .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
            );
        }
        for g in &doc.genres {
            d.add_facet(
                genre_facet_field,
                Facet::from_text(&format!("/{}", g))
                    .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
            );
        }
        if let Some(pd) = pub_date_value {
            d.add_date(pub_date_field, pd);
        }
        if published_year > 0 {
            d.add_u64(published_year_field, published_year);
        }
        d.add_text(title_sort_field, doc.title_sort.to_lowercase());
        if let Some(primary) = &doc.primary_author_sort {
            d.add_text(primary_author_sort_field, primary.to_lowercase());
        }
        d.add_date(created_at_field, created_at_value);
        if let Some(secs) = doc.updated_at {
            d.add_date(
                updated_at_field,
                tantivy::DateTime::from_timestamp_millis(secs.saturating_mul(1_000)),
            );
        }
        d.add_u64(popularity_field, doc.popularity.max(0) as u64);
        // NAPI-sourced docs don't carry a source provenance string;
        // default to the same "catalog" bucket reindex uses.
        d.add_text(source_field, "catalog");

        let mut writer = self.writer.write().await;
        let term = Term::from_field_text(edition_id_field, &doc.edition_id);
        writer.delete_term(term);
        writer.add_document(d)?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Single-doc upsert from a denormalized [`AuthorDoc`].
    ///
    /// Mirrors [`upsert_edition`]: any existing document whose
    /// `author_id` matches `doc.author_id` is deleted before the
    /// new one is added. Author documents carry the `kind = "author"`
    /// discriminator so [`HitKind::Person`] queries resolve them.
    pub async fn upsert_author(&self, doc: AuthorDoc) -> Result<(), SearchError> {
        let author_id_field = self.schema.get_field(fields::AUTHOR_ID).expect("author_id");
        let kind_field = self.schema.get_field(fields::KIND).expect("kind");
        let title_field = self.schema.get_field(fields::TITLE).expect("title");
        let authors_field = self.schema.get_field(fields::AUTHORS).expect("authors");
        let title_sort_field = self
            .schema
            .get_field(fields::TITLE_SORT)
            .expect("title_sort");
        let primary_author_sort_field = self
            .schema
            .get_field(fields::PRIMARY_AUTHOR_SORT)
            .expect("primary_author_sort");
        let source_field = self.schema.get_field(fields::SOURCE).expect("source");

        let mut d = doc!();
        d.add_text(author_id_field, &doc.author_id);
        d.add_text(kind_field, kinds::AUTHOR);
        d.add_text(title_field, &doc.name);
        d.add_text(authors_field, &doc.name);
        d.add_text(title_sort_field, doc.sort_name.to_lowercase());
        d.add_text(primary_author_sort_field, doc.sort_name.to_lowercase());
        d.add_text(source_field, &doc.source);

        let mut writer = self.writer.write().await;
        let term = Term::from_field_text(author_id_field, &doc.author_id);
        writer.delete_term(term);
        writer.add_document(d)?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Delete one edition from the index by its ULID.
    ///
    /// Tantivy can't surgically drop one document, so we delete by
    /// term on the indexed `edition_id` field. If `edition_id` was
    /// never indexed (e.g. the editor never reindexed) this is a
    /// harmless no-op.
    pub async fn delete_edition(&self, edition_id: livtet_types::DbId) -> Result<(), SearchError> {
        let edition_id_field = self
            .schema
            .get_field(fields::EDITION_ID)
            .expect("edition_id");
        let term = Term::from_field_text(edition_id_field, &edition_id.to_string());
        let mut writer = self.writer.write().await;
        writer.delete_term(term);
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    // ---- Query parser --------------------------------------------------

    /// Build a configured [`QueryParser`] covering every full-text field
    /// in the schema with the field boosts called out in the design
    /// plan:
    ///
    /// - title 4×
    /// - authors 2×
    /// - edition_description 1.5×
    /// - work_description 1×
    /// - identifier_values 1×
    ///
    /// Plus fuzzy-on-title and conjunction-by-default so plain user
    /// input parses the way readers expect.
    ///
    /// Use this when you need the raw parser — e.g. to call
    /// [`QueryParser::build_query_from_user_input_ast`] on a
    /// pre-composed AST produced by `livtet-search-types`'s
    /// composition engine. For plain string queries use
    /// [`build_query_parser`](Self::build_query_parser).
    pub fn get_query_parser(&self) -> QueryParser {
        let title = self.schema.get_field(fields::TITLE).expect("title");
        let authors = self.schema.get_field(fields::AUTHORS).expect("authors");
        let tags = self.schema.get_field(fields::TAGS).expect("tags");
        let genres = self.schema.get_field(fields::GENRES).expect("genres");
        let subjects = self.schema.get_field(fields::SUBJECTS).expect("subjects");
        let publishers = self
            .schema
            .get_field(fields::PUBLISHERS)
            .expect("publishers");
        let edition_description = self
            .schema
            .get_field(fields::EDITION_DESCRIPTION)
            .expect("edition_description");
        let work_description = self
            .schema
            .get_field(fields::WORK_DESCRIPTION)
            .expect("work_description");
        let edition_title = self
            .schema
            .get_field(fields::EDITION_TITLE)
            .expect("edition_title");
        let notes = self.schema.get_field(fields::NOTES).expect("notes");
        let identifier_values = self
            .schema
            .get_field(fields::IDENTIFIER_VALUES)
            .expect("identifier_values");
        let format = self.schema.get_field(fields::FORMAT).expect("format");
        let language = self.schema.get_field(fields::LANGUAGE).expect("language");

        let mut parser = QueryParser::for_index(
            &self.index,
            vec![
                title,
                edition_title,
                authors,
                tags,
                genres,
                subjects,
                publishers,
                identifier_values,
                edition_description,
                work_description,
                notes,
                format,
                language,
            ],
        );
        parser.set_field_boost(title, 4.0);
        parser.set_field_boost(authors, 2.0);
        parser.set_field_boost(edition_description, 1.5);
        parser.set_field_boost(work_description, 1.0);
        parser.set_field_boost(identifier_values, 1.0);
        // Conjunction default — multiple search terms are AND'd
        // unless the user types `OR`.
        parser.set_conjunction_by_default();
        // Fuzzy on title so a typo still hits "The Name of the
        // Wind" when the user types "The Naem of the Wnd".
        // Per the design plan: prefix=false (full term, no prefix
        // matches), distance=2 (allow two edits), transpose_cost_one
        // =false (use tantivy's default cost model).
        parser.set_field_fuzzy(title, false, 2, false);
        parser
    }

    /// Parse a free-text query string with the configured
    /// [`QueryParser`]. Equivalent to
    /// `self.get_query_parser().parse_query(query_str)`.
    pub fn build_query_parser(&self, query_str: &str) -> tantivy::Result<Box<dyn Query>> {
        Ok(self.get_query_parser().parse_query(query_str)?)
    }

    // ---- Search APIs ---------------------------------------------------

    /// Edition-level search. Returns one hit per matched edition.
    pub async fn search(&self, query_str: &str, limit: usize) -> tantivy::Result<Vec<SearchHit>> {
        self.search_with_options(query_str, limit, &SearchOptions::default())
            .await
    }

    /// Edition-level search with full option control. Internal
    /// workhorse — `search` and `search_works` both delegate here
    /// with different option sets.
    ///
    /// When [`SearchOptions::sort`] is `Some`, the top-N result is
    /// post-sorted by the corresponding fast field before truncation;
    /// see the field-level docs on `SearchOptions::sort` for the
    /// trade-off (tantivy's text-fast-field API doesn't support
    /// `order_by_fast_field::<String>` so all four `SortField`
    /// variants use the same read-and-rewrite path).
    #[tracing::instrument(
        level = "debug",
        name = "search.tantivy",
        skip(self, opts, query_str),
        fields(limit, collapse_to_works = opts.collapse_to_works, query_len = query_str.len())
    )]
    pub async fn search_with_options(
        &self,
        query_str: &str,
        limit: usize,
        opts: &SearchOptions,
    ) -> tantivy::Result<Vec<SearchHit>> {
        let start = std::time::Instant::now();
        let searcher = self.reader.searcher();
        // Parse + AND with the kind=edition discriminator and any
        // caller-supplied filters via the shared query backbone.
        let mut query =
            self.build_filtered_query(query_str, &livtet_types::WorkFilters::default())?;
        // When source_filter is set, AND it into the query.
        if let Some(sf) = &opts.source_filter {
            let kind_field = self.schema.get_field(fields::KIND).expect("kind");
            let source_field = self.schema.get_field(fields::SOURCE).expect("source");
            query = Box::new(tantivy::query::BooleanQuery::new(vec![
                (Occur::Must, query),
                (
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(source_field, sf),
                        IndexRecordOption::Basic,
                    )),
                ),
                (
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(kind_field, kinds::EDITION),
                        IndexRecordOption::Basic,
                    )),
                ),
            ]));
        }
        // When offset is requested, we need to fetch extra hits so
        // we can drop the first `offset` results in-memory.
        let effective_limit = limit.saturating_add(opts.offset);
        let base_limit = if opts.collapse_to_works {
            // Over-fetch so the per-work collapse has enough raw
            // hits to cover the limit even when many editions of
            // the same work are present.
            effective_limit.saturating_mul(opts.work_overfetch.max(1))
        } else {
            effective_limit
        };
        // When post-sorting by a fast field we don't know the
        // ranking of items beyond the score-best slice, so bump the
        // fetch so truncation to `base_limit` doesn't bias the
        // top-N toward score-best items.
        let fetch_limit = if opts.sort.is_some() {
            base_limit.saturating_mul(2).max(base_limit + 64)
        } else {
            base_limit
        };

        // `query` is borrowed below; clone it via tantivy's
        // `QueryClone` trait so we still hold a handle for snippet
        // and explanation generation in `build_hits`.
        let query_for_hit_build = query.box_clone();
        let mut top_docs =
            searcher.search(&*query, &TopDocs::with_limit(fetch_limit).order_by_score())?;

        tracing::debug!(
            target: "livtet.search.perf",
            elapsed_us = start.elapsed().as_micros(),
            hits = top_docs.len(),
            "tantivy search"
        );

        // Apply explicit sort (when requested) before handing off
        // to build_hits. For `Score` the slice is already in score
        // order so the helper short-circuits to a clone.
        if let Some(spec) = opts.sort.as_ref() {
            top_docs = sort_top_docs_by_spec(&searcher, &self.schema, top_docs, spec)?;
        }
        // Truncate to the user-requested (or work-overfetched)
        // count so build_hits and any collapse logic operate on the
        // intended slice.
        top_docs.truncate(base_limit);
        // Apply in-memory offset: drop the first `offset` hits.
        // This happens after sorting so the offset is relative to
        // the requested sort order, not the raw score order.
        if opts.offset > 0 && opts.offset < top_docs.len() {
            top_docs.drain(..opts.offset);
        } else if opts.offset >= top_docs.len() {
            top_docs.clear();
        }

        let hits = self
            .build_hits(&searcher, &*query_for_hit_build, &top_docs, opts)
            .await?;

        if opts.collapse_to_works {
            Ok(collapse_editions_to_works(hits, limit))
        } else {
            Ok(hits.into_iter().take(limit).collect())
        }
    }

    /// Search with a pre-built `Box<dyn Query>` (e.g. from
    /// [`WorkFiltersQuery::build_query`]) instead of a query string.
    /// Applies `kind = "edition"` filter and optional work-collapse
    /// just like [`SearchIndex::search_with_options`].
    #[tracing::instrument(
        level = "debug",
        name = "search.tantivy.query",
        skip(self, query, opts),
        fields(limit, collapse_to_works = opts.collapse_to_works)
    )]
    pub async fn search_with_query(
        &self,
        query: Box<dyn Query>,
        limit: usize,
        opts: &SearchOptions,
    ) -> tantivy::Result<Vec<SearchHit>> {
        let start = std::time::Instant::now();
        let searcher = self.reader.searcher();

        // When offset is requested, fetch extra hits so we can drop
        // the first `offset` results in-memory.
        let effective_limit = limit.saturating_add(opts.offset);
        let fetch_limit = if opts.collapse_to_works {
            effective_limit.saturating_mul(opts.work_overfetch.max(1))
        } else {
            effective_limit
        };

        // Filter out author documents from the result set.
        let kind_filter = self.schema.get_field(fields::KIND).expect("kind");
        let query_for_hit_build = query.box_clone();
        let edition_query: Box<dyn Query> = Box::new(tantivy::query::BooleanQuery::new(vec![
            (Occur::Must, query),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(kind_filter, kinds::EDITION),
                    IndexRecordOption::Basic,
                )),
            ),
        ]));

        let mut top_docs = searcher.search(
            &edition_query,
            &TopDocs::with_limit(fetch_limit).order_by_score(),
        )?;

        tracing::debug!(
            target: "livtet.search.perf",
            elapsed_us = start.elapsed().as_micros(),
            hits = top_docs.len(),
            "tantivy search_with_query"
        );

        // Apply in-memory offset: drop the first `offset` hits.
        if opts.offset > 0 && opts.offset < top_docs.len() {
            top_docs.drain(..opts.offset);
        } else if opts.offset >= top_docs.len() {
            top_docs.clear();
        }

        let hits = self
            .build_hits(&searcher, &*query_for_hit_build, &top_docs, opts)
            .await?;

        if opts.collapse_to_works {
            Ok(collapse_editions_to_works(hits, limit))
        } else {
            Ok(hits.into_iter().take(limit).collect())
        }
    }

    /// Search across every document kind (editions and authors).
    ///
    /// Unlike [`SearchIndex::search_with_options`] this method does
    /// NOT filter on `kind = "edition"` — it surfaces author
    /// documents as `HitKind::Person` hits alongside edition hits. Use
    /// this for the "people + works" dropdown UI; stick to
    /// [`search`](Self::search) for edition-only result lists.
    #[tracing::instrument(
        level = "debug",
        name = "search.tantivy.all_kinds",
        skip(self, opts),
        fields(limit, collapse_to_works = opts.collapse_to_works)
    )]
    pub async fn search_all_kinds(
        &self,
        query_str: &str,
        limit: usize,
        opts: &SearchOptions,
    ) -> tantivy::Result<Vec<SearchHit>> {
        let start = std::time::Instant::now();
        let searcher = self.reader.searcher();
        let query = self.build_query_parser(query_str)?;

        // When offset is requested, fetch extra hits so we can drop
        // the first `offset` results in-memory.
        let effective_limit = limit.saturating_add(opts.offset);
        let fetch_limit = if opts.collapse_to_works {
            effective_limit.saturating_mul(opts.work_overfetch.max(1))
        } else {
            effective_limit
        };

        let query_for_hit_build = query.box_clone();
        let mut top_docs =
            searcher.search(&*query, &TopDocs::with_limit(fetch_limit).order_by_score())?;

        tracing::debug!(
            target: "livtet.search.perf",
            elapsed_us = start.elapsed().as_micros(),
            hits = top_docs.len(),
            "tantivy search (all kinds)"
        );

        // Apply in-memory offset: drop the first `offset` hits.
        if opts.offset > 0 && opts.offset < top_docs.len() {
            top_docs.drain(..opts.offset);
        } else if opts.offset >= top_docs.len() {
            top_docs.clear();
        }

        let hits = self
            .build_hits(&searcher, &*query_for_hit_build, &top_docs, opts)
            .await?;

        if opts.collapse_to_works {
            Ok(collapse_editions_to_works(hits, limit))
        } else {
            Ok(hits.into_iter().take(limit).collect())
        }
    }

    /// Work-level search. Internally calls [`SearchIndex::search`]
    /// with the collapse flag, over-fetching by `WORK_GROUP_OVERFETCH`
    /// so the per-work grouping has enough raw data.
    pub async fn search_works(
        &self,
        query_str: &str,
        limit: usize,
    ) -> tantivy::Result<Vec<SearchHit>> {
        let opts = SearchOptions {
            collapse_to_works: true,
            ..SearchOptions::default()
        };
        self.search_with_options(query_str, limit, &opts).await
    }

    /// Facet-aware edition search. Returns hits plus facet counts
    /// for language / publisher / subject / genre.
    pub async fn search_with_facets(
        &self,
        query_str: &str,
        limit: usize,
    ) -> tantivy::Result<FacetedSearchResult> {
        let searcher = self.reader.searcher();
        // Use the shared query backbone so the filter/kind logic
        // stays in lock-step with `search_with_options`.
        let query = self.build_filtered_query(query_str, &livtet_types::WorkFilters::default())?;
        // Clone the user query so we can drive `build_hits` after
        // moving the original into the collector.
        let query_for_hit_build = query.box_clone();

        let edition_query: Box<dyn Query> = query;

        let mut collectors = tantivy::collector::MultiCollector::new();
        let top_handle = collectors.add_collector(TopDocs::with_limit(limit).order_by_score());
        let lang_handle =
            collectors.add_collector(FacetCollector::for_field(fields::LANGUAGE_FACET));
        let pub_handle =
            collectors.add_collector(FacetCollector::for_field(fields::PUBLISHER_FACET));
        let subj_handle =
            collectors.add_collector(FacetCollector::for_field(fields::SUBJECT_FACET));
        let genre_handle = collectors.add_collector(FacetCollector::for_field(fields::GENRE_FACET));
        // The `pub_date` fast field stores tantivy's `DateTime`. Annotating
        // the closure return type (`Vec<(Option<DateTime>, _), _>`) lets
        // Tantivy pick the right `FastValue` impl without inferring `()`
        // when the same expression could also be `(Score, DocAddress)`
        // for a plain `TopDocs`.
        let recent_handle = collectors.add_collector(
            TopDocs::with_limit(limit)
                .order_by_fast_field::<tantivy::DateTime>(fields::PUB_DATE, Order::Desc),
        );
        let mut multi = searcher.search(&edition_query, &collectors)?;
        let top = top_handle.extract(&mut multi);
        let lang_fc = lang_handle.extract(&mut multi);
        let pub_fc = pub_handle.extract(&mut multi);
        let subj_fc = subj_handle.extract(&mut multi);
        let genre_fc = genre_handle.extract(&mut multi);
        let recent = recent_handle.extract(&mut multi);

        let hits = self
            .build_hits(
                &searcher,
                &*query_for_hit_build,
                &top,
                &SearchOptions::default(),
            )
            .await?;

        Ok(FacetedSearchResult {
            hits,
            language_facets: facet_counts(&lang_fc),
            publisher_facets: facet_counts(&pub_fc),
            subject_facets: facet_counts(&subj_fc),
            genre_facets: facet_counts(&genre_fc),
            recently_added: recent.len(),
        })
    }

    // ---- Hit building --------------------------------------------------

    async fn build_hits(
        &self,
        searcher: &tantivy::Searcher,
        query: &dyn Query,
        top_docs: &[(f32, tantivy::DocAddress)],
        opts: &SearchOptions,
    ) -> tantivy::Result<Vec<SearchHit>> {
        let edition_id_field = self
            .schema
            .get_field(fields::EDITION_ID)
            .expect("edition_id");
        let work_id_field = self.schema.get_field(fields::WORK_ID).expect("work_id");
        let author_id_field = self.schema.get_field(fields::AUTHOR_ID).expect("author_id");
        let kind_field = self.schema.get_field(fields::KIND).expect("kind");
        let title_field = self.schema.get_field(fields::TITLE).expect("title");
        let edition_title_field = self
            .schema
            .get_field(fields::EDITION_TITLE)
            .expect("edition_title");
        let authors_field = self.schema.get_field(fields::AUTHORS).expect("authors");
        let pub_date_field = self.schema.get_field(fields::PUB_DATE).expect("pub_date");
        let format_field = self.schema.get_field(fields::FORMAT).expect("format");
        let language_field = self.schema.get_field(fields::LANGUAGE).expect("language");
        let source_field = self.schema.get_field(fields::SOURCE).expect("source");

        let snippet_field = self
            .schema
            .get_field(fields::EDITION_DESCRIPTION)
            .expect("edition_description");

        let snippet_generator = if opts.with_snippet {
            match SnippetGenerator::create(searcher, query, snippet_field) {
                Ok(mut g) => {
                    g.set_max_num_chars(opts.snippet_chars);
                    Some(g)
                }
                Err(_) => None,
            }
        } else {
            None
        };

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, addr) in top_docs {
            let doc: TantivyDocument = searcher.doc(*addr)?;
            let kind = doc
                .get_first(kind_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let edition_id = doc
                .get_first(edition_id_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let work_id = doc
                .get_first(work_id_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let author_id = doc
                .get_first(author_id_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let title = doc
                .get_first(title_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let edition_title = doc
                .get_first(edition_title_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let author_names: Vec<String> = doc
                .get_all(authors_field)
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            let format_name = doc
                .get_first(format_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let language_name = doc
                .get_first(language_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let published_date = doc
                .get_first(pub_date_field)
                .and_then(|v| v.as_datetime())
                // tantivy's DateTime only implements Debug, not Display.
                // The plan's contract is "ISO-8601 string"; Debug here
                // serialises the underlying OffsetDateTime in RFC-3339.
                .map(|d| format!("{:?}", d));

            let hit_source = doc
                .get_first(source_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "catalog".to_string());

            let explanation = if opts.explain {
                query
                    .explain(searcher, *addr)
                    .ok()
                    .map(|ex| ex.to_pretty_json())
            } else {
                None
            };

            let (snippet_text, snippet_highlighted) = match &snippet_generator {
                Some(snippet_gen) => {
                    let snippet = snippet_gen.snippet_from_doc(&doc);
                    let text = snippet.fragment().to_string();
                    // tantivy's `Snippet::highlighted()` returns
                    // `&[Range<usize>]` indexed into the fragment
                    // bytes. Map to `[u32; 2]` for IPC compatibility.
                    let ranges: Vec<[u32; 2]> = snippet.highlighted().iter()
                        .map(|r| [r.start as u32, r.end as u32])
                        .collect();
                    (Some(text), ranges)
                }
                None => (None, Vec::new()),
            };

            let hit_kind = match kind.as_str() {
                kinds::AUTHOR => HitKind::Person,
                _ => HitKind::Edition,
            };

            hits.push(SearchHit {
                kind: hit_kind,
                edition_id: if hit_kind == HitKind::Edition {
                    edition_id.clone()
                } else {
                    None
                },
                work_id: work_id.clone(),
                author_id: if hit_kind == HitKind::Person {
                    author_id.clone()
                } else {
                    None
                },
                title: title.clone(),
                work_title: None,
                edition_title,
                authors: if hit_kind == HitKind::Person {
                    Vec::new()
                } else {
                    author_names
                },
                isbn: None,
                format: format_name,
                language: language_name,
                published_date,
                score: *score,
                explanation,
                snippet_text,
                snippet_highlighted,
                grouped_edition_ids: Vec::new(),
                source: hit_source,
            });
        }

        // ISBN resolution requires a DB hop; we leave it to the
        // caller via [`EditionLookup::get_edition_isbns`] and merge
        // in the Tauri command. The search crate itself is DB-agnostic.
        Ok(hits)
    }

    // ---- Phase A additions: shared query backbone, count, ids ----

    /// Build the shared query backbone used by
    /// [`SearchIndex::search_with_options`],
    /// [`SearchIndex::search_with_facets`],
    /// [`SearchIndex::count_works_filtered`], and
    /// [`SearchIndex::matching_work_ids`].
    ///
    /// The returned `Box<dyn Query>` AND-combines:
    /// - The parsed user query (when `query_str` is non-empty), or
    ///   `AllQuery` when both the query and the filters are empty
    ///   (a Tantivy requirement — an empty-armed `BooleanQuery`
    ///   returns zero hits).
    /// - The filter clauses built from `filters` via
    ///   [`WorkFiltersQuery::build_query`]. Format / language
    ///   filters are not honoured in this helper; callers that need
    ///   to filter on `format_ids` or `language_ids` must pre-resolve
    ///   those ids to labels via [`WorkFiltersQuery::new`] /
    ///   [`WorkFiltersResolved::from_filters`] instead. (Existing
    ///   Tauri call sites already do this resolution before they
    ///   build a `SortSpec`.)
    /// - The `kind = edition` discriminator so author documents
    ///   (`kind = "author"`) are pruned from edition/work queries.
    fn build_filtered_query(
        &self,
        query_str: &str,
        filters: &livtet_types::WorkFilters,
    ) -> tantivy::Result<Box<dyn Query>> {
        let kind_filter = self.schema.get_field(fields::KIND).expect("kind");
        let resolved = WorkFiltersResolved {
            filters: filters.clone(),
            format_labels: Vec::new(),
            language_labels: Vec::new(),
        };
        // `WorkFiltersQuery::build_query` already returns
        // `AllQuery` when both the user text and all filters are
        // empty, so the BooleanQuery below always wraps at least
        // one Must clause plus the kind=edition TermQuery.
        let filter_query =
            WorkFiltersQuery::new(resolved, query_str.to_string()).build_query(&self.index)?;
        Ok(Box::new(tantivy::query::BooleanQuery::new(vec![
            (Occur::Must, filter_query),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(kind_filter, kinds::EDITION),
                    IndexRecordOption::Basic,
                )),
            ),
        ])))
    }

    /// Count the total number of works whose editions match the
    /// given query and filters. Backed by tantivy's [`Count`]
    /// collector, so the caller does not pay for hit materialisation.
    /// Used by the OPDS server for `<opensearch:totalResults>`.
    #[tracing::instrument(
        level = "debug",
        name = "search.count_works_filtered",
        skip(self, filters),
        fields(query_len = query_str.len())
    )]
    pub async fn count_works_filtered(
        &self,
        query_str: &str,
        filters: &livtet_types::WorkFilters,
    ) -> tantivy::Result<usize> {
        let searcher = self.reader.searcher();
        let query = self.build_filtered_query(query_str, filters)?;
        searcher.search(&*query, &Count)
    }

    /// Count documents matching a pre-built query (with `kind=edition`
    /// filter already baked in). Used when the caller has already
    /// resolved format/language labels and built the query via
    /// [`WorkFiltersQuery`].
    #[tracing::instrument(level = "debug", name = "search.count_with_query", skip(self, query))]
    pub async fn count_with_query(&self, query: Box<dyn Query>) -> tantivy::Result<usize> {
        let searcher = self.reader.searcher();
        let kind_filter = self.schema.get_field(fields::KIND).expect("kind");
        let edition_query: Box<dyn Query> = Box::new(tantivy::query::BooleanQuery::new(vec![
            (Occur::Must, query),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(kind_filter, kinds::EDITION),
                    IndexRecordOption::Basic,
                )),
            ),
        ]));
        searcher.search(&*edition_query, &Count)
    }

    /// Return the deduplicated work IDs of every work whose
    /// editions match the given query and filters, in tantivy's
    /// score-order. Used by the OPDS server for facet computation
    /// and per-page link rendering.
    ///
    /// The result is capped at [`OPDS_WORK_ID_LIMIT`] entries — the
    /// OPDS pagination UI doesn't render a page that wide, so
    /// fetching further would be wasted work. Duplicate editions
    /// of the same work collapse onto one `DbId` in insertion
    /// order.
    #[tracing::instrument(
        level = "debug",
        name = "search.matching_work_ids",
        skip(self, filters),
        fields(query_len = query_str.len())
    )]
    pub async fn matching_work_ids(
        &self,
        query_str: &str,
        filters: &livtet_types::WorkFilters,
    ) -> tantivy::Result<Vec<livtet_types::DbId>> {
        let searcher = self.reader.searcher();
        let query = self.build_filtered_query(query_str, filters)?;
        let top_docs = searcher.search(
            &*query,
            &TopDocs::with_limit(OPDS_WORK_ID_LIMIT).order_by_score(),
        )?;
        let work_id_field = self.schema.get_field(fields::WORK_ID).expect("work_id");
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out: Vec<livtet_types::DbId> = Vec::new();
        for (_score, addr) in &top_docs {
            let doc: TantivyDocument = searcher.doc(*addr)?;
            if let Some(v) = doc.get_first(work_id_field).and_then(|v| v.as_str()) {
                let key = v.to_string();
                if seen.insert(key.clone())
                    && let Ok(parsed) = key.parse::<livtet_types::DbId>()
                {
                    out.push(parsed);
                }
            }
        }
        Ok(out)
    }

    /// Return the deduplicated work IDs matching a pre-built query
    /// (with `kind=edition` filter already baked in). Used by the
    /// OPDS server when format/language labels have already been
    /// resolved and the query built via [`WorkFiltersQuery`].
    ///
    /// The result is capped at [`OPDS_WORK_ID_LIMIT`] entries.
    #[tracing::instrument(
        level = "debug",
        name = "search.matching_work_ids_from_query",
        skip(self, query)
    )]
    pub async fn matching_work_ids_from_query(
        &self,
        query: &dyn Query,
    ) -> tantivy::Result<Vec<livtet_types::DbId>> {
        let searcher = self.reader.searcher();
        let kind_filter = self.schema.get_field(fields::KIND).expect("kind");
        let edition_query: Box<dyn Query> = Box::new(tantivy::query::BooleanQuery::new(vec![
            (Occur::Must, query.box_clone() as Box<dyn Query>),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(kind_filter, kinds::EDITION),
                    IndexRecordOption::Basic,
                )),
            ),
        ]));
        let top_docs = searcher.search(
            &*edition_query,
            &TopDocs::with_limit(OPDS_WORK_ID_LIMIT).order_by_score(),
        )?;
        let work_id_field = self.schema.get_field(fields::WORK_ID).expect("work_id");
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out: Vec<livtet_types::DbId> = Vec::new();
        for (_score, addr) in &top_docs {
            let doc: TantivyDocument = searcher.doc(*addr)?;
            if let Some(v) = doc.get_first(work_id_field).and_then(|v| v.as_str()) {
                let key = v.to_string();
                if seen.insert(key.clone())
                    && let Ok(parsed) = key.parse::<livtet_types::DbId>()
                {
                    out.push(parsed);
                }
            }
        }
        Ok(out)
    }
}

/// Search result bundle that includes facet counts. Returned by
/// [`SearchIndex::search_with_facets`].
#[derive(Debug, Clone)]
pub struct FacetedSearchResult {
    pub hits: Vec<SearchHit>,
    pub language_facets: Vec<FacetCount>,
    pub publisher_facets: Vec<FacetCount>,
    pub subject_facets: Vec<FacetCount>,
    pub genre_facets: Vec<FacetCount>,
    pub recently_added: usize,
}

/// One row of a facet count.
#[derive(Debug, Clone)]
pub struct FacetCount {
    pub label: String,
    pub count: usize,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Sort key extracted from a Tantivy document by
/// [`sort_top_docs_by_spec`]. Two variants cover the four supported
/// [`livtet_types::SortField`] values; `Score` is handled as a
/// no-op short-circuit and so has no key.
#[derive(Clone, Debug)]
enum SortKey {
    Title(String),
    Date(tantivy::DateTime),
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (SortKey::Title(a), SortKey::Title(b)) => a.cmp(b),
            (SortKey::Date(a), SortKey::Date(b)) => a.cmp(b),
            // Defensive: should never happen — all docs in one call
            // share the same `SortField`.
            (SortKey::Title(_), SortKey::Date(_)) => std::cmp::Ordering::Equal,
            (SortKey::Date(_), SortKey::Title(_)) => std::cmp::Ordering::Equal,
        }
    }
}
impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for SortKey {
    fn eq(&self, other: &Self) -> bool {
        matches!(self.cmp(other), std::cmp::Ordering::Equal)
    }
}
impl Eq for SortKey {}

/// Post-sort a Tantivy `TopDocs` slice by the requested sort spec.
///
/// Tantivy's `TopDocs::order_by_fast_field` does not support a text
/// fast field directly (its `FastValue` bound is `u64`/`i64`/`f64`/
/// `DateTime`/`IpAddr`), so we always collect a score-ordered slice
/// and rewrite the ordering here. Date fields are read via tantivy's
/// stored accessor (those columns are `INDEXED | STORED | FAST`); the
/// fast-column iteration would be a future optimisation but doesn't
/// change the wire shape.
///
/// Returns a new `Vec` ordered according to `spec.direction`, with
/// insertion-order tie-breaking for `sort_by`.
fn sort_top_docs_by_spec(
    searcher: &tantivy::Searcher,
    schema: &tantivy::schema::Schema,
    top_docs: Vec<(f32, tantivy::DocAddress)>,
    spec: &livtet_types::SortSpec,
) -> tantivy::Result<Vec<(f32, tantivy::DocAddress)>> {
    use livtet_types::{SortDirection, SortField};

    // `Score` is the natural order of the input slice; skip the
    // round-trip through `searcher.doc`.
    if matches!(spec.field, SortField::Score) {
        return Ok(top_docs);
    }
    let field_name = match spec.field {
        SortField::Title => fields::TITLE_SORT,
        SortField::CreatedAt => fields::CREATED_AT,
        SortField::UpdatedAt => fields::UPDATED_AT,
        SortField::Score => unreachable!("handled above"),
    };
    let field = schema
        .get_field(field_name)
        .expect("sort field must exist in schema");
    let mut indexed: Vec<(usize, SortKey)> = Vec::with_capacity(top_docs.len());
    for (idx, (_score, addr)) in top_docs.iter().enumerate() {
        let doc: TantivyDocument = searcher.doc(*addr)?;
        let key = match spec.field {
            SortField::Title => SortKey::Title(
                doc.get_first(field)
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_default(),
            ),
            SortField::CreatedAt | SortField::UpdatedAt => SortKey::Date(
                doc.get_first(field)
                    .and_then(|v| v.as_datetime())
                    .unwrap_or(tantivy::DateTime::MIN),
            ),
            SortField::Score => unreachable!(),
        };
        indexed.push((idx, key));
    }
    // Sort by key in the requested direction, then by original
    // index to keep ties stable.
    indexed.sort_by(|a, b| match spec.direction {
        SortDirection::Asc => a.1.cmp(&b.1).then(a.0.cmp(&b.0)),
        SortDirection::Desc => b.1.cmp(&a.1).then(a.0.cmp(&b.0)),
    });
    let mut out: Vec<(f32, tantivy::DocAddress)> = Vec::with_capacity(top_docs.len());
    for (idx, _) in indexed {
        out.push(top_docs[idx]);
    }
    Ok(out)
}

fn collapse_editions_to_works(hits: Vec<SearchHit>, limit: usize) -> Vec<SearchHit> {
    let mut grouped: HashMap<String, SearchHit> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for h in hits {
        let key = h.work_id.clone();
        match grouped.get_mut(&key) {
            Some(existing) => {
                if let Some(eid) = &h.edition_id {
                    existing.grouped_edition_ids.push(eid.clone());
                }
                if existing.score < h.score {
                    existing.score = h.score;
                }
            }
            None => {
                let mut h = h;
                if let Some(eid) = &h.edition_id {
                    h.grouped_edition_ids.push(eid.clone());
                }
                order.push(key.clone());
                grouped.insert(key, h);
            }
        }
    }
    // Preserve the original score ordering: emit in the order the
    // first edition of each work was seen, then truncate.
    let mut out = Vec::with_capacity(limit);
    for key in order {
        if let Some(h) = grouped.remove(&key) {
            out.push(h);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

fn facet_counts(counts: &tantivy::collector::FacetCounts) -> Vec<FacetCount> {
    // `top_k` needs both the root facet prefix and a `k` cap. The
    // empty `""` prefix drills down through every facet under the
    // collector root, which is exactly what we want here. We then
    // take the first 20 entries to keep the wire payload bounded.
    counts
        .top_k("/", 20)
        .into_iter()
        .map(|(facet, count)| FacetCount {
            label: facet.to_string(),
            count: count as usize,
        })
        .collect()
}

/// Stable 64-bit hash of a work's ULID string. Used by
/// [`SearchIndex::search_works`] to group raw edition hits onto a
/// work. `std::hash::Hasher` would be overkill — we just want a
/// uniform 64-bit value.
fn hash_work_id(work_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    work_id.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------------
// Lookup traits
// ---------------------------------------------------------------------------

/// Lookup an individual work or a batch of works by id.
#[async_trait::async_trait]
pub trait WorkLookup: Send + Sync {
    async fn find(
        &self,
        conn: &DatabaseConnection,
        id: livtet_types::DbId,
    ) -> Result<Option<livtet_data::entities::works::Model>, livtet_data::orm::DbErr>;

    async fn find_many(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<Vec<livtet_data::entities::works::Model>, livtet_data::orm::DbErr>;
}

/// Lookup an individual edition or a batch by id, plus the ISBN
/// batch helper used at hit-build time.
#[async_trait::async_trait]
pub trait EditionLookup: Send + Sync {
    async fn find(
        &self,
        conn: &DatabaseConnection,
        id: livtet_types::DbId,
    ) -> Result<Option<livtet_data::entities::editions::Model>, livtet_data::orm::DbErr>;

    async fn find_many(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<Vec<livtet_data::entities::editions::Model>, livtet_data::orm::DbErr>;

    /// Resolve ISBNs for a batch of editions by joining
    /// `edition_identifiers` → `identifiers` where `kind = 'isbn'`.
    /// Each ISBN value is canonicalised to ISBN-13 via
    /// [`livtet_types::Isbn::parse`]; rows that fail to parse are
    /// kept verbatim so they still surface in the result.
    async fn get_edition_isbns(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<HashMap<livtet_types::DbId, Vec<String>>, livtet_data::orm::DbErr>;
}

/// Lookup an individual author or a batch by id.
#[async_trait::async_trait]
pub trait AuthorLookup: Send + Sync {
    async fn find(
        &self,
        conn: &DatabaseConnection,
        id: livtet_types::DbId,
    ) -> Result<Option<livtet_data::entities::authors::Model>, livtet_data::orm::DbErr>;

    async fn find_many(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<Vec<livtet_data::entities::authors::Model>, livtet_data::orm::DbErr>;
}

/// One categorical axis the saved-search engine can target. Each
/// variant maps to exactly one SeaORM entity table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Author,
    Genre,
    Subject,
    Series,
    Publisher,
    Tag,
}

impl ResourceKind {
    /// The lowercase string the kind is rendered as on the wire.
    /// Matches the identifier column values stored next to each
    /// `kind = "..."` discriminator.
    pub fn as_str(&self) -> &'static str {
        match self {
            ResourceKind::Author => "author",
            ResourceKind::Genre => "genre",
            ResourceKind::Subject => "subject",
            ResourceKind::Series => "series",
            ResourceKind::Publisher => "publisher",
            ResourceKind::Tag => "tag",
        }
    }
}

/// Per-axis existence and name lookup. The SeaORM implementation
/// issues one typed `Entity::find().filter(Column::Id.is_in(...))`
/// query per call — six known tables, no union scans.
#[async_trait::async_trait]
pub trait ResourceLookup: Send + Sync {
    /// Does the given id exist in this axis?
    async fn exists(
        &self,
        conn: &DatabaseConnection,
        kind: ResourceKind,
        id: livtet_types::DbId,
    ) -> Result<bool, livtet_data::orm::DbErr>;

    /// Resolve a batch of ids under a single axis to their display
    /// names. Missing ids are omitted from the result.
    async fn names(
        &self,
        conn: &DatabaseConnection,
        kind: ResourceKind,
        ids: &[livtet_types::DbId],
    ) -> Result<HashMap<livtet_types::DbId, String>, livtet_data::orm::DbErr>;
}

// ---------------------------------------------------------------------------
// Default traits / impls for unit tests
// ---------------------------------------------------------------------------

// `Default` no-op impls so test fixtures can build a `SearchIndex`
// against an in-memory directory without needing to stub out
// `DatabaseConnection`. The async-trait-free plain trait shapes
// above let tests use closures directly; we expose concrete fn
// pointers only where the planner explicitly needs them.

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

pub mod doc;
pub mod label_resolver;
pub mod sea_orm_resource_lookup;
pub mod user_input_translator;

pub use doc::{AuthorDoc, EditionDoc};
pub use label_resolver::LabelResolver;
pub use tantivy::query::{AllQuery, BooleanQuery, Query, TermQuery};
pub use user_input_translator::user_input_ast_to_query;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit(work_id: &str, edition_id: Option<&str>, score: f32) -> SearchHit {
        SearchHit {
            kind: HitKind::Edition,
            edition_id: edition_id.map(String::from),
            work_id: work_id.to_string(),
            author_id: None,
            title: format!("title-{work_id}"),
            work_title: None,
            edition_title: None,
            authors: Vec::new(),
            isbn: None,
            format: None,
            language: None,
            published_date: None,
            score,
            explanation: None,
            snippet_text: None,
            snippet_highlighted: Vec::new(),
            grouped_edition_ids: Vec::new(),
            source: "catalog".to_string(),
        }
    }

    #[test]
    fn collapse_editions_to_works_empty_input() {
        assert!(collapse_editions_to_works(Vec::new(), 10).is_empty());
    }

    #[test]
    fn collapse_editions_to_works_single_work() {
        let hits = vec![
            make_hit("w1", Some("e1"), 1.0),
            make_hit("w1", Some("e2"), 0.8),
        ];
        let collapsed = collapse_editions_to_works(hits, 10);
        assert_eq!(collapsed.len(), 1);
        let grouped = &collapsed[0].grouped_edition_ids;
        assert!(grouped.contains(&"e1".to_string()));
        assert!(grouped.contains(&"e2".to_string()));
    }

    #[test]
    fn collapse_editions_to_works_respects_limit() {
        let hits: Vec<SearchHit> = (0..5)
            .map(|i| make_hit(&format!("w{i}"), Some("e1"), 1.0))
            .collect();
        let collapsed = collapse_editions_to_works(hits, 3);
        assert_eq!(collapsed.len(), 3);
    }

    #[test]
    fn collapse_editions_to_works_preserves_highest_score() {
        let hits = vec![
            make_hit("w1", Some("e1"), 0.5),
            make_hit("w1", Some("e2"), 2.0),
            make_hit("w1", Some("e3"), 1.0),
        ];
        let collapsed = collapse_editions_to_works(hits, 10);
        assert_eq!(collapsed.len(), 1);
        assert!(collapsed[0].score >= 2.0 - f32::EPSILON);
    }

    #[test]
    fn collapse_editions_to_works_multiple_works() {
        let hits = vec![
            make_hit("w1", Some("e1"), 1.0),
            make_hit("w2", Some("e2"), 0.5),
            make_hit("w1", Some("e3"), 0.7),
        ];
        let collapsed = collapse_editions_to_works(hits, 10);
        assert_eq!(collapsed.len(), 2);
        // 1st seen work first, with both editions grouped; 2nd work second.
        assert_eq!(collapsed[0].work_id, "w1");
        assert_eq!(collapsed[0].grouped_edition_ids.len(), 2);
        assert_eq!(collapsed[1].work_id, "w2");
    }

    #[test]
    fn hash_work_id_is_deterministic() {
        assert_eq!(hash_work_id("abc"), hash_work_id("abc"));
    }

    #[test]
    fn hash_work_id_differs_per_input() {
        assert_ne!(hash_work_id("work-a"), hash_work_id("work-b"));
    }

    #[test]
    fn search_options_default_values() {
        let opts = SearchOptions::default();
        assert!(!opts.explain);
        assert!(opts.with_snippet);
        assert!(opts.snippet_chars > 0);
        assert!(!opts.collapse_to_works);
    }

    #[test]
    fn hit_kind_serde_round_trip() {
        for kind in [HitKind::Edition, HitKind::Work, HitKind::Person] {
            let json = serde_json::to_string(&kind).expect("ser");
            let back: HitKind = serde_json::from_str(&json).expect("de");
            assert_eq!(back, kind);
        }
    }
}
