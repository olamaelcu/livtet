//! Query building and read paths: free-text search, filtered counts,
//! work-ID enumeration, faceted search, and hit building.

use std::collections::HashMap;

use tantivy::{
    Index, Order, TantivyDocument, Term,
    collector::{Count, FacetCollector, TopDocs},
    query::{AllQuery, Occur, Query, QueryParser, TermQuery, TermSetQuery},
    schema::*,
    snippet::SnippetGenerator,
};

use crate::{
    index::SearchIndex,
    model::{FacetCount, FacetedSearchResult, HighlightRange, HitKind, SearchHit, SearchOptions},
    schema::{OPDS_WORK_ID_LIMIT, fields, kinds},
};

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

impl SearchIndex {
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
        let effective_limit = limit.saturating_add(opts.offset as usize);
        let base_limit = if opts.collapse_to_works {
            // Over-fetch so the per-work collapse has enough raw
            // hits to cover the limit even when many editions of
            // the same work are present.
            effective_limit.saturating_mul(opts.work_overfetch.max(1) as usize)
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
        let offset = opts.offset.max(0) as usize;
        if offset > 0 && offset < top_docs.len() {
            top_docs.drain(..offset);
        } else if offset >= top_docs.len() {
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
        let effective_limit = limit.saturating_add(opts.offset as usize);
        let fetch_limit = if opts.collapse_to_works {
            effective_limit.saturating_mul(opts.work_overfetch.max(1) as usize)
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
        let offset = opts.offset.max(0) as usize;
        if offset > 0 && offset < top_docs.len() {
            top_docs.drain(..offset);
        } else if offset >= top_docs.len() {
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
        let effective_limit = limit.saturating_add(opts.offset as usize);
        let fetch_limit = if opts.collapse_to_works {
            effective_limit.saturating_mul(opts.work_overfetch.max(1) as usize)
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
        let offset = opts.offset.max(0) as usize;
        if offset > 0 && offset < top_docs.len() {
            top_docs.drain(..offset);
        } else if offset >= top_docs.len() {
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
            recently_added: recent.len() as i64,
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
        let has_file_field = self.schema.get_field(fields::HAS_FILE).expect("has_file");

        let snippet_field = self
            .schema
            .get_field(fields::EDITION_DESCRIPTION)
            .expect("edition_description");

        let snippet_generator = if opts.with_snippet {
            match SnippetGenerator::create(searcher, query, snippet_field) {
                Ok(mut g) => {
                    g.set_max_num_chars(opts.snippet_chars.max(0) as usize);
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

            let has_file = doc
                .get_first(has_file_field)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

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
                    // bytes. Map to `HighlightRange` for IPC compatibility.
                    let ranges: Vec<HighlightRange> = snippet.highlighted().iter()
                        .map(|r| HighlightRange { start: r.start as u32, end: r.end as u32 })
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
                has_file,
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
            count: count as i64,
        })
        .collect()
}

/// Stable 64-bit hash of a work's ULID string. Used by
/// [`SearchIndex::search_works`] to group raw edition hits onto a
/// work. `std::hash::Hasher` would be overkill — we just want a
/// uniform 64-bit value.
pub(crate) fn hash_work_id(work_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    work_id.hash(&mut h);
    h.finish()
}

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
            has_file: false,
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
}
