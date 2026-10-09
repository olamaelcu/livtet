//! `Command::run_with` and each subcommand's own `run`.
//!
//! The other test files call the row-returning functions directly, which
//! leaves the dispatch wiring — the match arms that build a tokio runtime and
//! block on the right handler — unexercised. A command wired to the wrong
//! handler, or a `run` that forgets to print, would pass every other test here.
//!
//! These are plain `#[test]`s, not `#[tokio::test]`s: `run_with` builds its
//! own runtime, and building one inside another panics. The fixture is built
//! on a runtime held for the life of the test, and `Context` carries only
//! paths, so the handlers open their own connections.

use livtet_cli_common::cli::{Command, PathArgs};
use livtet_cli_common::edition::{EditionArgs, EditionCommand, FilesArgs, FilesCommand};
use livtet_cli_common::editions::{EditionsArgs, EditionsCommand};
use livtet_cli_common::{Context, ReindexArgs};
use livtet_test_utils::cli::{CliFixture, FixtureEdition};

/// A runtime, a fixture built on it, and a context pointing at the fixture.
/// The runtime is returned so it outlives the fixture's pool.
fn fixture() -> (tokio::runtime::Runtime, CliFixture, Context) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let fx = rt.block_on(CliFixture::new());
    let ctx = Context {
        db_path: fx.db_path.clone(),
        index_dir: fx.index_dir.clone(),
    };
    (rt, fx, ctx)
}

fn an_edition(fx: &CliFixture) -> &FixtureEdition {
    fx.edition("The Name of the Wind")
}

#[test]
fn editions_list_dispatches_and_prints() {
    let (_rt, _fx, ctx) = fixture();

    Command::Editions(EditionsArgs {
        command: EditionsCommand::List {
            limit: 20,
            offset: 0,
        },
    })
    .run_with(&ctx)
    .expect("editions list");
}

#[test]
fn editions_search_dispatches_and_prints() {
    let (_rt, _fx, ctx) = fixture();

    Command::Editions(EditionsArgs {
        command: EditionsCommand::Search {
            query: "wind".to_string(),
            limit: 20,
            offset: 0,
        },
    })
    .run_with(&ctx)
    .expect("editions search");
}

#[test]
fn editions_get_dispatches_and_prints() {
    let (_rt, fx, ctx) = fixture();
    let id = an_edition(&fx).id.to_string();

    Command::Editions(EditionsArgs {
        command: EditionsCommand::Get { id },
    })
    .run_with(&ctx)
    .expect("editions get");
}

/// A missing edition must surface as an error through the dispatch, not be
/// swallowed into a successful exit.
#[test]
fn editions_get_propagates_a_missing_edition() {
    let (_rt, _fx, ctx) = fixture();

    let err = Command::Editions(EditionsArgs {
        command: EditionsCommand::Get {
            id: livtet_types::DbId::new().to_string(),
        },
    })
    .run_with(&ctx)
    .expect_err("an absent edition must fail");
    assert!(err.to_string().contains("not found"), "got {err}");
}

#[test]
fn edition_files_dispatches_each_subcommand() {
    let (_rt, fx, ctx) = fixture();
    let id = an_edition(&fx).id.to_string();

    for command in [
        FilesCommand::Add {
            id: id.clone(),
            path: "/books/dispatched.epub".to_string(),
        },
        FilesCommand::List { limit: 20 },
        FilesCommand::Remove { id: id.clone() },
    ] {
        Command::Edition(EditionArgs {
            command: EditionCommand::Files(FilesArgs { command }),
        })
        .run_with(&ctx)
        .expect("edition files");
    }
}

#[test]
fn reindex_dispatches_with_confirmation_skipped() {
    let (_rt, fx, ctx) = fixture();
    let index_dir = fx
        .index_dir
        .parent()
        .expect("index dir has a parent")
        .join("dispatch-index");

    Command::Reindex(ReindexArgs {
        database: None,
        index_dir: Some(index_dir.clone()),
        yes: true,
        force: false,
    })
    .run_with(&ctx)
    .expect("reindex");
    assert!(index_dir.exists(), "the dispatched reindex must build it");
}

/// `Path` is dispatched by `run`, ahead of resolving a `Context`, so it works
/// on a machine where the data directory cannot be resolved.
#[test]
fn path_dispatches_through_run_without_a_context() {
    Command::Path(PathArgs {
        kind: "all".to_string(),
    })
    .run()
    .expect("path");
}

// ─── error paths the happy dispatch cannot reach ─────────────────────────────

/// A directory that was never built is *not* an error — `SearchReader::open`
/// creates it and the listing is simply empty. An index directory that is
/// actually a file is, and the error has to name the path it tried.
#[test]
fn listing_against_an_index_path_that_is_a_file_names_it() {
    let (_rt, fx, _ctx) = fixture();
    // The fixture's database file is a convenient non-directory.
    let not_a_directory = fx.db_path.clone();
    let ctx = Context {
        db_path: fx.db_path.clone(),
        index_dir: not_a_directory.clone(),
    };

    let err = Command::Editions(EditionsArgs {
        command: EditionsCommand::List {
            limit: 20,
            offset: 0,
        },
    })
    .run_with(&ctx)
    .expect_err("a file is not an index directory");
    let message = err.to_string();
    assert!(message.contains("Failed to open index"), "got {message}");
    assert!(
        message.contains(not_a_directory.as_str()),
        "the error must name the path it tried: {message}"
    );
}

/// A directory with no index in it is a valid empty index, not a failure.
#[test]
fn listing_against_an_unbuilt_index_directory_is_empty() {
    let (_rt, fx, _ctx) = fixture();
    let ctx = Context {
        db_path: fx.db_path.clone(),
        index_dir: fx
            .index_dir
            .parent()
            .expect("parent")
            .join("never-built-index"),
    };

    Command::Editions(EditionsArgs {
        command: EditionsCommand::List {
            limit: 20,
            offset: 0,
        },
    })
    .run_with(&ctx)
    .expect("an unbuilt index reads as empty");
}

/// `get` connects read-only, so a database file that does not exist cannot be
/// created on demand — it has to fail with the connection error.
#[test]
fn getting_against_a_missing_database_fails_to_connect() {
    let (_rt, fx, _ctx) = fixture();
    let ctx = Context {
        db_path: fx
            .index_dir
            .parent()
            .expect("parent")
            .join("no-such-database.db"),
        index_dir: fx.index_dir.clone(),
    };

    let err = Command::Editions(EditionsArgs {
        command: EditionsCommand::Get {
            id: livtet_types::DbId::new().to_string(),
        },
    })
    .run_with(&ctx)
    .expect_err("a read-only connection cannot create the file");
    assert!(err.to_string().contains("Failed to connect"), "got {err}");
}

/// Reindex opens the database read-write-create, so it cannot be pointed at a
/// path that is already a directory.
#[test]
fn reindexing_onto_a_directory_fails_to_connect() {
    let (_rt, fx, ctx) = fixture();
    let a_directory = fx.index_dir.clone();

    let err = Command::Reindex(ReindexArgs {
        database: Some(a_directory),
        index_dir: Some(fx.index_dir.parent().expect("parent").join("doomed-index")),
        yes: true,
        force: false,
    })
    .run_with(&ctx)
    .expect_err("a directory is not a database");
    assert!(err.to_string().contains("Failed to"), "got {err}");
}
