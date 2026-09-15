//! Tantivy schema: field names, kind discriminator, built schema.
//!
//! The schema is declared once in [`build_schema`] and reused for
//! every index on disk. Reindexing always wipes and rebuilds (a
//! schema change is not safe to do incrementally on Tantivy).

use tantivy::schema::*;

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

    /// Whether the edition has a row in `digital_inventory`
    /// (i.e. there is a file on disk). Indexed as a bool.
    pub const HAS_FILE: &str = "has_file";

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
pub const SCHEMA_VERSION: u32 = 3;

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

    b.add_bool_field(fields::HAS_FILE, STORED | FAST);

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
