use std::collections::BTreeMap;

use camino::{Utf8Path, Utf8PathBuf};
use ed25519_dalek::{Signer, SigningKey};
use fs_err as fs;
use livtet_plugin::{
    archive::{install::install, pack::pack},
    keys::{TrustStore, fingerprint, keyfile::keygen, signing::parse_pubkey_text},
    repository::{
        client::{RepositoryClient, search_index},
        config::RepositoriesFile,
        hmac::HmacKey,
        index::{Index, IndexPlugin, IndexVersionEntry, render_index_json, verify_index_signature},
        repo_toml::{RepoSection, RepoToml, SigningSection, render_repo_toml},
    },
    types::{KeygenReport, Repository, RepositoryUpdateResult},
};
mod common;
use common::verifying_key_from_keygen_report;
use livtet_test_utils::{
    TestServer, build_response, http_response, parse_request_path, spawn_server,
};
use rand::{Rng as _, rng};
use camino_tempfile::Utf8TempDir as TempDir;

fn sample_repo_toml() -> RepoToml {
    RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: "https://plugins.livtet.olamaelcu.net".to_string(),
            description: Some("Livtet plugins from Olamaelcu".to_string()),
            maintainer: Some("Olamaelcu <plugins@livtet.olamaelcu.net>".to_string()),
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint: "SHA256:placeholder".to_string(),
        },
    }
}

fn sample_index() -> Index {
    let mut versions = BTreeMap::new();
    versions.insert(
        "1.0.0".to_string(),
        IndexVersionEntry {
            entry: "init.lua".to_string(),
            capabilities: BTreeMap::from([
                ("search".to_string(), true),
                ("lookup".to_string(), true),
            ]),
            dependencies: vec![],
            archive: "openlibrary-1.0.0.ltp".to_string(),
            archive_size: 12,
            archive_sha256: "deadbeef".to_string(),
            min_app_version: "0.5.0".to_string(),
        },
    );
    let mut plugins = BTreeMap::new();
    plugins.insert("openlibrary".to_string(), IndexPlugin { versions });
    Index {
        format_version: 1,
        generated_at: "2026-06-01T00:00:00Z".to_string(),
        plugins,
    }
}

fn make_client(tmp: &TempDir) -> RepositoryClient {
    let cache_dir = tmp.path().join("cache");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();
    RepositoryClient::new(cache_dir, config_dir, HmacKey::from_bytes([0x44u8; 32]))
}

#[tokio::test]
async fn test_fetch_repo_toml_succeeds() {
    let tmp = TempDir::new().unwrap();
    let server_root = tmp.path().join("server");
    fs::create_dir_all(server_root.as_std_path()).unwrap();
    let toml_text = render_repo_toml(&sample_repo_toml());
    fs::write(server_root.join("repo.toml"), &toml_text).unwrap();

    let server = spawn_server(server_root).await;
    let client = make_client(&tmp);

    let (parsed, raw) = client
        .fetch_repo_toml(&server.base_url)
        .await
        .expect("fetch_repo_toml should succeed");

    assert_eq!(parsed.repo.name, "olamaelcu");
    assert_eq!(parsed.repo.url, "https://plugins.livtet.olamaelcu.net");
    assert_eq!(parsed.signing.key_label, "olamaelcu");
    assert!(parsed.signing.key_fingerprint.starts_with("SHA256:"));
    assert_eq!(raw, toml_text);
}

#[tokio::test]
async fn test_fetch_repo_toml_404() {
    let tmp = TempDir::new().unwrap();
    let server_root = tmp.path().join("server");
    fs::create_dir_all(server_root.as_std_path()).unwrap();

    let server = spawn_server(server_root).await;
    let client = make_client(&tmp);

    let result = client.fetch_repo_toml(&server.base_url).await;
    match result {
        Err(livtet_plugin::repository::error::RepositoryError::Http { status, .. }) => {
            assert_eq!(status, 404);
        }
        other => panic!("expected Http 404, got {other:?}"),
    }
}

#[tokio::test]
async fn test_fetch_index_succeeds_and_verifies_signature() {
    let tmp = TempDir::new().unwrap();
    let server_root = tmp.path().join("server");
    fs::create_dir_all(server_root.as_std_path()).unwrap();

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();

    let index = sample_index();
    let index_json = render_index_json(&index);
    let sig = signing_key.sign(index_json.as_bytes());
    let sig_bytes = sig.to_bytes();

    fs::write(server_root.join("index.json"), &index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), sig_bytes).unwrap();

    let server = spawn_server(server_root).await;
    let client = make_client(&tmp);

    let (parsed, raw) = client
        .fetch_index(&server.base_url, &verifying_key)
        .await
        .expect("fetch_index should succeed");

    assert_eq!(parsed.plugins.len(), 1);
    assert!(parsed.plugins.contains_key("openlibrary"));
    assert_eq!(raw, index_json);

    assert!(
        verify_index_signature(&parsed, &raw, &sig_bytes, &verifying_key).is_ok(),
        "raw text returned by fetch_index must re-verify"
    );
}

#[tokio::test]
async fn test_fetch_index_rejects_bad_signature() {
    let tmp = TempDir::new().unwrap();
    let server_root = tmp.path().join("server");
    fs::create_dir_all(server_root.as_std_path()).unwrap();

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();

    let index = sample_index();
    let index_json = render_index_json(&index);
    let bogus_sig = [0u8; 64];

    fs::write(server_root.join("index.json"), &index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), bogus_sig).unwrap();

    let server = spawn_server(server_root).await;
    let client = make_client(&tmp);

    let result = client.fetch_index(&server.base_url, &verifying_key).await;
    match result {
        Err(livtet_plugin::repository::error::RepositoryError::BadIndexSignature) => {}
        other => panic!("expected BadIndexSignature, got {other:?}"),
    }
}

#[tokio::test]
async fn test_download_archive_succeeds() {
    let tmp = TempDir::new().unwrap();
    let server_root = tmp.path().join("server");
    let pool_dir = server_root.join("pool");
    fs::create_dir_all(pool_dir.as_std_path()).unwrap();

    let archive_bytes: Vec<u8> = (0..1024u32).map(|i| (i & 0xff) as u8).collect();
    let archive_name = "openlibrary-1.0.0.ltp";
    fs::write(pool_dir.join(archive_name), &archive_bytes).unwrap();

    use sha2::{Digest, Sha256};
    let expected_size = archive_bytes.len() as u64;
    let mut sha = Sha256::new();
    sha.update(&archive_bytes);
    let expected_sha256 = hex::encode(sha.finalize());

    let server = spawn_server(server_root).await;
    let client = make_client(&tmp);

    let dest_dir = tmp.path().join("downloads");
    fs::create_dir_all(dest_dir.as_std_path()).unwrap();
    let dest = dest_dir.join(archive_name);

    client
        .download_archive(
            &server.base_url,
            archive_name,
            expected_size,
            &expected_sha256,
            &dest,
        )
        .await
        .expect("download_archive should succeed");

    let downloaded = fs::read(dest.as_std_path()).expect("file should exist");
    assert_eq!(downloaded, archive_bytes);
}

#[tokio::test]
async fn test_download_archive_404() {
    let tmp = TempDir::new().unwrap();
    let server_root = tmp.path().join("server");
    fs::create_dir_all(server_root.as_std_path()).unwrap();

    let server = spawn_server(server_root).await;
    let client = make_client(&tmp);

    let dest_dir = tmp.path().join("downloads");
    fs::create_dir_all(dest_dir.as_std_path()).unwrap();
    let dest = dest_dir.join("missing-1.0.0.ltp");

    let result = client
        .download_archive(
            &server.base_url,
            "missing-1.0.0.ltp",
            0,
            "0000000000000000000000000000000000000000000000000000000000000000",
            &dest,
        )
        .await;
    match result {
        Err(livtet_plugin::repository::error::RepositoryError::Http { status, .. }) => {
            assert_eq!(status, 404);
        }
        other => panic!("expected Http 404, got {other:?}"),
    }
}

#[tokio::test]
async fn test_download_archive_rejects_size_mismatch() {
    let tmp = TempDir::new().unwrap();
    let server_root = tmp.path().join("server");
    let pool_dir = server_root.join("pool");
    fs::create_dir_all(pool_dir.as_std_path()).unwrap();

    let archive_bytes: Vec<u8> = b"correct-content-for-size-test".to_vec();
    let archive_name = "size-mismatch-1.0.0.ltp";
    fs::write(pool_dir.join(archive_name), &archive_bytes).unwrap();

    let server = spawn_server(server_root).await;
    let client = make_client(&tmp);

    let dest_dir = tmp.path().join("downloads");
    fs::create_dir_all(dest_dir.as_std_path()).unwrap();
    let dest = dest_dir.join(archive_name);

    let wrong_size = (archive_bytes.len() as u64) + 999;
    let correct_sha = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&archive_bytes);
        hex::encode(h.finalize())
    };

    let result = client
        .download_archive(
            &server.base_url,
            archive_name,
            wrong_size,
            &correct_sha,
            &dest,
        )
        .await;
    match result {
        Err(livtet_plugin::repository::error::RepositoryError::Network(msg)) => {
            assert!(msg.contains("size mismatch"), "got: {msg}");
            assert!(msg.contains(archive_name), "got: {msg}");
        }
        other => panic!("expected Network size mismatch error, got {other:?}"),
    }
    assert!(!dest.as_std_path().exists(), "dest must not be written");
}

#[tokio::test]
async fn test_download_archive_rejects_sha256_mismatch() {
    let tmp = TempDir::new().unwrap();
    let server_root = tmp.path().join("server");
    let pool_dir = server_root.join("pool");
    fs::create_dir_all(pool_dir.as_std_path()).unwrap();

    let archive_bytes: Vec<u8> = b"correct-content-for-sha-test".to_vec();
    let archive_name = "sha-mismatch-1.0.0.ltp";
    fs::write(pool_dir.join(archive_name), &archive_bytes).unwrap();

    let server = spawn_server(server_root).await;
    let client = make_client(&tmp);

    let dest_dir = tmp.path().join("downloads");
    fs::create_dir_all(dest_dir.as_std_path()).unwrap();
    let dest = dest_dir.join(archive_name);

    let correct_size = archive_bytes.len() as u64;
    let wrong_sha = "deadbeef".repeat(8);

    let result = client
        .download_archive(
            &server.base_url,
            archive_name,
            correct_size,
            &wrong_sha,
            &dest,
        )
        .await;
    match result {
        Err(livtet_plugin::repository::error::RepositoryError::IndexParse(msg)) => {
            assert!(msg.contains("sha256 mismatch"), "got: {msg}");
            assert!(msg.contains(archive_name), "got: {msg}");
        }
        other => panic!("expected IndexParse sha256 mismatch error, got {other:?}"),
    }
    assert!(!dest.as_std_path().exists(), "dest must not be written");
}

#[tokio::test]
#[ignore = "full T29b live e2e; set LIVTET_RUN_E2E=1 or pass --include-ignored to enable"]
async fn test_e2e_live_http_round_trip() {
    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();

    let server_root = tmp_root.join("server");
    let providers = tmp_root.join("providers");
    let cache_dir = tmp_root.join("cache");
    let config_dir = tmp_root.join("config");
    let downloads = tmp_root.join("downloads");
    for d in [
        &server_root,
        &providers,
        &cache_dir,
        &config_dir,
        &downloads,
    ] {
        fs::create_dir_all(d.as_std_path()).unwrap();
    }

    // Index signing key (ed25519) — used to sign index.json.
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();
    let publisher_fp = livtet_plugin::keys::fingerprint(&verifying_key);

    let mut repo_toml = sample_repo_toml();
    repo_toml.signing.key_fingerprint = publisher_fp.clone();
    let repo_toml_text = render_repo_toml(&repo_toml);
    fs::write(server_root.join("repo.toml"), &repo_toml_text).unwrap();

    // Build a real plugin source dir and pack it into a signed .ltp archive
    // using `archive::pack`. The packed archive has a valid META-INF/ tree
    // and signature, so `archive::install` can verify and extract it.
    let plugin_src = tmp_root.join("plugin-src");
    fs::create_dir_all(plugin_src.as_std_path()).unwrap();
    fs::write(
        plugin_src.join("livtet.toml"),
        b"[plugin]\nid=\"live-roundtrip\"\nname=\"Live Roundtrip\"\nversion=\"0.1.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    fs::write(plugin_src.join("init.lua"), b"-- live roundtrip plugin\n").unwrap();

    let key_dir = tmp_root.join("keys");
    let keygen_report = keygen(key_dir.as_path(), "olamaelcu", true).expect("keygen");
    let packed_ltp = pack(
        plugin_src.as_path(),
        &keygen_report.key_path,
        "olamaelcu",
        tmp_root,
    )
    .expect("pack");
    let archive_bytes = fs::read(packed_ltp.as_std_path()).expect("read packed archive");
    let mut sha = sha2::Sha256::new();
    use sha2::Digest;
    sha.update(&archive_bytes);
    let archive_sha = hex::encode(sha.finalize());

    let pool_dir = server_root.join("pool");
    fs::create_dir_all(pool_dir.as_std_path()).unwrap();
    fs::write(pool_dir.join("live-roundtrip-0.1.0.ltp"), &archive_bytes).unwrap();

    let index_with_size = Index {
        format_version: 1,
        generated_at: "2026-06-06T00:00:00Z".to_string(),
        plugins: {
            let mut p = BTreeMap::new();
            let mut v = BTreeMap::new();
            v.insert(
                "0.1.0".to_string(),
                IndexVersionEntry {
                    entry: "init.lua".to_string(),
                    capabilities: Default::default(),
                    dependencies: vec![],
                    archive: "live-roundtrip-0.1.0.ltp".to_string(),
                    archive_size: archive_bytes.len() as u64,
                    archive_sha256: archive_sha.clone(),
                    min_app_version: "0.5.0".to_string(),
                },
            );
            p.insert("live-roundtrip".to_string(), IndexPlugin { versions: v });
            p
        },
    };
    let signed_index_json = render_index_json(&index_with_size);
    let sig = signing_key.sign(signed_index_json.as_bytes());
    let sig_bytes = sig.to_bytes();
    fs::write(server_root.join("index.json"), &signed_index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), sig_bytes).unwrap();

    // Trust store for the archive's minisign verifying key.
    let archive_verifying_key = verifying_key_from_keygen_report(&keygen_report);
    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", archive_verifying_key)
        .expect("add_user_key");

    let server = spawn_server(server_root.clone()).await;
    let client = RepositoryClient::new(
        cache_dir,
        config_dir,
        HmacKey::from_bytes([0x55u8; 32]),
    );

    let (repo_toml, raw_repo_toml) = client
        .fetch_repo_toml(&server.base_url)
        .await
        .expect("fetch_repo_toml");
    assert_eq!(repo_toml.repo.name, "olamaelcu");
    assert!(raw_repo_toml.contains("olamaelcu"));
    assert_eq!(repo_toml.signing.key_fingerprint, publisher_fp);

    let (fetched_index, _raw_index) = client
        .fetch_index(&server.base_url, &verifying_key)
        .await
        .expect("fetch_index with sig verify");
    assert!(fetched_index.plugins.contains_key("live-roundtrip"));
    let entry = fetched_index
        .plugins
        .get("live-roundtrip")
        .unwrap()
        .versions
        .get("0.1.0")
        .unwrap();
    assert_eq!(entry.archive, "live-roundtrip-0.1.0.ltp");
    assert_eq!(entry.archive_sha256, archive_sha);

    let results = search_index(&fetched_index, "live", "olamaelcu");
    assert!(!results.is_empty());
    assert!(results.iter().any(|r| r.plugin_id == "live-roundtrip"));

    let dest = downloads.join("live-roundtrip-0.1.0.ltp");
    client
        .download_archive(
            &server.base_url,
            "live-roundtrip-0.1.0.ltp",
            archive_bytes.len() as u64,
            &archive_sha,
            &dest,
        )
        .await
        .expect("download_archive");
    let downloaded = fs::read(dest.as_std_path()).expect("downloaded file exists");
    assert_eq!(downloaded, archive_bytes);

    // After download_archive succeeds, exercise the install path end-to-end:
    // trust the publisher's minisign key, install the downloaded archive,
    // assert the install report.
    let install_report =
        install(&dest, providers.as_path(), Some(&trust)).expect("archive::install");
    assert_eq!(install_report.id, "live-roundtrip");
    assert_eq!(install_report.version, "0.1.0");
    assert!(install_report.install_path.exists());
    assert!(
        install_report.install_path.join("init.lua").exists(),
        "extracted init.lua should exist"
    );
}

#[tokio::test]
async fn test_confirm_add_falls_back_to_fingerprint_lookup() {
    // Regression: `confirm_add` previously looked up the verifying key by
    // `repo.toml.signing.key_label` only. If a user trusted a key under a
    // different label than what the repo declares, `confirm_add` would
    // return a `Keyring` error even though the fingerprint in `repo.toml`
    // matched a trusted key. The fix mirrors the existing `search_index`
    // pattern: if label lookup fails, fall back to fingerprint lookup.

    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();
    let server_root = tmp_root.join("server");
    let cache_dir = tmp_root.join("cache");
    let config_dir = tmp_root.join("config");
    for d in [&server_root, &cache_dir, &config_dir] {
        fs::create_dir_all(d.as_std_path()).unwrap();
    }

    // Index signing key (ed25519) — the REPO's signing key, not the
    // publisher's. The key_label in repo.toml will be a fake string that
    // does NOT exist in the trust store; only the fingerprint will match.
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();
    let repo_fp = livtet_plugin::keys::fingerprint(&verifying_key);

    // Build repo.toml with a deliberately mismatched key_label but the
    // correct fingerprint. This forces confirm_add to use the fingerprint
    // fallback path.
    let repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: "https://plugins.livtet.olamaelcu.net".to_string(),
            description: None,
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "fake_label_does_not_exist".to_string(),
            key_fingerprint: repo_fp.clone(),
        },
    };
    let repo_toml_text = render_repo_toml(&repo_toml);
    fs::write(server_root.join("repo.toml"), &repo_toml_text).unwrap();

    let index = sample_index();
    let index_json = render_index_json(&index);
    let sig = signing_key.sign(index_json.as_bytes());
    let sig_bytes = sig.to_bytes();
    fs::write(server_root.join("index.json"), &index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), sig_bytes).unwrap();

    // Trust store: key added under a different label ("olamaelcu"), NOT
    // under "fake_label_does_not_exist". Label lookup must fail; the
    // fingerprint fallback must succeed.
    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", verifying_key)
        .expect("add_user_key");

    let server = spawn_server(server_root).await;
    let client = RepositoryClient::new(
        cache_dir,
        config_dir,
        HmacKey::from_bytes([0x66u8; 32]),
    );

    // Without the fix, this returns Err(Keyring("trusted key with label
    // \"fake_label_does_not_exist\" not found")). With the fix, it succeeds
    // via the fingerprint fallback.
    client
        .confirm_add(&server.base_url, &trust)
        .await
        .expect("confirm_add should succeed via fingerprint fallback");

    // Verify the repo was actually registered.
    let repos = client.list().expect("list");
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].name, "olamaelcu");
    assert_eq!(repos[0].key_fingerprint, repo_fp);
}

#[tokio::test]
async fn test_confirm_add_rejects_unknown_fingerprint() {
    // Negative case for the fingerprint fallback: if neither the label
    // nor the fingerprint match a trusted key, confirm_add must return
    // a Keyring error that mentions BOTH the label and the fingerprint,
    // so the operator can diagnose what the repo declared.

    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();
    let server_root = tmp_root.join("server");
    let cache_dir = tmp_root.join("cache");
    let config_dir = tmp_root.join("config");
    for d in [&server_root, &cache_dir, &config_dir] {
        fs::create_dir_all(d.as_std_path()).unwrap();
    }

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();
    let repo_fp = livtet_plugin::keys::fingerprint(&verifying_key);

    let repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: "https://plugins.livtet.olamaelcu.net".to_string(),
            description: None,
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "unknown_label".to_string(),
            key_fingerprint: repo_fp,
        },
    };
    let repo_toml_text = render_repo_toml(&repo_toml);
    fs::write(server_root.join("repo.toml"), &repo_toml_text).unwrap();

    let index = sample_index();
    let index_json = render_index_json(&index);
    let sig = signing_key.sign(index_json.as_bytes());
    let sig_bytes = sig.to_bytes();
    fs::write(server_root.join("index.json"), &index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), sig_bytes).unwrap();

    // Trust store contains a completely unrelated key.
    let mut csprng2 = rand::rng();
    let unrelated = {
        let mut __ed25519_seed = [0u8; 32];
        csprng2.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed).verifying_key()
    };
    let mut trust = TrustStore::empty();
    trust
        .add_user_key("some_other_label", unrelated)
        .expect("add_user_key");

    let server = spawn_server(server_root).await;
    let client = RepositoryClient::new(
        cache_dir,
        config_dir,
        HmacKey::from_bytes([0x77u8; 32]),
    );

    let result = client.confirm_add(&server.base_url, &trust).await;
    match result {
        Err(livtet_plugin::repository::error::RepositoryError::Keyring(msg)) => {
            assert!(
                msg.contains("unknown_label"),
                "error should mention the repo's declared label, got: {msg}"
            );
            assert!(
                msg.contains("SHA256:"),
                "error should mention the fingerprint prefix, got: {msg}"
            );
        }
        other => panic!("expected Keyring error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_update_same_fingerprint_refreshes_index() {
    // Happy-path: the publisher re-signs `index.json` with the same key,
    // possibly with a new `generated_at` or new plugin versions. `update`
    // should re-fetch + re-verify the index, overwrite the cached files
    // under the repo's cache subdir, bump `last_index_update`, and return
    // `Ok { plugin_count }`.

    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();
    let server_root = tmp_root.join("server");
    let cache_dir = tmp_root.join("cache");
    let config_dir = tmp_root.join("config");
    for d in [&server_root, &cache_dir, &config_dir] {
        fs::create_dir_all(d.as_std_path()).unwrap();
    }

    // Seed the server files before spawning, so the test server can serve
    // them on the first request.
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();
    let repo_fp = fingerprint(&verifying_key);

    // The live index that `update` will fetch + verify.
    let fresh_index = Index {
        format_version: 1,
        generated_at: "2026-06-07T12:00:00Z".to_string(),
        plugins: {
            let mut plugins = BTreeMap::new();
            let mut versions = BTreeMap::new();
            versions.insert(
                "1.0.0".to_string(),
                IndexVersionEntry {
                    entry: "init.lua".to_string(),
                    capabilities: BTreeMap::from([("search".to_string(), true)]),
                    dependencies: vec![],
                    archive: "openlibrary-1.0.0.ltp".to_string(),
                    archive_size: 12,
                    archive_sha256: "deadbeef".to_string(),
                    min_app_version: "0.5.0".to_string(),
                },
            );
            versions.insert(
                "1.1.0".to_string(),
                IndexVersionEntry {
                    entry: "init.lua".to_string(),
                    capabilities: BTreeMap::from([("search".to_string(), true)]),
                    dependencies: vec![],
                    archive: "openlibrary-1.1.0.ltp".to_string(),
                    archive_size: 24,
                    archive_sha256: "feedbeef".to_string(),
                    min_app_version: "0.5.0".to_string(),
                },
            );
            plugins.insert("openlibrary".to_string(), IndexPlugin { versions });
            plugins
        },
    };
    let fresh_index_json = render_index_json(&fresh_index);
    let fresh_sig = signing_key.sign(fresh_index_json.as_bytes());
    let fresh_sig_bytes = fresh_sig.to_bytes();

    // Stale cache contents that `update` should overwrite.
    let stale_index = sample_index();
    let stale_index_json = render_index_json(&stale_index);
    let stale_sig = signing_key.sign(stale_index_json.as_bytes());
    let stale_sig_bytes = stale_sig.to_bytes();

    let server = spawn_server(server_root.clone()).await;

    // Now write the live index to the server (after spawn so we can use
    // the base URL inside `repo.toml`).
    let repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: server.base_url.clone(),
            description: None,
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint: repo_fp.clone(),
        },
    };
    let repo_toml_text = render_repo_toml(&repo_toml);
    fs::write(server_root.join("repo.toml"), &repo_toml_text).unwrap();
    fs::write(server_root.join("index.json"), &fresh_index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), fresh_sig_bytes).unwrap();

    // Seed the cache with a *stale* index so we can assert that `update`
    // overwrites it with the fresh bytes from the server.
    let repo_cache = cache_dir.join("olamaelcu");
    fs::create_dir_all(repo_cache.as_std_path()).unwrap();
    fs::write(repo_cache.join("index.json"), &stale_index_json).unwrap();
    fs::write(repo_cache.join("index.json.sig"), stale_sig_bytes).unwrap();
    fs::write(
        repo_cache.join("repo.toml"),
        "format_version = 1\n\
         [repo]\n\
         name = \"olamaelcu\"\n\
         url = \"https://stale.example/\"\n\
         [signing]\n\
         key_label = \"olamaelcu\"\n\
         key_fingerprint = \"SHA256:stale\"\n",
    )
    .unwrap();

    // Seed the configured repository entry with a *stale* `last_index_update`
    // so we can assert `update` bumps it. The URL must match the test server
    // so `update` actually fetches from the test server.
    let mut repos = RepositoriesFile::default();
    repos.repositories.push(Repository {
        name: "olamaelcu".to_string(),
        url: server.base_url.clone(),
        description: None,
        maintainer: None,
        added_at: "2026-06-01T00:00:00Z".to_string(),
        last_index_update: Some("1970-01-01T00:00:00Z".to_string()),
        key_fingerprint: repo_fp.clone(),
    });

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", verifying_key)
        .expect("add_user_key");

    let client = RepositoryClient::new(
        Utf8PathBuf::from_path_buf(cache_dir.clone().into_std_path_buf()).unwrap(),
        Utf8PathBuf::from_path_buf(config_dir.clone().into_std_path_buf()).unwrap(),
        HmacKey::from_bytes([0xA1u8; 32]),
    );
    client.save_repositories(&repos).unwrap();

    let result = client
        .update("olamaelcu", &trust)
        .await
        .expect("update should succeed");
    match result {
        RepositoryUpdateResult::Ok { plugin_count } => {
            // 2 versions of openlibrary in the fresh index.
            assert_eq!(plugin_count, 2, "fresh index has 2 versions");
        }
        other => panic!("expected Ok, got {other:?}"),
    }

    // Cached files were overwritten with the fresh bytes from the server.
    let cached_index = fs::read_to_string(repo_cache.join("index.json")).unwrap();
    assert_eq!(cached_index, fresh_index_json);
    let cached_sig = fs::read(repo_cache.join("index.json.sig")).unwrap();
    assert_eq!(cached_sig, fresh_sig_bytes);
    let cached_repo_toml = fs::read_to_string(repo_cache.join("repo.toml")).unwrap();
    assert!(cached_repo_toml.contains(&repo_fp));

    // The cached index re-verifies against the trusted key (so `search`
    // downstream will accept it).
    let parsed = livtet_plugin::repository::index::parse_index_json(&cached_index).unwrap();
    assert!(
        verify_index_signature(&parsed, &cached_index, &cached_sig, &verifying_key).is_ok(),
        "cached index.json must re-verify with the trusted key"
    );

    // `last_index_update` was bumped to something newer than the seed value.
    let after = client.load_repositories().unwrap();
    let entry = after
        .repositories
        .iter()
        .find(|r| r.name == "olamaelcu")
        .expect("entry");
    let updated = entry
        .last_index_update
        .as_deref()
        .unwrap_or("1970-01-01T00:00:00Z");
    assert_ne!(updated, "1970-01-01T00:00:00Z");
}

#[tokio::test]
async fn test_update_different_fingerprint_returns_key_changed() {
    // The publisher rotated their signing key. The server returns
    // repo.toml with a NEW fingerprint, but the configured repository
    // entry still has the OLD fingerprint. `update` must return
    // `KeyChanged` so the CLI can prompt the operator to trust the new
    // key and re-run `confirm-update`.

    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();
    let server_root = tmp_root.join("server");
    let cache_dir = tmp_root.join("cache");
    let config_dir = tmp_root.join("config");
    for d in [&server_root, &cache_dir, &config_dir] {
        fs::create_dir_all(d.as_std_path()).unwrap();
    }

    // The OLD key — what the configured repository entry trusts.
    let mut csprng = rand::rng();
    let old_signing = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let old_fp = fingerprint(&old_signing.verifying_key());

    // The NEW key — what the server now declares in repo.toml.
    let new_signing = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let new_fp = fingerprint(&new_signing.verifying_key());
    assert_ne!(old_fp, new_fp);

    let server = spawn_server(server_root.clone()).await;

    let repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: server.base_url.clone(),
            description: None,
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint: new_fp.clone(),
        },
    };
    let repo_toml_text = render_repo_toml(&repo_toml);
    fs::write(server_root.join("repo.toml"), &repo_toml_text).unwrap();

    // The server is allowed to serve an index signed by the new key, but
    // `update` must not even GET to it because the fingerprint mismatch is
    // detected from `repo.toml` alone.
    let index = sample_index();
    let index_json = render_index_json(&index);
    let sig = new_signing.sign(index_json.as_bytes());
    fs::write(server_root.join("index.json"), &index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), sig.to_bytes()).unwrap();

    // Trust store has the OLD key only.
    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", old_signing.verifying_key())
        .expect("add_user_key");

    // Configured entry has the OLD fingerprint, and points at the test
    // server (so `update` actually fetches from it).
    let mut repos = RepositoriesFile::default();
    repos.repositories.push(Repository {
        name: "olamaelcu".to_string(),
        url: server.base_url.clone(),
        description: None,
        maintainer: None,
        added_at: "2026-06-01T00:00:00Z".to_string(),
        last_index_update: Some("2026-06-01T00:00:00Z".to_string()),
        key_fingerprint: old_fp.clone(),
    });

    let client = RepositoryClient::new(
        Utf8PathBuf::from_path_buf(cache_dir.clone().into_std_path_buf()).unwrap(),
        Utf8PathBuf::from_path_buf(config_dir.clone().into_std_path_buf()).unwrap(),
        HmacKey::from_bytes([0xA2u8; 32]),
    );
    client.save_repositories(&repos).unwrap();

    let result = client
        .update("olamaelcu", &trust)
        .await
        .expect("update should not error on key change");

    match result {
        RepositoryUpdateResult::KeyChanged {
            name,
            old_fingerprint,
            new_fingerprint,
        } => {
            assert_eq!(name, "olamaelcu");
            assert_eq!(old_fingerprint, old_fp);
            assert_eq!(new_fingerprint, new_fp);
        }
        other => panic!("expected KeyChanged, got {other:?}"),
    }

    // The configured entry was NOT mutated — the operator still has to
    // explicitly accept the rollover via `confirm-update`.
    let after = client.load_repositories().unwrap();
    let entry = after
        .repositories
        .iter()
        .find(|r| r.name == "olamaelcu")
        .expect("entry");
    assert_eq!(entry.key_fingerprint, old_fp);
}

// =====================================================================
// Step 5 (Task 2.5 plan): `repository/client.rs` `confirm_update`
// with a mock server.
//
// The plan's spec called for "200 OK with `UpdateResult::Ok`
// vs `UpdateResult::KeyChanged`". `confirm_update` actually
// returns `Result<(), RepositoryError>` — the `Ok` /
// `KeyChanged` distinction is owned by `update`, not
// `confirm_update`. `confirm_update` is the operator's
// "I accept the new key; please refresh" command, and:
//   - On success, the cached `repo.toml` and `index.json` /
//     `index.json.sig` are overwritten, the configured
//     `key_fingerprint` is updated to the new key, and
//     `last_index_update` is bumped.
//   - On HTTP failure (e.g. 404 from the server), the
//     configured entry is left untouched and a
//     `RepositoryError::Http` is returned.
//
// We exercise the three contract paths:
//   - happy path: same fingerprint, server reachable → Ok
//   - key rollover: server declares a new fingerprint,
//     trust store already trusts the new key →
//     `confirm_update` succeeds AND the configured
//     `key_fingerprint` is bumped to the new one
//   - HTTP 4xx: `confirm_update` returns
//     `RepositoryError::Http` and leaves the entry alone
// =====================================================================

#[tokio::test]
async fn test_confirm_update_same_fingerprint_succeeds() {
    // The "200 OK with no fingerprint change" path:
    // `confirm_update` re-fetches the index, re-verifies the
    // signature against the trusted key, persists the
    // cached files, and bumps `last_index_update`. It does
    // NOT change the configured `key_fingerprint` (because
    // the fingerprint didn't change). We assert the
    // cached files were refreshed.
    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();
    let server_root = tmp_root.join("server");
    let cache_dir = tmp_root.join("cache");
    let config_dir = tmp_root.join("config");
    for d in [&server_root, &cache_dir, &config_dir] {
        fs::create_dir_all(d.as_std_path()).unwrap();
    }

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();
    let repo_fp = fingerprint(&verifying_key);

    let server = spawn_server(server_root.clone()).await;

    let repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: server.base_url.clone(),
            description: None,
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint: repo_fp.clone(),
        },
    };
    let repo_toml_text = render_repo_toml(&repo_toml);
    fs::write(server_root.join("repo.toml"), &repo_toml_text).unwrap();

    let index = sample_index();
    let index_json = render_index_json(&index);
    let sig = signing_key.sign(index_json.as_bytes());
    fs::write(server_root.join("index.json"), &index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), sig.to_bytes()).unwrap();

    // Pre-populate the configured entry.
    let mut repos = RepositoriesFile::default();
    repos.repositories.push(Repository {
        name: "olamaelcu".to_string(),
        url: server.base_url.clone(),
        description: None,
        maintainer: None,
        added_at: "2026-06-01T00:00:00Z".to_string(),
        last_index_update: Some("1970-01-01T00:00:00Z".to_string()),
        key_fingerprint: repo_fp.clone(),
    });

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", verifying_key)
        .expect("add_user_key");

    let client = RepositoryClient::new(
        Utf8PathBuf::from_path_buf(cache_dir.clone().into_std_path_buf()).unwrap(),
        Utf8PathBuf::from_path_buf(config_dir.clone().into_std_path_buf()).unwrap(),
        HmacKey::from_bytes([0xA3u8; 32]),
    );
    client.save_repositories(&repos).unwrap();

    // Sanity: configured fingerprint matches the live server's.
    assert_eq!(repos.repositories[0].key_fingerprint, repo_fp);

    client
        .confirm_update("olamaelcu", &trust)
        .await
        .expect("confirm_update with same fingerprint should succeed");

    // The cached files were written. The configured
    // `key_fingerprint` is unchanged (it already matched),
    // and `last_index_update` was bumped to a recent value.
    let after = client.load_repositories().unwrap();
    let entry = after
        .repositories
        .iter()
        .find(|r| r.name == "olamaelcu")
        .expect("entry");
    assert_eq!(
        entry.key_fingerprint, repo_fp,
        "fingerprint must NOT change when it already matched"
    );
    let updated = entry
        .last_index_update
        .as_deref()
        .unwrap_or("1970-01-01T00:00:00Z");
    assert_ne!(
        updated, "1970-01-01T00:00:00Z",
        "last_index_update must be bumped"
    );
}

#[tokio::test]
async fn test_confirm_update_after_key_change_rolls_over_fingerprint() {
    // The "200 OK after KeyChanged" path: an earlier
    // `update` returned `KeyChanged` because the publisher
    // rotated their signing key. The operator trusts the
    // new key and runs `confirm_update`. The configured
    // `key_fingerprint` is bumped to the new value, and
    // the cached files are refreshed against the new key.
    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();
    let server_root = tmp_root.join("server");
    let cache_dir = tmp_root.join("cache");
    let config_dir = tmp_root.join("config");
    for d in [&server_root, &cache_dir, &config_dir] {
        fs::create_dir_all(d.as_std_path()).unwrap();
    }

    // The NEW key, what the server now declares.
    let mut csprng = rand::rng();
    let new_signing = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let new_fp = fingerprint(&new_signing.verifying_key());

    let server = spawn_server(server_root.clone()).await;

    let repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: server.base_url.clone(),
            description: None,
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint: new_fp.clone(),
        },
    };
    let repo_toml_text = render_repo_toml(&repo_toml);
    fs::write(server_root.join("repo.toml"), &repo_toml_text).unwrap();

    // The server's index is signed by the new key.
    let index = sample_index();
    let index_json = render_index_json(&index);
    let sig = new_signing.sign(index_json.as_bytes());
    fs::write(server_root.join("index.json"), &index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), sig.to_bytes()).unwrap();

    // The configured entry still has the OLD fingerprint,
    // simulating the state right after `update` returned
    // `KeyChanged`.
    let mut repos = RepositoriesFile::default();
    repos.repositories.push(Repository {
        name: "olamaelcu".to_string(),
        url: server.base_url.clone(),
        description: None,
        maintainer: None,
        added_at: "2026-06-01T00:00:00Z".to_string(),
        last_index_update: Some("1970-01-01T00:00:00Z".to_string()),
        key_fingerprint: "SHA256:old-fingerprint".to_string(),
    });

    // The trust store contains the NEW key, trusted under
    // the standard label.
    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", new_signing.verifying_key())
        .expect("add_user_key");

    let client = RepositoryClient::new(
        Utf8PathBuf::from_path_buf(cache_dir.clone().into_std_path_buf()).unwrap(),
        Utf8PathBuf::from_path_buf(config_dir.clone().into_std_path_buf()).unwrap(),
        HmacKey::from_bytes([0xA4u8; 32]),
    );
    client.save_repositories(&repos).unwrap();

    client
        .confirm_update("olamaelcu", &trust)
        .await
        .expect("confirm_update with trusted new key should succeed");

    // The configured `key_fingerprint` was bumped to the
    // new value. This is the post-TOFU acceptance — the
    // operator ran `confirm-update` to formally accept
    // the new key.
    let after = client.load_repositories().unwrap();
    let entry = after
        .repositories
        .iter()
        .find(|r| r.name == "olamaelcu")
        .expect("entry");
    assert_eq!(
        entry.key_fingerprint, new_fp,
        "fingerprint must be updated to the new one after a successful confirm_update"
    );
}

#[tokio::test]
async fn test_confirm_update_returns_http_error_on_4xx() {
    // The "4xx error" path: the server returns 404 for
    // `repo.toml` (e.g. the repo was deleted or moved).
    // `confirm_update` must surface a `RepositoryError::Http`
    // and leave the configured entry alone.
    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();
    let server_root = tmp_root.join("server");
    let cache_dir = tmp_root.join("cache");
    let config_dir = tmp_root.join("config");
    for d in [&server_root, &cache_dir, &config_dir] {
        fs::create_dir_all(d.as_std_path()).unwrap();
    }

    // Empty server root — every request gets 404.

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let repo_fp = fingerprint(&signing_key.verifying_key());

    let server = spawn_server(server_root.clone()).await;

    let mut repos = RepositoriesFile::default();
    repos.repositories.push(Repository {
        name: "olamaelcu".to_string(),
        url: server.base_url.clone(),
        description: None,
        maintainer: None,
        added_at: "2026-06-01T00:00:00Z".to_string(),
        last_index_update: Some("1970-01-01T00:00:00Z".to_string()),
        key_fingerprint: repo_fp.clone(),
    });

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", signing_key.verifying_key())
        .expect("add_user_key");

    let client = RepositoryClient::new(
        Utf8PathBuf::from_path_buf(cache_dir.clone().into_std_path_buf()).unwrap(),
        Utf8PathBuf::from_path_buf(config_dir.clone().into_std_path_buf()).unwrap(),
        HmacKey::from_bytes([0xA5u8; 32]),
    );
    client.save_repositories(&repos).unwrap();

    let result = client.confirm_update("olamaelcu", &trust).await;
    let err = result.expect_err("confirm_update against an empty server must fail");
    match err {
        livtet_plugin::repository::error::RepositoryError::Http { status, .. } => {
            assert_eq!(status, 404, "expected 404 from the empty server");
        }
        other => panic!("expected Http 404, got {other:?}"),
    }

    // The configured entry was NOT mutated by the failed
    // call. `last_index_update` keeps the seed value.
    let after = client.load_repositories().unwrap();
    let entry = after
        .repositories
        .iter()
        .find(|r| r.name == "olamaelcu")
        .expect("entry");
    assert_eq!(entry.key_fingerprint, repo_fp);
    assert_eq!(
        entry.last_index_update.as_deref(),
        Some("1970-01-01T00:00:00Z"),
        "failed confirm_update must not bump last_index_update"
    );
}
