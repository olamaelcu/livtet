mod common;
use camino::Utf8PathBuf;
use camino_tempfile::Utf8TempDir as TempDir;
use common::fixture_path;
use fs_err as fs;
#[cfg(feature = "bundled")]
use livtet_plugins::discovery::scan_embedded_plugins;
use livtet_plugins::{
    discovery::{PluginSource, scan_plugins},
    manifest::{PluginRuntime, PluginType},
};

#[test]
fn test_scan_finds_folder_plugin() {
    let temp = TempDir::new().unwrap();
    let temp_path = temp.path().to_path_buf();

    let provider_dir = temp_path.join("test-provider");
    fs::create_dir_all(&provider_dir).unwrap();
    fs::copy(
        fixture_path("test-provider/livtet.toml"),
        provider_dir.join("livtet.toml"),
    )
    .unwrap();

    let plugins = scan_plugins(&temp_path).unwrap();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "test-provider");
    assert_eq!(plugins[0].source, PluginSource::Folder);
    assert_eq!(plugins[0].manifest.plugin.id, "test-provider");
    assert_eq!(plugins[0].manifest.plugin.name, "Test Provider");
}

#[test]
fn test_scan_finds_legacy_lua_file() {
    let temp = TempDir::new().unwrap();
    let temp_path = temp.path().to_path_buf();

    fs::copy(
        fixture_path("legacy-provider.lua"),
        temp_path.join("legacy-provider.lua"),
    )
    .unwrap();

    let plugins = scan_plugins(&temp_path).unwrap();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "legacy-provider");
    assert_eq!(plugins[0].source, PluginSource::LegacyFile);
    assert_eq!(plugins[0].manifest.plugin.id, "legacy-provider");
    assert_eq!(plugins[0].manifest.plugin.name, "legacy-provider");
    assert_eq!(plugins[0].manifest.plugin.version, "0.1.0");
    assert_eq!(plugins[0].manifest.plugin.plugin_type, PluginType::Provider);
    assert_eq!(plugins[0].manifest.plugin.runtime, PluginRuntime::Lua);
    assert_eq!(plugins[0].manifest.plugin.entry, "legacy-provider.lua");
}

#[test]
fn test_scan_skips_invalid_plugin_with_warning() {
    let temp = TempDir::new().unwrap();
    let temp_path = temp.path().to_path_buf();

    let provider_dir = temp_path.join("invalid-provider");
    fs::create_dir_all(&provider_dir).unwrap();
    fs::copy(
        fixture_path("invalid-provider/livtet.toml"),
        provider_dir.join("livtet.toml"),
    )
    .unwrap();

    let plugins = scan_plugins(&temp_path).unwrap();
    assert_eq!(plugins.len(), 0);
}

#[test]
fn test_scan_empty_directory() {
    let temp = TempDir::new().unwrap();
    let temp_path = temp.path().to_path_buf();

    let plugins = scan_plugins(&temp_path).unwrap();
    assert_eq!(plugins.len(), 0);
}

#[test]
fn test_scan_nonexistent_directory() {
    let path = Utf8PathBuf::from("/tmp/nonexistent-livtet-plugins-dir-12345");
    let plugins = scan_plugins(&path).unwrap();
    assert!(plugins.is_empty());
}

#[test]
fn test_scan_plugins_walks_version_subfolders() {
    let temp = TempDir::new().unwrap();
    let temp_path = temp.path().to_path_buf();

    let providers = temp_path.join("providers");
    let v1 = providers.join("my-plugin").join("1.0.0");
    let v2 = providers.join("my-plugin").join("2.0.0");
    fs::create_dir_all(&v1).unwrap();
    fs::create_dir_all(&v2).unwrap();
    fs::write(
        v1.join("livtet.toml"),
        "[plugin]\nid=\"my-plugin\"\nname=\"My Plugin\"\nversion=\"1.0.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    fs::write(v1.join("init.lua"), "-- v1\n").unwrap();
    fs::write(
        v2.join("livtet.toml"),
        "[plugin]\nid=\"my-plugin\"\nname=\"My Plugin\"\nversion=\"2.0.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    fs::write(v2.join("init.lua"), "-- v2\n").unwrap();

    let plugins = scan_plugins(&providers).unwrap();
    let versions: Vec<(String, String)> = plugins
        .iter()
        .map(|p| (p.id.clone(), p.manifest.plugin.version.clone()))
        .collect();
    assert_eq!(versions.len(), 2);
    assert!(versions.contains(&("my-plugin".to_string(), "1.0.0".to_string())));
    assert!(versions.contains(&("my-plugin".to_string(), "2.0.0".to_string())));
}

/// When the `bundled` feature is enabled, the `scan_embedded_plugins`
/// function returns one `DiscoveredPlugin` per bundled plugin in the
/// binary, with `source = PluginSource::Embedded` and the manifest
/// read from memory.
#[cfg(feature = "bundled")]
#[test]
fn test_scan_embedded_returns_all_lua_plugins() {
    let plugins = scan_embedded_plugins();
    let ids: Vec<&str> = plugins.iter().map(|p| p.id.as_str()).collect();
    for expected in ["openlibrary", "overdrive", "worldcat"] {
        assert!(
            ids.contains(&expected),
            "missing bundled plugin {expected}; got {ids:?}"
        );
    }
    for plugin in &plugins {
        assert_eq!(
            plugin.source,
            PluginSource::Embedded,
            "{} not marked Embedded",
            plugin.id
        );
        assert_eq!(
            plugin.manifest.plugin.id, plugin.id,
            "manifest id mismatch for {}",
            plugin.id
        );
    }
}
