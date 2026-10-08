//! `livtet editions` — query commands against a fixture install.
//!
//! Every test points a [`Context`] at a [`CliFixture`]'s temp directory, so
//! nothing here touches the real database or index.

use livtet_cli_common::Context;
use livtet_cli_common::editions::{
    EditionDetail, get_detail, linkify_with, list_rows, render_detail, render_table, search_rows,
};
use livtet_test_utils::cli::CliFixture;

/// A context pointed at the fixture. `CliFixture` names its database
/// `test.db` where `Context::in_dir` expects `livtet.db`, so the paths are
/// taken from the fixture directly rather than derived from its directory.
fn context(fx: &CliFixture) -> Context {
    Context {
        db_path: fx.db_path.clone(),
        index_dir: fx.index_dir.clone(),
    }
}

#[tokio::test]
async fn list_returns_every_seeded_edition() {
    let fx = CliFixture::new().await;
    let rows = list_rows(&context(&fx), 20, 0).await.expect("list");

    assert_eq!(rows.len(), fx.editions.len(), "rows: {rows:#?}");
    let mut titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    titles.sort_unstable();
    assert_eq!(
        titles,
        vec![
            "The Fifth Season",
            "The Name of the Wind",
            "The Name of the Wind (Ebook)",
        ]
    );
}

#[tokio::test]
async fn list_rows_carry_author_format_and_on_disk_state() {
    let fx = CliFixture::new().await;
    let rows = list_rows(&context(&fx), 20, 0).await.expect("list");

    let ebook = rows
        .iter()
        .find(|r| r.title == "The Name of the Wind (Ebook)")
        .expect("ebook edition in rows");
    assert_eq!(ebook.author, "Patrick Rothfuss");
    assert_eq!(ebook.format, "Ebook");
    assert!(
        ebook.on_disk,
        "the ebook is the fixture's edition with a file"
    );

    let hardcover = rows
        .iter()
        .find(|r| r.title == "The Name of the Wind")
        .expect("hardcover edition in rows");
    assert_eq!(hardcover.format, "Hardcover");
    assert!(!hardcover.on_disk);
}

#[tokio::test]
async fn list_honours_limit_and_offset() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);

    assert_eq!(list_rows(&ctx, 1, 0).await.expect("limit 1").len(), 1);
    assert_eq!(list_rows(&ctx, 0, 0).await.expect("limit 0").len(), 0);

    let all = list_rows(&ctx, 20, 0).await.expect("all");
    let skipped = list_rows(&ctx, 20, 1).await.expect("offset 1");
    assert_eq!(skipped.len(), all.len() - 1);
    assert!(
        !skipped.contains(&all[0]),
        "offset 1 must drop the first row"
    );
}

#[tokio::test]
async fn search_narrows_to_matching_editions() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);

    let jemisin = search_rows(&ctx, "Jemisin", 20, 0).await.expect("search");
    assert_eq!(jemisin.len(), 1, "rows: {jemisin:#?}");
    assert_eq!(jemisin[0].title, "The Fifth Season");

    let wind = search_rows(&ctx, "wind", 20, 0).await.expect("search");
    assert_eq!(wind.len(), 2, "both Wind editions match: {wind:#?}");
}

#[tokio::test]
async fn search_for_an_absent_term_returns_nothing() {
    let fx = CliFixture::new().await;
    let rows = search_rows(&context(&fx), "zzzznotathing", 20, 0)
        .await
        .expect("search");
    assert!(rows.is_empty(), "rows: {rows:#?}");
}

#[tokio::test]
async fn get_resolves_authors_format_and_file() {
    let fx = CliFixture::new().await;
    let ebook = fx.edition("The Name of the Wind (Ebook)");

    let detail = get_detail(&context(&fx), &ebook.id.to_string())
        .await
        .expect("get");

    assert_eq!(detail.title, "The Name of the Wind (Ebook)");
    assert_eq!(detail.authors, vec!["Patrick Rothfuss"]);
    assert_eq!(detail.format.as_deref(), Some("Ebook"));
    assert_eq!(
        detail.file_path.as_deref(),
        ebook.file_path.as_ref().map(|p| p.as_str()),
        "the inventory row is keyed by edition_id, not by its own primary key"
    );
}

#[tokio::test]
async fn get_reports_no_file_for_an_edition_without_one() {
    let fx = CliFixture::new().await;
    let hardcover = fx.edition("The Name of the Wind");

    let detail = get_detail(&context(&fx), &hardcover.id.to_string())
        .await
        .expect("get");
    assert_eq!(detail.file_path, None);
}

#[tokio::test]
async fn get_rejects_a_malformed_id() {
    let fx = CliFixture::new().await;
    let err = get_detail(&context(&fx), "not-a-ulid")
        .await
        .expect_err("malformed id must fail");
    assert!(err.to_string().contains("Invalid edition ID"), "got: {err}");
}

#[tokio::test]
async fn get_reports_a_missing_edition() {
    let fx = CliFixture::new().await;
    let absent = livtet_types::DbId::new();
    let err = get_detail(&context(&fx), &absent.to_string())
        .await
        .expect_err("absent edition must fail");
    assert!(err.to_string().contains("not found"), "got: {err}");
}

// ─── rendering ──────────────────────────────────────────────────────────────

#[test]
fn render_table_reports_a_count_even_when_empty() {
    assert_eq!(render_table(&[]), "0 edition(s)");
}

#[tokio::test]
async fn rendered_list_table_is_stable() {
    let fx = CliFixture::new().await;
    let mut rows = list_rows(&context(&fx), 20, 0).await.expect("list");
    // Index relevance order is not something this snapshot should assert.
    rows.sort_by(|a, b| a.title.cmp(&b.title));
    // ULIDs differ per run, so the ID column cannot be snapshotted.
    for row in &mut rows {
        row.id = "<ulid>".to_string();
    }
    insta::assert_snapshot!(render_table(&rows));
}

#[test]
fn rendered_detail_block_is_stable() {
    insta::assert_snapshot!(render_detail(&EditionDetail {
        title: "The Name of the Wind (Ebook)".to_string(),
        authors: vec!["Patrick Rothfuss".to_string()],
        format: Some("Ebook".to_string()),
        file_path: Some("/books/the-name-of-the-wind.epub".to_string()),
    }));
}

#[test]
fn rendered_detail_block_falls_back_to_dashes() {
    insta::assert_snapshot!(render_detail(&EditionDetail {
        title: "Untitled".to_string(),
        authors: Vec::new(),
        format: None,
        file_path: None,
    }));
}

#[test]
fn linkify_emits_an_osc8_sequence_only_for_a_plain_tty() {
    let path = "/books/a.epub";
    let linked = linkify_with(path, true, false, false);
    assert!(linked.starts_with("\x1b]8;;file:///books/a.epub"));
    assert!(linked.contains(path));

    // Every suppression reason yields the bare path.
    assert_eq!(linkify_with(path, false, false, false), path);
    assert_eq!(linkify_with(path, true, true, false), path);
    assert_eq!(linkify_with(path, true, false, true), path);
}

#[test]
fn linkify_makes_a_relative_path_absolute_in_the_uri() {
    let linked = linkify_with("books/a.epub", true, false, false);
    assert!(linked.contains("file:///books/a.epub"), "got: {linked:?}");
}
