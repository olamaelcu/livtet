//! Hit and result types returned by the search APIs.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::schema::{DEFAULT_SNIPPET_CHARS, WORK_GROUP_OVERFETCH};

// ---------------------------------------------------------------------------
// SearchHit / HitKind
// ---------------------------------------------------------------------------

/// What kind of document a [`SearchHit`] came from.
///
/// Serialised as `"edition"`, `"work"`, or `"person"` (snake_case)
/// so the specta-generated TS type stays narrow.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum HitKind {
    Edition,
    Work,
    Person,
}

/// A `[start, end)` byte range into `snippet_text` that should be
/// rendered highlighted. Replaces the previous `[u32; 2]` pair so the
/// type crosses the UniFFI boundary with named fields.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct HighlightRange {
    pub start: u32,
    pub end: u32,
}

impl From<[u32; 2]> for HighlightRange {
    fn from([start, end]: [u32; 2]) -> Self {
        Self { start, end }
    }
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
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
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
    pub snippet_highlighted: Vec<HighlightRange>,

    /// When `HitKind::Work`, the edition IDs collapsed into this
    /// work hit. Empty for edition/person hits.
    pub grouped_edition_ids: Vec<String>,

    /// Provenance of this hit ("catalog" for site-owned rows,
    /// plugin id prefix for imported data). See the `source`
    /// field documentation.
    pub source: String,

    /// Whether this edition has a row in `digital_inventory`
    /// (i.e. there is a file on disk).
    pub has_file: bool,
}

// ---------------------------------------------------------------------------
// SearchOptions
// ---------------------------------------------------------------------------

/// Options that control one search call.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
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
    pub snippet_chars: i64,
    /// When true, edition hits are collapsed onto works. Equivalent
    /// to calling [`SearchReader::search_works`](crate::SearchReader::search_works).
    pub collapse_to_works: bool,
    /// Over-fetch multiplier for the work-collapse path. The default
    /// is 8 (`WORK_GROUP_OVERFETCH`).
    pub work_overfetch: i64,
    /// Optional explicit sort. When `Some`,
    /// [`SearchReader::search_with_options`](crate::SearchReader::search_with_options) sorts the top-N result
    /// by the corresponding field (`Title` / `CreatedAt` /
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
    /// relevant field. Title sorting uses the stored `title`
    /// (lowercased), which matches the indexer's `title_sort` for
    /// reindexed documents. The over-fetch is bumped to
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
    pub offset: i64,
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
            snippet_chars: DEFAULT_SNIPPET_CHARS as i64,
            collapse_to_works: false,
            work_overfetch: WORK_GROUP_OVERFETCH as i64,
            sort: None,
            offset: 0,
            source_filter: None,
        }
    }
}

/// Search result bundle that includes facet counts. Returned by
/// [`SearchReader::search_with_facets`](crate::SearchReader::search_with_facets).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FacetedSearchResult {
    pub hits: Vec<SearchHit>,
    pub language_facets: Vec<FacetCount>,
    pub publisher_facets: Vec<FacetCount>,
    pub subject_facets: Vec<FacetCount>,
    pub genre_facets: Vec<FacetCount>,
    pub recently_added: i64,
}

/// One row of a facet count.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FacetCount {
    pub label: String,
    pub count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

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
