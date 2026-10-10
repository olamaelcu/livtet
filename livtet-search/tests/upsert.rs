//! Tests for direct-doc upsert methods on `SearchIndex`.
//!
//! These tests exercise [`SearchIndex::upsert_edition`] and
//! [`SearchIndex::upsert_author`] against a fresh Tantivy index
//! on a tempdir. No SQLite database is involved.

use camino_tempfile::Utf8TempDir as TempDir;
use livtet_search::{AuthorDoc, EditionDoc, SearchIndex, SearchOptions};

fn fresh_index() -> (SearchIndex, TempDir) {
    let dir = camino_tempfile::tempdir().expect("tempdir");
    let idx = SearchIndex::open(dir.path()).expect("open fresh index");
    (idx, dir)
}

fn make_edition(
    edition_id: impl Into<String>,
    work_id: impl Into<String>,
    title: impl Into<String>,
    authors: Vec<String>,
) -> EditionDoc {
    EditionDoc {
        edition_id: edition_id.into(),
        work_id: work_id.into(),
        title: title.into(),
        edition_title: None,
        work_description: None,
        edition_description: None,
        authors,
        authors_ids: vec![],
        tags: vec![],
        genres: vec![],
        subjects: vec![],
        publishers: vec![],
        identifier_kinds: vec![],
        identifier_values: vec![],
        notes: None,
        format: None,
        language: None,
        pub_date: None,
        published_year: None,
        title_sort: "".to_string(),
        primary_author_sort: None,
        created_at: 0,
        updated_at: None,
        popularity: 0,
    }
}

#[tokio::test]
async fn upsert_edition_writes_searchable_doc() {
    let (idx, _dir) = fresh_index();
    let doc = make_edition(
        "e1",
        "w1",
        "The Great Gatsby",
        vec!["F. Scott Fitzgerald".into()],
    );
    idx.upsert_edition(doc).await.expect("upsert");

    let hits = idx.search("Gatsby", 10).await.expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "The Great Gatsby");
}

#[tokio::test]
async fn upsert_edition_replaces_existing() {
    let (idx, _dir) = fresh_index();
    let doc_a = make_edition("e1", "w1", "Old Title", vec![]);
    idx.upsert_edition(doc_a).await.expect("first upsert");

    let doc_b = make_edition("e1", "w1", "New Title", vec![]);
    idx.upsert_edition(doc_b).await.expect("second upsert");

    let hits = idx.search("New Title", 10).await.expect("search");
    assert_eq!(hits.len(), 1, "should find the new title");
    assert_eq!(hits[0].title, "New Title");

    let old_hits = idx.search("Old Title", 10).await.expect("search old");
    assert!(old_hits.is_empty(), "old title should have been replaced");
}

#[tokio::test]
async fn upsert_edition_idempotent() {
    let (idx, _dir) = fresh_index();
    let doc = make_edition("e1", "w1", "Idempotent Book", vec![]);
    idx.upsert_edition(doc).await.expect("first upsert");

    let doc2 = make_edition("e1", "w1", "Idempotent Book", vec![]);
    idx.upsert_edition(doc2).await.expect("second upsert");

    let hits = idx.search("Idempotent", 10).await.expect("search");
    assert_eq!(hits.len(), 1, "duplicate upsert should still yield one doc");
}

#[tokio::test]
async fn upsert_edition_writes_every_populated_field() {
    let (idx, _dir) = fresh_index();
    // Every optional field populated and every multi-valued field non-empty,
    // so `upsert_edition` runs each `if let Some(..)` / facet-adding branch
    // rather than the empty fast-paths the other tests take.
    let doc = EditionDoc {
        edition_id: "rich-1".into(),
        work_id: "work-1".into(),
        title: "The Left Hand of Darkness".into(),
        edition_title: Some("The Left Hand of Darkness (50th Anniversary)".into()),
        work_description: Some("A genethlic winter world".into()),
        edition_description: Some("Deckle-edged anniversary printing".into()),
        authors: vec!["Ursula K. Le Guin".into()],
        authors_ids: vec!["author-1".into()],
        tags: vec!["sci-fi".into(), "classic".into()],
        genres: vec!["Science Fiction".into()],
        subjects: vec!["Gender".into(), "Anthropology".into()],
        publishers: vec!["Ace Books".into()],
        identifier_kinds: vec!["isbn".into()],
        identifier_values: vec!["9780441478125".into()],
        notes: Some("signed first printing".into()),
        format: Some("hardcover".into()),
        language: Some("English".into()),
        pub_date: Some(1_600_000_000),
        published_year: Some(1969),
        title_sort: "left hand of darkness".into(),
        primary_author_sort: Some("le guin, ursula k.".into()),
        created_at: 1_500_000_000,
        updated_at: Some(1_600_000_500),
        popularity: 42,
    };
    idx.upsert_edition(doc).await.expect("upsert rich doc");

    let hits = idx.search("Darkness", 10).await.expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "The Left Hand of Darkness");
}

#[test]
fn reindex_event_is_terminal_distinguishes_loading_from_indexing() {
    use livtet_search::ReindexEvent;
    assert!(!ReindexEvent::Loading.is_terminal());
    assert!(ReindexEvent::Indexing { done: 1, total: 2 }.is_terminal());
}

#[tokio::test]
async fn upsert_author_writes_searchable_doc() {
    let (idx, _dir) = fresh_index();
    let author = AuthorDoc {
        author_id: "a1".into(),
        name: "Ursula K. Le Guin".into(),
        sort_name: "le guin, ursula k.".into(),
        birth_year: Some(1929),
        death_year: Some(2018),
        source: "catalog".into(),
    };
    idx.upsert_author(author).await.expect("upsert author");

    let hits = idx
        .search_all_kinds("Le Guin", 10, &SearchOptions::default())
        .await
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Ursula K. Le Guin");
}
