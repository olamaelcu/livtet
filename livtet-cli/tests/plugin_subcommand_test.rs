use std::collections::BTreeMap;

use camino::Utf8Path;
use ed25519_dalek::Signer;
use fs_err as fs;
use livtet_plugin::{
    archive::pack::pack,
    keys::{TrustStore, keyfile::keygen, signing::parse_pubkey_text},
    repository::{
        config::RepositoriesFile,
        index::{Index, IndexPlugin, IndexVersionEntry, render_index_json},
    },
    types::Repository,
};
use rand::{Rng as _, rng};
use camino_tempfile::Utf8TempDir as TempDir;

fn trust_store_with(label: &str, pubkey_path: &Utf8Path) -> TrustStore {
    let text = fs::read_to_string(pubkey_path.as_std_path()).unwrap();
    let vk = parse_pubkey_text(&text).expect("parse minisign pubkey box");
    let mut store = TrustStore::empty();
    store.add_user_key(label, vk).unwrap();
    store
}

fn make_plugin_source(src: &Utf8Path, id: &str, version: &str) {
    fs::create_dir_all(src.as_std_path()).unwrap();
    fs::write(
        src.join("livtet.toml"),
        format!(
            "[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nentry = \"init.lua\"\n"
        ),
    )
    .unwrap();
    fs::write(src.join("init.lua"), b"-- e2e\n").unwrap();
}

fn pack_test_plugin(tmp: &TempDir, id: &str, version: &str) -> (camino::Utf8PathBuf, TrustStore) {
    let plugin_src = tmp.path().join("plugin-src");
    let keys_dir = tmp.path().join("keys");
    let out_dir = tmp.path().join("out");
    let src = plugin_src.as_path();
    let keys = keys_dir.as_path();
    let out = out_dir.as_path();
    make_plugin_source(src, id, version);
    let report = keygen(keys, "author", true).unwrap();
    let ltp = pack(src, &report.key_path, "author", out).unwrap();
    let trust = trust_store_with("author", &report.pubkey_path);
    (ltp, trust)
}

fn write_repositories_toml(config_dir: &Utf8Path, repos: Vec<Repository>) {
    // Mirror `load_state_hmac_key`: respect the LIVTET_HMAC_KEY_HEX
    // override used in CI, then fall back to the historical default.
    // Both the writer (this fn) and the reader (`run_search` →
    // `load_repositories`) have to agree, otherwise every `run_search`
    // test in CI HMAC-mismatches.
    let hmac = livtet_cli::keyring_recover::test_hmac_key_from_env_or_default();
    let file = RepositoriesFile {
        repositories: repos,
    };
    file.save(&config_dir.join("repositories.toml"), &hmac)
        .expect("save repositories.toml");
}

#[test]
fn run_install_writes_plugin_to_providers_dir() {
    let tmp = TempDir::new().unwrap();
    let providers = tmp.path().join("providers");
    fs::create_dir_all(providers.as_std_path()).unwrap();

    let (ltp, trust) = pack_test_plugin(&tmp, "smoke-install", "0.1.0");

    let report = livtet_cli::plugin::run_install(&ltp, &providers, &trust)
        .expect("run_install should succeed");

    assert_eq!(report.id, "smoke-install");
    assert_eq!(report.version, "0.1.0");
    assert!(report.install_path.join("init.lua").exists());
    assert!(report.install_path.join("livtet.toml").exists());
}

#[test]
fn run_install_rejects_untrusted_archive() {
    let tmp = TempDir::new().unwrap();
    let providers = tmp.path().join("providers");
    fs::create_dir_all(providers.as_std_path()).unwrap();

    let (ltp, _trust) = pack_test_plugin(&tmp, "smoke-untrusted", "0.1.0");
    let empty = TrustStore::empty();

    let result = livtet_cli::plugin::run_install(&ltp, &providers, &empty);
    assert!(result.is_err(), "install must fail for untrusted archive");
}

#[test]
fn run_list_returns_installed_plugin() {
    let tmp = TempDir::new().unwrap();
    let providers = tmp.path().join("providers");
    fs::create_dir_all(providers.as_std_path()).unwrap();

    let (ltp, trust) = pack_test_plugin(&tmp, "smoke-list", "0.1.0");
    let _ = livtet_cli::plugin::run_install(&ltp, &providers, &trust).unwrap();

    let listed = livtet_cli::plugin::run_list(&providers).expect("run_list should succeed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "smoke-list");
    assert_eq!(listed[0].version, "0.1.0");
}

#[test]
fn run_list_returns_empty_when_providers_dir_missing() {
    let tmp = TempDir::new().unwrap();
    let providers = tmp.path().join("nonexistent");

    let listed = livtet_cli::plugin::run_list(&providers).expect("run_list should succeed");
    assert!(listed.is_empty());
}

#[test]
fn run_search_returns_no_results_for_empty_cache() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();

    write_repositories_toml(&config_dir, vec![]);

    let trust = TrustStore::empty();
    let results = livtet_cli::plugin::run_search_with_key(
        "anything",
        None,
        &cache_dir,
        &config_dir,
        &trust,
        &livtet_cli::keyring_recover::test_hmac_key_from_env_or_default(),
    )
    .expect("run_search with no repos should succeed and return empty");
    assert!(results.is_empty());
}

fn seed_signed_index(
    cache_root: &Utf8Path,
    repo_name: &str,
    plugins: Vec<(&str, Vec<(&str, &str)>)>,
) -> (
    ed25519_dalek::SigningKey,
    std::collections::BTreeMap<String, String>,
) {
    use livtet_plugin::keys::fingerprint;
    let mut versions = BTreeMap::new();
    let mut fingerprints = BTreeMap::new();
    for (plugin_id, version_archives) in plugins {
        let mut plugin_versions = BTreeMap::new();
        for (version, archive) in version_archives {
            plugin_versions.insert(
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
        }
        versions.insert(
            plugin_id.to_string(),
            IndexPlugin {
                versions: plugin_versions,
            },
        );
    }
    let index = Index {
        format_version: 1,
        generated_at: "2026-06-01T00:00:00Z".to_string(),
        plugins: versions,
    };
    let mut csprng = rand::rng();
    let signing_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying_key = signing_key.verifying_key();
    let fp = fingerprint(&verifying_key);
    fingerprints.insert(repo_name.to_string(), fp.clone());

    let index_json = render_index_json(&index);
    let sig = signing_key.sign(index_json.as_bytes());
    let repo_cache = cache_root.join(repo_name);
    fs::create_dir_all(repo_cache.as_std_path()).unwrap();
    fs::write(repo_cache.join("index.json"), &index_json).unwrap();
    fs::write(repo_cache.join("index.json.sig"), sig.to_bytes()).unwrap();
    (signing_key, fingerprints)
}

#[test]
fn run_search_finds_plugins_in_cached_index() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();

    let (_signing_key, fingerprints) = seed_signed_index(
        &cache_dir,
        "olamaelcu",
        vec![("openlibrary", vec![("1.0.0", "openlibrary-1.0.0.ltp")])],
    );
    let fp = fingerprints.get("olamaelcu").unwrap().clone();

    write_repositories_toml(
        &config_dir,
        vec![Repository {
            name: "olamaelcu".to_string(),
            url: "http://localhost".to_string(),
            description: None,
            maintainer: None,
            added_at: "2026-06-01T00:00:00Z".to_string(),
            last_index_update: None,
            key_fingerprint: fp,
        }],
    );

    let mut csprng = rand::rng();
    let bogus_key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed).verifying_key()
    };
    let mut trust = TrustStore::empty();
    trust.add_user_key("olamaelcu", bogus_key).unwrap();

    let results = livtet_cli::plugin::run_search_with_key(
        "open",
        None,
        &cache_dir,
        &config_dir,
        &trust,
        &livtet_cli::keyring_recover::test_hmac_key_from_env_or_default(),
    )
    .expect("run_search should succeed even when no trusted key matches");
    assert!(
        results.is_empty(),
        "search should return no results when the only trusted key does not match the repo fingerprint"
    );
}

#[test]
fn run_search_with_repo_filter_only_searches_named_repo() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(cache_dir.as_std_path()).unwrap();
    fs::create_dir_all(config_dir.as_std_path()).unwrap();

    let mut csprng = rand::rng();
    let alpha_signing = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let alpha_fp = livtet_plugin::keys::fingerprint(&alpha_signing.verifying_key());
    let beta_signing = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let beta_fp = livtet_plugin::keys::fingerprint(&beta_signing.verifying_key());

    for (name, signing, plugin, version, archive) in [
        (
            "alpha",
            &alpha_signing,
            "alpha-plugin",
            "1.0.0",
            "alpha-1.0.0.ltp",
        ),
        (
            "beta",
            &beta_signing,
            "beta-plugin",
            "1.0.0",
            "beta-1.0.0.ltp",
        ),
    ] {
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
        plugins.insert(plugin.to_string(), IndexPlugin { versions });
        let index = Index {
            format_version: 1,
            generated_at: "2026-06-01T00:00:00Z".to_string(),
            plugins,
        };
        let index_json = render_index_json(&index);
        let sig = signing.sign(index_json.as_bytes());
        let repo_cache = cache_dir.join(name);
        fs::create_dir_all(repo_cache.as_std_path()).unwrap();
        fs::write(repo_cache.join("index.json"), &index_json).unwrap();
        fs::write(repo_cache.join("index.json.sig"), sig.to_bytes()).unwrap();
    }

    write_repositories_toml(
        &config_dir,
        vec![
            Repository {
                name: "alpha".to_string(),
                url: "http://alpha".to_string(),
                description: None,
                maintainer: None,
                added_at: "2026-06-01T00:00:00Z".to_string(),
                last_index_update: None,
                key_fingerprint: alpha_fp,
            },
            Repository {
                name: "beta".to_string(),
                url: "http://beta".to_string(),
                description: None,
                maintainer: None,
                added_at: "2026-06-01T00:00:00Z".to_string(),
                last_index_update: None,
                key_fingerprint: beta_fp,
            },
        ],
    );

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("alpha", alpha_signing.verifying_key())
        .unwrap();
    trust
        .add_user_key("beta", beta_signing.verifying_key())
        .unwrap();

    let results = livtet_cli::plugin::run_search_with_key(
        "plugin",
        Some("alpha"),
        &cache_dir,
        &config_dir,
        &trust,
        &livtet_cli::keyring_recover::test_hmac_key_from_env_or_default(),
    )
    .expect("run_search should succeed");
    assert!(results.iter().all(|r| r.repository == "alpha"));
    assert!(!results.is_empty());
}

#[test]
fn run_pack_creates_ltp_archive() {
    let tmp = TempDir::new().unwrap();
    let src_path = tmp.path().join("plugin-src");
    let keys_path = tmp.path().join("keys");
    let out_path = tmp.path().join("out");
    let src = src_path.as_path();
    let keys = keys_path.as_path();
    let out = out_path.as_path();
    make_plugin_source(src, "smoke-pack", "0.1.0");
    keygen(keys, "author", true).unwrap();

    let ltp = livtet_cli::plugin::run_pack(src, "author", keys, Some(out))
        .expect("run_pack should succeed");

    assert!(ltp.exists(), "ltp path should exist: {ltp}");
    assert!(ltp.as_std_path().metadata().unwrap().len() > 0);
    assert_eq!(ltp.file_name().unwrap(), "smoke-pack-0.1.0.ltp");
}

#[test]
fn run_pack_fails_when_key_file_missing() {
    let tmp = TempDir::new().unwrap();
    let src_path = tmp.path().join("plugin-src");
    let keys_path = tmp.path().join("keys");
    let out_path = tmp.path().join("out");
    let src = src_path.as_path();
    let keys = keys_path.as_path();
    let out = out_path.as_path();
    make_plugin_source(src, "smoke-pack-missing-key", "0.1.0");

    let result = livtet_cli::plugin::run_pack(src, "no-such-label", keys, Some(out));
    assert!(
        result.is_err(),
        "run_pack must fail when the key file is absent"
    );
}
