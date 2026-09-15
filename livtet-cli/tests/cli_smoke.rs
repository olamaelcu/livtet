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