//! Black-box integration tests for the `livtet` binary.
//!
//! These tests spawn the compiled `livtet` binary as a child process via
//! `assert_cmd` and assert on its real stdout / stderr / exit code. They
//! do NOT call into `livtet_cli::*` library functions directly — that
//! style of test is already covered by the per-subcommand integration
//! test files (e.g. `plugin_subcommand_tests.rs`,
//! `repo_add_online_tests.rs`).
//!
//! The point of THIS file is to verify the *end-user experience*:
//! clap parsing, env-var routing, output formatting, and exit codes as
//! observed by a shell user. P9 of the M5 testing plan.
//!
//! State isolation:
//! * `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME` are redirected into a
//!   `tempfile::TempDir` so we never touch the developer's real
//!   `~/.config/net.olamaelcu.livtet`, `~/.local/share/net.olamaelcu.livtet`, or trust stores.
//! * `LIVTET_HMAC_KEY_HEX` is set to a 64-char hex string of zeros
//!   so the CLI's HMAC keyring fallback is deterministic and the OS
//!   keyring is not consulted.
//! * `RUST_LOG=error` keeps `tracing_subscriber` quiet during the test
//!   run; integration tests don't need `info!` noise from the binary.

use std::os::unix::fs::PermissionsExt as _;

use assert_cmd::Command;
use fs_err as fs;
use predicates::prelude::*;
use camino_tempfile::Utf8TempDir as TempDir;

/// Configure a freshly-built `Command` for the `livtet` binary with
/// all state-isolating env vars pointed at `tmp`. We return the
/// `Command` by value so callers can chain `.args(...)` and `.assert()`
/// without fighting `&mut Command` lifetime annotations.
fn isolated_cmd(tmp: &TempDir) -> Command {
    // 32 zero bytes (64 hex chars) — satisfies LIVTET_HMAC_KEY_HEX's
    // "must be 64-char hex string" requirement and keeps HMAC output
    // deterministic across test runs.
    let hmac_hex = "00".repeat(32);
    let mut cmd = Command::cargo_bin("livtet-cli").expect("livtet binary built by cargo test");
    cmd.env("HOME", tmp.path())
        .env("XDG_CONFIG_HOME", tmp.path().join("config"))
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("LIVTET_HMAC_KEY_HEX", hmac_hex)
        // Silence the binary's tracing subscriber so assertions on
        // stdout/stderr are predictable. `tracing_subscriber` reads
        // `RUST_LOG` directly.
        .env_remove("RUST_LOG")
        .env("RUST_LOG", "error");
    cmd
}

#[test]
fn plugin_list_reports_empty_with_fresh_providers_dir() {
    let tmp = TempDir::new().expect("tempdir");
    isolated_cmd(&tmp)
        .args(["plugin", "list"])
        .assert()
        .success()
        // `cmd_list` (in plugin.rs) prints to stderr via
        // `output::info` when no plugins are installed. The message is
        // stable wording; we check for the key phrase.
        .stderr(predicate::str::contains("no plugins installed"));
}

#[test]
fn plugin_keygen_writes_minisign_keypair_to_keys_dir() {
    let tmp = TempDir::new().expect("tempdir");
    let keys_dir = tmp.path().join("keys");

    isolated_cmd(&tmp)
        .args([
            "plugin",
            "keygen",
            "--label",
            "p9-cli-test",
            "--passphrase",
            "disabled",
            "--keys-dir",
            keys_dir.as_os_str().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generated signing key"))
        .stdout(predicate::str::contains("Label: p9-cli-test"))
        // The "Pubkey file:" line carries the resolved path. We just
        // check the stem so the assertion survives tempdir random
        // suffixes.
        .stdout(predicate::str::contains("Pubkey file:"))
        .stdout(predicate::str::contains("Fingerprint: SHA256:"));

    // The keygen subcommand must actually have created the files on
    // disk — assert_cmd alone only proves the process exit, this
    // proves the side effect.
    assert!(
        keys_dir.join("p9-cli-test.key").exists(),
        "keygen must write {}/p9-cli-test.key",
        keys_dir.as_std_path().display()
    );
    assert!(
        keys_dir.join("p9-cli-test.pub").exists(),
        "keygen must write {}/p9-cli-test.pub",
        keys_dir.as_std_path().display()
    );
}

#[test]
fn repo_list_json_returns_empty_array_with_fresh_config() {
    let tmp = TempDir::new().expect("tempdir");
    // `repo list --json` reaches into the HMAC-protected
    // `repositories.toml` file under XDG_DATA_HOME. With an empty
    // data dir, the cached file is missing, and the JSON output is
    // literally `"[]"` followed by a newline (rendered by
    // `serde_json::to_string_pretty` on an empty `Vec`).
    isolated_cmd(&tmp)
        .args(["repo", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));
}

#[test]
fn keyring_recover_writes_passphrase_env_file_with_0600_perms() {
    let tmp = TempDir::new().expect("tempdir");
    // We do NOT need LIVTET_HMAC_KEY_HEX for this test: the
    // `keyring-recover` subcommand derives a fresh key from the
    // passphrase and writes it to disk without consulting any prior
    // HMAC source. Clear it to make the test's intent explicit.
    let expected = tmp
        .path()
        .join("config")
        .join("net.olamaelcu.livtet")
        .join("passphrase.env");

    Command::cargo_bin("livtet-cli")
        .expect("livtet binary")
        .env("HOME", tmp.path())
        .env("XDG_CONFIG_HOME", tmp.path().join("config"))
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env_remove("LIVTET_HMAC_KEY_HEX")
        .env("RUST_LOG", "error")
        .args(["keyring-recover", "--passphrase", "p9-cli-test-passphrase"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Wrote passphrase-derived HMAC key",
        ))
        .stdout(predicate::str::contains("passphrase.env"));

    // The recovery file must exist and be mode 0600 (owner R/W only).
    // This is a real security property: a world-readable HMAC key file
    // would let any local user forge state files.
    let meta = fs_err::metadata(&expected).expect("recovery file exists");
    assert!(
        meta.is_file(),
        "{} should be a regular file",
        expected.as_std_path().display()
    );
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "passphrase.env must be mode 0600, got {mode:o}"
    );

    // The file should contain a 64-char hex key (32 bytes) on a single
    // `LIVTET_PASSPHRASE_HMAC_KEY=` line.
    let body = fs::read_to_string(&expected).expect("read passphrase.env");
    let line = body
        .trim()
        .strip_prefix("LIVTET_PASSPHRASE_HMAC_KEY=")
        .expect("file starts with LIVTET_PASSPHRASE_HMAC_KEY=");
    assert_eq!(
        line.len(),
        64,
        "derived key must be 64 hex chars (32 bytes)"
    );
    assert!(
        line.chars().all(|c| c.is_ascii_hexdigit()),
        "derived key must be hex"
    );
}
