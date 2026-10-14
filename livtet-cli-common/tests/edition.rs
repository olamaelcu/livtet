//! `livtet edition files` — mutation commands against a fixture install.

use livtet_cli_common::Context;
use livtet_cli_common::edition::{
    FileChange, FileRow, add_file, list_files_rows, remove_file, render_added, render_file_table,
    render_removed,
};
use livtet_test_utils::cli::CliFixture;

fn context(fx: &CliFixture) -> Context {
    Context {
        db_path: fx.db_path.clone(),
        index_dir: fx.index_dir.clone(),
        profile_id: None,
    }
}

#[tokio::test]
async fn add_associates_a_file_with_an_edition_that_had_none() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);
    let hardcover = fx.edition("The Name of the Wind");

    let change = add_file(&ctx, &hardcover.id.to_string(), "/books/new.epub")
        .await
        .expect("add");

    assert!(change.changed);
    assert_eq!(change.title, "The Name of the Wind");
    assert_eq!(change.file_path.as_deref(), Some("/books/new.epub"));

    let rows = list_files_rows(&ctx, 20).await.expect("list");
    assert_eq!(
        rows.iter()
            .filter(|r| r.edition_id == hardcover.id.to_string())
            .count(),
        1
    );
}

/// The inventory row is keyed by `digital_inventory.id`; `edition_id` is a
/// separate UNIQUE column. Looking it up by primary key found nothing, so a
/// second add tried to INSERT and hit the UNIQUE constraint instead of
/// replacing the path.
#[tokio::test]
async fn add_replaces_an_existing_association_rather_than_inserting() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);
    let ebook = fx.edition("The Name of the Wind (Ebook)");
    let id = ebook.id.to_string();

    add_file(&ctx, &id, "/books/first.epub")
        .await
        .expect("first add");
    let change = add_file(&ctx, &id, "/books/second.epub")
        .await
        .expect("second add must replace, not conflict");
    assert_eq!(change.file_path.as_deref(), Some("/books/second.epub"));

    let rows = list_files_rows(&ctx, 20).await.expect("list");
    let mine: Vec<&FileRow> = rows.iter().filter(|r| r.edition_id == id).collect();
    assert_eq!(mine.len(), 1, "one inventory row per edition: {mine:#?}");
    assert_eq!(mine[0].file_path, "/books/second.epub");
}

#[tokio::test]
async fn remove_drops_an_existing_association() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);
    let ebook = fx.edition("The Name of the Wind (Ebook)");
    let id = ebook.id.to_string();

    let change = remove_file(&ctx, &id).await.expect("remove");
    assert!(change.changed, "the ebook starts with a file");
    assert_eq!(change.file_path, None);

    let rows = list_files_rows(&ctx, 20).await.expect("list");
    assert!(!rows.iter().any(|r| r.edition_id == id), "rows: {rows:#?}");
}

#[tokio::test]
async fn remove_reports_no_change_when_there_was_no_file() {
    let fx = CliFixture::new().await;
    let hardcover = fx.edition("The Name of the Wind");

    let change = remove_file(&context(&fx), &hardcover.id.to_string())
        .await
        .expect("remove");
    assert!(!change.changed);
}

#[tokio::test]
async fn remove_is_idempotent() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);
    let id = fx.edition("The Name of the Wind (Ebook)").id.to_string();

    assert!(remove_file(&ctx, &id).await.expect("first").changed);
    assert!(
        !remove_file(&ctx, &id).await.expect("second").changed,
        "a second remove finds nothing to remove"
    );
}

#[tokio::test]
async fn mutations_reject_a_malformed_id() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);

    for err in [
        add_file(&ctx, "not-a-ulid", "/books/a.epub")
            .await
            .expect_err("add"),
        remove_file(&ctx, "not-a-ulid").await.expect_err("remove"),
    ] {
        assert!(err.to_string().contains("Invalid edition ID"), "got: {err}");
    }
}

#[tokio::test]
async fn mutations_report_a_missing_edition() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);
    let absent = livtet_types::DbId::new().to_string();

    for err in [
        add_file(&ctx, &absent, "/books/a.epub")
            .await
            .expect_err("add"),
        remove_file(&ctx, &absent).await.expect_err("remove"),
    ] {
        assert!(err.to_string().contains("not found"), "got: {err}");
    }
}

#[tokio::test]
async fn list_files_shows_only_editions_with_files_and_honours_limit() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);

    // The fixture seeds exactly one edition with a file.
    let rows = list_files_rows(&ctx, 20).await.expect("list");
    assert_eq!(rows.len(), 1, "rows: {rows:#?}");
    assert_eq!(rows[0].title, "The Name of the Wind (Ebook)");
    assert_eq!(rows[0].work_title, "The Name of the Wind (Ebook)");
    assert_eq!(rows[0].file_format, "epub");

    add_file(
        &ctx,
        &fx.edition("The Fifth Season").id.to_string(),
        "/b.epub",
    )
    .await
    .expect("add");
    assert_eq!(list_files_rows(&ctx, 20).await.expect("list").len(), 2);
    assert_eq!(list_files_rows(&ctx, 1).await.expect("limit 1").len(), 1);
}

// ─── rendering ──────────────────────────────────────────────────────────────

fn change(changed: bool, path: Option<&str>) -> FileChange {
    FileChange {
        edition_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
        title: "The Name of the Wind".to_string(),
        file_path: path.map(str::to_string),
        changed,
    }
}

#[test]
fn render_added_names_the_path_and_edition() {
    insta::assert_snapshot!(render_added(&change(true, Some("/books/a.epub"))));
}

#[test]
fn render_removed_distinguishes_a_real_removal_from_a_no_op() {
    insta::assert_snapshot!("removed", render_removed(&change(true, None)));
    insta::assert_snapshot!("nothing_to_remove", render_removed(&change(false, None)));
}

#[test]
fn render_file_table_says_so_when_empty() {
    assert_eq!(render_file_table(&[]), "No editions with associated files");
}

#[test]
fn rendered_file_table_is_stable() {
    insta::assert_snapshot!(render_file_table(&[FileRow {
        edition_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
        title: "The Name of the Wind (Ebook)".to_string(),
        work_title: "The Name of the Wind".to_string(),
        file_path: "/books/the-name-of-the-wind.epub".to_string(),
        file_format: "epub".to_string(),
    }]));
}
