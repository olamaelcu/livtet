use assert_cmd::Command;
use predicates::prelude::*;

fn cmd() -> Command {
    let mut cmd = Command::cargo_bin("livtet-cli").expect("livtet-cli binary");
    cmd.env_remove("RUST_LOG").env("RUST_LOG", "error");
    cmd
}

#[test]
fn path_all_succeeds() {
    cmd()
        .args(["path", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bundle"));
}

#[test]
fn path_unknown_kind_fails() {
    cmd()
        .args(["path", "nope"])
        .assert()
        .failure()
        .code(predicate::ne(0))
        .stderr(predicate::str::contains("unknown path kind"));
}

#[test]
fn editions_help_lists_subcommands() {
    cmd()
        .args(["editions", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("get"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("search"));
}

#[test]
fn editions_list_help_shows_limit_and_offset() {
    cmd()
        .args(["editions", "list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--limit"))
        .stdout(predicate::str::contains("--offset"));
}

#[test]
fn editions_search_help_shows_query() {
    cmd()
        .args(["editions", "search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("QUERY"));
}

#[test]
fn editions_get_no_id_fails() {
    cmd()
        .args(["editions", "get"])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

#[test]
fn editions_get_invalid_id_fails_clearly() {
    cmd()
        .args(["editions", "get", "not-a-real-id"])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

#[test]
fn editions_list_piped_output_has_no_osc8() {
    // `Open` may create an empty index dir if absent, but output must
    // never contain OSC8 escapes when piped (not a TTY).
    cmd()
        .args(["editions", "list", "--limit", "5"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\x1b]8").not());
}

#[test]
fn reindex_help_shows_force() {
    cmd()
        .args(["reindex", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--force"))
        .stdout(predicate::str::contains("--database"))
        .stdout(predicate::str::contains("--index-dir"));
}

#[test]
fn reindex_fresh_db_builds_index() {
    let dir = camino_tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("test.db");
    let idx = dir.path().join("search-index");

    cmd()
        .args([
            "reindex",
            "--database",
            db.as_str(),
            "--index-dir",
            idx.as_str(),
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Indexed"));
    assert!(
        idx.join("search_schema_version.json").is_file(),
        "reindex must write the schema version sidecar"
    );
}

#[test]
fn reindex_noop_when_current() {
    let dir = camino_tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("test.db");
    let idx = dir.path().join("search-index");

    cmd()
        .args([
            "reindex",
            "--database",
            db.as_str(),
            "--index-dir",
            idx.as_str(),
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Indexed"));

    // Second run without --force is a no-op.
    cmd()
        .args([
            "reindex",
            "--database",
            db.as_str(),
            "--index-dir",
            idx.as_str(),
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("already current"));
}

#[test]
fn reindex_force_rebuilds_even_when_current() {
    let dir = camino_tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("test.db");
    let idx = dir.path().join("search-index");

    cmd()
        .args([
            "reindex",
            "--database",
            db.as_str(),
            "--index-dir",
            idx.as_str(),
            "--yes",
        ])
        .assert()
        .success();

    // --force rebuilds unconditionally.
    cmd()
        .args([
            "reindex",
            "--database",
            db.as_str(),
            "--index-dir",
            idx.as_str(),
            "--yes",
            "--force",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Indexed"));
}

#[test]
fn edition_help_lists_files_subcommand() {
    cmd()
        .args(["edition", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files"));
}

#[test]
fn edition_files_help_shows_add_remove_list() {
    cmd()
        .args(["edition", "files", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("add"))
        .stdout(predicate::str::contains("remove"))
        .stdout(predicate::str::contains("list"));
}

#[test]
fn edition_files_add_shows_usage() {
    cmd()
        .args(["edition", "files", "add", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ID"))
        .stdout(predicate::str::contains("PATH"));
}

#[test]
fn edition_files_remove_shows_usage() {
    cmd()
        .args(["edition", "files", "remove", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ID"));
}

#[test]
fn edition_files_list_shows_limit() {
    cmd()
        .args(["edition", "files", "list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--limit"));
}

#[test]
fn edition_files_no_id_fails() {
    cmd()
        .args(["edition", "files", "add"])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

#[test]
fn edition_files_add_invalid_id_fails() {
    cmd()
        .args(["edition", "files", "add", "not-a-real-id", "/some/path"])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

#[test]
fn edition_files_remove_invalid_id_fails() {
    cmd()
        .args(["edition", "files", "remove", "not-a-real-id"])
        .assert()
        .failure()
        .code(predicate::ne(0));
}
