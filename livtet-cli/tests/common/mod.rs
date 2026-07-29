//! Shared test helpers for `livtet-cli` integration tests.
//!
//! Factored out of `smoke_e2e.rs` so that the online TOFU tests in
//! `repo_add_online_tests.rs` (and any future HTTP-mocked tests) can reuse
//! the same mock HTTP server implementation.

#![allow(dead_code)]

use std::collections::BTreeMap;

use assert_cmd::Command;
use camino::{Utf8Path, Utf8PathBuf};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use fs_err as fs;
use livtet_plugin::{
    keys::fingerprint,
    repository::{
        index::{Index, IndexPlugin, IndexVersionEntry, render_index_json},
        repo_toml::{RepoSection, RepoToml, SigningSection, render_repo_toml},
    },
};
pub use livtet_test_utils::{
    TestServer, build_response, http_response, parse_request_path, spawn_server,
};
use camino_tempfile::Utf8TempDir as TempDir;

/// A test environment with a temporary directory and a running mock
/// HTTP server.  The server root is at `tmp/www/`.
pub struct TestContext {
    pub tmp: camino_tempfile::Utf8TempDir,
    pub server: TestServer,
}

/// Create a `TestContext` with a fresh temp dir and a mock HTTP server
/// serving files from `tmp/www/`.
pub async fn setup_test_env() -> TestContext {
    let tmp = camino_tempfile::Utf8TempDir::new().unwrap();
    let root = tmp.path().join("www");
    fs::create_dir_all(root.as_std_path()).unwrap();
    let server = spawn_server(root).await;
    TestContext { tmp, server }
}

/// Build a minimal `Index` with a single plugin entry.
pub fn sample_index(plugin_id: &str, version: &str, archive: &str) -> Index {
    let mut versions = BTreeMap::new();
    versions.insert(
        version.to_string(),
        IndexVersionEntry {
            entry: "init.lua".to_string(),
            capabilities: BTreeMap::new(),
            dependencies: vec![],
            archive: archive.to_string(),
            archive_size: 0,
            archive_sha256: String::new(),
            min_app_version: "0.5.0".to_string(),
        },
    );
    let mut plugins = BTreeMap::new();
    plugins.insert(plugin_id.to_string(), IndexPlugin { versions });
    Index {
        format_version: 1,
        generated_at: "2026-06-06T00:00:00Z".to_string(),
        plugins,
    }
}

/// Write a signed repository (repo.toml + index.json + index.json.sig)
/// into `server_root`.
#[allow(clippy::too_many_arguments)]
pub fn write_signed_repo(
    server_root: &Utf8Path,
    name: &str,
    url: &str,
    signing_key: &SigningKey,
    verifying_key: &VerifyingKey,
    plugin_id: &str,
    plugin_version: &str,
    archive_name: &str,
) {
    let repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: name.to_string(),
            url: url.to_string(),
            description: Some("Test repo".to_string()),
            maintainer: None,
        },
        signing: SigningSection {
            key_label: name.to_string(),
            key_fingerprint: fingerprint(verifying_key),
        },
    };
    fs::write(server_root.join("repo.toml"), render_repo_toml(&repo_toml)).unwrap();

    let index = sample_index(plugin_id, plugin_version, archive_name);
    let index_json = render_index_json(&index);
    let sig_bytes = signing_key.sign(index_json.as_bytes()).to_bytes();
    fs::write(server_root.join("index.json"), &index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), sig_bytes).unwrap();
}

/// Create a signed repository: run `repo init` then `repo keygen`.
/// Returns the repo directory path.
///
/// Uses [`isolated_cmd`] for both subcommands so `repo keygen` writes
/// the signing key into the test's tmp config dir (where the caller's
/// `repo sign` / `repo publish` / `repo unpublish` invocations — also
/// run via `isolated_cmd` — will look it up). Calling `Command::cargo_bin`
/// directly here would leak the key into the developer's real
/// `~/.config/net.olamaelcu.livtet/`, leaving the test unable to find it.
pub fn setup_signed_repo(tmp: &TempDir, name: &str, url: &str, fingerprint: &str) -> Utf8PathBuf {
    let repo_dir = tmp.path().to_path_buf().join("repo");
    isolated_cmd(tmp)
        .arg("repo")
        .arg("init")
        .arg("--repo-dir")
        .arg(repo_dir.as_str())
        .arg("--name")
        .arg(name)
        .arg("--url")
        .arg(url)
        .arg("--key-fingerprint")
        .arg(fingerprint)
        .assert()
        .success();
    // `repo keygen` writes to the config dir (isolated via env vars),
    // not to the repo dir — no `--repo-dir` flag on this subcommand.
    isolated_cmd(tmp)
        .arg("repo")
        .arg("keygen")
        .arg("--name")
        .arg("repo")
        .arg("--passphrase")
        .arg("disabled")
        .assert()
        .success();
    repo_dir
}

/// Configure a freshly-built `Command` for the `livtet` binary with
/// all state-isolating env vars pointed at `tmp`. We return the
/// `Command` by value so callers can chain `.args(...)` and `.assert()`
/// without fighting `&mut Command` lifetime annotations.
///
/// The HMAC keyring fallback is forced to a deterministic 64-char hex
/// string of zeros via `LIVTET_HMAC_KEY_HEX`; that keeps HMAC output
/// stable across runs and prevents the binary from consulting the
/// developer's real OS keyring.
pub fn isolated_cmd(tmp: &camino_tempfile::Utf8TempDir) -> Command {
    let hmac_hex = "00".repeat(32);
    let mut cmd = Command::cargo_bin("livtet-cli").expect("livtet binary built by cargo test");
    cmd.env("HOME", tmp.path())
        .env("XDG_CONFIG_HOME", tmp.path().join("config"))
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("LIVTET_HMAC_KEY_HEX", hmac_hex)
        .env_remove("RUST_LOG")
        .env("RUST_LOG", "error");
    cmd
}
