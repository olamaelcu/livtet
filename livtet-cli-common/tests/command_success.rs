//! Success paths for the editions/edition commands. `query_failures.rs`
//! drives the `.map_err` arms by hiding tables; this drives the other side —
//! a populated search result, a detail lookup that resolves a format and
//! authors, and file add/update/remove against the inventory.

use livtet_cli_common::Context;
use livtet_cli_common::edition::{add_file, remove_file};
use livtet_cli_common::editions::{get_detail, search_rows};
use livtet_cli_common::reindex::ReindexArgs;
use livtet_test_utils::cli::CliFixture;

fn context(fx: &CliFixture) -> Context {
    Context {
        db_path: fx.db_path.clone(),
        index_dir: fx.index_dir.clone(),
        profile_id: None,
    }
}

async fn reindex(fx: &CliFixture) {
    ReindexArgs {
        database: None,
        index_dir: None,
        yes: true,
        force: false,
    }
    .run(&context(fx))
    .await
    .expect("reindex so search has content");
}

#[tokio::test]
async fn search_rows_renders_authors_and_detail_resolves_format_and_authors() {
    let fx = CliFixture::new().await;
    reindex(&fx).await;

    // The catalog holds three editions titled around "The Name of the Wind",
    // each linked to an author, so the hit->row conversion takes the
    // join-authors branch rather than the "—" placeholder.
    let hits = search_rows(&context(&fx), "Name", 20, 0)
        .await
        .expect("search succeeds against the rebuilt index");
    assert!(!hits.is_empty(), "'Name' should match the seeded editions");
    assert!(
        hits.iter().any(|r| r.author.contains("Rothfuss")),
        "a populated hit must carry its author: {hits:?}"
    );

    // Detail for the ebook resolves its format and author rows.
    let ebook = fx.edition("The Name of the Wind (Ebook)").id.to_string();
    get_detail(&context(&fx), &ebook)
        .await
        .expect("detail resolves format + authors");
}

#[tokio::test]
async fn add_file_inserts_then_updates_and_remove_file_deletes() {
    let fx = CliFixture::new().await;

    // Insert path, with an extension that file_format_for upper-cases.
    let hardcover = fx.edition("The Name of the Wind").id.to_string();
    add_file(&context(&fx), &hardcover, "/books/wind.epub")
        .await
        .expect("add_file inserts a new inventory row");

    // Update path on an edition that already has a file, with a path that has
    // no extension so file_format_for leaves the format unset.
    let ebook = fx.edition("The Name of the Wind (Ebook)").id.to_string();
    add_file(&context(&fx), &ebook, "/books/wind-no-ext")
        .await
        .expect("add_file updates the existing inventory row");

    remove_file(&context(&fx), &ebook)
        .await
        .expect("remove_file deletes the inventory row");
}
