//! Integration tests for `livtet-search`.
//!
//! These tests exercise the public surface of the crate end-to-end:
//!
//! - [`build_schema`] returns a valid Tantivy [`Schema`] containing
//!   every field the design plan mandates.
//! - A `SearchIndex::reindex` against an in-memory SQLite DB
//!   populated with a work, two editions, authors, an identifier
//!   and an edition_identifier link produces hits that the
//!   [`SearchIndex::search`] / [`SearchIndex::search_works`]
//!   APIs can retrieve.
//! - [`SeaOrmEditionLookup::get_edition_isbns`] canonicalises the
//!   stored ISBN-13.
//! - Snippet generation returns plain text with at least one
//!   highlight byte range.
//! - The Tantivy explanation tree serialises to JSON via
//!   `to_pretty_json()`.
//!
//! The fixtures follow the same pattern as
//! `crates/livtet-core/src/crud/edition_info_test.rs`: connect
//! to an in-memory SQLite pool with `cache=private`, run the
//! `livtet-migration` Migrator, then seed the rows needed for
//! each scenario.

use camino_tempfile::Utf8TempDir as TempDir;
use livtet_data::orm::{ActiveModelTrait, DatabaseConnection, Set};
use livtet_data::sql::{AssertSqlSafe, sqlite::SqlitePoolOptions};
use livtet_data::{
    entities::{
        authors, edition_authors, edition_genres, edition_identifiers, edition_publishers,
        edition_subjects, edition_tags, editions, formats, genres, identifiers, languages,
        publishers, series, series_entries, subjects, tags, works,
    },
    migration::Migrator,
};
use livtet_search::{
    AuthorLookup, EditionLookup, HitKind, ResourceKind, ResourceLookup, SearchHit, SearchIndex,
    SearchOptions, WorkFiltersQuery, WorkLookup, build_schema,
    sea_orm_resource_lookup::{SeaOrmEditionLookup, SeaOrmResourceLookup},
};
use livtet_types::{DbId, WorkFilters};
use tantivy::schema::Value;
async fn fresh_db() -> DatabaseConnection {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:?cache=private")
        .await
        .expect("connect to in-memory sqlite");
    livtet_data::sql::query(AssertSqlSafe("PRAGMA foreign_keys=ON"))
        .execute(&pool)
        .await
        .expect("enable foreign keys");
    Migrator::run(&pool).await.expect("run migrations");
    livtet_data::orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(pool)
}

fn now_p() -> time::PrimitiveDateTime {
    let now = time::OffsetDateTime::now_utc();
    time::PrimitiveDateTime::new(now.date(), now.time())
}

/// Insert one work, two editions of it, one author linked to both
/// editions, and one ISBN identifier linked to the first edition.
/// Returns `(work_id, edition_a_id, edition_b_id, author_id,
/// isbn_id)`. The ISBN value is `"9780061120084"` — a valid ISBN-13
/// so `livtet_types::Isbn::parse` returns the canonical form
/// unchanged.
async fn seed_work_with_two_editions(db: &DatabaseConnection) -> (DbId, DbId, DbId, DbId, DbId) {
    let now = now_p();

    let work_id = DbId::new();
    let edition_a_id = DbId::new();
    let edition_b_id = DbId::new();
    let author_id = DbId::new();
    let isbn_id = DbId::new();

    // Work
    let work = works::ActiveModel {
        id: Set(work_id),
        title: Set("The Name of the Wind".into()),
        description: Set(Some(
            "A fantasy novel about a legendary figure named Kvothe.".into(),
        )),
        sort_title: Set(None),
        series_type: Set(None),
        language_id: Set(None),
        created_at: Set(now),
        updated_at: Set(None),
        preferred_edition_id: Set(None),
    };
    work.insert(db).await.expect("insert work");

    // Edition A (matching title)
    let ed_a = editions::ActiveModel {
        id: Set(edition_a_id),
        work_id: Set(work_id),
        group_id: Set(None),
        title: Set(Some("The Name of the Wind".into())),
        published_date: Set(None),
        format_id: Set(None),
        language_id: Set(None),
        notes: Set(None),
        description: Set(Some("The first edition of Rothfuss's debut novel.".into())),
        created_at: Set(now),
        updated_at: Set(None),
    };
    ed_a.insert(db).await.expect("insert edition a");

    // Edition B (matching title, secondary subtitle)
    let ed_b = editions::ActiveModel {
        id: Set(edition_b_id),
        work_id: Set(work_id),
        group_id: Set(None),
        title: Set(Some("The Name of the Wind (Special Edition)".into())),
        published_date: Set(None),
        format_id: Set(None),
        language_id: Set(None),
        notes: Set(None),
        description: Set(Some("Anniversary hardcover with extra commentary.".into())),
        created_at: Set(now),
        updated_at: Set(None),
    };
    ed_b.insert(db).await.expect("insert edition b");

    // Author
    let author = authors::ActiveModel {
        id: Set(author_id),
        name: Set("Patrick Rothfuss".into()),
    };
    author.insert(db).await.expect("insert author");

    // Link author to both editions
    for eid in [edition_a_id, edition_b_id] {
        let ea = edition_authors::ActiveModel {
            edition_id: Set(eid),
            author_id: Set(author_id),
            role: Set("author".into()),
        };
        ea.insert(db).await.expect("insert edition_authors");
    }

    // ISBN-13 identifier (valid checksum: 9780061120084)
    let ident = identifiers::ActiveModel {
        id: Set(isbn_id),
        value: Set("9780061120084".into()),
        kind: Set("isbn".into()),
    };
    ident.insert(db).await.expect("insert identifier");

    // Link to edition A only
    let ei = edition_identifiers::ActiveModel {
        edition_id: Set(edition_a_id),
        identifier_id: Set(isbn_id),
    };
    ei.insert(db).await.expect("insert edition_identifier");

    (work_id, edition_a_id, edition_b_id, author_id, isbn_id)
}

async fn fresh_index(db: &DatabaseConnection) -> (SearchIndex, TempDir) {
    let dir = camino_tempfile::tempdir().expect("tempdir");
    SearchIndex::migrate_to(dir.path(), db)
        .await
        .expect("migrate_to");
    let index = SearchIndex::open(dir.path()).expect("open index");
    (index, dir)
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

#[test]
fn build_schema_contains_every_field_from_the_design_plan() {
    let schema = build_schema();
    let expected = [
        "edition_id",
        "work_id",
        "work_id_hash",
        "author_id",
        "kind",
        "title",
        "edition_title",
        "work_description",
        "edition_description",
        "authors",
        "tags",
        "genres",
        "subjects",
        "publishers",
        "identifier_kinds",
        "identifier_values",
        "notes",
        "format",
        "language",
        "language_facet",
        "publisher_facet",
        "subject_facet",
        "genre_facet",
        "pub_date",
        "published_year",
        "title_sort",
        "primary_author_sort",
        "created_at",
        "popularity",
        "tag_id",
        "genre_id",
        "subject_id",
        "series_id",
        "publisher_id",
    ];
    for name in expected {
        assert!(
            schema.get_field(name).is_ok(),
            "schema is missing field {name}"
        );
    }
    // The plan explicitly forbids an `isbn` schema field.
    assert!(
        schema.get_field("isbn").is_err(),
        "schema must not declare an `isbn` field"
    );
}

// ---------------------------------------------------------------------------
// Title search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_returns_hits_for_title_match() {
    let db = fresh_db().await;
    let (work_id, edition_a_id, edition_b_id, _author_id, _isbn_id) =
        seed_work_with_two_editions(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let hits = index.search("wind", 10).await.expect("search");
    assert_eq!(hits.len(), 2, "both editions should match `wind`");
    for hit in &hits {
        assert_eq!(hit.work_id, work_id.to_string());
        assert_eq!(hit.kind, HitKind::Edition);
        let edition_id = hit.edition_id.clone().expect("edition id");
        assert!(
            edition_id == edition_a_id.to_string() || edition_id == edition_b_id.to_string(),
            "edition id {edition_id} should be one of the seeded ids"
        );
        // Title field should reflect the resolved title.
        assert!(hit.title.to_lowercase().contains("wind"));
    }
}

#[tokio::test]
async fn search_works_collapses_editions_into_works() {
    let db = fresh_db().await;
    let (work_id, edition_a_id, edition_b_id, _author_id, _isbn_id) =
        seed_work_with_two_editions(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let hits = index.search_works("wind", 10).await.expect("search_works");
    assert_eq!(hits.len(), 1, "two editions collapse to one work");
    let hit = &hits[0];
    assert_eq!(hit.work_id, work_id.to_string());
    assert_eq!(hit.kind, HitKind::Edition);
    let grouped = &hit.grouped_edition_ids;
    assert!(
        grouped.contains(&edition_a_id.to_string()),
        "grouped ids missing edition a; got {grouped:?}"
    );
    assert!(
        grouped.contains(&edition_b_id.to_string()),
        "grouped ids missing edition b; got {grouped:?}"
    );
}

// ---------------------------------------------------------------------------
// ISBN resolution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edition_lookup_resolves_canonical_isbn() {
    let db = fresh_db().await;
    let (_work_id, edition_a_id, _edition_b_id, _author_id, _isbn_id) =
        seed_work_with_two_editions(&db).await;
    let lookup = SeaOrmEditionLookup;
    let map = lookup
        .get_edition_isbns(&db, &[edition_a_id])
        .await
        .expect("get_edition_isbns");
    let isbns = map
        .get(&edition_a_id)
        .expect("edition a should have one isbn");
    assert_eq!(isbns.len(), 1);
    assert_eq!(isbns[0], "9780061120084");
}

// ---------------------------------------------------------------------------
// Snippet + explanation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn snippet_contains_marker_bytes_for_matching_query() {
    let db = fresh_db().await;
    let (_work_id, _edition_a_id, _edition_b_id, _author_id, _isbn_id) =
        seed_work_with_two_editions(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let opts = SearchOptions {
        with_snippet: true,
        snippet_chars: 180,
        explain: false,
        ..SearchOptions::default()
    };
    let hits = index
        .search_with_options("debut", 10, &opts)
        .await
        .expect("search_with_options");
    assert!(!hits.is_empty(), "should hit on the edition description");
    let hit = &hits[0];
    let snippet = hit
        .snippet_text
        .clone()
        .expect("snippet text should be populated");
    assert!(
        !snippet.is_empty(),
        "snippet text must not be empty for a matching hit"
    );
    // We just confirm the bytes are within bounds — frontend
    // rendering logic isn't in scope here.
    for range in &hit.snippet_highlighted {
        assert!(range[0] <= range[1]);
        assert!((range[1] as usize) <= snippet.len());
    }
}

#[tokio::test]
async fn search_options_explain_emits_pretty_json_for_each_hit() {
    let db = fresh_db().await;
    let (_work_id, _edition_a_id, _edition_b_id, _author_id, _isbn_id) =
        seed_work_with_two_editions(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let opts = SearchOptions {
        explain: true,
        ..SearchOptions::default()
    };
    let hits = index
        .search_with_options("wind", 10, &opts)
        .await
        .expect("search_with_options");
    assert!(!hits.is_empty());
    for hit in &hits {
        let explanation = hit
            .explanation
            .clone()
            .expect("explanation should be populated when explain=true");
        let parsed: serde_json::Value = serde_json::from_str(&explanation)
            .expect("explanation must be valid pretty-printed JSON");
        assert!(
            parsed.is_object(),
            "explanation JSON should be an object; got {parsed}"
        );
    }
}

// ---------------------------------------------------------------------------
// Resource lookup — per-axis
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resource_lookup_returns_true_for_existing_author_axis() {
    let db = fresh_db().await;
    let (_work_id, _edition_a_id, _edition_b_id, author_id, _isbn_id) =
        seed_work_with_two_editions(&db).await;

    let lookup = SeaOrmResourceLookup;
    assert!(
        lookup
            .exists(&db, livtet_search::ResourceKind::Author, author_id)
            .await
            .expect("exists author"),
        "seeded author should exist"
    );
    let missing_id = DbId::new();
    assert!(
        !lookup
            .exists(&db, livtet_search::ResourceKind::Author, missing_id)
            .await
            .expect("exists missing"),
        "random id should not exist"
    );
}

#[tokio::test]
async fn resource_lookup_names_returns_author_name() {
    let db = fresh_db().await;
    let (_work_id, _edition_a_id, _edition_b_id, author_id, _isbn_id) =
        seed_work_with_two_editions(&db).await;

    let lookup = SeaOrmResourceLookup;
    let names = lookup
        .names(&db, livtet_search::ResourceKind::Author, &[author_id])
        .await
        .expect("names");
    assert_eq!(
        names.get(&author_id).map(String::as_str),
        Some("Patrick Rothfuss")
    );
}

// ---------------------------------------------------------------------------
// Sanity: an empty DB indexes cleanly and returns zero hits.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_index_returns_no_hits() {
    let db = fresh_db().await;
    let (index, _dir) = fresh_index(&db).await;
    let hits: Vec<SearchHit> = index.search("anything", 10).await.expect("search");
    assert!(hits.is_empty());
}

// ===========================================================================
// Below: comprehensive seed + ~25 integration tests covering every public
// axis (filters, facets, person hits, snippets, delete, lookups, ISBN
// edge cases).
// ===========================================================================

/// Bundle of ids and labels produced by [`seed_comprehensive`]. Keeps the
/// signature tidy so tests can pattern-match `let s = seed(db).await;` and
/// pull out whichever axis they need.
#[allow(clippy::too_many_arguments)]
struct Seed {
    work_id: DbId,
    edition_a_id: DbId,
    edition_b_id: DbId,
    edition_c_id: DbId,
    author_id: DbId,
    author2_id: DbId,
    series_id: DbId,
    tag_id: DbId,
    genre_id: DbId,
    subject_id: DbId,
    publisher_id: DbId,
    publisher2_id: DbId,
}

/// Seed a richer fixture: two works (one with three editions), two
/// authors, one series entry, two tags, two genres (one shared),
/// two subjects, two publishers, two formats, two languages, two ISBN
/// identifiers (one canonical ISBN-13, one ISBN-10 that needs
/// canonicalisation, one malformed that's kept verbatim). Designed so
/// most filter tests can pick a single axis and verify that the
/// returned hits cover the right editions.
async fn seed_comprehensive(db: &DatabaseConnection) -> Seed {
    let now = now_p();

    let work_id = DbId::new();
    let work2_id = DbId::new();
    let edition_a_id = DbId::new();
    let edition_b_id = DbId::new();
    let edition_c_id = DbId::new();
    let author_id = DbId::new();
    let author2_id = DbId::new();
    let series_id = DbId::new();
    let tag_id = DbId::new();
    let tag2_id = DbId::new();
    let genre_id = DbId::new();
    let subject_id = DbId::new();
    let publisher_id = DbId::new();
    let publisher2_id = DbId::new();
    let format_id = DbId::new();
    let format2_id = DbId::new();
    let language_id = DbId::new();
    let language2_id = DbId::new();

    // ---- Vocabulary tables MUST be inserted before any editions/works
    // because editions has FKs to works/languages/formats, and edition_*
    // junctions FK to tags/genres/subjects/publishers. The classic FK
    // ordering problem.

    // Formats — lowercase names because reindex indexes them through
    // the `en_stem` tokenizer which lowercases on the way in. The
    // filter query uses `Term::from_field_text` which does NOT
    // tokenize, so the indexed and queried terms must be lowercase.
    for (fid, name) in [(format_id, "epub"), (format2_id, "hardcover")] {
        formats::ActiveModel {
            id: Set(fid),
            name: Set(name.into()),
            metadata_schema: Set(serde_json::Value::Null),
            progress_unit: Set(None),
        }
        .insert(db)
        .await
        .expect("insert format");
    }
    for (lid, name) in [(language_id, "english"), (language2_id, "spanish")] {
        languages::ActiveModel {
            id: Set(lid),
            name: Set(name.into()),
            code: Set(if name == "english" {
                "en".into()
            } else {
                "es".into()
            }),
            flag_emoji: Set(None),
            created_at: Set(now),
            updated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert language");
    }
    for (tid, name) in [(tag_id, "epic-fantasy"), (tag2_id, "debut")] {
        tags::ActiveModel {
            id: Set(tid),
            name: Set(name.into()),
            created_at: Set(now),
            updated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tag");
    }
    genres::ActiveModel {
        id: Set(genre_id),
        name: Set("fantasy".into()),
        created_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert genre");
    subjects::ActiveModel {
        id: Set(subject_id),
        name: Set("fiction".into()),
        created_at: Set(now),
        updated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert subject");
    for (pid, name) in [(publisher_id, "DAW Books"), (publisher2_id, "Gnome Press")] {
        publishers::ActiveModel {
            id: Set(pid),
            name: Set(name.into()),
            website: Set(None),
            logo_url: Set(None),
            created_at: Set(now),
            updated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert publisher");
    }
    series::ActiveModel {
        id: Set(series_id),
        name: Set("Kingkiller Chronicle".into()),
        sort_title: Set(None),
        series_type: Set(None),
        created_at: Set(now),
        updated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert series");
    for (aid, name) in [
        (author_id, "Patrick Rothfuss"),
        (author2_id, "Brandon Sanderson"),
    ] {
        authors::ActiveModel {
            id: Set(aid),
            name: Set(name.into()),
        }
        .insert(db)
        .await
        .expect("insert author");
    }

    // ---- Works (FKs to languages via language_id is null so no order
    // issue here)
    works::ActiveModel {
        id: Set(work_id),
        title: Set("The Name of the Wind".into()),
        description: Set(Some(
            "A fantasy novel about a legendary figure named Kvothe.".into(),
        )),
        sort_title: Set(None),
        series_type: Set(None),
        language_id: Set(None),
        created_at: Set(now),
        updated_at: Set(None),
        preferred_edition_id: Set(None),
    }
    .insert(db)
    .await
    .expect("insert work 1");

    works::ActiveModel {
        id: Set(work2_id),
        title: Set("The Wise Man's Fear".into()),
        description: Set(Some("Second book in the Kingkiller Chronicle.".into())),
        sort_title: Set(None),
        series_type: Set(None),
        language_id: Set(None),
        created_at: Set(now),
        updated_at: Set(None),
        preferred_edition_id: Set(None),
    }
    .insert(db)
    .await
    .expect("insert work 2");

    // Editions — formats/languages must already exist.
    for (eid, title, desc, format, language, year) in [
        (
            edition_a_id,
            "The Name of the Wind",
            "A towering hardcover debut with deckled edges.",
            format_id,
            language_id,
            2007i32,
        ),
        (
            edition_b_id,
            "The Name of the Wind (Special Edition)",
            "Tenth anniversary hardcover with bonus commentary and gilded pages.",
            format2_id,
            language2_id,
            2014i32,
        ),
        (
            edition_c_id,
            "The Name of the Wind",
            "Mass-market paperback reprint priced for the classroom.",
            format_id,
            language_id,
            2012i32,
        ),
    ] {
        editions::ActiveModel {
            id: Set(eid),
            work_id: Set(work_id),
            group_id: Set(None),
            title: Set(Some(title.into())),
            published_date: Set(Some(
                time::Date::from_calendar_date(year, time::Month::October, 1).unwrap(),
            )),
            format_id: Set(Some(format)),
            language_id: Set(Some(language)),
            notes: Set(None),
            description: Set(Some(desc.into())),
            created_at: Set(now),
            updated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert edition");
    }

    // Work 2 has just one edition (so work-collapse tests can tell it apart).
    let edition_d_id = DbId::new();
    editions::ActiveModel {
        id: Set(edition_d_id),
        work_id: Set(work2_id),
        group_id: Set(None),
        title: Set(Some("The Wise Man's Fear".into())),
        published_date: Set(None),
        format_id: Set(Some(format_id)),
        language_id: Set(Some(language_id)),
        notes: Set(None),
        description: Set(Some(
            "Sequel novel chronicling Kvothe's arrival at the Maer's court.".into(),
        )),
        created_at: Set(now),
        updated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert edition d");

    // Edition ↔ author: rothfuss on every edition, sanderson on edition_c only.
    for eid in [edition_a_id, edition_b_id, edition_c_id, edition_d_id] {
        edition_authors::ActiveModel {
            edition_id: Set(eid),
            author_id: Set(author_id),
            role: Set("author".into()),
        }
        .insert(db)
        .await
        .expect("link rothfuss");
    }
    edition_authors::ActiveModel {
        edition_id: Set(edition_c_id),
        author_id: Set(author2_id),
        role: Set("author".into()),
    }
    .insert(db)
    .await
    .expect("link sanderson on c");

    // Series entry — edition_a only.
    series_entries::ActiveModel {
        series_id: Set(series_id),
        edition_id: Set(edition_a_id),
        position: Set(1),
        created_at: Set(now),
    }
    .insert(db)
    .await
    .ok();

    // tag1 on editions a, b, c; tag2 on edition a only.
    for eid in [edition_a_id, edition_b_id, edition_c_id] {
        edition_tags::ActiveModel {
            edition_id: Set(eid),
            tag_id: Set(tag_id),
        }
        .insert(db)
        .await
        .expect("link tag");
    }
    edition_tags::ActiveModel {
        edition_id: Set(edition_a_id),
        tag_id: Set(tag2_id),
    }
    .insert(db)
    .await
    .expect("link debut tag");

    // Genre (fantasy) on every work1 edition.
    for eid in [edition_a_id, edition_b_id, edition_c_id] {
        edition_genres::ActiveModel {
            edition_id: Set(eid),
            genre_id: Set(genre_id),
        }
        .insert(db)
        .await
        .expect("link genre");
    }

    // Subject (fiction) on editions a, b.
    for eid in [edition_a_id, edition_b_id] {
        edition_subjects::ActiveModel {
            edition_id: Set(eid),
            subject_id: Set(subject_id),
        }
        .insert(db)
        .await
        .expect("link subject");
    }

    // Publishers — DAW Books on all 3 work1 editions, Gnome Press on a/c only.
    for eid in [edition_a_id, edition_b_id, edition_c_id] {
        edition_publishers::ActiveModel {
            edition_id: Set(eid),
            publisher_id: Set(publisher_id),
        }
        .insert(db)
        .await
        .expect("link DAW");
    }
    for eid in [edition_a_id, edition_c_id] {
        edition_publishers::ActiveModel {
            edition_id: Set(eid),
            publisher_id: Set(publisher2_id),
        }
        .insert(db)
        .await
        .expect("link Gnome");
    }

    // Identifiers
    let isbn_canon_id = DbId::new();
    identifiers::ActiveModel {
        id: Set(isbn_canon_id),
        value: Set("9780061120084".into()),
        kind: Set("isbn".into()),
    }
    .insert(db)
    .await
    .expect("insert isbn canon");
    edition_identifiers::ActiveModel {
        edition_id: Set(edition_a_id),
        identifier_id: Set(isbn_canon_id),
    }
    .insert(db)
    .await
    .expect("link isbn canon");

    let isbn10_id = DbId::new();
    identifiers::ActiveModel {
        id: Set(isbn10_id),
        value: Set("020161622X".into()),
        kind: Set("isbn".into()),
    }
    .insert(db)
    .await
    .expect("insert isbn10");
    edition_identifiers::ActiveModel {
        edition_id: Set(edition_b_id),
        identifier_id: Set(isbn10_id),
    }
    .insert(db)
    .await
    .expect("link isbn10");

    // Malformed ISBN — kept verbatim by `get_edition_isbns`.
    let isbn_bad_id = DbId::new();
    identifiers::ActiveModel {
        id: Set(isbn_bad_id),
        value: Set("not-a-real-isbn".into()),
        kind: Set("isbn".into()),
    }
    .insert(db)
    .await
    .expect("insert isbn bad");
    edition_identifiers::ActiveModel {
        edition_id: Set(edition_c_id),
        identifier_id: Set(isbn_bad_id),
    }
    .insert(db)
    .await
    .expect("link isbn bad");

    Seed {
        work_id,
        edition_a_id,
        edition_b_id,
        edition_c_id,
        author_id,
        author2_id,
        series_id,
        tag_id,
        genre_id,
        subject_id,
        publisher_id,
        publisher2_id,
    }
}

// ---------------------------------------------------------------------------
// WorkFiltersQuery — per-axis filters.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn build_query_with_format_label_filter() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let resolved = livtet_search::WorkFiltersResolved::from_filters(
        WorkFilters::default(),
        vec!["epub".into()],
        Vec::new(),
    );
    let q = WorkFiltersQuery::new(resolved, String::new());
    let query = q
        .build_query(index.index())
        .expect("build_query should accept format-only filter");

    let searcher = index.index().reader().expect("reader").searcher();
    let mut top = tantivy::collector::TopDocs::with_limit(20).order_by_score();
    let docs = searcher.search(&*query, &mut top).expect("search");
    assert!(!docs.is_empty(), "format filter must return hits");
    let mut edition_ids: Vec<String> = Vec::new();
    for (_, addr) in &docs {
        let doc: tantivy::TantivyDocument = searcher.doc(*addr).expect("retrieve doc");
        if let Some(v) = doc.get_first(index.schema().get_field("edition_id").unwrap()) {
            if let Some(s) = v.as_str() {
                edition_ids.push(s.to_string());
            }
        }
    }
    let a = seed.edition_a_id.to_string();
    let b = seed.edition_b_id.to_string();
    let c = seed.edition_c_id.to_string();
    assert!(edition_ids.contains(&a), "epub edition a missing");
    assert!(edition_ids.contains(&c), "epub edition c missing");
    assert!(
        !edition_ids.contains(&b),
        "hardcover edition b must not match epub filter"
    );
}

#[tokio::test]
async fn build_query_with_language_label_filter() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let resolved = livtet_search::WorkFiltersResolved::from_filters(
        WorkFilters::default(),
        Vec::new(),
        vec!["english".into()],
    );
    let q = WorkFiltersQuery::new(resolved, String::new());
    let query = q.build_query(index.index()).expect("build_query");

    let searcher = index.index().reader().expect("reader").searcher();
    let top = tantivy::collector::TopDocs::with_limit(20).order_by_score();
    let docs = searcher.search(&*query, &top).expect("search");
    let count = docs.len();
    assert!(count >= 2, "english editions > 1; got {count}");
}

#[tokio::test]
async fn build_query_with_tag_id_filter() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let filters = WorkFilters {
        tag_ids: vec![seed.tag_id],
        ..WorkFilters::default()
    };
    let q = WorkFiltersQuery::from_filters(filters, String::new());
    let query = q.build_query(index.index()).expect("build_query");
    let searcher = index.index().reader().expect("reader").searcher();
    let top = tantivy::collector::TopDocs::with_limit(20).order_by_score();
    let docs = searcher.search(&*query, &top).expect("search");
    assert_eq!(
        docs.len(),
        3,
        "tag1 is on editions a/b/c; should match exactly 3"
    );
}

#[tokio::test]
async fn build_query_with_author_id_filter() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    // Filter by author2 (Sanderson) — only edition_c has both authors.
    // Authorship is encoded as a tagged junction on edition, so the
    // author_id filter alone should narrow to editions linked to that
    // author, i.e. only edition_c.
    let filters = WorkFilters {
        author_ids: vec![seed.author2_id],
        ..WorkFilters::default()
    };
    let q = WorkFiltersQuery::from_filters(filters, String::new());
    let query = q.build_query(index.index()).expect("build_query");
    let searcher = index.index().reader().expect("reader").searcher();
    let top = tantivy::collector::TopDocs::with_limit(20).order_by_score();
    let docs = searcher.search(&*query, &top).expect("search");
    // edition_c and the separate author document both carry sanderson's
    // author_id — the kind filter on SearchIndex::search_with_query
    // would prune the author doc; the raw query here does not. Assert
    // on the edition row only.
    let edition_field = index.schema().get_field("edition_id").expect("edition_id");
    let mut edition_ids: Vec<String> = Vec::new();
    for (_, addr) in &docs {
        let d: tantivy::TantivyDocument = searcher.doc(*addr).expect("doc");
        if let Some(v) = d.get_first(edition_field)
            && let Some(s) = v.as_str()
        {
            edition_ids.push(s.to_string());
        }
    }
    let c = seed.edition_c_id.to_string();
    assert!(
        edition_ids.contains(&c),
        "edition_c must match sanderson filter; got {edition_ids:?}"
    );
}

#[tokio::test]
async fn build_query_with_publisher_id_filter() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let filters = WorkFilters {
        publisher_ids: vec![_seed.publisher2_id],
        ..WorkFilters::default()
    };
    let q = WorkFiltersQuery::from_filters(filters, String::new());
    let query = q.build_query(index.index()).expect("build_query");
    let searcher = index.index().reader().expect("reader").searcher();
    let top = tantivy::collector::TopDocs::with_limit(20).order_by_score();
    let docs = searcher.search(&*query, &top).expect("search");
    assert_eq!(docs.len(), 2, "Gnome Press is on a + c");
}

#[tokio::test]
async fn build_query_with_genre_id_filter() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let filters = WorkFilters {
        genre_ids: vec![_seed.genre_id],
        ..WorkFilters::default()
    };
    let q = WorkFiltersQuery::from_filters(filters, String::new());
    let query = q.build_query(index.index()).expect("build_query");
    let searcher = index.index().reader().expect("reader").searcher();
    let top = tantivy::collector::TopDocs::with_limit(20).order_by_score();
    let docs = searcher.search(&*query, &top).expect("search");
    assert_eq!(docs.len(), 3, "fantasy genre is on a/b/c");
}

#[tokio::test]
async fn build_query_with_subject_id_filter() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let filters = WorkFilters {
        subject_ids: vec![_seed.subject_id],
        ..WorkFilters::default()
    };
    let q = WorkFiltersQuery::from_filters(filters, String::new());
    let query = q.build_query(index.index()).expect("build_query");
    let searcher = index.index().reader().expect("reader").searcher();
    let top = tantivy::collector::TopDocs::with_limit(20).order_by_score();
    let docs = searcher.search(&*query, &top).expect("search");
    assert_eq!(docs.len(), 2, "subject fiction on a + b");
}

#[tokio::test]
async fn build_query_text_only_no_filters() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let q = WorkFiltersQuery::from_filters(WorkFilters::default(), "wind".into());
    let query = q.build_query(index.index()).expect("build_query");
    let searcher = index.index().reader().expect("reader").searcher();
    let top = tantivy::collector::TopDocs::with_limit(20).order_by_score();
    let docs = searcher.search(&*query, &top).expect("search");
    assert!(
        docs.len() >= 3,
        "text query 'wind' should match the 3 work1 editions"
    );
}

// ---------------------------------------------------------------------------
// search_with_query: pre-built Box<dyn Query> integration.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_with_query_accepts_pre_built_query() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let resolved = livtet_search::WorkFiltersResolved::from_filters(
        WorkFilters::default(),
        vec!["epub".into()],
        Vec::new(),
    );
    let q = WorkFiltersQuery::new(resolved, String::new());
    let query = q.build_query(index.index()).expect("build_query");

    let hits = index
        .search_with_query(query, 10, &SearchOptions::default())
        .await
        .expect("search_with_query");
    assert!(!hits.is_empty(), "search_with_query should return hits");
    for h in &hits {
        assert_eq!(h.kind, HitKind::Edition);
    }
}

// ---------------------------------------------------------------------------
// search_with_facets: language/publisher/subject/genre counts.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_with_facets_returns_facet_counts() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let result = index
        .search_with_facets("wind", 10)
        .await
        .expect("search_with_facets");
    // The 3 editions of "The Name of the Wind" all match; the
    // Title token "wind" also hits edition_d because the identifier
    // index / fuzzy on title can broaden the match. The structural
    // thing being asserted here is that the facets wire up — not the
    // exact hit count.
    assert!(
        result.hits.len() >= 3,
        "all 3 work1 editions match 'wind' (got {})",
        result.hits.len()
    );
    let lang = result
        .language_facets
        .iter()
        .find(|fc| fc.label == "/english")
        .expect("english facet must be present");
    assert!(lang.count >= 2, "english count >= 2; got {}", lang.count);
    let pub_facet = result
        .publisher_facets
        .iter()
        .find(|fc| fc.label == "/DAW Books")
        .or_else(|| {
            result
                .publisher_facets
                .iter()
                .find(|fc| fc.label == "/daw books")
        });
    assert!(
        pub_facet.is_some(),
        "publisher facet must list DAW Books (case from reindex); facets={:?}",
        result.publisher_facets
    );
}

// ---------------------------------------------------------------------------
// Person hits: author documents surface via search_all_kinds.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn person_hit_returned_for_author_document() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let hits = index
        .search_all_kinds("Rothfuss", 10, &SearchOptions::default())
        .await
        .expect("search_all_kinds");
    let person_hits: Vec<&SearchHit> = hits.iter().filter(|h| h.kind == HitKind::Person).collect();
    assert!(
        !person_hits.is_empty(),
        "should return at least one person hit for the author name"
    );
    let person = person_hits[0];
    assert_eq!(person.kind, HitKind::Person);
    assert_eq!(
        person.author_id,
        Some(seed.author_id.to_string()),
        "person hit must carry the author_id"
    );
    assert_eq!(person.edition_id, None);
    assert!(person.authors.is_empty());
}

// ---------------------------------------------------------------------------
// Snippet: matching query produces a highlight range.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn snippet_contains_marker_bytes_for_matching_query_explicit() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let opts = SearchOptions {
        with_snippet: true,
        snippet_chars: 240,
        ..SearchOptions::default()
    };
    // "deckled" only appears in edition_a's edition_description, so
    // this should produce exactly one snippet highlight there.
    let hits = index
        .search_with_options("deckled", 5, &opts)
        .await
        .expect("search");
    assert!(!hits.is_empty());
    let hit = &hits[0];
    let snippet = hit.snippet_text.clone().expect("snippet present");
    assert!(!snippet.is_empty());
    assert!(
        !hit.snippet_highlighted.is_empty(),
        "match against 'deckled' should produce at least one highlight range"
    );
}

// ---------------------------------------------------------------------------
// Delete by edition_id.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_edition_removes_it_from_search() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let pre = index.search("legendary", 10).await.expect("pre");
    let pre_ids: Vec<String> = pre.iter().filter_map(|h| h.edition_id.clone()).collect();
    assert!(pre_ids.contains(&seed.edition_a_id.to_string()));

    index
        .delete_edition(seed.edition_a_id)
        .await
        .expect("delete_edition");

    let post = index.search("legendary", 10).await.expect("post");
    let post_ids: Vec<String> = post.iter().filter_map(|h| h.edition_id.clone()).collect();
    assert!(
        !post_ids.contains(&seed.edition_a_id.to_string()),
        "edition_a must be gone from search hits"
    );
}

// ---------------------------------------------------------------------------
// Lookup traits — EditionLookup.find / find_many
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edition_lookup_find_returns_seeded_edition() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let lookup = SeaOrmEditionLookup;

    let found = lookup
        .find(&db, seed.edition_a_id)
        .await
        .expect("find edition_a");
    let model = found.expect("edition_a exists");
    assert_eq!(model.id, seed.edition_a_id);
    assert_eq!(model.title.as_deref(), Some("The Name of the Wind"));
}

#[tokio::test]
async fn edition_lookup_find_many_returns_seeded() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let lookup = SeaOrmEditionLookup;

    let ids = [seed.edition_a_id, seed.edition_b_id, seed.edition_c_id];
    let found = lookup.find_many(&db, &ids).await.expect("find_many");
    assert_eq!(found.len(), 3);
    let returned: Vec<DbId> = found.iter().map(|e| e.id).collect();
    for id in &ids {
        assert!(returned.contains(id), "missing edition {id}");
    }
}

#[tokio::test]
async fn work_lookup_find_returns_seeded_work() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let lookup = livtet_search::sea_orm_resource_lookup::SeaOrmWorkLookup;

    let found = lookup.find(&db, seed.work_id).await.expect("find work");
    let model = found.expect("work exists");
    assert_eq!(model.id, seed.work_id);
    assert_eq!(model.title, "The Name of the Wind");
}

#[tokio::test]
async fn author_lookup_find_returns_seeded_author() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let lookup = livtet_search::sea_orm_resource_lookup::SeaOrmAuthorLookup;

    let found = lookup.find(&db, seed.author_id).await.expect("find author");
    let author = found.expect("author exists");
    assert_eq!(author.name, "Patrick Rothfuss");
}

#[tokio::test]
async fn resource_lookup_exists_for_all_axes() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let lookup = SeaOrmResourceLookup;

    let cases = [
        (ResourceKind::Author, seed.author_id as DbId),
        (ResourceKind::Tag, seed.tag_id),
        (ResourceKind::Genre, seed.genre_id),
        (ResourceKind::Subject, seed.subject_id),
        (ResourceKind::Publisher, seed.publisher_id),
        (ResourceKind::Series, seed.series_id),
    ];
    for (kind, id) in cases {
        assert!(
            lookup.exists(&db, kind, id).await.expect("exists"),
            "{kind:?} id {id} should exist"
        );
        let missing = DbId::new();
        assert!(
            !lookup
                .exists(&db, kind, missing)
                .await
                .expect("exists missing"),
            "{kind:?} id {missing} should not exist"
        );
    }
}

#[tokio::test]
async fn resource_lookup_names_for_genre_axis() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let lookup = SeaOrmResourceLookup;

    let names = lookup
        .names(&db, ResourceKind::Genre, &[seed.genre_id])
        .await
        .expect("names");
    assert_eq!(
        names.get(&seed.genre_id).map(String::as_str),
        Some("fantasy")
    );
}

// ---------------------------------------------------------------------------
// ISBN edge cases: canonicalisation, ISBN-10 → ISBN-13, malformed input.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edition_isbns_canonicalise_isbn_10_to_isbn_13() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let lookup = SeaOrmEditionLookup;

    let map = lookup
        .get_edition_isbns(&db, &[seed.edition_b_id])
        .await
        .expect("get_edition_isbns");
    let isbns = map.get(&seed.edition_b_id).expect("ed b should have isbn");
    assert_eq!(isbns.len(), 1);
    // 020161622X → 9780201616224 (Programming Perl ISBN-10 with ISBN-13 prefix 978).
    assert_eq!(isbns[0], "9780201616224");
}

#[tokio::test]
async fn edition_isbns_keep_malformed_values_verbatim() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let lookup = SeaOrmEditionLookup;

    let map = lookup
        .get_edition_isbns(&db, &[seed.edition_c_id])
        .await
        .expect("get_edition_isbns");
    let isbns = map.get(&seed.edition_c_id).expect("ed c should have isbn");
    assert_eq!(isbns, &vec!["not-a-real-isbn".to_string()]);
}

#[tokio::test]
async fn edition_isbns_empty_for_no_identifiers() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    // Fresh work with no identifiers at all.
    let work_id = DbId::new();
    let edition_id = DbId::new();
    let now = now_p();
    works::ActiveModel {
        id: Set(work_id),
        title: Set("Tome Without ISBN".into()),
        description: Set(None),
        sort_title: Set(None),
        series_type: Set(None),
        language_id: Set(None),
        created_at: Set(now),
        updated_at: Set(None),
        preferred_edition_id: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert empty work");
    editions::ActiveModel {
        id: Set(edition_id),
        work_id: Set(work_id),
        group_id: Set(None),
        title: Set(None),
        published_date: Set(None),
        format_id: Set(None),
        language_id: Set(None),
        notes: Set(None),
        description: Set(None),
        created_at: Set(now),
        updated_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert empty edition");

    let lookup = SeaOrmEditionLookup;
    let map = lookup
        .get_edition_isbns(&db, &[edition_id])
        .await
        .expect("empty isbn");
    let entry = map.get(&edition_id);
    assert!(
        entry.is_none(),
        "edition with no identifiers should not appear in isbn map"
    );
}

#[tokio::test]
async fn edition_isbns_return_already_canonical_isbn_13() {
    let db = fresh_db().await;
    let seed = seed_comprehensive(&db).await;
    let lookup = SeaOrmEditionLookup;

    let map = lookup
        .get_edition_isbns(&db, &[seed.edition_a_id])
        .await
        .expect("get_edition_isbns");
    let isbns = map.get(&seed.edition_a_id).expect("ed a should have isbn");
    assert_eq!(isbns.len(), 1);
    assert_eq!(isbns[0], "9780061120084");
}

// ---------------------------------------------------------------------------
// SortSpec — `build_sort` should encode WorkSortBy into SortSpec correctly.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn build_sort_mirror_effective_limit_for_newest_cap() {
    let filters = WorkFilters {
        sort_by: Some(livtet_types::WorkSortBy::NewestCap),
        ..WorkFilters::default()
    };
    let q = WorkFiltersQuery::from_filters(filters, String::new());
    let spec = q.build_sort();
    assert_eq!(spec.field, livtet_types::SortField::CreatedAt);
    assert_eq!(spec.direction, livtet_types::SortDirection::Desc);
    assert_eq!(spec.limit, Some(100));
}

#[tokio::test]
async fn build_sort_title_asc_by_default() {
    let filters = WorkFilters {
        sort_by: Some(livtet_types::WorkSortBy::Title),
        sort_direction: Some(livtet_types::SortDirection::Asc),
        ..WorkFilters::default()
    };
    let q = WorkFiltersQuery::from_filters(filters, String::new());
    let spec = q.build_sort();
    assert_eq!(spec.field, livtet_types::SortField::Title);
    assert_eq!(spec.direction, livtet_types::SortDirection::Asc);
    assert_eq!(spec.limit, None);
}

// ---------------------------------------------------------------------------
// Schema smoke: ensure the comprehensive seed produces a fully populated
// index — every field with values should be present in some document.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reindex_populates_every_facet_axis() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    let result = index
        .search_with_facets("wind", 20)
        .await
        .expect("search_with_facets for wind");
    assert!(!result.hits.is_empty(), "seed should produce indexed hits");

    let lang_labels: Vec<&str> = result
        .language_facets
        .iter()
        .map(|fc| fc.label.as_str())
        .collect();
    assert!(
        lang_labels.contains(&"/english"),
        "english facet label present; got {lang_labels:?}"
    );

    let publisher_labels: Vec<&str> = result
        .publisher_facets
        .iter()
        .map(|fc| fc.label.as_str())
        .collect();
    assert!(
        publisher_labels.iter().any(|l| l.contains("DAW")),
        "publisher facet must include DAW Books; got {publisher_labels:?}"
    );
}

// ===========================================================================
// user_input_translator: lower a tantivy::query_grammar::UserInputAst
// (the format produced by `livtet_search_types::SavedSearches::render`)
// into a `Box<dyn Query>` and execute it against the index.
// ===========================================================================

/// Construct a `title:<phrase>` literal AST node.
fn ast_title_literal(phrase: &str) -> tantivy::query_grammar::UserInputAst {
    tantivy::query_grammar::UserInputAst::Leaf(Box::new(
        tantivy::query_grammar::UserInputLeaf::Literal(tantivy::query_grammar::UserInputLiteral {
            field_name: Some("title".into()),
            phrase: phrase.into(),
            delimiter: tantivy::query_grammar::Delimiter::None,
            slop: 0,
            prefix: false,
        }),
    ))
}

/// Construct an `edition_description:<phrase>` literal AST node.
fn ast_edition_description_literal(phrase: &str) -> tantivy::query_grammar::UserInputAst {
    tantivy::query_grammar::UserInputAst::Leaf(Box::new(
        tantivy::query_grammar::UserInputLeaf::Literal(tantivy::query_grammar::UserInputLiteral {
            field_name: Some("edition_description".into()),
            phrase: phrase.into(),
            delimiter: tantivy::query_grammar::Delimiter::None,
            slop: 0,
            prefix: false,
        }),
    ))
}

/// Run `query` against the index's reader and return the score-ordered
/// `(score, DocAddress)` slice the `TopDocs` collector emits.
async fn run_query(
    index: &SearchIndex,
    query: &dyn tantivy::query::Query,
) -> Vec<(f32, tantivy::DocAddress)> {
    let searcher = index.index().reader().expect("reader").searcher();
    let top = tantivy::collector::TopDocs::with_limit(50).order_by_score();
    searcher.search(query, &top).expect("search")
}

#[tokio::test]
async fn user_input_ast_simple_term_lowers_to_term_query() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    // Bare literal against the parser's default fields. The `wind`
    // token must match the title field of editions a/b/c and
    // therefore produce at least 3 hits.
    let ast = tantivy::query_grammar::UserInputAst::Leaf(Box::new(
        tantivy::query_grammar::UserInputLeaf::Literal(tantivy::query_grammar::UserInputLiteral {
            field_name: None,
            phrase: "wind".into(),
            delimiter: tantivy::query_grammar::Delimiter::None,
            slop: 0,
            prefix: false,
        }),
    ));
    let query = livtet_search::user_input_translator::user_input_ast_to_query(&index, ast)
        .expect("user_input_ast_to_query");

    let docs = run_query(&index, &*query).await;
    assert!(
        docs.len() >= 3,
        "bare 'wind' must hit at least the 3 work1 editions; got {}",
        docs.len()
    );
}

#[tokio::test]
async fn user_input_ast_field_term_lowers_to_term_query() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    // `title:wind` lowers to a tantivy TermQuery (per docs
    // `QueryParser::build_query_from_user_input_ast` description).
    // Eds a/b/c all carry "Wind" in the title field.
    let ast = ast_title_literal("wind");
    let query = livtet_search::user_input_translator::user_input_ast_to_query(&index, ast)
        .expect("user_input_ast_to_query");

    let docs = run_query(&index, &*query).await;
    assert!(
        docs.len() >= 3,
        "title:wind must hit at least the 3 work1 editions; got {}",
        docs.len()
    );
}

#[tokio::test]
async fn user_input_ast_and_or_not_combinators_produce_boolean_query() {
    let db = fresh_db().await;
    let _seed = seed_comprehensive(&db).await;
    let (index, _dir) = fresh_index(&db).await;

    // The parser config (`get_query_parser`) sets
    // `set_field_fuzzy(title, false, 2, false)`, so a `title:wind`
    // query also matches edition_d via Levenshtein-distance-2
    // against "Wise" (3-edit distance "wind"↔"wise" = 2 since
    // `n→s` and `d→e` count). "edition_description" is NOT fuzzy
    // configured, so it behaves like a plain literal match. We pick
    // terms whose 2-edit-distant analogues don't appear elsewhere
    // (e.g. "deckled" in edition_a's description only) to keep the
    // assertions unambiguous.

    // - `title:wind` → 4 hits: a (Wind), b (Wind in subtitle),
    //   c (Wind), d (Wise via fuzzy distance 2).
    let q_wind = livtet_search::user_input_translator::user_input_ast_to_query(
        &index,
        ast_title_literal("wind"),
    )
    .expect("user_input_ast_to_query wind");
    let docs_wind = run_query(&index, &*q_wind).await;
    assert_eq!(
        docs_wind.len(),
        4,
        "title:wind (with fuzzy on title) must hit exactly a/b/c/d — the last one via fuzzy 'wind'↔'Wise' at distance 2"
    );

    // - `title:fear` → 1 hit: only edition_d (no fuzzy path hits
    //   "Wise" because Lev('fear','wise') > 2).
    let q_fear = livtet_search::user_input_translator::user_input_ast_to_query(
        &index,
        ast_title_literal("fear"),
    )
    .expect("user_input_ast_to_query fear");
    let docs_fear = run_query(&index, &*q_fear).await;
    assert_eq!(
        docs_fear.len(),
        1,
        "title:fear must hit exactly 1 document: edition_d"
    );

    // - `edition_description:deckled` → 1 hit: only edition_a.
    let q_deckled = livtet_search::user_input_translator::user_input_ast_to_query(
        &index,
        ast_edition_description_literal("deckled"),
    )
    .expect("user_input_ast_to_query deckled");
    let docs_deckled = run_query(&index, &*q_deckled).await;
    assert_eq!(
        docs_deckled.len(),
        1,
        "edition_description:deckled must hit exactly 1 document: edition_a"
    );

    // - `title:wind AND NOT edition_description:deckled`: the
    //   MustNot drops edition_a from the 4 above → 3 hits (b/c/d).
    let q_and_not = livtet_search::user_input_translator::user_input_ast_to_query(
        &index,
        tantivy::query_grammar::UserInputAst::Clause(vec![
            (
                Some(tantivy::query_grammar::Occur::Must),
                ast_title_literal("wind"),
            ),
            (
                Some(tantivy::query_grammar::Occur::MustNot),
                ast_edition_description_literal("deckled"),
            ),
        ]),
    )
    .expect("user_input_ast_to_query AND NOT");
    let docs_and_not = run_query(&index, &*q_and_not).await;
    assert_eq!(
        docs_and_not.len(),
        3,
        "title:wind AND NOT edition_description:deckled must drop edition_a, leaving b/c/d"
    );
    assert!(
        docs_and_not.len() < docs_wind.len(),
        "MustNot must strictly reduce hits (got {} from {} baseline)",
        docs_and_not.len(),
        docs_wind.len()
    );

    // - Outer OR: `(title:wind AND NOT edition_description:deckled)
    //   OR title:fear` — edition_d is already in the AND-NOT arm,
    //   so the OR adds nothing and the total stays at 3. Both
    //   arms `docs_and_not` and `docs_or` must agree on the size.
    let q_or = livtet_search::user_input_translator::user_input_ast_to_query(
        &index,
        tantivy::query_grammar::UserInputAst::Clause(vec![
            (
                Some(tantivy::query_grammar::Occur::Should),
                tantivy::query_grammar::UserInputAst::Clause(vec![
                    (
                        Some(tantivy::query_grammar::Occur::Must),
                        ast_title_literal("wind"),
                    ),
                    (
                        Some(tantivy::query_grammar::Occur::MustNot),
                        ast_edition_description_literal("deckled"),
                    ),
                ]),
            ),
            (
                Some(tantivy::query_grammar::Occur::Should),
                ast_title_literal("fear"),
            ),
        ]),
    )
    .expect("user_input_ast_to_query OR");
    let docs_or = run_query(&index, &*q_or).await;
    assert_eq!(
        docs_or.len(),
        3,
        "(title:wind AND NOT edition_description:deckled) OR title:fear must hit exactly 3 documents: b, c, d"
    );
    assert_eq!(
        docs_or.len(),
        docs_and_not.len(),
        "OR with title:fear must add no new docs (edition_d already in AND-NOT arm)"
    );
}
