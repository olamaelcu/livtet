//! `livtet reindex` against a fixture install.
//!
//! The fixture already carries a built index, so these point `--index-dir`
//! at a fresh directory inside its temp dir to exercise a real rebuild
//! rather than reusing one.

use camino::Utf8PathBuf;
use livtet_cli_common::{Context, ReindexArgs};
use livtet_search::{SearchOptions, SearchReader};
use livtet_test_utils::cli::CliFixture;

fn context(fx: &CliFixture) -> Context {
    Context {
        db_path: fx.db_path.clone(),
        index_dir: fx.index_dir.clone(),
    }
}

/// A directory beside the fixture's own index, so the two don't collide.
fn scratch_index(fx: &CliFixture, name: &str) -> Utf8PathBuf {
    fx.index_dir
        .parent()
        .expect("index dir has a parent")
        .join(name)
}

fn args(index_dir: &Utf8PathBuf, yes: bool, force: bool) -> ReindexArgs {
    ReindexArgs {
        database: None,
        index_dir: Some(index_dir.clone()),
        yes,
        force,
    }
}

async fn edition_count(index_dir: &Utf8PathBuf) -> usize {
    let reader = SearchReader::open(index_dir).expect("open index");
    reader
        .search_with_options("", 50, &SearchOptions::default())
        .await
        .expect("search")
        .len()
}

#[tokio::test]
async fn builds_a_usable_index_in_an_empty_directory() {
    let fx = CliFixture::new().await;
    let index_dir = scratch_index(&fx, "fresh-index");
    assert!(!index_dir.exists(), "starting from nothing");

    args(&index_dir, true, false)
        .run(&context(&fx))
        .await
        .expect("reindex");

    assert!(index_dir.exists());
    assert_eq!(
        edition_count(&index_dir).await,
        fx.editions.len(),
        "every seeded edition must be searchable afterwards"
    );
}

/// The second run takes the "already current" branch: `migrate_to` is a
/// no-op, `force` is unset, so nothing is rebuilt.
#[tokio::test]
async fn a_second_run_without_force_is_a_no_op() {
    let fx = CliFixture::new().await;
    let index_dir = scratch_index(&fx, "idempotent-index");
    let ctx = context(&fx);

    args(&index_dir, true, false)
        .run(&ctx)
        .await
        .expect("first");
    args(&index_dir, true, false)
        .run(&ctx)
        .await
        .expect("second run must succeed, not error");

    assert_eq!(edition_count(&index_dir).await, fx.editions.len());
}

/// `--force` takes the wipe-and-rebuild path even when the schema is
/// current, which is the only route through `remove_dir_all` + `open` +
/// `reindex_with_progress`.
#[tokio::test]
async fn force_rebuilds_an_already_current_index() {
    let fx = CliFixture::new().await;
    let index_dir = scratch_index(&fx, "forced-index");
    let ctx = context(&fx);

    args(&index_dir, true, false)
        .run(&ctx)
        .await
        .expect("first");
    args(&index_dir, true, true)
        .run(&ctx)
        .await
        .expect("forced rebuild");

    assert_eq!(
        edition_count(&index_dir).await,
        fx.editions.len(),
        "a forced rebuild must end with the same documents, not an empty index"
    );
}

#[tokio::test]
async fn force_on_an_empty_directory_still_builds() {
    let fx = CliFixture::new().await;
    let index_dir = scratch_index(&fx, "forced-fresh-index");

    args(&index_dir, true, true)
        .run(&context(&fx))
        .await
        .expect("forced build from nothing");

    assert_eq!(edition_count(&index_dir).await, fx.editions.len());
}

/// Without `--yes` the command asks first. There is no terminal under
/// `cargo test`, so the prompt fails rather than blocking, and the command
/// must surface that instead of rebuilding anyway.
#[tokio::test]
async fn refuses_to_run_unconfirmed() {
    let fx = CliFixture::new().await;
    let index_dir = scratch_index(&fx, "unconfirmed-index");

    let err = args(&index_dir, false, false)
        .run(&context(&fx))
        .await
        .expect_err("must not rebuild without confirmation");
    let message = err.to_string();
    assert!(
        message.contains("confirmation prompt failed") || message.contains("Aborted by user"),
        "got: {message}"
    );
    assert!(
        !index_dir.exists(),
        "nothing should have been written before the prompt"
    );
}

/// The flags override the context rather than the context winning.
#[tokio::test]
async fn index_dir_flag_overrides_the_context() {
    let fx = CliFixture::new().await;
    let index_dir = scratch_index(&fx, "override-index");

    args(&index_dir, true, false)
        .run(&context(&fx))
        .await
        .expect("reindex");

    assert!(
        index_dir.exists(),
        "the flag's directory is the one that gets built"
    );
}

#[tokio::test]
async fn database_flag_targets_a_different_catalog() {
    let fx = CliFixture::new().await;
    let other = CliFixture::new().await;
    let index_dir = scratch_index(&fx, "other-db-index");

    // Point at `other`'s database while the context still names `fx`'s.
    ReindexArgs {
        database: Some(other.db_path.clone()),
        index_dir: Some(index_dir.clone()),
        yes: true,
        force: false,
    }
    .run(&context(&fx))
    .await
    .expect("reindex");

    assert_eq!(
        edition_count(&index_dir).await,
        other.editions.len(),
        "the flag's database is the one that gets indexed"
    );
}
