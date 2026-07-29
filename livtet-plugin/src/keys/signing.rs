use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::archive::error::ArchiveError;

/// Byte offset of the ed25519 seed inside a minisign 0.9 `SecretKey::to_bytes()`
/// serialization.
///
/// The C2SP-compatible layout is:
///
///   sig_alg(2) + kdf_alg(2) + chk_alg(2) + kdf_salt(32) + kdf_opslimit(8)
///   + kdf_memlimit(8) + keynum(8) = 62 bytes of header,
///
/// followed by the expanded ed25519 secret key (64 bytes: 32-byte seed then
///     32-byte public key) and a 32-byte BLAKE2b checksum.
const MINISIGN_SK_SEED_OFFSET: usize = 62;

pub fn sign_bytes(key: &SigningKey, message: &[u8]) -> Result<[u8; 64], ArchiveError> {
    let sig = key.sign(message);
    Ok(sig.to_bytes())
}

pub fn verify_bytes(
    key: &VerifyingKey,
    message: &[u8],
    signature: &[u8],
) -> Result<(), ArchiveError> {
    if signature.len() != 64 {
        return Err(ArchiveError::InvalidSignature);
    }
    let sig_bytes: [u8; 64] = signature
        .try_into()
        .map_err(|_| ArchiveError::InvalidSignature)?;
    let sig = Signature::from_bytes(&sig_bytes);
    key.verify(message, &sig)
        .map_err(|_| ArchiveError::InvalidSignature)
}

pub fn parse_pubkey_from_minisign_box(box_str: &str) -> Result<VerifyingKey, ArchiveError> {
    use minisign::PublicKeyBox;
    let trimmed = box_str.trim();
    let pk_box = PublicKeyBox::from_string(trimmed)
        .map_err(|e| ArchiveError::Key(format!("minisign pubkey box: {e}")))?;
    let pk = pk_box
        .into_public_key()
        .map_err(|e| ArchiveError::Key(format!("extract pubkey: {e}")))?;
    let serialized = pk.to_bytes();
    let bytes: [u8; 32] = serialized[serialized.len() - 32..]
        .try_into()
        .map_err(|_| ArchiveError::Key("pubkey not 32 bytes".to_string()))?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| ArchiveError::Key(format!("ed25519: {e}")))
}

pub fn parse_pubkey_text(text: &str) -> Result<VerifyingKey, ArchiveError> {
    parse_pubkey_from_minisign_box(text)
}

/// Extract the raw 32-byte ed25519 seed from a parsed minisign secret key.
///
/// The seed is what `ed25519_dalek::SigningKey::from_bytes` needs to
/// reconstruct a signing key compatible with both archive signing (pack.rs)
/// and index signing (publisher).
pub fn minisign_secret_key_seed(sk: &minisign::SecretKey) -> Result<[u8; 32], ArchiveError> {
    let raw = sk.to_bytes();
    let end = MINISIGN_SK_SEED_OFFSET + 32;
    if raw.len() < end {
        return Err(ArchiveError::Key(format!(
            "minisign secret key serialization shorter than expected (got {} bytes, need at least {end})",
            raw.len()
        )));
    }
    raw[MINISIGN_SK_SEED_OFFSET..end]
        .try_into()
        .map_err(|_| ArchiveError::Key("failed to slice ed25519 seed".to_string()))
}

/// Load a minisign secret keyfile from disk and return both the parsed
/// `minisign::SecretKey` (useful for deriving the public key via
/// `PublicKey::from_secret_key`) and an `ed25519_dalek::SigningKey` built from
/// the embedded seed (useful for producing raw 64-byte ed25519 signatures
/// compatible with the rest of the livtet codebase).
///
/// Encrypted keys are unlocked using the `LIVTET_KEY_PASSPHRASE` environment
/// variable.
pub fn load_minisign_signing_key(
    key_path: &camino::Utf8Path,
) -> Result<(minisign::SecretKey, SigningKey), ArchiveError> {
    use fs_err as fs;

    let text = fs::read_to_string(key_path).map_err(|_| ArchiveError::NoSigningKey {
        label: key_path.to_string(),
    })?;
    let sk_box = minisign::SecretKeyBox::from_string(text.trim())
        .map_err(|e| ArchiveError::Key(format!("minisign SecretKeyBox: {e}")))?;
    let sk = if sk_box.clone().into_unencrypted_secret_key().is_ok() {
        let sk_box = minisign::SecretKeyBox::from_string(text.trim())
            .map_err(|e| ArchiveError::Key(format!("minisign SecretKeyBox re-parse: {e}")))?;
        sk_box
            .into_unencrypted_secret_key()
            .map_err(|e| ArchiveError::Key(format!("minisign open unencrypted: {e}")))?
    } else {
        let passphrase = std::env::var("LIVTET_KEY_PASSPHRASE").ok();
        sk_box
            .into_secret_key(passphrase)
            .map_err(|e| ArchiveError::Key(format!("minisign open encrypted: {e}")))?
    };
    let seed = minisign_secret_key_seed(&sk)?;
    let signing_key = SigningKey::from_bytes(&seed);
    Ok((sk, signing_key))
}
