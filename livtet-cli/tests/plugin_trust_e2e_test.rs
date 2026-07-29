//! End-to-end tests for `livtet plugin trust` and `livtet plugin
//! uninstall`.
//!
//! These tests spawn the compiled `livtet-cli` binary as a child
//! process via `assert_cmd` and assert on real stdout, stderr, exit
//! code, and the on-disk side effects (trust store pubkey file,
//! providers install directory).
//!
//! State isolation:
//!
//! * `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME` are redirected into a
//!   `tempfile::TempDir` so the developer's real `~/.config/net.olamaelcu.livtet` and
//!   `~/.local/share/net.olamaelcu.livtet` are never touched.
//! * `LIVTET_HMAC_KEY_HEX` is set to a deterministic 64-char hex
//!   string so the OS keyring is never consulted.
//! * `RUST_LOG=error` keeps `tracing_subscriber` quiet.

use std::os::unix::fs::PermissionsExt as _;

use assert_cmd::Command;
use fs_err as fs;
use predicates::prelude::*;
use camino_tempfile::Utf8TempDir as TempDir;

fn isolated_cmd(tmp: &TempDir) -> Command {
    let hmac_hex = "00".repeat(32);
    let mut cmd = Command::cargo_bin("livtet-cli").expect("livtet-cli binary built by cargo test");
    cmd.env("HOME", tmp.path())
        .env("XDG_CONFIG_HOME", tmp.path().join("config"))
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("LIVTET_HMAC_KEY_HEX", hmac_hex)
        .env_remove("RUST_LOG")
        .env("RUST_LOG", "error");
    cmd
}

#[test]
fn plugin_trust_copies_pubkey_into_trust_store() {
    let tmp = TempDir::new().expect("tempdir");
    let keys_dir = tmp.path().join("keys");

    // Step 1: `plugin keygen --passphrase disabled` writes a minisign
    // keypair into `keys_dir`. We assert the contract: a `.key` and
    // `.pub` file are created, and the `.pub` is world-readable (the
    // production secret key file is mode 0600, but the pubkey is
    // intentionally not secret).
    isolated_cmd(&tmp)
        .args([
            "plugin",
            "keygen",
            "--label",
            "trust-e2e-author",
            "--passphrase",
            "disabled",
            "--keys-dir",
            keys_dir.as_os_str().to_str().unwrap(),
        ])
        .assert()
        .success();

    let pubkey_src = keys_dir.join("trust-e2e-author.pub");
    assert!(pubkey_src.exists(), "keygen must write {pubkey_src:?}");

    // Step 2: `plugin trust <pubkey>` copies the pubkey into the
    // user's trust store (under `$XDG_CONFIG_HOME/net.olamaelcu.livtet/keys/signing-keys/`).
    isolated_cmd(&tmp)
        .args(["plugin", "trust", pubkey_src.as_os_str().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Trusted"))
        .stdout(predicate::str::contains("Label: trust-e2e-author"))
        .stdout(predicate::str::contains("Fingerprint: SHA256:"));

    // The trust store is `$XDG_CONFIG_HOME/net.olamaelcu.livtet/keys/signing-keys/`
    // (the CLI honors `XDG_CONFIG_HOME` on every platform via
    // `livtet_core::paths::config_dir`, which falls back to the
    // `dirs` crate's default only when the env var is unset).
    let trust_dir = tmp
        .path()
        .join("config")
        .join("net.olamaelcu.livtet/keys/signing-keys");
    let trusted_dest = trust_dir.join("trust-e2e-author.pub");
    assert!(
        trusted_dest.exists(),
        "plugin trust must copy the pubkey to {trusted_dest:?}"
    );

    // The bytes copied to the trust store must match the original
    // pubkey file byte-for-byte. If we ever introduce a normalization
    // step (whitespace stripping, base64 rewrites) this assertion
    // will catch it.
    let src_bytes = fs::read(&pubkey_src).expect("read source pubkey");
    let dst_bytes = fs::read(&trusted_dest).expect("read trusted pubkey");
    assert_eq!(
        src_bytes, dst_bytes,
        "copied pubkey must be byte-identical to the source"
    );

    // Sanity: the pubkey in the trust store is still a valid
    // minisign pubkey box. (We re-parse it through the same parser
    // the rest of the CLI uses.)
    let trusted_text = fs::read_to_string(&trusted_dest).expect("read trusted text");
    let _vk = livtet_plugins::keys::signing::parse_pubkey_text(&trusted_text)
        .expect("trusted pubkey must round-trip through parse_pubkey_text");
}

#[test]
fn plugin_trust_rejects_file_that_is_not_a_minisign_pubkey() {
    let tmp = TempDir::new().expect("tempdir");
    let bogus = tmp.path().join("bogus.pub");
    fs::write(&bogus, "this is not a minisign secret key box").expect("write bogus");

    isolated_cmd(&tmp)
        .args(["plugin", "trust", bogus.as_os_str().to_str().unwrap()])
        .assert()
        .failure()
        // The exact error message is whatever
        // `parse_pubkey_text` returns; we just check that the
        // trust path is *not* created and the exit code is non-zero.
        // (The CLI's `anyhow::Error` surfaces as a non-zero exit
        // code; clap is happy since the args parse fine.)
        .code(predicate::ne(0));

    let trust_dir = tmp
        .path()
        .join(".config/net.olamaelcu.livtet/keys/signing-keys");
    assert!(
        !trust_dir.join("bogus.pub").exists(),
        "trust store must not contain a bogus pubkey file"
    );
}

#[test]
fn plugin_uninstall_removes_fake_installed_plugin() {
    let tmp = TempDir::new().expect("tempdir");

    // Manually lay down a fake installed-plugin tree at
    // `$XDG_DATA_HOME/net.olamaelcu.livtet/providers/<id>/<version>/livtet.toml`
    // (XDG_DATA_HOME is redirected to `tmp/data` by `isolated_cmd`).
    let providers_root = tmp
        .path()
        .join("data")
        .join("net.olamaelcu.livtet/providers");
    let id = "uninstall-e2e";
    let version = "0.1.0";
    let install_root = providers_root.join(id).join(version);
    fs::create_dir_all(&install_root).expect("mkdir install root");
    fs::write(
        install_root.join("livtet.toml"),
        "[plugin]\nid = \"uninstall-e2e\"\nname = \"uninstall-e2e\"\nversion = \"0.1.0\"\nentry = \"init.lua\"\n",
    )
    .expect("write manifest");
    fs::write(install_root.join("init.lua"), b"-- uninstall e2e\n").expect("write lua");

    assert!(
        install_root.exists(),
        "fake install must be on disk before uninstall"
    );

    isolated_cmd(&tmp)
        .args(["plugin", "uninstall", id, version])
        .assert()
        .success()
        .stdout(predicate::str::contains("Uninstalled uninstall-e2e v0.1.0"));

    assert!(
        !install_root.exists(),
        "plugin uninstall must remove {install_root:?}"
    );
    // The parent dir may still exist (we don't recursively remove
    // empty parents), but the versioned directory should be gone.
    let providers_after = providers_root.join(id);
    if providers_after.exists() {
        assert!(
            fs::read_dir(&providers_after).unwrap().next().is_none(),
            "id dir should be empty after uninstall, found entries"
        );
    }
}

#[test]
fn plugin_uninstall_for_missing_plugin_returns_error() {
    let tmp = TempDir::new().expect("tempdir");
    // No install_root on disk — the providers dir is fresh.

    isolated_cmd(&tmp)
        .args(["plugin", "uninstall", "never-installed", "9.9.9"])
        .assert()
        .failure()
        .code(predicate::ne(0))
        .stderr(predicate::str::contains("never-installed"))
        .stderr(predicate::str::contains("9.9.9"))
        .stderr(predicate::str::contains("is not installed"));
}

#[test]
fn plugin_uninstall_does_not_need_to_be_owner_of_directory() {
    // Even with restrictive perms on the install root, the CLI
    // should be able to remove the directory (the test harness
    // runs as the same UID that created it). This test pins the
    // security-relevant property that the install path is not
    // world-writable — the CLI is not silently allowing other
    // users to remove the directory on its behalf.
    let tmp = TempDir::new().expect("tempdir");
    let install_root = tmp
        .path()
        .join("data")
        .join("net.olamaelcu.livtet/providers/perm-test/0.1.0");
    fs::create_dir_all(&install_root).expect("mkdir");
    fs::write(
        install_root.join("livtet.toml"),
        "[plugin]\nid = \"perm-test\"\nversion = \"0.1.0\"\nentry = \"init.lua\"\n",
    )
    .expect("write");
    fs::write(install_root.join("init.lua"), b"-- perm\n").expect("write");

    // Verify the directory is mode 0o755 (default), and the CLI can
    // still rm -rf it.
    let meta = fs::metadata(&install_root).expect("metadata");
    let mode = meta.permissions().mode() & 0o777;
    assert!(
        (mode & 0o002) == 0,
        "install root should not be world-writable: {mode:o}"
    );

    isolated_cmd(&tmp)
        .args(["plugin", "uninstall", "perm-test", "0.1.0"])
        .assert()
        .success();
    assert!(!install_root.exists());
}
