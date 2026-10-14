//! What the commands do when a query fails mid-way.
//!
//! `editions get` makes four or five sequential queries — the edition, its
//! inventory row, its format, its edition-author links, then the authors —
//! and the file commands make three or four. A failure part-way through must
//! surface as an error rather than returning a record with the missing half
//! silently blank, which is what every one of those `.map_err` arms is for.
//! None of them had a caller.
//!
//! Each failure is induced by renaming one table out from under a live
//! command. Renaming rather than dropping avoids SQLite's foreign-key
//! checks, which refuse to drop a parent whose children still hold rows.

use livtet_cli_common::Context;
use livtet_cli_common::edition::{add_file, list_files_rows, remove_file};
use livtet_cli_common::editions::{get_detail, list_rows, search_rows};
use livtet_test_utils::cli::CliFixture;
use livtet_test_utils::fault;

fn context(fx: &CliFixture) -> Context {
    Context {
        db_path: fx.db_path.clone(),
        index_dir: fx.index_dir.clone(),
        profile_id: None,
    }
}

/// Rename `table` so the next query against it fails with "no such table".
async fn hide_table(fx: &CliFixture, table: &str) {
    fault::hide_table(fx.pool(), table).await;
}

/// Every table `editions get` reads, in the order it reads them.
const GET_DETAIL_TABLES: [&str; 5] = [
    "editions",
    "digital_inventory",
    "formats",
    "edition_authors",
    "authors",
];

#[tokio::test]
async fn get_reports_a_failure_in_any_of_its_queries() {
    for table in GET_DETAIL_TABLES {
        let fx = CliFixture::new().await;
        let id = fx.edition("The Name of the Wind (Ebook)").id.to_string();
        hide_table(&fx, table).await;

        let err = get_detail(&context(&fx), &id)
            .await
            .expect_err(&format!("get must fail once {table} is gone"));
        let message = err.to_string();
        assert!(
            message.contains("Query failed"),
            "hiding {table} gave: {message}"
        );
    }
}

#[tokio::test]
async fn adding_a_file_reports_a_failed_lookup() {
    for table in ["editions", "digital_inventory"] {
        let fx = CliFixture::new().await;
        let id = fx.edition("The Name of the Wind").id.to_string();
        hide_table(&fx, table).await;

        let err = add_file(&context(&fx), &id, "/books/a.epub")
            .await
            .expect_err(&format!("add must fail once {table} is gone"));
        assert!(
            err.to_string().contains("Query failed"),
            "hiding {table} gave: {err}"
        );
    }
}

#[tokio::test]
async fn removing_a_file_reports_a_failed_lookup() {
    for table in ["editions", "digital_inventory"] {
        let fx = CliFixture::new().await;
        let id = fx.edition("The Name of the Wind (Ebook)").id.to_string();
        hide_table(&fx, table).await;

        let err = remove_file(&context(&fx), &id)
            .await
            .expect_err(&format!("remove must fail once {table} is gone"));
        assert!(
            err.to_string().contains("Query failed"),
            "hiding {table} gave: {err}"
        );
    }
}

/// `list_files_rows` joins inventory to editions to works, so a failure in
/// any of the three has to surface rather than yielding rows with blank
/// titles.
#[tokio::test]
async fn listing_files_reports_a_failure_in_any_join() {
    for table in ["digital_inventory", "editions", "works"] {
        let fx = CliFixture::new().await;
        hide_table(&fx, table).await;

        let err = list_files_rows(&context(&fx), 20)
            .await
            .expect_err(&format!("list must fail once {table} is gone"));
        assert!(
            err.to_string().contains("Query failed"),
            "hiding {table} gave: {err}"
        );
    }
}

/// The index-backed queries do not touch the database, so hiding a table
/// must leave them working — the split between index reads and SeaORM reads
/// is the point of those two code paths.
#[tokio::test]
async fn index_backed_queries_are_unaffected_by_the_database() {
    let fx = CliFixture::new().await;
    let ctx = context(&fx);
    hide_table(&fx, "editions").await;

    let listed = list_rows(&ctx, 20, 0)
        .await
        .expect("list reads the index, not the database");
    assert_eq!(listed.len(), fx.editions.len());

    let found = search_rows(&ctx, "wind", 20, 0)
        .await
        .expect("search reads the index, not the database");
    assert_eq!(found.len(), 2);
}
