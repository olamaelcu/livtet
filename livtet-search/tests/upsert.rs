//! Tests for direct-doc upsert methods on `SearchIndex`.
//!
//! These tests exercise [`SearchIndex::upsert_edition`] and
//! [`SearchIndex::upsert_author`] against a fresh Tantivy index
//! on a tempdir. No SQLite database is involved.

use livtet_search::{AuthorDoc, EditionDoc, SearchIndex, SearchOptions};
use camino_tempfile::Utf8TempDir as TempDir;

fn fresh_index() -> (SearchIndex, TempDir) {
    let dir = camino_tempfile::tempdir().expect("tempdir");
    let idx = SearchIndex::open(dir.path())
        .expect("open fresh index");
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
