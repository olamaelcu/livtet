//! Black-box e2e tests for the 10 `livtet repo *` subcommands:
//! `init`, `add`, `confirm-add`, `remove`, `list`, `update`,
//! `confirm-update`, `keygen`, `publish`, `sign`, `unpublish`.
//!
//! These tests spawn the compiled `livtet-cli` binary as a child
//! process via `assert_cmd` and assert on real stdout, stderr, exit
//! code, and on-disk side effects. The state-isolating env vars
//! (HOME, XDG_CONFIG_HOME, XDG_DATA_HOME, LIVTET_HMAC_KEY_HEX)
//! keep every invocation from touching the developer's real
//! `~/.config/net.olamaelcu.livtet`, `~/.local/share/net.olamaelcu.livtet`, or OS keyring.
//!
//! The HTTP-mocked subcommands (`add`, `confirm-add`, `update`)
//! use the in-process mock server from `tests/common/mod.rs`. The
//! on-disk subcommands (`init`, `keygen`, `publish`, `sign`,
//! `unpublish`) operate entirely on the local filesystem.

mod common;

use std::os::unix::fs::PermissionsExt as _;

use assert_cmd::Command;
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir as TempDir;
use common::{sample_index, setup_test_env, write_signed_repo};
use ed25519_dalek::{Signer, VerifyingKey};
use fs_err as fs;
use livtet_plugins::{
    keys::{fingerprint, keyfile::keygen},
    repository::{
        index::{Index, render_index_json},
        repo_toml::{RepoSection, RepoToml, SigningSection, render_repo_toml},
    },
};
use predicates::prelude::*;
use rand::Rng as _;
use tokio::net::TcpListener;

fn isolated_cmd(tmp: &TempDir) -> Command {
    common::isolated_cmd(tmp)
}

/// Seed the trust store with a specific `VerifyingKey` so the
/// `repo *` HTTP-mocked tests can pin a server's signing key
/// to a known trust entry. The trust dir lives at
/// `$XDG_CONFIG_HOME/livtet/keys/signing-keys/<label>.pub` (the
/// path the CLI's `plugin trust` subcommand writes to).
///
/// We write the pubkey box directly (constructed via the
/// `minisign` crate) rather than going through the binary's
/// `plugin trust`, because the test's signing key is generated
/// by `SigningKey::generate()` and not by `plugin keygen`, so
/// there is no matching `.key` file to feed into the binary.
fn seed_trust_store(tmp: &TempDir, label: &str, verifying_key: &VerifyingKey) -> Utf8PathBuf {
    seed_trust_store_with_key(tmp, label, verifying_key)
}

/// Build a canonical minisign pubkey box from a `VerifyingKey` and
/// write it to `dest`. This is what `load_trust_store` parses via
/// `parse_pubkey_text`, so the resulting file makes the trust
/// store recognize the verifying key as a "known pubkey" (so
/// `find_user_key_by_fingerprint` matches against the trust store
/// entry by fingerprint).
fn write_minisign_pubkey_box(verifying_key: &VerifyingKey, dest: impl AsRef<Utf8Path>) {
    use minisign::PublicKey;

    // minisign's `PublicKey::from_bytes` expects the canonical
    // 42-byte layout: 2-byte sig_alg + 8-byte keynum + 32-byte
    // ed25519 pubkey. ed25519-dalek's `VerifyingKey::to_bytes`
    // gives us the 32-byte pubkey; we wrap it in the minisign
    // envelope manually (sig_alg = 0x0001 = EdDSA on Curve25519,
    // keynum = 0x0000_0000_0000_0000 = 0).
    let mut pk_bytes = [0u8; 42];
    pk_bytes[0..2].copy_from_slice(&[0x00, 0x01]);
    pk_bytes[2..10].copy_from_slice(&[0u8; 8]);
    pk_bytes[10..42].copy_from_slice(&verifying_key.to_bytes());
    let pk = PublicKey::from_bytes(&pk_bytes).expect("build minisign PublicKey");
    let pk_box = pk.to_box().expect("box minisign PublicKey");
    fs::write(dest.as_ref(), pk_box.to_string()).expect("write minisign pubkey box");
}

/// Seed the trust store with a `VerifyingKey` we already have
/// (e.g. from a randomly generated test server key). This is the
/// bridge between the test's `SigningKey::generate()` and the
/// CLI's trust store, which only accepts canonical minisign pubkey
/// boxes.
fn seed_trust_store_with_key(
    tmp: &TempDir,
    label: &str,
    verifying_key: &VerifyingKey,
) -> Utf8PathBuf {
    let trust_dir = tmp
        .path()
        .join("config")
        .join("net.olamaelcu.livtet")
        .join("keys")
        .join("signing-keys");
    fs::create_dir_all(&trust_dir).unwrap();
    let dest = trust_dir.join(format!("{label}.pub"));
    write_minisign_pubkey_box(verifying_key, &dest);
    dest
}

fn pack_test_plugin(tmp: &TempDir, id: &str, version: &str) -> Utf8PathBuf {
    use livtet_plugins::archive::pack::pack as archive_pack;
    let plugin_src = tmp.path().join("plugin-src");
    let out_dir = tmp.path().join("out");
    let keys = tmp.path().join("keys");
    let src = plugin_src.as_path();
    let out = out_dir.as_path();
    let kdir = keys.as_path();
    fs::create_dir_all(plugin_src.as_path()).unwrap();
    fs::write(
        src.join("livtet.toml"),
        format!(
            "[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nentry = \"init.lua\"\n"
        ),
    )
    .unwrap();
    fs::write(src.join("init.lua"), b"-- repo e2e\n").unwrap();
    let report = keygen(kdir, "publish-author", true).unwrap();
    archive_pack(src, &report.key_path, "publish-author", out).unwrap()
}

// ---------------------------------------------------------------------
// repo init
// ---------------------------------------------------------------------

#[test]
fn repo_init_creates_skeleton_with_repo_toml_and_index_json() {
    let tmp = TempDir::new().expect("tempdir");
    let repo_dir = tmp.path().join("repo");

    isolated_cmd(&tmp)
        .args([
            "repo",
            "init",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
            "--name",
            "test-repo",
            "--url",
            "https://example.com/repo",
            "--key-fingerprint",
            "SHA256:deadbeef",
            "--key-label",
            "alice",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized repository"))
        .stdout(predicate::str::contains("Name: test-repo"))
        .stdout(predicate::str::contains("URL: https://example.com/repo"))
        .stdout(predicate::str::contains("Key fingerprint: SHA256:deadbeef"))
        .stdout(predicate::str::contains("Key label: alice"));

    let repo_toml = repo_dir.join("repo.toml");
    let index_json = repo_dir.join("index.json");
    assert!(repo_toml.exists(), "init must write {repo_toml:?}");
    assert!(index_json.exists(), "init must write {index_json:?}");
    let pool = repo_dir.join("pool");
    assert!(pool.is_dir(), "init must create the pool/ directory");

    let toml_text = fs::read_to_string(&repo_toml).expect("read repo.toml");
    let parsed = livtet_plugins::repository::repo_toml::parse_repo_toml(&toml_text)
        .expect("repo.toml must parse");
    assert_eq!(parsed.repo.name, "test-repo");
    assert_eq!(parsed.signing.key_fingerprint, "SHA256:deadbeef");
}

#[test]
fn repo_init_succeeds_on_existing_dir() {
    // init must not fail if the target dir already exists; it just
    // overwrites repo.toml + index.json and ensures pool/ exists.
    let tmp = TempDir::new().expect("tempdir");
    let repo_dir = tmp.path().join("already-exists");
    fs::create_dir_all(&repo_dir).unwrap();
    fs::write(repo_dir.join("sentinel"), b"hi").unwrap();

    isolated_cmd(&tmp)
        .args([
            "repo",
            "init",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
            "--name",
            "olamaelcu",
            "--url",
            "https://example.com/olamaelcu",
            "--key-fingerprint",
            "SHA256:00",
        ])
        .assert()
        .success();

    assert!(repo_dir.join("repo.toml").exists());
    // The sentinel file from before init must still be there —
    // init does not rm -rf the dir.
    assert!(repo_dir.join("sentinel").exists());
}

// ---------------------------------------------------------------------
// repo add
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repo_add_returns_needs_tofu_for_unknown_repo() {
    let ctx = setup_test_env().await;
    let server_root = ctx.tmp.path().join("www");

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();

    write_signed_repo(
        &server_root,
        "olamaelcu",
        &ctx.server.base_url,
        &signing_key,
        &verifying_key,
        "smoke-e2e",
        "0.1.0",
        "smoke-e2e-0.1.0.ltp",
    );

    isolated_cmd(&ctx.tmp)
        .args(["repo", "add", "--url", &ctx.server.base_url])
        .assert()
        .success()
        .stdout(predicate::str::contains("Resolving"))
        .stdout(predicate::str::contains("Repo name: olamaelcu"))
        .stdout(predicate::str::contains(
            "Signing key fingerprint (SHA256):",
        ))
        .stdout(predicate::str::contains("plugin trust"))
        .stdout(predicate::str::contains("repo confirm-add"));
}

#[tokio::test]
async fn repo_add_fails_for_unreachable_server() {
    let ctx = setup_test_env().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);
    let dead_url = format!("http://{addr}");

    isolated_cmd(&ctx.tmp)
        .args(["repo", "add", "--url", &dead_url])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

#[test]
fn repo_add_ok_branch_prints_already_trusted_format() {
    // The `Ok` branch in `cmd_add` is currently dead in production
    // (the client's `add` only ever returns `NeedsTofuConfirmation`),
    // but the print format is part of the CLI's contract. We pin it
    // here by calling the helper directly so a future refactor of
    // either side (client returning Ok, or the CLI reformatting the
    // message) trips this test.
    use livtet_cli::repo::render_repository_add_ok_message;

    assert_eq!(
        render_repository_add_ok_message("olamaelcu", 3),
        "Added olamaelcu (already trusted; 3 plugins)"
    );
    assert_eq!(
        render_repository_add_ok_message("alice", 0),
        "Added alice (already trusted; 0 plugins)"
    );
}

// ---------------------------------------------------------------------
// repo confirm-add
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repo_confirm_add_caches_index_for_trusted_repo() {
    let ctx = setup_test_env().await;
    let server_root = ctx.tmp.path().join("www");

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();

    write_signed_repo(
        &server_root,
        "olamaelcu",
        &ctx.server.base_url,
        &signing_key,
        &verifying_key,
        "smoke-e2e",
        "0.1.0",
        "smoke-e2e-0.1.0.ltp",
    );

    // Pre-trust the verifying key.
    let _pubkey = seed_trust_store(&ctx.tmp, "olamaelcu", &verifying_key);

    isolated_cmd(&ctx.tmp)
        .args(["repo", "confirm-add", "--url", &ctx.server.base_url])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added. Fetched index.json:"))
        .stdout(predicate::str::contains("1 plugin versions"));

    let cached_index = ctx
        .tmp
        .path()
        .join("data/net.olamaelcu.livtet/repos/olamaelcu/index.json");
    assert!(
        cached_index.exists(),
        "confirm-add must cache the index at {cached_index:?}"
    );
    let cached_sig = ctx
        .tmp
        .path()
        .join("data/net.olamaelcu.livtet/repos/olamaelcu/index.json.sig");
    assert!(
        cached_sig.exists(),
        "confirm-add must cache the index signature at {cached_sig:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repo_confirm_add_fails_for_untrusted_repo() {
    let ctx = setup_test_env().await;
    let server_root = ctx.tmp.path().join("www");
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();
    write_signed_repo(
        &server_root,
        "olamaelcu",
        &ctx.server.base_url,
        &signing_key,
        &verifying_key,
        "smoke-e2e",
        "0.1.0",
        "smoke-e2e-0.1.0.ltp",
    );

    // No trust store seeding → confirm-add must fail.
    isolated_cmd(&ctx.tmp)
        .args(["repo", "confirm-add", "--url", &ctx.server.base_url])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

// ---------------------------------------------------------------------
// repo list
// ---------------------------------------------------------------------

#[test]
fn repo_list_json_returns_empty_array_for_fresh_config() {
    let tmp = TempDir::new().expect("tempdir");
    isolated_cmd(&tmp)
        .args(["repo", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));
}

#[test]
fn repo_list_human_reports_no_repositories_for_fresh_config() {
    let tmp = TempDir::new().expect("tempdir");
    isolated_cmd(&tmp)
        .args(["repo", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("No repositories configured"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repo_list_prints_repo_row_after_add() {
    let ctx = setup_test_env().await;
    let server_root = ctx.tmp.path().join("www");

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();

    write_signed_repo(
        &server_root,
        "olamaelcu",
        &ctx.server.base_url,
        &signing_key,
        &verifying_key,
        "list-me",
        "0.1.0",
        "list-me-0.1.0.ltp",
    );

    // First add (TOFU), then trust the key, then confirm-add so
    // the repo lands in repositories.toml. We can shortcut by
    // going straight through confirm-add with the pre-trusted key.
    let _pubkey = seed_trust_store(&ctx.tmp, "olamaelcu", &verifying_key);
    isolated_cmd(&ctx.tmp)
        .args(["repo", "confirm-add", "--url", &ctx.server.base_url])
        .assert()
        .success();

    isolated_cmd(&ctx.tmp)
        .args(["repo", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("olamaelcu"))
        .stdout(predicate::str::contains(&ctx.server.base_url));
}

// ---------------------------------------------------------------------
// repo remove
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repo_remove_drops_repository_from_repositories_toml() {
    let ctx = setup_test_env().await;
    let server_root = ctx.tmp.path().join("www");

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();

    write_signed_repo(
        &server_root,
        "olamaelcu",
        &ctx.server.base_url,
        &signing_key,
        &verifying_key,
        "remove-me",
        "0.1.0",
        "remove-me-0.1.0.ltp",
    );
    let _pubkey = seed_trust_store(&ctx.tmp, "olamaelcu", &verifying_key);
    isolated_cmd(&ctx.tmp)
        .args(["repo", "confirm-add", "--url", &ctx.server.base_url])
        .assert()
        .success();

    isolated_cmd(&ctx.tmp)
        .args(["repo", "remove", "--name-or-url", "olamaelcu"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed olamaelcu"));

    // repositories.toml should no longer mention olamaelcu.
    let repos_toml = ctx
        .tmp
        .path()
        .join("config/net.olamaelcu.livtet/repositories.toml");
    let text = fs::read_to_string(&repos_toml).expect("read repositories.toml");
    assert!(
        !text.contains("olamaelcu"),
        "olamaelcu must be removed from repositories.toml; got: {text}"
    );
}

#[test]
fn repo_remove_fails_for_unknown_repo() {
    let tmp = TempDir::new().expect("tempdir");
    isolated_cmd(&tmp)
        .args(["repo", "remove", "--name-or-url", "never-existed"])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

// ---------------------------------------------------------------------
// repo update (with key rollover non-zero exit contract)
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repo_update_with_key_rollover_returns_nonzero_exit_code() {
    // CI contract: `repo update` against a repo whose signing key
    // has changed (TOFU) must exit non-zero so CI scripts can
    // detect the rollover.
    let ctx = setup_test_env().await;
    let server_root = ctx.tmp.path().join("www");

    let mut csprng = rand::rng();
    let old_signing = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let old_verifying = old_signing.verifying_key();
    write_signed_repo(
        &server_root,
        "olamaelcu",
        &ctx.server.base_url,
        &old_signing,
        &old_verifying,
        "smoke-e2e",
        "0.1.0",
        "smoke-e2e-0.1.0.ltp",
    );

    // Pre-trust the verifying key (matches the server).
    let _pubkey = seed_trust_store(&ctx.tmp, "olamaelcu", &old_verifying);

    // confirm-add so repositories.toml has an entry.
    isolated_cmd(&ctx.tmp)
        .args(["repo", "confirm-add", "--url", &ctx.server.base_url])
        .assert()
        .success();

    // Now rotate the server's signing key. The trust store still
    // has the old key, so `repo update` will detect the rollover
    // and return non-zero.
    let new_signing = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let new_verifying = new_signing.verifying_key();
    let new_repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: ctx.server.base_url.clone(),
            description: Some("Test repo".to_string()),
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint: fingerprint(&new_verifying),
        },
    };
    fs::write(
        server_root.join("repo.toml"),
        render_repo_toml(&new_repo_toml),
    )
    .unwrap();
    let new_index = sample_index("smoke-e2e", "0.2.0", "smoke-e2e-0.2.0.ltp");
    let new_index_json = render_index_json(&new_index);
    let new_sig = new_signing.sign(new_index_json.as_bytes()).to_bytes();
    fs::write(server_root.join("index.json"), &new_index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), new_sig).unwrap();

    // The binary must exit non-zero on key rollover, with a
    // message that names the new fingerprint and points at
    // `repo confirm-update`.
    isolated_cmd(&ctx.tmp)
        .args(["repo", "update", "--name-or-url", "olamaelcu"])
        .assert()
        .failure()
        .code(predicate::ne(0))
        .stderr(predicate::str::contains("Signing key changed"))
        .stderr(predicate::str::contains("confirm-update"));
}

#[test]
fn repo_update_fails_for_unknown_repo() {
    let tmp = TempDir::new().expect("tempdir");
    isolated_cmd(&tmp)
        .args(["repo", "update", "--name-or-url", "never-existed"])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

// ---------------------------------------------------------------------
// repo confirm-update
// ---------------------------------------------------------------------

#[test]
fn repo_confirm_update_fails_for_unknown_repo() {
    let tmp = TempDir::new().expect("tempdir");
    isolated_cmd(&tmp)
        .args(["repo", "confirm-update", "never-existed"])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

// ---------------------------------------------------------------------
// repo keygen
// ---------------------------------------------------------------------

#[test]
fn repo_keygen_writes_minisign_keypair() {
    let tmp = TempDir::new().expect("tempdir");

    isolated_cmd(&tmp)
        .args([
            "repo",
            "keygen",
            "--name",
            "alice",
            "--passphrase",
            "disabled",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created"))
        .stdout(predicate::str::contains("alice.key"))
        .stdout(predicate::str::contains("alice.pub"))
        .stdout(predicate::str::contains("Fingerprint: SHA256:"));

    let key = tmp
        .path()
        .join("config/net.olamaelcu.livtet/keys/repo-keys/alice.key");
    let pubkey = tmp
        .path()
        .join("config/net.olamaelcu.livtet/keys/repo-keys/alice.pub");
    assert!(key.exists(), "repo keygen must write {key:?}");
    assert!(pubkey.exists(), "repo keygen must write {pubkey:?}");

    // Permissions: the .key file should be 0o600 on Unix.
    let mode = fs_err::metadata(&key)
        .expect("key metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, ".key must be mode 0600, got {mode:o}");
}

#[test]
fn repo_keygen_requires_name_flag() {
    // clap requires --name. Omitting it must produce a non-zero
    // exit and a stderr message that mentions the flag.
    let tmp = TempDir::new().expect("tempdir");
    isolated_cmd(&tmp)
        .args(["repo", "keygen", "--passphrase", "disabled"])
        .assert()
        .failure()
        .code(predicate::ne(0))
        .stderr(predicate::str::contains("--name").or(predicate::str::contains("required")));
}

// ---------------------------------------------------------------------
// repo publish
// ---------------------------------------------------------------------

#[test]
fn repo_publish_appends_signed_index_entry() {
    let tmp = TempDir::new().expect("tempdir");
    let repo_dir = common::setup_signed_repo(
        &tmp,
        "test-repo",
        "https://example.com",
        "SHA256:placeholder",
    );
    let ltp = pack_test_plugin(&tmp, "publish-me", "0.1.0");

    // Trust the plugin's signing key via the binary's `plugin trust`
    // subcommand, which writes to the same trust dir the rest of
    // the CLI uses (`~/.config/net.olamaelcu.livtet/keys/signing-keys/`).
    let plugin_keys = tmp.path().join("keys");
    isolated_cmd(&tmp)
        .args([
            "plugin",
            "trust",
            plugin_keys
                .join("publish-author.pub")
                .as_os_str()
                .to_str()
                .unwrap(),
        ])
        .assert()
        .success();

    isolated_cmd(&tmp)
        .args([
            "repo",
            "publish",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
            "--plugin",
            ltp.as_std_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Packed publish-me-0.1.0.ltp"))
        .stdout(predicate::str::contains("appended to index.json"));

    let pool_archive = repo_dir.join("pool/publish-me-0.1.0.ltp");
    assert!(pool_archive.exists(), "{pool_archive:?} must exist");
    let index_text = fs::read_to_string(repo_dir.join("index.json")).expect("read index");
    assert!(
        index_text.contains("publish-me"),
        "index.json must mention publish-me, got: {index_text}"
    );
    let index: Index =
        livtet_plugins::repository::index::parse_index_json(&index_text).expect("parse index");
    let plugin = index.plugins.get("publish-me").expect("plugin entry");
    assert!(plugin.versions.contains_key("0.1.0"));
}

#[test]
fn repo_publish_fails_for_untrusted_archive() {
    let tmp = TempDir::new().expect("tempdir");
    let repo_dir = common::setup_signed_repo(
        &tmp,
        "olamaelcu",
        "https://example.com/olamaelcu",
        "SHA256:00",
    );
    let ltp = pack_test_plugin(&tmp, "untrusted-plugin", "0.1.0");

    // DO NOT trust the plugin's signing key.
    isolated_cmd(&tmp)
        .args([
            "repo",
            "publish",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
            "--plugin",
            ltp.as_std_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

// ---------------------------------------------------------------------
// repo sign
// ---------------------------------------------------------------------

#[test]
fn repo_sign_writes_signed_index() {
    let tmp = TempDir::new().expect("tempdir");
    let repo_dir = common::setup_signed_repo(
        &tmp,
        "test-repo",
        "https://example.com",
        "SHA256:placeholder",
    );

    isolated_cmd(&tmp)
        .args([
            "repo",
            "sign",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Signed"));

    let sig_path = repo_dir.join("index.json.sig");
    assert!(sig_path.exists(), "{sig_path:?} must exist after sign");
    let sig_bytes = fs::read(&sig_path).expect("read sig");
    assert!(
        !sig_bytes.is_empty() && sig_bytes.len() <= 1024,
        "ed25519 signature should be ~64 bytes, got {}",
        sig_bytes.len()
    );
}

#[test]
fn repo_sign_fails_for_missing_repo_key() {
    let tmp = TempDir::new().expect("tempdir");
    let repo_dir = tmp.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    fs::write(repo_dir.join("index.json"), "{}\n").unwrap();

    isolated_cmd(&tmp)
        .args([
            "repo",
            "sign",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(predicate::ne(0));
}

// ---------------------------------------------------------------------
// repo unpublish
// ---------------------------------------------------------------------

#[test]
fn repo_unpublish_specific_version_drops_archive_and_index_entry() {
    let tmp = TempDir::new().expect("tempdir");
    let repo_dir = common::setup_signed_repo(
        &tmp,
        "test-repo",
        "https://example.com",
        "SHA256:placeholder",
    );

    // Pre-populate the pool + index so we don't need to call
    // publish first.
    fs::create_dir_all(repo_dir.join("pool")).unwrap();
    fs::write(repo_dir.join("pool/unpub-me-0.1.0.ltp"), b"FAKE_LTP").unwrap();
    let index = sample_index("unpub-me", "0.1.0", "unpub-me-0.1.0.ltp");
    fs::write(repo_dir.join("index.json"), render_index_json(&index)).unwrap();
    // Sign the index so unpublish (which re-signs) has a valid
    // starting point.
    isolated_cmd(&tmp)
        .args([
            "repo",
            "sign",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
        ])
        .assert()
        .success();

    isolated_cmd(&tmp)
        .args([
            "repo",
            "unpublish",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
            "--plugin",
            "unpub-me",
            "--version",
            "0.1.0",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unpublished unpub-me v0.1.0"));

    let pool_archive = repo_dir.join("pool/unpub-me-0.1.0.ltp");
    assert!(!pool_archive.exists(), "{pool_archive:?} must be gone");
    let index_text = fs::read_to_string(repo_dir.join("index.json")).expect("read index");
    let parsed: Index =
        livtet_plugins::repository::index::parse_index_json(&index_text).expect("parse");
    if let Some(plugin) = parsed.plugins.get("unpub-me") {
        assert!(
            !plugin.versions.contains_key("0.1.0"),
            "0.1.0 must be removed from index"
        );
    }
}

#[test]
fn repo_unpublish_for_missing_version_is_noop_success() {
    // The publisher's `unpublish_version` is documented as a
    // no-op when the requested (plugin, version) tuple is not in
    // the index — removing a non-existent version is idempotent,
    // not an error. Pin the contract here: the binary exits 0
    // and prints the "Unpublished ..." line.
    let tmp = TempDir::new().expect("tempdir");
    let repo_dir = common::setup_signed_repo(
        &tmp,
        "test-repo",
        "https://example.com",
        "SHA256:placeholder",
    );
    isolated_cmd(&tmp)
        .args([
            "repo",
            "sign",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
        ])
        .assert()
        .success();

    isolated_cmd(&tmp)
        .args([
            "repo",
            "unpublish",
            "--repo-dir",
            repo_dir.as_os_str().to_str().unwrap(),
            "--plugin",
            "ghost",
            "--version",
            "9.9.9",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unpublished ghost v9.9.9"));
}
