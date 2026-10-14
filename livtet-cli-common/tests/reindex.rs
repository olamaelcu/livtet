//! `livtet reindex` end to end against a `CliFixture`'s real database and
//! index directory. `--yes` skips the interactive confirmation; the two
//! runs exercise the migrate (no-op schema) path and the `--force`
//! wipe-and-rebuild path.

use livtet_cli_common::Context;
use livtet_cli_common::reindex::ReindexArgs;
use livtet_test_utils::cli::CliFixture;

fn context(fx: &CliFixture) -> Context {
    Context {
        db_path: fx.db_path.clone(),
        index_dir: fx.index_dir.clone(),
        profile_id: None,
    }
}

fn args(force: bool) -> ReindexArgs {
    ReindexArgs {
        database: None,
        index_dir: None,
        yes: true,
        force,
    }
}

#[tokio::test]
async fn reindex_rebuilds_from_the_database() {
    // `CliFixture::new` already seeds a fixed catalog, so the index has
    // content to rebuild from.
    let fx = CliFixture::new().await;

    args(false)
        .run(&context(&fx))
        .await
        .expect("reindex via the migrate path");
}

#[tokio::test]
async fn reindex_force_wipes_and_rebuilds_a_current_index() {
    let fx = CliFixture::new().await;

    // A first pass brings the on-disk schema to the current version.
    args(false)
        .run(&context(&fx))
        .await
        .expect("initial reindex");

    // With the schema already current, --force takes the explicit
    // clear-directory + reopen + rebuild branch.
    args(true).run(&context(&fx)).await.expect("forced reindex");
}
