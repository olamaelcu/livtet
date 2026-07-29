use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize, de::Error};

use crate::{
    capability::Capability,
    error::{PluginError, PluginResult},
    plugin_requires::PluginRequires,
};

/// Top-level manifest container.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
}

/// Core metadata for a plugin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default = "default_provider_type", rename = "type")]
    pub plugin_type: PluginType,
    #[serde(default = "default_runtime")]
    pub runtime: PluginRuntime,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<Capability, bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub requires: BTreeMap<PluginRequires, bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exports: Option<ExportConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencyDecl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<WebConfig>,
    /// Declarative schema for plugin settings. Each entry maps a
    /// setting key to its field type, default, description, and
    /// validation constraints. The frontend's generic settings form
    /// renders inputs directly from this schema; plugins that need
    /// custom UI can ignore it and ship a settings web component
    /// instead.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub settings: HashMap<String, SettingSchema>,
    /// LuaRocks rock names this plugin depends on. Each rock is
    /// installed via `luarocks install --tree <app_data_dir>/lua_modules <rock>`
    /// on first run on desktop. On mobile (FFI), these rocks must be
    /// vendored in the plugin's data directory or via
    /// `livtet-lua-stdlib` (not this crate's concern).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rocks: Vec<String>,
    /// OAuth providers the plugin wants to redeem tokens against.
    /// Each entry maps a host-opaque provider id (e.g.
    /// `livtet_cloud`) to the scopes the plugin needs. The host
    /// combines the manifest-declared scopes with any already-
    /// granted scopes and prompts the user for the union at
    /// `host.oauth_redeem_token` time.
    ///
    /// The plugin must also declare `oauth = true` in
    /// `[plugin.requires]` for the host to surface
    /// `host.oauth_redeem_token` / `host.oauth_get_token` /
    /// `host.oauth_revoke_token` and run the redemption flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthConfig>,
}

/// OAuth provider configuration block under `[plugin.oauth]`.
///
/// Declares which third-party providers a plugin wants to talk
/// to and which scopes each one needs. The host treats
/// `provider_id` as an opaque string and looks up the actual
/// authorization / token endpoints from its own registry, so
/// plugins don't need to bake URLs into their manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuthConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<OAuthProviderDecl>,
}

/// One provider entry under `[plugin.oauth.providers]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuthProviderDecl {
    /// Host-opaque provider id (e.g. `livtet_cloud`,
    /// `openlibrary`). The host matches it against its built-in
    /// provider table at consent time.
    pub provider_id: String,
    /// Scopes the plugin needs (e.g. `read:library`,
    /// `write:library`). Scope strings are opaque to the host —
    /// it passes them through verbatim.
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl OAuthConfig {
    /// Look up the scopes declared for `provider_id`, or `None`
    /// if the provider isn't declared in this plugin's manifest.
    /// Used by the host when it builds the authorization request
    /// to decide which scopes to request.
    pub fn scopes_for(&self, provider_id: &str) -> Option<&[String]> {
        self.providers
            .iter()
            .find(|p| p.provider_id == provider_id)
            .map(|p| p.scopes.as_slice())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PluginType {
    Provider,
    Library,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PluginRuntime {
    Lua,
    Rhai,
    Wasm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DependencyDecl {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default = "default_true")]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportConfig {
    #[serde(default = "Vec::new")]
    pub modules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// One field-type token accepted in a plugin's `[plugin.settings]`
/// table. `string` is a single-line input, `text` is a multi-line
/// textarea, `number` renders a numeric stepper, `boolean` renders a
/// switch, and `url` is a single-line input validated as a URL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum SettingFieldType {
    #[default]
    String,
    Number,
    Boolean,
    Text,
    Url,
}

/// Schema for a single plugin setting declared in `[plugin.settings]`.
///
/// Mirrors the `[plugin.settings.<key>]` section in `livtet.toml`. The
/// host parses this at load time, exposes it to the frontend via
/// `PluginInfo.settings_schema`, and the generic settings form
/// renders inputs directly from the schema. Plugins that need
/// custom UI (e.g. Overdrive's Test Connection button) can still
/// register a `plugin_web_command` capability alongside.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct SettingSchema {
    /// The on-wire field type. Re-exported as `field_type` in the
    /// JSON shape to keep `type` free for a future top-level
    /// metadata key without breaking existing manifests.
    #[serde(rename = "type")]
    pub field_type: SettingFieldType,
    /// Default value shipped in the schema itself. Rendered as the
    /// initial value before the user touches anything; if no row
    /// exists in `plugin_settings` for this plugin+key, the form
    /// uses this default. Stored in the DB as a JSON-encoded
    /// string once the user saves.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    #[specta(type = specta_typescript::Unknown<serde_json::Value>)]
    pub default: serde_json::Value,
    /// Human-readable label/hint shown next to the input. The form
    /// component falls back to the key name when this is empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Minimum allowed numeric value (inclusive). Empty for
    /// non-numeric fields. Forwarded to `<wa-number-input min>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Maximum allowed numeric value (inclusive). Empty for
    /// non-numeric fields. Forwarded to `<wa-number-input max>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Render the field as a password input. Only honoured when
    /// `field_type` is `String`. Frontend toggles the eye
    /// icon; the *value* is still saved to the DB as plain text
    /// for `SetSettingRequest` (use `SecretRequest` for
    /// keychain-backed values).
    #[serde(default)]
    pub secret: bool,
}

pub fn default_provider_type() -> PluginType {
    PluginType::Provider
}

pub fn default_runtime() -> PluginRuntime {
    PluginRuntime::Lua
}

pub fn default_entry() -> String {
    "init.lua".to_string()
}

pub fn default_true() -> bool {
    true
}

impl PluginManifest {
    /// Create a legacy single-file manifest from a filename stem.
    pub fn from_legacy_file(stem: &str) -> Self {
        Self {
            plugin: PluginMeta {
                id: stem.to_string(),
                name: stem.to_string(),
                version: "0.1.0".to_string(),
                plugin_type: default_provider_type(),
                runtime: default_runtime(),
                entry: format!("{stem}.lua"),
                description: None,
                capabilities: BTreeMap::new(),
                requires: BTreeMap::new(),
                exports: None,
                dependencies: Vec::new(),
                web: None,
                settings: HashMap::new(),
                rocks: Vec::new(),
                oauth: None,
            },
        }
    }

    /// Parse a manifest from a TOML string.
    pub fn from_toml(s: &str) -> PluginResult<Self> {
        let manifest: Self = toml::from_str(s)?;
        manifest.plugin.validate()?;
        Ok(manifest)
    }
}

impl PluginMeta {
    /// Validate manifest fields.
    pub fn validate(&self) -> PluginResult<()> {
        // ID validation
        if self.id.is_empty() {
            return Err(PluginError::ManifestParse(toml::de::Error::custom(
                "plugin id must not be empty",
            )));
        }
        if self.id.len() > 64 {
            return Err(PluginError::ManifestParse(toml::de::Error::custom(
                "plugin id must be at most 64 characters",
            )));
        }
        let first = self.id.chars().next().unwrap();
        if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
            return Err(PluginError::ManifestParse(toml::de::Error::custom(
                "plugin id must start with a lowercase letter or digit",
            )));
        }
        for ch in self.id.chars() {
            if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '_' && ch != '-' {
                return Err(PluginError::ManifestParse(toml::de::Error::custom(
                    "plugin id contains invalid characters (only a-z, 0-9, _, - allowed)",
                )));
            }
        }

        // Name validation
        if self.name.is_empty() {
            return Err(PluginError::ManifestParse(toml::de::Error::custom(
                "plugin name must not be empty",
            )));
        }

        // Version validation (simple semver: major.minor.patch)
        if self.version.is_empty() {
            return Err(PluginError::ManifestParse(toml::de::Error::custom(
                "plugin version must not be empty",
            )));
        }
        let parts: Vec<&str> = self.version.split('.').collect();
        if parts.len() != 3 {
            return Err(PluginError::ManifestParse(toml::de::Error::custom(
                "plugin version must be valid semver (major.minor.patch)",
            )));
        }
        for part in &parts {
            if part.parse::<u64>().is_err() {
                return Err(PluginError::ManifestParse(toml::de::Error::custom(
                    "plugin version must be valid semver (major.minor.patch)",
                )));
            }
        }

        // Entry validation
        if self.entry.is_empty() {
            return Err(PluginError::ManifestParse(toml::de::Error::custom(
                "plugin entry must not be empty",
            )));
        }

        // Rocks validation
        let mut seen_rocks: HashMap<&str, ()> = HashMap::new();
        for rock in &self.rocks {
            if rock.is_empty() {
                return Err(PluginError::ManifestParse(toml::de::Error::custom(
                    "plugin rock name must not be empty",
                )));
            }
            if rock.contains('/') || rock.contains('\\') || rock.contains("..") {
                return Err(PluginError::ManifestParse(toml::de::Error::custom(
                    "plugin rock name contains invalid path characters",
                )));
            }
            if seen_rocks.insert(rock.as_str(), ()).is_some() {
                return Err(PluginError::ManifestParse(toml::de::Error::custom(
                    "plugin rock name listed more than once",
                )));
            }
        }

        Ok(())
    }

    /// Whether this plugin declares the `system_secrets` require in its
    /// `[plugin.requires]` table. Used by the Lua bridge as the manifest-
    /// side half of the two-gate check (capability declaration + grant
    /// sidecar allowlist).
    pub fn has_capability_system_secrets(&self) -> bool {
        self.requires.contains_key(&PluginRequires::SystemSecrets)
    }

    /// I18n keys for every declared capability, sorted in BTreeMap order.
    pub fn capability_i18n_keys(&self) -> Vec<String> {
        self.capabilities.keys().map(|c| c.i18n_key()).collect()
    }

    /// I18n keys for every declared require, sorted in BTreeMap order.
    pub fn requires_i18n_keys(&self) -> Vec<String> {
        self.requires.keys().map(|r| r.i18n_key()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `[plugin.settings]` entry with `type = "url"` must parse into
    /// `SettingFieldType::Url`. The koreader bundled plugin uses this
    /// for the Kosync server URL field; without the `Url` variant the
    /// mobile FFI `init_plugins` step logs a parse error and skips the
    /// plugin.
    #[test]
    fn setting_field_type_parses_url() {
        let toml = r#"
            [plugin]
            id = "koreader"
            name = "KOReader"
            version = "1.0.0"
            type = "provider"
            runtime = "lua"
            entry = "init.lua"

            [plugin.settings.sync_server_url]
            type = "url"
            default = ""
            description = "Kosync server URL"
        "#;
        let manifest: PluginManifest = toml::from_str(toml).expect("manifest should parse");
        let field = manifest
            .plugin
            .settings
            .get("sync_server_url")
            .expect("sync_server_url setting present");
        assert_eq!(field.field_type, SettingFieldType::Url);
    }

    /// The existing `string` variant must keep parsing after we add
    /// the `url` variant — guards against accidentally breaking the
    /// default case.
    #[test]
    fn setting_field_type_parses_string() {
        let toml = r#"
            [plugin]
            id = "koreader"
            name = "KOReader"
            version = "1.0.0"
            type = "provider"
            runtime = "lua"
            entry = "init.lua"

            [plugin.settings.koreader_path]
            type = "string"
            default = "~/.config/koreader"
            description = "Path to KOReader settings directory"
        "#;
        let manifest: PluginManifest = toml::from_str(toml).expect("manifest should parse");
        let field = manifest
            .plugin
            .settings
            .get("koreader_path")
            .expect("koreader_path setting present");
        assert_eq!(field.field_type, SettingFieldType::String);
    }

    #[test]
    fn unknown_capability_fails_serde() {
        let toml = r#"
            [plugin]
            id = "test"
            name = "Test"
            version = "1.0.0"
            type = "provider"
            runtime = "lua"
            entry = "init.lua"

            [plugin.capabilities]
            seach = true
        "#;
        let result: Result<PluginManifest, _> = toml::from_str(toml);
        assert!(
            result.is_err(),
            "unknown capability 'seach' must fail serde"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("seach"),
            "error should name the bad key; got: {err}"
        );
    }

    #[test]
    fn unknown_requires_fails_serde() {
        let toml = r#"
            [plugin]
            id = "test"
            name = "Test"
            version = "1.0.0"
            type = "provider"
            runtime = "lua"
            entry = "init.lua"

            [plugin.capabilities]
            search = true

            [plugin.requires]
            http = true
            http_not_a_key = false
        "#;
        let result: Result<PluginManifest, _> = toml::from_str(toml);
        assert!(result.is_err(), "unknown requires key must fail serde");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("http_not_a_key"),
            "error should name the bad key; got: {err}"
        );
    }

    #[test]
    fn oauth_requires_parses_ok() {
        let toml = r#"
            [plugin]
            id = "test"
            name = "Test"
            version = "1.0.0"
            type = "provider"
            runtime = "lua"
            entry = "init.lua"

            [plugin.capabilities]
            search = true

            [plugin.requires]
            http = true
            oauth = false
        "#;
        let manifest: PluginManifest = toml::from_str(toml).expect("oauth should deserialize OK");
        assert!(
            manifest
                .plugin
                .requires
                .contains_key(&PluginRequires::Oauth),
            "oauth should be in the requires map"
        );
        // validate() should pass (the warn is emitted, not returned as error)
        assert!(
            manifest.plugin.validate().is_ok(),
            "validate() should let oauth through"
        );
    }

    #[test]
    fn oauth_config_parses_with_providers() {
        // A plugin declaring oauth = true in [plugin.requires] and a
        // matching [plugin.oauth.providers] block should round-trip
        // through toml. The host later uses
        // `manifest.plugin.oauth.as_ref().and_then(|c| c.scopes_for(provider))`
        // to look up the scopes to request.
        let toml = r#"
            [plugin]
            id = "test"
            name = "Test"
            version = "1.0.0"
            type = "provider"
            runtime = "lua"
            entry = "init.lua"

            [plugin.capabilities]
            search = true

            [plugin.requires]
            http = true
            oauth = true

            [[plugin.oauth.providers]]
            provider_id = "livtet_cloud"
            scopes = ["read:library", "write:library"]

            [[plugin.oauth.providers]]
            provider_id = "openlibrary"
            scopes = ["read"]
        "#;
        let manifest = PluginManifest::from_toml(toml).expect("oauth config should parse");
        assert!(
            manifest
                .plugin
                .requires
                .contains_key(&PluginRequires::Oauth)
        );
        let oauth = manifest
            .plugin
            .oauth
            .as_ref()
            .expect("oauth block should be present");
        assert_eq!(oauth.providers.len(), 2);
        assert_eq!(
            oauth.scopes_for("livtet_cloud"),
            Some(&["read:library".to_string(), "write:library".to_string()][..])
        );
        assert_eq!(
            oauth.scopes_for("openlibrary"),
            Some(&["read".to_string()][..])
        );
        assert_eq!(oauth.scopes_for("nonexistent"), None);
    }

    #[test]
    fn oauth_config_optional() {
        // Plugins that don't declare oauth at all should still
        // deserialize cleanly — `oauth` is `Option<OAuthConfig>`
        // and absent means `None`.
        let toml = r#"
            [plugin]
            id = "test"
            name = "Test"
            version = "1.0.0"
            type = "provider"
            runtime = "lua"
            entry = "init.lua"

            [plugin.capabilities]
            search = true
        "#;
        let manifest =
            PluginManifest::from_toml(toml).expect("manifest without oauth should parse");
        assert!(manifest.plugin.oauth.is_none());
    }

    #[test]
    fn has_capability_system_secrets_returns_false_when_absent() {
        let toml = r#"
            [plugin]
            id = "test"
            name = "Test"
            version = "1.0.0"
            type = "provider"
            runtime = "lua"
            entry = "init.lua"

            [plugin.capabilities]
            search = true

            [plugin.requires]
            http = true
        "#;
        let manifest: PluginManifest = toml::from_str(toml).unwrap();
        assert!(!manifest.plugin.has_capability_system_secrets());
    }
}
