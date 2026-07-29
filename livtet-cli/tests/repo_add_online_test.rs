//! Online T19 (HTTP fetch) tests for `livtet repo add` / `confirm-add`.
//!
//! Covers the `RepositoryClient::add` (online TOFU) and `confirm_add`
//! (verify + cache) flows against an in-process mock HTTP server. No
//! outbound network required.

mod common;

use camino::Utf8Path;
use common::{TestContext, sample_index, setup_test_env, write_signed_repo, *};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use fs_err as fs;
use livtet_plugins::{
    keys::{TrustStore, fingerprint},
    repository::{
        client::RepositoryClient,
        error::RepositoryError,
        hmac::HmacKey,
        index::render_index_json,
        repo_toml::{RepoSection, RepoToml, SigningSection, render_repo_toml},
    },
    types::RepositoryAddResult,
};
use rand::{Rng as _, rng};
use tokio::net::TcpListener;

fn make_client(ctx: &TestContext) -> (RepositoryClient, SigningKey, VerifyingKey) {
    let cache_dir = ctx.tmp.path().join("cache");
    let config_dir = ctx.tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();

    let client = RepositoryClient::new(cache_dir, config_dir, HmacKey::from_bytes([0x77u8; 32]));
    (client, signing_key, verifying_key)
}

#[tokio::test]
async fn repo_add_online_returns_needs_tofu_for_known_repo() {
    let ctx = setup_test_env().await;
    let (client, signing_key, verifying_key) = make_client(&ctx);
    let server_root = ctx.tmp.path().join("www");
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

    let result = client
        .add(&ctx.server.base_url)
        .await
        .expect("add should succeed");
    match result {
        RepositoryAddResult::NeedsTofuConfirmation {
            name,
            url,
            fingerprint,
        } => {
            assert_eq!(name, "olamaelcu");
            assert_eq!(url, ctx.server.base_url);
            assert!(fingerprint.starts_with("SHA256:"));
        }
        other => panic!("expected NeedsTofuConfirmation, got {other:?}"),
    }
}

#[tokio::test]
async fn repo_add_online_fails_when_server_unreachable() {
    let ctx = setup_test_env().await;
    let (client, _signing_key, _verifying_key) = make_client(&ctx);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let dead_url = format!("http://{addr}");
    let result = client.add(&dead_url).await;
    match result {
        Err(RepositoryError::Network(_)) => {}
        other => panic!("expected Network error, got {other:?}"),
    }
}

#[tokio::test]
async fn repo_add_online_fails_when_repo_toml_404() {
    let ctx = setup_test_env().await;
    let (client, _signing_key, _verifying_key) = make_client(&ctx);

    let result = client.add(&ctx.server.base_url).await;
    match result {
        Err(RepositoryError::Http { status, .. }) => assert_eq!(status, 404),
        other => panic!("expected Http 404, got {other:?}"),
    }
}

#[tokio::test]
async fn repo_add_online_fails_when_repo_toml_malformed() {
    let ctx = setup_test_env().await;
    let server_root = ctx.tmp.path().join("www");
    fs::write(
        server_root.join("repo.toml"),
        b"this is not valid toml ==== [[[",
    )
    .unwrap();

    let (client, _signing_key, _verifying_key) = make_client(&ctx);

    let result = client.add(&ctx.server.base_url).await;
    match result {
        Err(RepositoryError::IndexParse(_)) => {}
        other => panic!("expected IndexParse, got {other:?}"),
    }
}

#[tokio::test]
async fn repo_confirm_add_caches_repo_toml_and_index() {
    let ctx = setup_test_env().await;
    let (client, signing_key, verifying_key) = make_client(&ctx);
    let cache_root = ctx.tmp.path().join("cache");
    let config_root = ctx.tmp.path().join("config");
    let server_root = ctx.tmp.path().join("www");

    let plugin_id = "cache-me";
    let plugin_version = "0.1.0";
    let archive_name = "cache-me-0.1.0.ltp";
    write_signed_repo(
        &server_root,
        "olamaelcu",
        &ctx.server.base_url,
        &signing_key,
        &verifying_key,
        plugin_id,
        plugin_version,
        archive_name,
    );

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", verifying_key)
        .expect("add_user_key");

    client
        .confirm_add(&ctx.server.base_url, &trust)
        .await
        .expect("confirm_add should succeed");

    let cached = cache_root.join("olamaelcu");
    let cached_toml = cached.join("repo.toml");
    let cached_index = cached.join("index.json");
    let cached_sig = cached.join("index.json.sig");

    assert!(
        cached_toml.as_std_path().exists(),
        "repo.toml must be cached at {}",
        cached_toml
    );
    assert!(
        cached_index.as_std_path().exists(),
        "index.json must be cached at {}",
        cached_index
    );
    assert!(
        cached_sig.as_std_path().exists(),
        "index.json.sig must be cached at {}",
        cached_sig
    );

    let parsed = livtet_plugins::repository::repo_toml::parse_repo_toml(
        &fs::read_to_string(cached_toml.as_std_path()).unwrap(),
    )
    .expect("cached repo.toml must parse");
    assert_eq!(parsed.repo.name, "olamaelcu");

    let config_repos = config_root.join("repositories.toml");
    assert!(config_repos.as_std_path().exists());
}

#[tokio::test]
async fn repo_confirm_add_rejects_bad_signature() {
    let ctx = setup_test_env().await;
    let (client, _signing_key, verifying_key) = make_client(&ctx);
    let server_root = ctx.tmp.path().join("www");

    let repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: String::new(),
            description: None,
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint: fingerprint(&verifying_key),
        },
    };
    let mut toml = repo_toml.clone();
    toml.repo.url = ctx.server.base_url.clone();
    fs::write(server_root.join("repo.toml"), render_repo_toml(&toml)).unwrap();

    let index = sample_index("plugin", "0.1.0", "plugin-0.1.0.ltp");
    fs::write(server_root.join("index.json"), render_index_json(&index)).unwrap();
    fs::write(server_root.join("index.json.sig"), [0u8; 64]).unwrap();

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", verifying_key)
        .expect("add_user_key");

    let result = client.confirm_add(&ctx.server.base_url, &trust).await;
    match result {
        Err(RepositoryError::BadIndexSignature) => {}
        other => panic!("expected BadIndexSignature, got {other:?}"),
    }
}

#[tokio::test]
async fn repo_confirm_add_rejects_missing_trusted_key() {
    let ctx = setup_test_env().await;
    let (client, _signing_key, _verifying_key) = make_client(&ctx);
    let server_root = ctx.tmp.path().join("www");

    let repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: String::new(),
            description: None,
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint:
                "SHA256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
        },
    };
    let mut toml = repo_toml.clone();
    toml.repo.url = ctx.server.base_url.clone();
    fs::write(server_root.join("repo.toml"), render_repo_toml(&toml)).unwrap();

    let trust = TrustStore::empty();
    let result = client.confirm_add(&ctx.server.base_url, &trust).await;
    match result {
        Err(RepositoryError::Keyring(_)) => {}
        other => panic!("expected Keyring error, got {other:?}"),
    }
}

#[tokio::test]
async fn repo_confirm_add_rejects_duplicate_url() {
    let ctx = setup_test_env().await;
    let (client, signing_key, verifying_key) = make_client(&ctx);
    let server_root = ctx.tmp.path().join("www");
    write_signed_repo(
        &server_root,
        "olamaelcu",
        &ctx.server.base_url,
        &signing_key,
        &verifying_key,
        "dup",
        "0.1.0",
        "dup-0.1.0.ltp",
    );

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", verifying_key)
        .expect("add_user_key");

    client
        .confirm_add(&ctx.server.base_url, &trust)
        .await
        .expect("first confirm_add should succeed");

    let result = client.confirm_add(&ctx.server.base_url, &trust).await;
    match result {
        Err(RepositoryError::AlreadyAdded(_)) => {}
        other => panic!("expected AlreadyAdded, got {other:?}"),
    }
}
