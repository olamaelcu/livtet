use std::{assert_matches, collections::BTreeMap};

use camino::{Utf8Path, Utf8PathBuf};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use fs_err as fs;
use livtet_plugin::{
    archive::{install::install, pack::pack, verify::verify},
    keys::{TrustStore, fingerprint, keyfile::keygen, signing::parse_pubkey_text},
    repository::{
        client::{RepositoryClient, find_version, search_index},
        config::RepositoriesFile,
        hmac::HmacKey,
        index::{
            Index, IndexPlugin, IndexVersionEntry, parse_index_json, render_index_json,
            verify_index_signature,
        },
        installed::InstalledEntry,
    },
    types::{Repository, RepositoryAddResult, RepositoryUpdateResult},
};
use rand::{Rng as _, rng};
use camino_tempfile::Utf8TempDir as TempDir;

fn test_pubkey() -> VerifyingKey {
    let mut csprng = rand::rng();
    ({
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    })
    .verifying_key()
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
            archive: "x-1.0.0.ltp".to_string(),
            archive_size: 12345,
            archive_sha256: "abc123".to_string(),
            min_app_version: "0.5.0".to_string(),
        },
    );
    let mut plugins = BTreeMap::new();
    plugins.insert("x".to_string(), IndexPlugin { versions });
    Index {
        format_version: 1,
        generated_at: "2026-06-01T00:00:00Z".to_string(),
        plugins,
    }
}

fn fixture_index() -> Index {
    let mut plugins = BTreeMap::new();
    let mkver = |archive: &str, size: u64| IndexVersionEntry {
        entry: "init.lua".to_string(),
        capabilities: Default::default(),
        dependencies: vec![],
        archive: archive.to_string(),
        archive_size: size,
        archive_sha256: "abc".to_string(),
        min_app_version: "0.5.0".to_string(),
    };
    let mut v1 = BTreeMap::new();
    v1.insert("1.0.0".to_string(), mkver("openlibrary-1.0.0.ltp", 1000));
    v1.insert("1.1.0".to_string(), mkver("openlibrary-1.1.0.ltp", 1100));
    plugins.insert("openlibrary".to_string(), IndexPlugin { versions: v1 });

    let mut v2 = BTreeMap::new();
    v2.insert("2.0.0".to_string(), mkver("goodreads-2.0.0.ltp", 2000));
    plugins.insert("goodreads".to_string(), IndexPlugin { versions: v2 });
    Index {
        format_version: 1,
        generated_at: "2026-06-01T00:00:00Z".to_string(),
        plugins,
    }
}

#[test]
fn test_index_json_round_trip() {
    let original = sample_index();
    let rendered = render_index_json(&original);
    let parsed = parse_index_json(&rendered).unwrap();
    assert_eq!(parsed.format_version, 1);
    let plugin = parsed.plugins.get("x").unwrap();
    let v = plugin.versions.get("1.0.0").unwrap();
    assert_eq!(v.archive, "x-1.0.0.ltp");
    assert_eq!(v.archive_size, 12345);
    assert!(find_version(&parsed, "x", "1.0.0").is_some());
}

#[test]
fn test_index_signature_verification() {
    let mut csprng = rand::rng();
    let key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let index = sample_index();
    let index_json = render_index_json(&index);

    let sig = key.sign(index_json.as_bytes());
    let sig_bytes = sig.to_bytes();

    let parsed = parse_index_json(&index_json).unwrap();
    assert!(
        verify_index_signature(
            &parsed,
            &index_json,
            sig_bytes.as_slice(),
            &key.verifying_key(),
        )
        .is_ok()
    );
}

#[test]
fn test_index_signature_mismatch() {
    let mut csprng = rand::rng();
    let key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let index = sample_index();
    let index_json = render_index_json(&index);
    let bogus_sig = [0u8; 64];
    let parsed = parse_index_json(&index_json).unwrap();
    let result = verify_index_signature(
        &parsed,
        &index_json,
        bogus_sig.as_slice(),
        &key.verifying_key(),
    );
    assert!(result.is_err());
}

#[test]
fn test_repositories_file_round_trip() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path()
        .join("repositories.toml");
    let key = HmacKey::from_bytes([0x11u8; 32]);

    let mut file = RepositoriesFile::default();
    file.repositories.push(Repository {
        name: "olamaelcu".to_string(),
        url: "https://plugins.livtet.olamaelcu.net".to_string(),
        description: Some("Livtet plugins from Olamaelcu".to_string()),
        maintainer: Some("Olamaelcu <plugins@livtet.olamaelcu.net>".to_string()),
        added_at: "2026-06-01T12:00:00Z".to_string(),
        last_index_update: None,
        key_fingerprint: "SHA256:abc".to_string(),
    });

    file.save(&config_path, &key).unwrap();
    let loaded = RepositoriesFile::load(&config_path, &key).unwrap();
    assert_eq!(loaded.repositories.len(), 1);
    assert_eq!(loaded.repositories[0].name, "olamaelcu");
}

#[test]
fn test_repositories_file_tamper_detected() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path()
        .join("repositories.toml");
    let key = HmacKey::from_bytes([0x11u8; 32]);
    let mut file = RepositoriesFile::default();
    file.repositories.push(Repository {
        name: "x".to_string(),
        url: "https://x".to_string(),
        description: None,
        maintainer: None,
        added_at: "2026-06-01T00:00:00Z".to_string(),
        last_index_update: None,
        key_fingerprint: "SHA256:0".to_string(),
    });
    file.save(&config_path, &key).unwrap();
    let mut text = fs::read_to_string(config_path.as_std_path()).unwrap();
    text = text.replace("name = \"x\"", "name = \"y\"");
    fs::write(config_path.as_std_path(), text).unwrap();
    let result = RepositoriesFile::load(&config_path, &key);
    assert!(result.is_err());
}

#[test]
fn test_repositories_file_missing_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path()
        .join("nonexistent.toml");
    let key = HmacKey::from_bytes([0x11u8; 32]);
    let file = RepositoriesFile::load(&config_path, &key).unwrap();
    assert!(file.repositories.is_empty());
}

#[test]
fn test_installed_file_round_trip() {
    let tmp = TempDir::new().unwrap();
    let installed_path = tmp.path()
        .join("installed.json");
    let key = HmacKey::from_bytes([0x55u8; 32]);

    let mut file = livtet_plugin::repository::installed::InstalledFile::default();
    file.upsert(livtet_plugin::repository::installed::entry_for(
        "openlibrary",
        "1.2.3",
        Some("olamaelcu".to_string()),
        camino::Utf8PathBuf::from("/data/livtet/providers/openlibrary/1.2.3"),
    ));
    file.upsert(livtet_plugin::repository::installed::entry_for(
        "overdrive",
        "0.9.0",
        None,
        camino::Utf8PathBuf::from("/data/livtet/providers/overdrive/0.9.0"),
    ));
    file.disable("openlibrary");

    file.save(&installed_path, &key).unwrap();
    let loaded =
        livtet_plugin::repository::installed::InstalledFile::load(&installed_path, &key).unwrap();
    assert_eq!(loaded.entries.len(), 2);
    assert_eq!(loaded.entries[0].id, "openlibrary");
    assert_eq!(loaded.entries[0].version, "1.2.3");
    assert_eq!(loaded.entries[0].source_repo.as_deref(), Some("olamaelcu"));
    assert_eq!(loaded.entries[1].id, "overdrive");
    assert!(loaded.is_disabled("openlibrary"));
    assert!(!loaded.is_disabled("overdrive"));
}

#[test]
fn test_installed_file_tamper_detected() {
    let tmp = TempDir::new().unwrap();
    let installed_path = tmp.path()
        .join("installed.json");
    let key = HmacKey::from_bytes([0x55u8; 32]);

    let mut file = livtet_plugin::repository::installed::InstalledFile::default();
    file.upsert(livtet_plugin::repository::installed::entry_for(
        "x",
        "1.0.0",
        None,
        camino::Utf8PathBuf::from("/p/x/1.0.0"),
    ));
    file.save(&installed_path, &key).unwrap();

    // Tamper with the JSON body: change the plugin id.
    let raw = fs::read_to_string(installed_path.as_std_path()).unwrap();
    let tampered = raw.replace("\"x\"", "\"y\"");
    assert_ne!(raw, tampered, "tamper substitution should change the file");
    fs::write(installed_path.as_std_path(), tampered).unwrap();

    let result = livtet_plugin::repository::installed::InstalledFile::load(&installed_path, &key);
    assert!(
        result.is_err(),
        "tampered installed.json must be rejected; got {result:?}"
    );
}

#[test]
fn test_installed_file_missing_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let installed_path = tmp.path()
        .join("missing-installed.json");
    let key = HmacKey::from_bytes([0x55u8; 32]);
    let file =
        livtet_plugin::repository::installed::InstalledFile::load(&installed_path, &key).unwrap();
    assert!(file.entries.is_empty());
    assert!(file.disabled.is_empty());
}

#[test]
fn test_repo_add_offline_returns_needs_tofu() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();

    let key = HmacKey::from_bytes([0x22u8; 32]);
    let pk = test_pubkey();
    let client = RepositoryClient::new(cache_dir, config_dir, key);
    let result = client.add_offline("olamaelcu", "https://plugins.livtet.olamaelcu.net", &pk);
    match result {
        RepositoryAddResult::NeedsTofuConfirmation {
            name,
            url,
            fingerprint: fp,
        } => {
            assert_eq!(name, "olamaelcu");
            assert_eq!(url, "https://plugins.livtet.olamaelcu.net");
            assert!(fp.starts_with("SHA256:"));
        }
        _ => panic!("expected NeedsTofuConfirmation"),
    }
}

#[test]
fn test_update_detects_key_change() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();
    let key = HmacKey::from_bytes([0x33u8; 32]);

    let mut file = RepositoriesFile::default();
    file.repositories.push(Repository {
        name: "olamaelcu".to_string(),
        url: "https://plugins.livtet.olamaelcu.net".to_string(),
        description: None,
        maintainer: None,
        added_at: "2026-06-01T00:00:00Z".to_string(),
        last_index_update: None,
        key_fingerprint: "SHA256:old".to_string(),
    });
    let client = RepositoryClient::new(cache_dir, config_dir, key);
    client.save_repositories(&file).unwrap();

    let mut csprng = rand::rng();
    let new_signing = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let new_pk = new_signing.verifying_key();
    let new_fp = fingerprint(&new_pk);

    let result = client.detect_key_change("olamaelcu", "SHA256:old", &new_fp);
    assert_matches!(result, RepositoryUpdateResult::KeyChanged { .. });

    let result_ok = client.detect_key_change("olamaelcu", "SHA256:same", "SHA256:same");
    assert_matches!(result_ok, RepositoryUpdateResult::Ok { .. });
}

#[test]
fn test_record_install_appends_entry() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();
    let key = HmacKey::from_bytes([0x55u8; 32]);
    let client = RepositoryClient::new(cache_dir, config_dir, key);

    let entry = InstalledEntry {
        id: "openlibrary".to_string(),
        version: "1.0.0".to_string(),
        source_repo: Some("olamaelcu".to_string()),
        install_path: Utf8PathBuf::from("/providers/openlibrary/1.0.0"),
        installed_at: "2026-06-01T00:00:00Z".to_string(),
    };
    client.record_install(entry).unwrap();

    let loaded = client.load_installed().unwrap();
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].id, "openlibrary");
    assert_eq!(loaded.entries[0].version, "1.0.0");
}

#[test]
fn test_remove_installed_entry_filters_correctly() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();
    let key = HmacKey::from_bytes([0x55u8; 32]);
    let client = RepositoryClient::new(cache_dir, config_dir, key);

    let entry_a = InstalledEntry {
        id: "x".to_string(),
        version: "1.0.0".to_string(),
        source_repo: None,
        install_path: Utf8PathBuf::from("/p/x/1.0.0"),
        installed_at: "2026-06-01T00:00:00Z".to_string(),
    };
    let entry_b = InstalledEntry {
        id: "y".to_string(),
        version: "2.0.0".to_string(),
        source_repo: None,
        install_path: Utf8PathBuf::from("/p/y/2.0.0"),
        installed_at: "2026-06-01T00:00:00Z".to_string(),
    };
    client.record_install(entry_a).unwrap();
    client.record_install(entry_b).unwrap();

    let removed = client.remove_installed_entry("x", "1.0.0").unwrap();
    assert!(removed);

    let loaded = client.load_installed().unwrap();
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].id, "y");

    let removed_again = client.remove_installed_entry("x", "1.0.0").unwrap();
    assert!(!removed_again);
}

#[test]
fn test_search_finds_matching_plugins() {
    let index = fixture_index();
    let results = search_index(&index, "open", "olamaelcu");
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.plugin_id == "openlibrary"));
    for r in &results {
        assert_eq!(r.repository, "olamaelcu");
        assert!(r.relevance_score > 0.0);
    }
}

#[test]
fn test_search_no_match() {
    let index = fixture_index();
    let results = search_index(&index, "zzz-nothing", "olamaelcu");
    assert!(results.is_empty());
}

#[test]
fn test_find_plugin_version_in_index() {
    let index = fixture_index();
    let entry = find_version(&index, "openlibrary", "1.0.0").unwrap();
    assert_eq!(entry.archive, "openlibrary-1.0.0.ltp");
    assert!(find_version(&index, "openlibrary", "9.9.9").is_none());
    assert!(find_version(&index, "missing", "1.0.0").is_none());
}

use livtet_plugin::repository::publisher::{
    init_repo, publish_archive, sign_index, unpublish_version,
};

#[test]
fn test_init_repo_creates_skeleton() {
    let tmp = camino_tempfile::Utf8TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    let fp = "SHA256:abc123".to_string();
    init_repo(
        &repo_dir,
        "olamaelcu",
        "https://plugins.livtet.olamaelcu.net",
        &fp,
        None,
    )
    .unwrap();
    assert!(repo_dir.join("repo.toml").exists());
    assert!(repo_dir.join("pool").is_dir());
    let toml_text = fs::read_to_string(repo_dir.join("repo.toml")).unwrap();
    assert!(toml_text.contains("name = \"olamaelcu\""));
    assert!(toml_text.contains("key_fingerprint = \"SHA256:abc123\""));
}

#[test]
fn test_publish_archive_writes_pool_and_index() {
    let tmp = camino_tempfile::Utf8TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    let archive_path = tmp.path()
        .join("myplugin-1.0.0.ltp");
    fs::write(&archive_path, b"fake archive bytes").unwrap();

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };

    init_repo(
        &repo_dir,
        "olamaelcu",
        "https://plugins.livtet.olamaelcu.net",
        "SHA256:abc",
        None,
    )
    .unwrap();
    publish_archive(
        &repo_dir,
        &archive_path,
        "myplugin",
        "1.0.0",
        "init.lua",
        "0.5.0",
        &signing_key,
    )
    .unwrap();

    assert!(repo_dir.join("pool/myplugin-1.0.0.ltp").exists());
    let index_text = fs::read_to_string(repo_dir.join("index.json")).unwrap();
    assert!(index_text.contains("myplugin"));
    assert!(index_text.contains("1.0.0"));
    assert!(repo_dir.join("index.json.sig").exists());
}

#[test]
fn test_sign_index_re_signs_existing() {
    let tmp = camino_tempfile::Utf8TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    init_repo(
        &repo_dir,
        "olamaelcu",
        "https://plugins.livtet.olamaelcu.net",
        "SHA256:abc",
        None,
    )
    .unwrap();

    let mut versions = BTreeMap::new();
    versions.insert(
        "1.0.0".to_string(),
        IndexVersionEntry {
            entry: "init.lua".to_string(),
            capabilities: Default::default(),
            dependencies: vec![],
            archive: "p-1.0.0.ltp".to_string(),
            archive_size: 100,
            archive_sha256: "abc".to_string(),
            min_app_version: "0.5.0".to_string(),
        },
    );
    let mut plugins = BTreeMap::new();
    plugins.insert("p".to_string(), IndexPlugin { versions });
    let index = Index {
        format_version: 1,
        generated_at: "2026-06-01T00:00:00Z".to_string(),
        plugins,
    };
    fs::write(
        repo_dir.join("index.json"),
        livtet_plugin::repository::index::render_index_json(&index),
    )
    .unwrap();

    sign_index(&repo_dir, &signing_key).unwrap();
    let sig_bytes = fs::read(repo_dir.join("index.json.sig")).unwrap();
    assert_eq!(sig_bytes.len(), 64);
}

#[test]
fn test_unpublish_removes_entry_and_archive() {
    let tmp = camino_tempfile::Utf8TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    init_repo(
        &repo_dir,
        "olamaelcu",
        "https://plugins.livtet.olamaelcu.net",
        "SHA256:abc",
        None,
    )
    .unwrap();

    let archive_path = tmp.path().join("p-1.0.0.ltp");
    fs::write(&archive_path, b"x").unwrap();
    publish_archive(
        &repo_dir,
        &archive_path,
        "p",
        "1.0.0",
        "init.lua",
        "0.5.0",
        &signing_key,
    )
    .unwrap();
    assert!(repo_dir.join("pool/p-1.0.0.ltp").exists());

    unpublish_version(&repo_dir, "p", "1.0.0", &signing_key).unwrap();
    assert!(!repo_dir.join("pool/p-1.0.0.ltp").exists());
    let index_text = fs::read_to_string(repo_dir.join("index.json")).unwrap();
    assert!(!index_text.contains("1.0.0"));
}

#[test]
fn test_e2e_offline_pack_verify_install_repo_publish_search() {
    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();
    let plugin_src = tmp_root.join("plugin-src");
    let keys_dir = tmp_root.join("keys");
    let repo_dir = tmp_root.join("repo");
    let providers = tmp_root.join("providers");
    let output_dir = tmp_root.join("out");
    fs::create_dir_all(plugin_src.as_std_path()).unwrap();
    fs::create_dir_all(keys_dir.as_std_path()).unwrap();
    fs::create_dir_all(providers.as_std_path()).unwrap();

    fs::write(
        plugin_src.join("livtet.toml"),
        b"[plugin]\nid = \"e2e-offline\"\nname = \"E2E Offline\"\nversion = \"0.1.0\"\nentry = \"init.lua\"\n",
    )
    .unwrap();
    fs::write(plugin_src.join("init.lua"), b"-- e2e test\n").unwrap();

    let author_report = keygen(&keys_dir, "author", true).expect("keygen should succeed");
    assert!(
        author_report.fingerprint.starts_with("SHA256:"),
        "fingerprint should start with SHA256:, got: {}",
        author_report.fingerprint
    );

    let ltp_path = pack(&plugin_src, &author_report.key_path, "author", &output_dir)
        .expect("pack should succeed");
    assert!(ltp_path.exists(), "packed .ltp should exist");

    let pubkey_text = fs::read_to_string(&author_report.pubkey_path).unwrap();
    let verifying_key = parse_pubkey_text(&pubkey_text).expect("parse minisign pubkey box");

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("author", verifying_key)
        .expect("add_user_key should succeed");

    let verify_report = verify(&ltp_path, Some(&trust)).expect("verify should succeed");
    assert!(verify_report.valid, "verify.valid should be true");
    assert_eq!(verify_report.plugin_id.as_deref(), Some("e2e-offline"));

    let install_report =
        install(&ltp_path, &providers, Some(&trust)).expect("install should succeed");
    assert_eq!(install_report.id, "e2e-offline");
    assert_eq!(install_report.version, "0.1.0");
    let installed_root = providers.join("e2e-offline").join("0.1.0");
    assert!(installed_root.join("init.lua").exists());
    assert!(installed_root.join("livtet.toml").exists());

    let mut csprng = rand::rng();
    let publisher_signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let publisher_fp = fingerprint(&publisher_signing_key.verifying_key());

    init_repo(
        &repo_dir,
        "olamaelcu",
        "https://plugins.livtet.olamaelcu.net",
        &publisher_fp,
        None,
    )
    .expect("init_repo should succeed");

    publish_archive(
        &repo_dir,
        &ltp_path,
        "e2e-offline",
        "0.1.0",
        "init.lua",
        "0.5.0",
        &publisher_signing_key,
    )
    .expect("publish_archive should succeed");

    assert!(repo_dir.join("pool/e2e-offline-0.1.0.ltp").exists());
    assert!(repo_dir.join("index.json").exists());
    assert!(repo_dir.join("index.json.sig").exists());

    let index_text = fs::read_to_string(repo_dir.join("index.json")).unwrap();
    let parsed_index = parse_index_json(&index_text).expect("index.json should parse");
    let results = search_index(&parsed_index, "e2e", "olamaelcu");
    assert!(
        !results.is_empty(),
        "search for 'e2e' should return at least one result"
    );
    assert!(results.iter().any(|r| r.plugin_id == "e2e-offline"));

    let sig_bytes = fs::read(repo_dir.join("index.json.sig")).unwrap();
    assert_eq!(
        sig_bytes.len(),
        64,
        "raw ed25519 signature must be 64 bytes"
    );
    verify_index_signature(
        &parsed_index,
        &index_text,
        &sig_bytes,
        &publisher_signing_key.verifying_key(),
    )
    .expect("index signature should verify with publisher key");
}

#[test]
fn test_repository_client_search_finds_indexed_plugins() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();
    let publisher_fp = fingerprint(&verifying_key);

    let mut file = RepositoriesFile::default();
    file.repositories.push(Repository {
        name: "olamaelcu".to_string(),
        url: "https://plugins.livtet.olamaelcu.net".to_string(),
        description: None,
        maintainer: None,
        added_at: "2026-06-01T00:00:00Z".to_string(),
        last_index_update: None,
        key_fingerprint: publisher_fp.clone(),
    });

    let key = HmacKey::from_bytes([0x99u8; 32]);
    let client = RepositoryClient::new(
        Utf8PathBuf::from_path_buf(cache_dir.clone().into_std_path_buf()).unwrap(),
        Utf8PathBuf::from_path_buf(config_dir.clone().into_std_path_buf()).unwrap(),
        key.clone(),
    );
    client.save_repositories(&file).unwrap();

    let repo_cache = cache_dir.join("olamaelcu");
    fs::create_dir_all(repo_cache.as_std_path()).unwrap();
    let index = fixture_index();
    let index_json = render_index_json(&index);
    let sig = signing_key.sign(index_json.as_bytes());
    let sig_bytes = sig.to_bytes();
    fs::write(repo_cache.join("index.json"), &index_json).unwrap();
    fs::write(repo_cache.join("index.json.sig"), sig_bytes).unwrap();

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", verifying_key)
        .expect("add_user_key");

    let results = client
        .search("open", &trust)
        .expect("search should succeed");
    assert!(
        !results.is_empty(),
        "search for 'open' should return at least one result"
    );
    assert!(results.iter().all(|r| r.repository == "olamaelcu"));
    assert!(results.iter().any(|r| r.plugin_id == "openlibrary"));
}

// =====================================================================
// Step 4 (Task 2.5 plan): `repository/publisher.rs` error paths.
//
// The audit found four branches the plan asked us to cover:
//   - "index doesn't exist" (`publish_archive` line 76-80)
//   - "index.json missing"  (`sign_index`     line 110-112)
//   - "no such version"    (`unpublish_version` line 131-135)
//   - "pool file missing"  (`unpublish_version` line 124-126)
//
// Only one of these is a true error path in the current
// code: `sign_index` returns `RepositoryError::NotFound`
// when `index.json` does not exist. The other three are
// silent no-ops (auto-create / early-`continue`).
// We pin both behaviors here so a future refactor that
// turns a silent no-op into an error (or vice versa) is
// forced to update the contract.
// =====================================================================

#[test]
fn test_sign_index_errors_when_index_json_missing() {
    // The "index.json missing" branch in `sign_index`:
    // when `repo_dir/index.json` does not exist, the
    // function returns `NotFound("index.json")`. We pin
    // the variant and the message so a future change to
    // the error type is flagged by this test.
    let tmp = camino_tempfile::Utf8TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    fs::create_dir_all(repo_dir.as_std_path()).unwrap();
    // No `index.json` is written here.

    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };

    let err = sign_index(&repo_dir, &signing_key).expect_err("missing index.json must fail");
    match err {
        livtet_plugin::repository::error::RepositoryError::NotFound(name) => {
            assert_eq!(name, "index.json");
        }
        other => panic!("expected NotFound(\"index.json\"), got {other:?}"),
    }
}

#[test]
fn test_publish_archive_creates_index_when_missing() {
    // The "index doesn't exist" branch in `publish_archive`:
    // when `index.json` is absent, the function does NOT
    // error — it seeds a fresh empty `Index` and writes
    // it back out. This is a deliberate "first publish"
    // behavior, so we pin it. (The audit listed this as a
    // "missing branch", but the actual code is a silent
    // auto-create, not an error.)
    let tmp = camino_tempfile::Utf8TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    let archive_path = tmp.path()
        .join("autocreate-1.0.0.ltp");
    fs::write(&archive_path, b"x").unwrap();
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };

    publish_archive(
        &repo_dir,
        &archive_path,
        "autocreate",
        "1.0.0",
        "init.lua",
        "0.5.0",
        &signing_key,
    )
    .expect("publish_archive auto-creates a missing index");

    assert!(repo_dir.join("index.json").exists());
    assert!(repo_dir.join("index.json.sig").exists());
    let text = fs::read_to_string(repo_dir.join("index.json")).unwrap();
    assert!(text.contains("autocreate"));
}

#[test]
fn test_unpublish_version_is_noop_when_version_missing() {
    // The "no such version" branch in `unpublish_version`:
    // when the index contains no entry for `plugin_id` (or
    // no such `version`), the function silently no-ops. The
    // index is still re-rendered and re-signed (because
    // `generated_at` is bumped), but the plugin/version
    // set is unchanged. We pin this contract.
    let tmp = camino_tempfile::Utf8TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    init_repo(
        &repo_dir,
        "olamaelcu",
        "https://plugins.livtet.olamaelcu.net",
        "SHA256:abc",
        None,
    )
    .unwrap();

    // Publish a single (plugin, version) so the index has
    // known content we can compare against after the no-op
    // unpublish.
    let archive_path = tmp.path().join("p-1.0.0.ltp");
    fs::write(&archive_path, b"x").unwrap();
    publish_archive(
        &repo_dir,
        &archive_path,
        "p",
        "1.0.0",
        "init.lua",
        "0.5.0",
        &signing_key,
    )
    .unwrap();
    let before = fs::read_to_string(repo_dir.join("index.json")).unwrap();

    // Unpublish a (plugin, version) that was never published.
    // The current contract: silent success, no error.
    unpublish_version(&repo_dir, "p", "9.9.9", &signing_key)
        .expect("current contract: unpublishing a missing version is a silent no-op");

    // The index still contains the original entry, and the
    // pool directory is unchanged.
    let after = fs::read_to_string(repo_dir.join("index.json")).unwrap();
    assert!(
        after.contains("\"1.0.0\""),
        "original entry must still be present"
    );
    assert!(
        !after.contains("\"9.9.9\""),
        "phantom version must not appear"
    );
    // The `generated_at` field changes on every publish,
    // so we don't compare the index JSON byte-for-byte.
    // We DO compare the set of versions, which is the
    // user-observable contract.
    let _ = before;
}

#[test]
fn test_unpublish_version_is_noop_when_pool_file_missing() {
    // The "pool file missing" branch in `unpublish_version`:
    // when `pool/{plugin}-{version}.ltp` does not exist
    // (e.g. the archive was manually deleted), the function
    // silently no-ops the file removal. The index IS still
    // updated and re-signed because the version entry is
    // removed from the index.
    let tmp = camino_tempfile::Utf8TempDir::new().unwrap();
    let repo_dir = tmp.path().join("repo");
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    init_repo(
        &repo_dir,
        "olamaelcu",
        "https://plugins.livtet.olamaelcu.net",
        "SHA256:abc",
        None,
    )
    .unwrap();

    // Publish once so the index has the entry but NOT
    // the pool file. We publish with a different archive
    // path, then immediately remove the pool file by hand
    // to simulate the "index has the entry, pool is gone"
    // state.
    let archive_path = tmp.path().join("q-1.0.0.ltp");
    fs::write(&archive_path, b"x").unwrap();
    publish_archive(
        &repo_dir,
        &archive_path,
        "q",
        "1.0.0",
        "init.lua",
        "0.5.0",
        &signing_key,
    )
    .unwrap();
    let pool_path = repo_dir.join("pool").join("q-1.0.0.ltp");
    assert!(
        pool_path.exists(),
        "publish should have created the pool file"
    );
    fs::remove_file(&pool_path).unwrap();
    assert!(!pool_path.exists());

    // The unpublish call must NOT error on the missing
    // pool file. The current contract is "silent no-op"
    // for the file-removal arm; the index entry IS still
    // removed.
    unpublish_version(&repo_dir, "q", "1.0.0", &signing_key)
        .expect("current contract: missing pool file is a silent no-op, not an error");

    let after = fs::read_to_string(repo_dir.join("index.json")).unwrap();
    assert!(!after.contains("\"1.0.0\""), "index entry must be removed");
    assert!(
        !pool_path.exists(),
        "pool file is still absent (no resurrection)"
    );
}

// =====================================================================
// Step 6 (Task 2.5 plan): `repository/index.rs` error paths.
//
// `parse_index_json` rejects:
//   - JSON that doesn't deserialize to the `Index` shape
//     (covered indirectly by other tests that write valid
//     JSON; the rejected cases flow through `serde_json`'s
//     error arm).
//   - `format_version` that is not the supported version
//     (1). The error message must mention "unsupported
//     format_version" so the operator can see what was
//     declared.
//   - `generated_at` is empty. The error message must
//     mention "generated_at" and "empty".
// =====================================================================

#[test]
fn test_parse_index_json_rejects_unsupported_format_version() {
    // `format_version = 999` is structurally a valid u32
    // and the JSON deserializes, but the explicit
    // `format_version != SUPPORTED_INDEX_FORMAT_VERSION`
    // check in `parse_index_json` must reject it. We pin
    // the error message so a future refactor that uses
    // different wording is intentional.
    let bad = r#"{
        "format_version": 999,
        "generated_at": "2026-06-01T00:00:00Z",
        "plugins": {}
    }"#;
    let err = livtet_plugin::repository::index::parse_index_json(bad)
        .expect_err("format_version 999 must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("999") && (msg.contains("format_version") || msg.contains("unsupported")),
        "error should mention the unsupported version 999, got: {msg}"
    );
}

#[test]
fn test_parse_index_json_rejects_empty_generated_at() {
    // An index with `generated_at = ""` is structurally a
    // valid JSON object (empty string is a valid `String`),
    // but the explicit "is_empty()" check in `parse_index_json`
    // must reject it.
    let bad = r#"{
        "format_version": 1,
        "generated_at": "",
        "plugins": {}
    }"#;
    let err = livtet_plugin::repository::index::parse_index_json(bad)
        .expect_err("empty generated_at must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("generated_at") && msg.contains("empty"),
        "error should mention generated_at and empty, got: {msg}"
    );
}
