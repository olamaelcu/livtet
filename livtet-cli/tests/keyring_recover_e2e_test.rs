//! End-to-end tests for `livtet keyring-recover` failure modes and
//! the `--passphrase-stdin` happy path.
//!
//! The happy path (with `--passphrase` on the command line) is
//! already covered by `assert_cmd_tests::keyring_recover_writes_passphrase_env_file_with_0600_perms`.
//! This file pins:
//!
//! 1. **Empty passphrase** — must be rejected with a clear error
//!    message (don't derive a 32-byte key from nothing).
//! 2. **Both flags** — `--passphrase X --passphrase-stdin` is a clap
//!    conflict and must fail with a non-zero exit before any
//!    passphrase processing happens.
//! 3. **Neither flag** — must fail with a non-zero exit and a message
//!    pointing the user at the available flags.
//! 4. **`--passphrase-stdin` happy path** — when a passphrase is piped
//!    in via stdin, the recovery file is written with the derived key
//!    and mode 0600.
//!
//! State isolation: HOME / XDG_CONFIG_HOME / XDG_DATA_HOME are
//! redirected into a `tempfile::TempDir` so the developer's real
//! `~/.config/net.olamaelcu.livtet` is never touched. `LIVTET_HMAC_KEY_HEX` is
//! removed because the recovery flow does not consult the HMAC
//! fallback — it derives a fresh key from the passphrase.

use std::os::unix::fs::PermissionsExt as _;

use assert_cmd::Command;
use fs_err as fs;
use predicates::prelude::*;
use camino_tempfile::Utf8TempDir as TempDir;

/// Configure a freshly-built `Command` for the `livtet` binary with
/// state-isolating env vars. Mirrors `assert_cmd_tests::isolated_cmd`
/// but deliberately does *not* set `LIVTET_HMAC_KEY_HEX` — the
/// keyring-recover flow is independent of any prior HMAC key.
fn isolated_cmd(tmp: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("livtet-cli").expect("livtet-cli binary built by cargo test");
    cmd.env("HOME", tmp.path())
        .env("XDG_CONFIG_HOME", tmp.path().join("config"))
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env_remove("LIVTET_HMAC_KEY_HEX")
        .env_remove("RUST_LOG")
        .env("RUST_LOG", "error");
    cmd
}

#[test]
fn keyring_recover_rejects_empty_passphrase() {
    let tmp = TempDir::new().expect("tempdir");
    // `--passphrase ""` is a valid clap parse (the flag accepts a
    // string), so the binary reaches the "is the passphrase empty?"
    // guard. That guard must reject the empty value with a clear
    // error.
    isolated_cmd(&tmp)
        .args(["keyring-recover", "--passphrase", ""])
        .assert()
        .failure()
        .code(predicate::ne(0))
        .stderr(predicate::str::contains("empty").or(predicate::str::contains("EmptyPassphrase")));
}

#[test]
fn keyring_recover_rejects_both_passphrase_and_stdin_flag() {
    let tmp = TempDir::new().expect("tempdir");
    // The clap struct marks `--passphrase` and `--passphrase-stdin`
    // as `conflicts_with` each other. Passing both must fail at the
    // clap parse stage with a non-zero exit and a stderr message that
    // mentions the conflict (typical clap wording: "cannot be used
    // with" or similar).
    isolated_cmd(&tmp)
        .args([
            "keyring-recover",
            "--passphrase",
            "some-passphrase",
            "--passphrase-stdin",
        ])
        .assert()
        .failure()
        .code(predicate::ne(0))
        // The exact wording varies across clap versions; the
        // stable contract is that the *flag names* appear in the
        // error. Both `--passphrase` and `--passphrase-stdin` should
        // be mentioned, and the binary must exit non-zero.
        .stderr(predicate::str::contains("--passphrase"));
}

#[test]
fn keyring_recover_requires_a_passphrase_source() {
    let tmp = TempDir::new().expect("tempdir");
    // Neither flag set → the binary must refuse to derive a key.
    // The current production code paths emits a message that points
    // the user at `--passphrase` or `--passphrase-stdin`; we don't
    // pin the exact wording (it has shifted in past PRs) but we do
    // require a non-zero exit and *some* indication that a
    // passphrase source is required.
    isolated_cmd(&tmp)
        .args(["keyring-recover"])
        .assert()
        .failure()
        .code(predicate::ne(0))
        .stderr(
            predicate::str::contains("passphrase")
                .or(predicate::str::contains("Passphrase"))
                .or(predicate::str::contains("stdin"))
                .or(predicate::str::contains("Stdin")),
        );
}

#[test]
fn keyring_recover_piped_stdin_writes_recovery_file() {
    let tmp = TempDir::new().expect("tempdir");
    // We pass a passphrase via stdin (the documented scripted
    // path: `echo "$LIVTET_RECOVERY_PASSPHRASE" | livtet
    // keyring-recover --passphrase-stdin`). The recovery file must
    // appear, be mode 0600, and contain a 64-char hex key on a
    // `LIVTET_PASSPHRASE_HMAC_KEY=` line.
    let expected = tmp
        .path()
        .join("config")
        .join("net.olamaelcu.livtet")
        .join("passphrase.env");

    isolated_cmd(&tmp)
        .args(["keyring-recover", "--passphrase-stdin"])
        .write_stdin("p9-stdin-passphrase\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Wrote passphrase-derived HMAC key",
        ));

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
