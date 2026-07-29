use std::collections::BTreeMap;

use camino::{Utf8Path, Utf8PathBuf};
use ed25519_dalek::Signer;
use fs_err as fs;
use livtet_plugin::{
    archive::pack::pack as archive_pack,
    keys::{
        TrustStore, fingerprint,
        keyfile::keygen,
        signing::{load_minisign_signing_key, parse_pubkey_text},
    },
    repository::{
        client::RepositoryClient,
        index::{Index, IndexPlugin, IndexVersionEntry, render_index_json},
        repo_toml::{RepoSection, RepoToml, SigningSection, render_repo_toml},
    },
};
use camino_tempfile::Utf8TempDir as TempDir;

mod common;

use common::spawn_server;

fn make_plugin_source(src: &Utf8Path, id: &str, version: &str) {
    fs::create_dir_all(src.as_std_path()).unwrap();
    fs::write(
        src.join("livtet.toml"),
        format!(
            "[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nentry = \"init.lua\"\n"
        ),
    )
    .unwrap();
    fs::write(src.join("init.lua"), b"-- smoke e2e\n").unwrap();
}

fn sample_index(plugin_id: &str, version: &str, archive: &str) -> Index {
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

#[tokio::test]
async fn smoke_full_distribution_flow_with_mocked_repo() {
    let tmp = TempDir::new().unwrap();
    let tmp_root = tmp.path();

    let server_root = tmp_root.join("server");
    let providers = tmp_root.join("providers");
    let cache_dir = tmp_root.join("cache");
    let config_dir = tmp_root.join("config");
    let trust_dir = tmp_root.join("trust");
    let keys_dir = tmp_root.join("keys");
    for d in [
        &server_root,
        &providers,
        &cache_dir,
        &config_dir,
        &trust_dir,
        &keys_dir,
    ] {
        fs::create_dir_all(d.as_std_path()).unwrap();
    }

    let plugin_id = "smoke-e2e";
    let plugin_version = "0.1.0";
    let archive_name = format!("{plugin_id}-{plugin_version}.ltp");

    // Single keypair drives both the archive signature (pack) and the
    // repository index signature so the smoke test exercises the exact
    // path producers and consumers go through in production.
    let report = keygen(&keys_dir, "olamaelcu", true).expect("keygen");
    let pubkey_text = fs::read_to_string(&report.pubkey_path).expect("read pubkey");
    let verifying_key = parse_pubkey_text(&pubkey_text).expect("parse minisign pubkey box");
    let publisher_fp = fingerprint(&verifying_key);
    let (_sk, signing_key) =
        load_minisign_signing_key(&report.key_path).expect("load minisign signing key");

    let plugin_src = tmp_root.join("plugin-src");
    make_plugin_source(&plugin_src, plugin_id, plugin_version);
    let out_dir = tmp_root.join("packed");
    fs::create_dir_all(out_dir.as_std_path()).unwrap();
    let packed = archive_pack(&plugin_src, &report.key_path, "olamaelcu", &out_dir)
        .expect("pack should succeed");
    assert!(packed.as_std_path().exists());

    let index = sample_index(plugin_id, plugin_version, &archive_name);
    let index_json = render_index_json(&index);
    let sig_bytes = signing_key.sign(index_json.as_bytes()).to_bytes();
    fs::write(server_root.join("index.json"), &index_json).unwrap();
    fs::write(server_root.join("index.json.sig"), sig_bytes).unwrap();

    let mut repo_toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: "olamaelcu".to_string(),
            url: String::new(),
            description: Some("Smoke test repository".to_string()),
            maintainer: None,
        },
        signing: SigningSection {
            key_label: "olamaelcu".to_string(),
            key_fingerprint: publisher_fp.clone(),
        },
    };
    let server = spawn_server(server_root.clone()).await;
    repo_toml.repo.url = server.base_url.clone();
    let repo_toml_text = render_repo_toml(&repo_toml);
    fs::write(server_root.join("repo.toml"), &repo_toml_text).unwrap();

    let mut trust = TrustStore::empty();
    trust
        .add_user_key("olamaelcu", verifying_key)
        .expect("add_user_key");

    let client = RepositoryClient::new(
        Utf8PathBuf::from_path_buf(cache_dir.clone().into_std_path_buf()).unwrap(),
        Utf8PathBuf::from_path_buf(config_dir.clone().into_std_path_buf()).unwrap(),
        livtet_cli::keyring_recover::test_hmac_key_from_env_or_default(),
    );

    client
        .confirm_add(&server.base_url, &trust)
        .await
        .expect("confirm_add should succeed against the mocked repo server");

    fs::copy(
        report.pubkey_path.as_std_path(),
        trust_dir.join("olamaelcu.pub").as_std_path(),
    )
    .expect("copy pubkey to trust dir");

    let trust = livtet_cli::plugin::load_trust_store(&trust_dir).expect("load trust");
    assert!(
        trust.user_key_by_label("olamaelcu").is_some(),
        "trust store must contain the olamaelcu key after the smoke flow"
    );

    let results = livtet_cli::plugin::run_search_with_key(
        plugin_id,
        None,
        &cache_dir,
        &config_dir,
        &trust,
        &livtet_cli::keyring_recover::test_hmac_key_from_env_or_default(),
    )
    .expect("run_search should succeed");
    assert!(
        results.iter().any(|r| r.plugin_id == plugin_id),
        "search should find the smoke-e2e plugin"
    );

    let install_report = livtet_cli::plugin::run_install(&packed, &providers, &trust)
        .expect("run_install should succeed");
    assert_eq!(install_report.id, plugin_id);
    assert_eq!(install_report.version, plugin_version);

    let listed = livtet_cli::plugin::run_list(&providers).expect("run_list should succeed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, plugin_id);
    assert_eq!(listed[0].version, plugin_version);
}
