use assert_cmd::Command;
use predicates::prelude::*;

fn cmd() -> Command {
    let mut cmd = Command::cargo_bin("livtet-cli").expect("livtet-cli binary");
    cmd.env_remove("RUST_LOG").env("RUST_LOG", "error");
    cmd
}

#[test]
fn help_lists_seed_and_path() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("path"));
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
