//! Concurrency: a single writer handle ([`SearchIndex`], which holds
//! Tantivy's exclusive index-writer lock) and any number of read-only
//! handles ([`SearchReader`], which take no writer lock) must be able to
//! hold the same index directory open at the same time, and readers must
//! observe commits made by the writer.

use camino_tempfile::Utf8TempDir as TempDir;
use livtet_search::{EditionDoc, SearchError, SearchIndex, SearchReader};

fn fresh_dir() -> TempDir {
    camino_tempfile::tempdir().expect("tempdir")
}

fn make_edition(edition_id: &str, work_id: &str, title: &str) -> EditionDoc {
    EditionDoc {
        edition_id: edition_id.to_string(),
        work_id: work_id.to_string(),
        title: title.to_string(),
        edition_title: None,
        work_description: None,
        edition_description: None,
        authors: vec![],
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
        title_sort: title.to_lowercase(),
        primary_author_sort: None,
        created_at: 0,
        updated_at: None,
        popularity: 0,
    }
}

#[tokio::test]
async fn reader_opens_alongside_writer() {
    let dir = fresh_dir();
    let writer = SearchIndex::open(dir.path()).expect("writer open");
    let reader =
        SearchReader::open(dir.path()).expect("reader must open while the writer lock is held");

    assert!(
        reader
            .search("nonexistent", 10)
            .await
            .expect("search")
            .is_empty()
    );
    drop(reader);
    drop(writer);
}

#[tokio::test]
async fn reader_bootstraps_fresh_dir() {
    let dir = fresh_dir();
    let reader = SearchReader::open(dir.path()).expect("reader open fresh");

    assert!(
        dir.path().join("search_schema_version.json").is_file(),
        "reader must write the schema sidecar for a fresh dir"
    );
    assert!(
        reader
            .search("nonexistent", 10)
            .await
            .expect("search")
            .is_empty()
    );
}

#[tokio::test]
async fn reader_observes_writer_commits() {
    let dir = fresh_dir();
    let writer = SearchIndex::open(dir.path()).expect("writer open");
    let reader = SearchReader::open(dir.path()).expect("reader open");

    assert!(
        reader
            .search("Gatsby", 10)
            .await
            .expect("search before commit")
            .is_empty(),
        "reader opened before the write must start empty"
    );

    writer
        .upsert_edition(make_edition("e1", "w1", "The Great Gatsby"))
        .await
        .expect("upsert");

    let hits = reader
        .search("Gatsby", 10)
        .await
        .expect("search after commit");
    assert_eq!(hits.len(), 1, "reader must reload and see the new commit");
    assert_eq!(hits[0].title, "The Great Gatsby");
}

#[tokio::test]
async fn many_readers_coexist_with_writer() {
    let dir = fresh_dir();
    let writer = SearchIndex::open(dir.path()).expect("writer open");
    writer
        .upsert_edition(make_edition("e1", "w1", "Dune"))
        .await
        .expect("upsert");

    let readers: Vec<SearchReader> = (0..4)
        .map(|_| SearchReader::open(dir.path()).expect("reader open"))
        .collect();

    for reader in &readers {
        assert_eq!(reader.search("Dune", 10).await.expect("search").len(), 1);
    }
}

#[tokio::test]
async fn second_writer_still_fails() {
    let dir = fresh_dir();
    let _first = SearchIndex::open(dir.path()).expect("first writer");

    let err = match SearchIndex::open(dir.path()) {
        Ok(_) => panic!("second writer must fail"),
        Err(e) => e,
    };
    assert!(
        matches!(err, SearchError::Tantivy(_)),
        "expected tantivy lock error, got {err:?}"
    );
}
