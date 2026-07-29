pub mod keyfile;
pub mod passphrase;
pub mod revocation;
pub mod signing;

use std::collections::HashSet;

use camino::Utf8Path;
use ed25519_dalek::VerifyingKey;
use sha2::{Digest, Sha256};

use crate::types::{TrustedKey, TrustedKeySource};

/// Legacy (label, base64-encoded-32-byte-ed25519-pubkey) table. Kept for
/// any caller that wants to feed keys in without going through the
/// minisign ASCII-armored parser. New code should use
/// [`bundled_trusted_keys`] which handles the minisign `signer.pub`
/// format emitted by `mise run plugin-key-rotate`.
pub const BUILTIN_TRUSTED_KEYS: &[(&str, &str)] = &[];

/// Trust keys compiled into the binary alongside bundled plugins.
///
/// Behind the `bundled` feature: pulls the ASCII-armored minisign
/// `signer.pub` from `livtet-lua-plugins` at link time. Without the
/// feature (CLI tools, tests, non-Tauri consumers), returns an empty
/// list so `TrustStore::load()` stays usable.
pub fn bundled_trusted_keys() -> Vec<(String, VerifyingKey)> {
    #[cfg(feature = "bundled")]
    {
        use crate::keys::signing::parse_pubkey_text;
        match parse_pubkey_text(livtet_lua_plugins::BUNDLED_SIGNER_PUB_TEXT) {
            Ok(vk) => vec![("livtet-lua-plugins-signer".to_string(), vk)],
            Err(e) => {
                tracing::error!(
                    "bundled signer pubkey failed to parse (build.rs should have caught this): {e}"
                );
                Vec::new()
            }
        }
    }
    #[cfg(not(feature = "bundled"))]
    {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
pub struct TrustStore {
    builtin_keys: Vec<(String, VerifyingKey)>,
    user_keys: Vec<(String, VerifyingKey)>,
    revoked: HashSet<String>,
}

impl TrustStore {
    pub fn empty() -> Self {
        Self {
            builtin_keys: Vec::new(),
            user_keys: Vec::new(),
            revoked: HashSet::new(),
        }
    }

    pub fn load() -> Result<Self, crate::archive::error::ArchiveError> {
        let mut store = Self::empty();
        for (label, b64) in BUILTIN_TRUSTED_KEYS {
            let key = parse_pubkey_b64(b64)?;
            store.builtin_keys.push((label.to_string(), key));
        }
        for (label, key) in bundled_trusted_keys() {
            store.builtin_keys.push((label, key));
        }
        Ok(store)
    }

    pub fn add_user_key(
        &mut self,
        label: &str,
        key: VerifyingKey,
    ) -> Result<(), crate::archive::error::ArchiveError> {
        if label.is_empty() {
            return Err(crate::archive::error::ArchiveError::Key(
                "label cannot be empty".to_string(),
            ));
        }
        self.revoked.remove(&fingerprint(&key));
        self.user_keys.push((label.to_string(), key));
        Ok(())
    }

    /// Push a key onto the builtin trust list. Unlike
    /// `add_user_key`, there is no revocation-clearing
    /// side-effect: builtins are not user-controlled and
    /// therefore cannot have been previously revoked. The
    /// label is also unconstrained — builtins come from
    /// the compiled-in `BUILTIN_TRUSTED_KEYS` constant and
    /// are trusted by the application itself.
    pub fn add_builtin_key(&mut self, label: &str, key: VerifyingKey) {
        self.builtin_keys.push((label.to_string(), key));
    }

    pub fn revoke(
        &mut self,
        key: &VerifyingKey,
    ) -> Result<(), crate::archive::error::ArchiveError> {
        self.revoked.insert(fingerprint(key));
        self.user_keys.retain(|(_, k)| k != key);
        Ok(())
    }

    pub fn is_revoked(&self, fingerprint: &str) -> bool {
        self.revoked.contains(fingerprint)
    }

    pub fn is_trusted(&self, key: &VerifyingKey) -> bool {
        let fp = fingerprint(key);
        if self.revoked.contains(&fp) {
            return false;
        }
        self.builtin_keys.iter().any(|(_, k)| k == key)
            || self.user_keys.iter().any(|(_, k)| k == key)
    }

    pub fn user_key_by_label(&self, label: &str) -> Option<&VerifyingKey> {
        self.user_keys
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, k)| k)
    }

    pub fn user_keys_snapshot(&self) -> Vec<(String, VerifyingKey)> {
        self.user_keys.clone()
    }

    pub fn list_trusted(&self) -> Vec<TrustedKey> {
        let mut out = Vec::new();
        for (label, key) in &self.builtin_keys {
            out.push(TrustedKey {
                label: label.clone(),
                fingerprint: fingerprint(key),
                source: TrustedKeySource::Builtin,
            });
        }
        for (label, key) in &self.user_keys {
            let fp = fingerprint(key);
            if self.revoked.contains(&fp) {
                out.push(TrustedKey {
                    label: label.clone(),
                    fingerprint: fp,
                    source: TrustedKeySource::Revoked,
                });
            } else {
                out.push(TrustedKey {
                    label: label.clone(),
                    fingerprint: fp,
                    source: TrustedKeySource::User,
                });
            }
        }
        out
    }

    pub fn load_from_dir(dir: &Utf8Path) -> Result<Self, crate::archive::error::ArchiveError> {
        use fs_err as fs;
        let mut store = Self::empty();
        if !dir.exists() {
            return Ok(store);
        }
        for entry in fs::read_dir(dir.as_std_path())? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("pub") {
                continue;
            }
            let label = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "trusted".to_string());
            let text = match fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            if let Ok(vk) = signing::parse_pubkey_text(&text) {
                let _ = store.add_user_key(&label, vk);
            }
        }
        Ok(store)
    }
}

pub fn fingerprint(key: &VerifyingKey) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.to_bytes());
    format!("SHA256:{}", hex::encode(hasher.finalize()))
}

pub fn parse_pubkey_b64(s: &str) -> Result<VerifyingKey, crate::archive::error::ArchiveError> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| crate::archive::error::ArchiveError::Key(format!("base64 decode: {e}")))?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
        crate::archive::error::ArchiveError::Key("key must be 32 bytes".to_string())
    })?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|e| crate::archive::error::ArchiveError::Key(format!("ed25519: {e}")))
}
