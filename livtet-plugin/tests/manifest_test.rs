use std::collections::BTreeMap;

use livtet_plugin::{
    PluginManifest, PluginRuntime, PluginType, capability::Capability,
    plugin_requires::PluginRequires,
};

#[test]
fn test_parse_full_manifest() {
    let toml = r#"
[plugin]
id = "my-plugin"
name = "My Plugin"
version = "1.2.3"
description = "A test plugin"
type = "library"
runtime = "lua"
entry = "main.lua"

[plugin.capabilities]
search = true

[plugin.requires]
http = true

[plugin.exports]
modules = ["utils", "helpers"]

[[plugin.dependencies]]
id = "other-plugin"
version = "2.0.0"
required = true

[plugin.web]
enabled = false
"#;

    let manifest = PluginManifest::from_toml(toml).unwrap();
    let meta = manifest.plugin;
    assert_eq!(meta.id, "my-plugin");
    assert_eq!(meta.name, "My Plugin");
    assert_eq!(meta.version, "1.2.3");
    assert_eq!(meta.description, Some("A test plugin".to_string()));
    assert_eq!(meta.plugin_type, PluginType::Library);
    assert_eq!(meta.runtime, PluginRuntime::Lua);
    assert_eq!(meta.entry, "main.lua");

    let mut expected_caps = BTreeMap::new();
    expected_caps.insert(Capability::Search, true);
    assert_eq!(meta.capabilities, expected_caps);

    let mut expected_reqs = BTreeMap::new();
    expected_reqs.insert(PluginRequires::Http, true);
    assert_eq!(meta.requires, expected_reqs);

    let exports = meta.exports.as_ref().unwrap();
    assert_eq!(exports.modules, vec!["utils", "helpers"]);

    assert_eq!(meta.dependencies.len(), 1);
    let dep = &meta.dependencies[0];
    assert_eq!(dep.id, "other-plugin");
    assert_eq!(dep.version, Some("2.0.0".to_string()));
    assert!(dep.required);

    let web = meta.web.as_ref().unwrap();
    assert!(!web.enabled);
}

#[test]
fn test_parse_minimal_manifest() {
    let toml = r#"
[plugin]
id = "minimal"
name = "Minimal"
version = "0.1.0"
"#;

    let manifest = PluginManifest::from_toml(toml).unwrap();
    let meta = manifest.plugin;
    assert_eq!(meta.id, "minimal");
    assert_eq!(meta.name, "Minimal");
    assert_eq!(meta.version, "0.1.0");
    assert_eq!(meta.plugin_type, PluginType::Provider); // default
    assert_eq!(meta.runtime, PluginRuntime::Lua); // default
    assert_eq!(meta.entry, "init.lua"); // default
    assert_eq!(meta.description, None);
    assert!(meta.capabilities.is_empty());
    assert!(meta.requires.is_empty());
    assert!(meta.exports.is_none());
    assert!(meta.dependencies.is_empty());
    assert!(meta.web.is_none());
}

#[test]
fn test_legacy_single_file_manifest() {
    let manifest = PluginManifest::from_legacy_file("legacy_plugin");
    let meta = manifest.plugin;
    assert_eq!(meta.id, "legacy_plugin");
    assert_eq!(meta.name, "legacy_plugin");
    assert_eq!(meta.version, "0.1.0");
    assert_eq!(meta.plugin_type, PluginType::Provider);
    assert_eq!(meta.runtime, PluginRuntime::Lua);
    assert_eq!(meta.entry, "legacy_plugin.lua");
}

#[test]
fn test_reject_invalid_id_uppercase() {
    let toml = r#"
[plugin]
id = "BadId"
name = "Bad"
version = "0.1.0"
"#;
    let result = PluginManifest::from_toml(toml);
    assert!(result.is_err());
}

#[test]
fn test_reject_invalid_id_spaces() {
    let toml = r#"
[plugin]
id = "bad id"
name = "Bad"
version = "0.1.0"
"#;
    let result = PluginManifest::from_toml(toml);
    assert!(result.is_err());
}

#[test]
fn test_reject_invalid_id_too_long() {
    let toml = format!(
        r#"
[plugin]
id = "{}"
name = "Bad"
version = "0.1.0"
"#,
        "a".repeat(65)
    );
    let result = PluginManifest::from_toml(&toml);
    assert!(result.is_err());
}

#[test]
fn test_reject_empty_version() {
    let toml = r#"
[plugin]
id = "no-version"
name = "No Version"
version = ""
"#;
    let result = PluginManifest::from_toml(toml);
    assert!(result.is_err());
}

#[test]
fn test_reject_invalid_semver() {
    let toml = r#"
[plugin]
id = "bad-version"
name = "Bad Version"
version = "1.0"
"#;
    let result = PluginManifest::from_toml(toml);
    assert!(result.is_err());
}

#[test]
fn test_reject_empty_name() {
    let toml = r#"
[plugin]
id = "no-name"
name = ""
version = "0.1.0"
"#;
    let result = PluginManifest::from_toml(toml);
    assert!(result.is_err());
}

#[test]
fn test_reject_empty_entry() {
    let toml = r#"
[plugin]
id = "no-entry"
name = "No Entry"
version = "0.1.0"
entry = ""
"#;
    let result = PluginManifest::from_toml(toml);
    assert!(result.is_err());
}

#[test]
fn test_default_values() {
    let toml = r#"
[plugin]
id = "defaults"
name = "Defaults"
version = "0.1.0"
"#;
    let manifest = PluginManifest::from_toml(toml).unwrap();
    assert_eq!(manifest.plugin.plugin_type, PluginType::Provider);
    assert_eq!(manifest.plugin.runtime, PluginRuntime::Lua);
    assert_eq!(manifest.plugin.entry, "init.lua");
}

#[test]
fn test_valid_id_with_underscore_and_hyphen() {
    let toml = r#"
[plugin]
id = "my-plugin_v2"
name = "Valid"
version = "0.1.0"
"#;
    let result = PluginManifest::from_toml(toml);
    assert!(result.is_ok());
}

#[test]
fn test_valid_id_starting_with_digit() {
    let toml = r#"
[plugin]
id = "123abc"
name = "Valid"
version = "0.1.0"
"#;
    let result = PluginManifest::from_toml(toml);
    assert!(result.is_ok());
}

// =====================================================================
// Step 8 (Task 2.5 plan): `manifest.rs::validate()` direct
// tests. The validate method is `pub` and is reachable via
// `from_toml`; we exercise the first-character rules
// directly by constructing a `PluginMeta` (the only public
// way to reach the rule) and calling validate. The TOML
// tests above already cover the round-trip; these tests
// pin the rule in isolation so a future refactor of
// `from_toml` that changes the order of validation is
// flagged here.
// =====================================================================

use livtet_plugin::manifest::PluginMeta;

fn meta_with_id(id: &str) -> PluginMeta {
    // Build a PluginMeta with the requested id, otherwise
    // passing validate. We bypass `from_toml` so the
    // first-char rule is the only one under test.
    PluginMeta {
        id: id.to_string(),
        name: "name".to_string(),
        version: "0.1.0".to_string(),
        plugin_type: livtet_plugin::PluginType::Provider,
        runtime: livtet_plugin::PluginRuntime::Lua,
        entry: "init.lua".to_string(),
        description: None,
        capabilities: BTreeMap::new(),
        requires: BTreeMap::new(),
        exports: None,
        dependencies: Vec::new(),
        web: None,
        settings: std::collections::HashMap::new(),
        rocks: Vec::new(),
        oauth: None,
    }
}

#[test]
fn test_validate_rejects_id_starting_with_underscore() {
    // `_` is a valid in-id character (the second check
    // allows it) but not a valid first character (the
    // first-char check only allows lowercase letters and
    // digits). `_my_plugin` must be rejected.
    let meta = meta_with_id("_my_plugin");
    let err = meta.validate().expect_err("id starting with _ must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("lowercase letter or digit") || msg.contains("plugin id"),
        "expected a 'plugin id must start with a lowercase letter or digit' error, got: {msg}"
    );
}

#[test]
fn test_validate_rejects_id_starting_with_hyphen() {
    // `-` is a valid in-id character but not a valid first
    // character. `-my-plugin` must be rejected with the
    // same first-character error.
    let meta = meta_with_id("-my-plugin");
    let err = meta.validate().expect_err("id starting with - must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("lowercase letter or digit") || msg.contains("plugin id"),
        "expected a 'plugin id must start with a lowercase letter or digit' error, got: {msg}"
    );
}

#[test]
fn test_validate_accepts_id_starting_with_digit() {
    // `1my-plugin` is currently accepted: the first-char
    // check allows `is_ascii_digit()`. We pin this
    // contract: digits are valid first characters.
    // (This is a deliberate design choice — see the
    // existing `test_valid_id_starting_with_digit` round
    // trip test. If a future change tightens the rule to
    // only lowercase first characters, this test must
    // be updated to assert the rejection instead.)
    let meta = meta_with_id("1my-plugin");
    meta.validate()
        .expect("id starting with a digit must be accepted by the current contract");
}

#[test]
fn test_validate_rejects_id_with_uppercase_char() {
    // An uppercase character anywhere in the id is
    // rejected by the in-id check. The first-char check
    // catches `BadId` (uppercase first char), but a
    // lowercase first char with an uppercase later
    // character (e.g. `my_Plugin`) trips the in-id
    // check instead. We test both here.
    let meta_uppercase_first = meta_with_id("BadId");
    let err = meta_uppercase_first
        .validate()
        .expect_err("id with uppercase first char must fail");
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("lowercase") || msg.contains("plugin id"),
        "expected a first-char error for `BadId`, got: {msg}"
    );

    // Lowercase first, uppercase later.
    let meta_mixed = meta_with_id("my_Plugin");
    let err2 = meta_mixed
        .validate()
        .expect_err("id with uppercase char anywhere must fail");
    let msg2 = err2.to_string();
    assert!(
        msg2.contains("invalid characters") || msg2.contains("plugin id"),
        "expected an 'invalid characters' error for `my_Plugin`, got: {msg2}"
    );
}
