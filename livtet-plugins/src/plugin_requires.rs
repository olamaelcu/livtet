//! Host function families a plugin may opt into via `[plugin.requires]`
//! in its `livtet.toml`. Closed enum; any unknown key in the manifest
//! fails serde deserialization with a clear message listing the known
//! set.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{Display, EnumIter, EnumString, IntoEnumIterator, IntoStaticStr};

/// The sealed set of host-function families a plugin can declare in
/// its `[plugin.requires]` table. The deserializer maps TOML string
/// keys onto this enum (e.g. `http`, `system_secrets`). Unknown keys
/// fail serde with an error listing the known set. The enum is sealed
/// to `crates/livtet-plugins` — downstream crates cannot add new
/// variants without changing the plugin host.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Display,
    EnumString,
    EnumIter,
    IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum PluginRequires {
    /// `http` — plugin calls `host.http_get`.
    Http,
    /// `secrets` — plugin calls `host.get_secret` / `host.set_secret`
    /// (the runtime keyring-backed store, NOT the compile-time SOPS
    /// bridge).
    Secrets,
    /// `system_secrets` — plugin calls `host.get_system_secret`.
    /// Compile-time-injected SOPS values; see ADR 0032 / 0033.
    SystemSecrets,
    /// `oauth` — plugin calls `host.oauth_redeem_token` /
    /// `host.oauth_get_token` / `host.oauth_revoke_token`. Implemented
    /// by the desktop Tauri host; returns `HostError::Unsupported` on
    /// mobile.
    Oauth,
    /// `filesystem` — plugin calls `host.fs_copy` / `host.fs_symlink`.
    /// Gated by the `write_paths` field in the grant sidecar; see
    /// ADR/0033 for the gating pattern.
    Filesystem,
}

impl PluginRequires {
    /// Stable string form used in manifests, IPC payloads, and the
    /// wire protocol.
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Parse a manifest/IPC string back into a `PluginRequires`.
    /// Returns `None` for unknown keys so future manifests with
    /// unrecognised requires degrade gracefully (the host logs a
    /// warning and continues) rather than aborting parse.
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        s.parse().ok()
    }

    /// I18n key the web UI uses to localize the human-friendly name.
    /// Convention: `plugin.requires.<snake_case>`. Single literal
    /// lives here; the web side does not duplicate it.
    pub fn i18n_key(self) -> String {
        format!("plugin.requires.{}", self.as_str())
    }
}

impl Serialize for PluginRequires {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PluginRequires {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        Self::from_str_lossy(&s).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown plugin requires {s:?}; expected one of {:?}",
                PluginRequires::iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_round_trips_via_from_str_lossy() {
        for req in PluginRequires::iter() {
            assert_eq!(PluginRequires::from_str_lossy(req.as_str()), Some(req));
        }
    }

    #[test]
    fn from_str_lossy_rejects_unknown_key() {
        assert_eq!(PluginRequires::from_str_lossy("nope"), None);
        assert_eq!(PluginRequires::from_str_lossy(""), None);
        assert_eq!(PluginRequires::from_str_lossy("Http"), None);
    }

    #[test]
    fn serde_string_round_trip() {
        let original = PluginRequires::SystemSecrets;
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"system_secrets\"");
        let back: PluginRequires = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn serde_rejects_unknown_string() {
        let result: Result<PluginRequires, _> = serde_json::from_str("\"telepathy\"");
        assert!(result.is_err());
    }

    #[test]
    fn serde_round_trips_btreemap_of_requires() {
        let original: std::collections::BTreeMap<PluginRequires, bool> = [
            (PluginRequires::Http, true),
            (PluginRequires::Secrets, false),
            (PluginRequires::SystemSecrets, true),
            (PluginRequires::Oauth, false),
            (PluginRequires::Filesystem, true),
        ]
        .into_iter()
        .collect();

        let json = serde_json::to_string(&original).unwrap();
        let back: std::collections::BTreeMap<PluginRequires, bool> =
            serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn toml_round_trip_matches_manifest_string_form() {
        let original: std::collections::BTreeMap<PluginRequires, bool> = [
            (PluginRequires::Http, true),
            (PluginRequires::Secrets, true),
        ]
        .into_iter()
        .collect();

        let toml_str = toml::to_string(&original).unwrap();
        let back: std::collections::BTreeMap<PluginRequires, bool> =
            toml::from_str(&toml_str).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn from_str_accepts_every_known_requires_string() {
        for req in PluginRequires::iter() {
            let s = req.as_str();
            let parsed: PluginRequires = s
                .parse()
                .unwrap_or_else(|_| panic!("known requires string {s:?} failed to parse"));
            assert_eq!(parsed, req);
        }
    }

    #[test]
    fn from_str_rejects_unknown_string() {
        let r: Result<PluginRequires, strum::ParseError> = "telepathy".parse();
        assert!(r.is_err());
    }

    #[test]
    fn from_str_rejects_empty_string() {
        let r: Result<PluginRequires, strum::ParseError> = "".parse();
        assert!(r.is_err());
    }

    #[test]
    fn from_str_is_case_sensitive() {
        for req in PluginRequires::iter() {
            let lower = req.as_str();
            let upper = lower.to_uppercase();
            if upper == lower {
                continue;
            }
            let r: Result<PluginRequires, strum::ParseError> = upper.parse();
            assert!(r.is_err());
        }
    }

    #[test]
    fn from_str_rejects_string_with_trailing_whitespace() {
        for req in PluginRequires::iter().take(2) {
            let with_newline = format!("{}\n", req.as_str());
            let r: Result<PluginRequires, strum::ParseError> = with_newline.parse();
            assert!(r.is_err());

            let with_space = format!("{} ", req.as_str());
            let r: Result<PluginRequires, strum::ParseError> = with_space.parse();
            assert!(r.is_err());
        }
    }

    #[test]
    fn i18n_key_format_is_stable() {
        assert_eq!(PluginRequires::Http.i18n_key(), "plugin.requires.http");
        assert_eq!(
            PluginRequires::SystemSecrets.i18n_key(),
            "plugin.requires.system_secrets"
        );
    }
}
