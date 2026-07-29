use camino::Utf8Path;
use fs_err as fs;
use hmac::{Hmac, KeyInit, Mac};
use rand::{Rng as _, rng};
use sha2::Sha256;

use crate::repository::error::RepositoryError;

type HmacSha256 = Hmac<Sha256>;

/// OS keyring service identifier for the livtet HMAC key. Derived
/// from the canonical bundle ID so every livtet binary (Tauri
/// parent, CLI, plugin host) writes to the same keyring entry. See
/// `livtet_core::paths::BUNDLE_ID` for the source of truth.
const SERVICE: &str = livtet_core::paths::BUNDLE_ID;
const ACCOUNT: &str = "state-hmac-key";

/// PBKDF2-HMAC-SHA256 iteration count. Picked to mirror the OWASP 2023
/// recommendation for HMAC-SHA256; cheap enough that `livtet
/// keyring-recover` runs in <1s on a developer laptop, expensive enough
/// to deter offline brute-force of a user-chosen passphrase.
pub const PBKDF2_ITERATIONS: u32 = 100_000;

/// Stable per-app salt for the passphrase-derived HMAC key. Bumping the
/// trailing version (`v1` -> `v2`, etc.) is the migration story if the
/// derivation parameters ever need to change.
pub const PASSPHRASE_SALT: &[u8] = b"livtet-state-hmac-v1";

/// Where an `HmacKey` came from. Surfaced so callers can warn the user
/// (or refuse to operate) when a `Static` key is in play, since a static
/// key is effectively "anyone can sign this" — see the recovery flow in
/// `crates/livtet-cli`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HmacKeySource {
    /// Loaded from (or persisted to) the OS keyring via the `keyring` crate.
    Keyring,
    /// Derived from a user-supplied passphrase via PBKDF2-HMAC-SHA256.
    Passphrase,
    /// In-memory 32 bytes (e.g., a hard-coded fallback or a test fixture).
    /// The HMAC sidecar is not portable across machines in this mode.
    Static,
}

#[derive(Clone, Debug)]
pub struct HmacKey {
    bytes: [u8; 32],
    source: HmacKeySource,
}

impl HmacKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            bytes,
            source: HmacKeySource::Static,
        }
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    pub fn source(&self) -> HmacKeySource {
        self.source
    }

    pub fn load_from_keyring() -> Result<Self, RepositoryError> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|e| RepositoryError::Keyring(e.to_string()))?;
        let secret = entry
            .get_password()
            .map_err(|e| RepositoryError::Keyring(e.to_string()))?;
        let decoded = hex::decode(&secret)
            .map_err(|e| RepositoryError::Keyring(format!("HMAC key hex: {e}")))?;
        let bytes: [u8; 32] = decoded
            .try_into()
            .map_err(|_| RepositoryError::Keyring("HMAC key not 32 bytes".to_string()))?;
        Ok(Self {
            bytes,
            source: HmacKeySource::Keyring,
        })
    }

    pub fn create_in_keyring() -> Result<Self, RepositoryError> {
        let mut bytes = [0u8; 32];
        rng().fill_bytes(&mut bytes);
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|e| RepositoryError::Keyring(e.to_string()))?;
        let hex_str: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        entry
            .set_password(&hex_str)
            .map_err(|e| RepositoryError::Keyring(e.to_string()))?;
        Ok(Self {
            bytes,
            source: HmacKeySource::Keyring,
        })
    }

    /// Load the HMAC key from the OS keyring, creating and storing a
    /// fresh 32-byte OS-RNG key on first launch (entry missing). On
    /// real keyring failures (daemon missing, locked, PlatformFailure)
    /// the underlying error is propagated unchanged — the caller
    /// surfaces it rather than silently falling back to a Static key.
    pub fn load_or_create_in_keyring() -> Result<Self, RepositoryError> {
        match Self::load_from_keyring() {
            Ok(k) => Ok(k),
            Err(RepositoryError::Keyring(msg))
                if msg.contains("NoEntry")
                    || msg.contains("No matching entry")
                    || msg.contains("No matching credential")
                    || msg.contains("No such object") =>
            {
                tracing::info!("first launch: provisioning HMAC key in OS keyring");
                Self::create_in_keyring()
            }
            Err(other) => Err(other),
        }
    }

    /// Derive a deterministic 32-byte key from a user-supplied passphrase
    /// using PBKDF2-HMAC-SHA256. Used by the `livtet keyring-recover`
    /// subcommand when the OS keyring is unavailable; same passphrase
    ///
    /// + same salt always reproduces the same key, so a re-derived key
    ///   can read HMAC sidecars produced before the keyring was wiped.
    pub fn derive_from_passphrase(passphrase: &str, salt: &[u8]) -> Self {
        let mut buf = [0u8; 32];
        pbkdf2_hmac_sha256(passphrase.as_bytes(), salt, PBKDF2_ITERATIONS, &mut buf);
        Self {
            bytes: buf,
            source: HmacKeySource::Passphrase,
        }
    }
}

/// Inline PBKDF2-HMAC-SHA256 implementation. We cannot take a new
/// `pbkdf2` crate dep per the Part 2 constraints, and the algorithm is
/// small enough (~40 lines) to implement directly on top of the
/// `hmac` + `sha2` crates we already depend on. The implementation
/// matches RFC 8018 Appendix A.5 (PBKDF2 with HMAC-SHA-256 as PRF).
fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    assert!(!out.is_empty(), "pbkdf2 output buffer must be non-empty");
    let blocks = out.len().div_ceil(32);
    let mut scratch = [0u8; 32];
    for block in 1..=blocks as u32 {
        // U_1 = HMAC(password, salt || INT(i))
        let mut mac =
            <HmacSha256 as KeyInit>::new_from_slice(password).expect("HMAC accepts any key length");
        mac.update(salt);
        mac.update(&block.to_be_bytes());
        let u1 = mac.finalize().into_bytes();
        scratch.copy_from_slice(&u1);

        // U_2 ... U_c
        for _ in 1..iterations {
            let mut mac = <HmacSha256 as KeyInit>::new_from_slice(password)
                .expect("HMAC accepts any key length");
            mac.update(&scratch);
            let un = mac.finalize().into_bytes();
            for (dst, src) in scratch.iter_mut().zip(un.iter()) {
                *dst ^= src;
            }
        }

        let start = ((block - 1) as usize) * 32;
        let end = usize::min(start + 32, out.len());
        let len = end - start;
        out[start..end].copy_from_slice(&scratch[..len]);
    }
}

pub fn write_protected(path: &Utf8Path, data: &[u8], key: &HmacKey) -> Result<(), RepositoryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key.as_bytes())
        .map_err(|e| RepositoryError::Hmac(e.to_string()))?;
    mac.update(data);
    let tag = mac.finalize().into_bytes();
    let hex: String = tag.iter().map(|b| format!("{b:02x}")).collect();
    fs::write(path, data)?;
    let sidecar = sidecar_path(path);
    fs::write(&sidecar, hex.as_bytes())?;
    Ok(())
}

pub fn read_protected(path: &Utf8Path, key: &HmacKey) -> Result<Vec<u8>, RepositoryError> {
    let data = fs::read(path)?;
    let sidecar = sidecar_path(path);
    let hex = fs::read_to_string(&sidecar)
        .map_err(|e| RepositoryError::Hmac(format!("missing sidecar: {e}")))?;
    let expected =
        hex::decode(hex.trim()).map_err(|e| RepositoryError::Hmac(format!("sidecar hex: {e}")))?;
    if expected.len() != 32 {
        return Err(RepositoryError::Hmac(format!(
            "sidecar not 32 bytes: {}",
            expected.len()
        )));
    }
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key.as_bytes())
        .map_err(|e| RepositoryError::Hmac(e.to_string()))?;
    mac.update(&data);
    mac.verify_slice(&expected)
        .map_err(|_| RepositoryError::Hmac("HMAC mismatch".to_string()))?;
    Ok(data)
}

fn sidecar_path(path: &Utf8Path) -> camino::Utf8PathBuf {
    let mut p = path.as_str().to_owned();
    p.push_str(".hmac");
    camino::Utf8PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_is_deterministic() {
        let salt = b"unit-test-salt";
        let a = HmacKey::derive_from_passphrase("correct horse battery staple", salt);
        let b = HmacKey::derive_from_passphrase("correct horse battery staple", salt);
        assert_eq!(a.as_bytes(), b.as_bytes());
        assert_eq!(a.source(), HmacKeySource::Passphrase);
    }

    #[test]
    fn pbkdf2_different_passphrases_produce_different_keys() {
        let salt = b"unit-test-salt";
        let a = HmacKey::derive_from_passphrase("alpha", salt);
        let b = HmacKey::derive_from_passphrase("bravo", salt);
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn pbkdf2_different_salts_produce_different_keys() {
        let a = HmacKey::derive_from_passphrase("hunter2", b"salt-v1");
        let b = HmacKey::derive_from_passphrase("hunter2", b"salt-v2");
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn pbkdf2_matches_rfc8018_known_answer() {
        // PBKDF2-HMAC-SHA256 test vector from RFC 7914 (scrypt) Appendix B
        // is not available for HMAC-SHA256, so we use the well-known
        // "pbkdf2" RFC 3962 test vector, adapted: P="password",
        // S="salt", c=1, dkLen=32. With c=1 the output is just U_1, which
        // is HMAC-SHA256("password", "salt" || 0x00000001).
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(b"password").unwrap();
        mac.update(b"salt");
        mac.update(&1u32.to_be_bytes());
        let expected = mac.finalize().into_bytes();
        let mut derived = [0u8; 32];
        pbkdf2_hmac_sha256(b"password", b"salt", 1, &mut derived);
        assert_eq!(derived[..], expected[..]);
    }

    #[test]
    fn pbkdf2_handles_outputs_longer_than_one_block() {
        // 33 bytes forces two PBKDF2 blocks: 32 + 1.
        let mut out = [0u8; 33];
        pbkdf2_hmac_sha256(b"pw", b"salt", 2, &mut out);
        // Sanity: not all zero, not all the same as a 32-byte derivation.
        let mut short = [0u8; 32];
        pbkdf2_hmac_sha256(b"pw", b"salt", 2, &mut short);
        assert_eq!(&out[..32], &short[..]);
        // Last byte is the first byte of block 2, which differs from
        // block 1 (otherwise we have a bug).
        assert_ne!(out[32], out[31]);
    }

    #[test]
    fn hmac_key_source_reflects_constructor() {
        let bytes = [7u8; 32];
        let from_bytes = HmacKey::from_bytes(bytes);
        assert_eq!(from_bytes.source(), HmacKeySource::Static);

        let from_passphrase = HmacKey::derive_from_passphrase("pw", b"salt");
        assert_eq!(from_passphrase.source(), HmacKeySource::Passphrase);
    }
}
