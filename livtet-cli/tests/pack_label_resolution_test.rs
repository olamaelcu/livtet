use camino::Utf8Path;
use camino_tempfile::Utf8TempDir as TempDir;
use fs_err as fs;
use livtet_cli::plugin::{resolve_pack_label, run_pack};
use livtet_plugins::keys::keyfile::keygen;

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

#[test]
fn resolve_pack_label_flag_wins() {
    let resolved = resolve_pack_label(Some("flag-key"), Some("env-key"));
    assert_eq!(resolved, "flag-key");
}

#[test]
fn resolve_pack_label_env_used_when_no_flag() {
    let resolved = resolve_pack_label(None, Some("env-key"));
    assert_eq!(resolved, "env-key");
}

#[test]
fn resolve_pack_label_default_used_when_neither() {
    let resolved = resolve_pack_label(None, None);
    assert_eq!(resolved, "default");
}

#[test]
fn resolve_pack_label_empty_flag_treated_as_unset() {
    let resolved = resolve_pack_label(Some(""), None);
    assert_eq!(resolved, "default");
}

#[test]
fn run_pack_with_resolved_label_uses_correct_keyfile() {
    let tmp = TempDir::new().unwrap();
    let src_path = tmp.path().join("plugin-src");
    let keys_path = tmp.path().join("keys");
    let out_path = tmp.path().join("out");
    let src = src_path.as_path();
    let keys = keys_path.as_path();
    let out = out_path.as_path();
    make_plugin_source(src, "smoke-pack-explicit", "0.1.0");
    keygen(keys, "explicit-key", true).expect("keygen should succeed");

    let ltp = run_pack(src, "explicit-key", keys, Some(out))
        .expect("run_pack with explicit-key should succeed");

    assert!(ltp.exists(), "ltp path should exist: {ltp}");
    assert!(ltp.as_std_path().metadata().unwrap().len() > 0);
    assert_eq!(ltp.file_name().unwrap(), "smoke-pack-explicit-0.1.0.ltp");
}

#[test]
fn run_pack_falls_back_to_default_label() {
    let tmp = TempDir::new().unwrap();
    let src_path = tmp.path().join("plugin-src");
    let keys_path = tmp.path().join("keys");
    let out_path = tmp.path().join("out");
    let src = src_path.as_path();
    let keys = keys_path.as_path();
    let out = out_path.as_path();
    make_plugin_source(src, "smoke-pack-default", "0.1.0");
    keygen(keys, "default", true).expect("keygen should succeed");

    let resolved = resolve_pack_label(None, None);
    assert_eq!(resolved, "default");

    let ltp = run_pack(src, &resolved, keys, Some(out))
        .expect("run_pack with default label should succeed");

    assert!(ltp.exists(), "ltp path should exist: {ltp}");
    assert_eq!(ltp.file_name().unwrap(), "smoke-pack-default-0.1.0.ltp");
}
