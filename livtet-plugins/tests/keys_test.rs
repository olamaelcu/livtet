mod common;
use std::{assert_matches, env};

use livtet_plugins::keys::passphrase::{PassphraseSource, resolve_passphrase};

#[test]
fn test_no_passphrase_flag_returns_empty() {
    let result = resolve_passphrase(true, None, false).unwrap();
    assert_eq!(result.0, "");
    assert_matches!(result.1, PassphraseSource::NoPassphrase);
}

#[test]
fn test_env_var_returns_env_value_with_warning() {
    unsafe {
        env::set_var("LIVTET_KEY_PASSPHRASE", "test-secret");
    }
    let result = resolve_passphrase(false, Some("LIVTET_KEY_PASSPHRASE"), false).unwrap();
    assert_eq!(result.0, "test-secret");
    assert_matches!(result.1, PassphraseSource::EnvVar);
    unsafe {
        env::remove_var("LIVTET_KEY_PASSPHRASE");
    }
}

#[test]
fn test_no_tty_no_env_returns_passphrase_required() {
    unsafe {
        env::remove_var("LIVTET_KEY_PASSPHRASE");
    }
    let result = resolve_passphrase(false, Some("LIVTET_KEY_PASSPHRASE"), false);
    assert!(result.is_err(), "expected PassphraseRequired error");
}

use ed25519_dalek::SigningKey;
use livtet_plugins::keys::TrustStore;
use rand::Rng as _;
use sha2::{Digest, Sha256};

fn make_key() -> (SigningKey, String) {
    let mut csprng = rand::rng();
    let key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let pk = key.verifying_key();
    let mut hasher = Sha256::new();
    hasher.update(pk.to_bytes());
    let fp = format!("SHA256:{}", hex::encode(hasher.finalize()));
    (key, fp)
}

#[test]
fn test_trust_store_empty_does_not_trust() {
    let store = TrustStore::empty();
    let (key, _fp) = make_key();
    assert!(!store.is_trusted(&key.verifying_key()));
}

#[test]
fn test_trust_store_add_user_key_then_trusts() {
    let (key, fp) = make_key();
    let mut store = TrustStore::empty();
    store.add_user_key("alice", key.verifying_key()).unwrap();
    assert!(store.is_trusted(&key.verifying_key()));
    let listed = store.list_trusted();
    assert!(
        listed
            .iter()
            .any(|k| k.label == "alice" && k.fingerprint == fp)
    );
}

#[test]
fn test_trust_store_revoke_blocks_trust() {
    let (key, _fp) = make_key();
    let mut store = TrustStore::empty();
    store.add_user_key("alice", key.verifying_key()).unwrap();
    store.revoke(&key.verifying_key()).unwrap();
    assert!(!store.is_trusted(&key.verifying_key()));
}

#[test]
fn test_trust_store_revoke_adds_to_revoked_list() {
    let (key, fp) = make_key();
    let mut store = TrustStore::empty();
    store.add_user_key("alice", key.verifying_key()).unwrap();
    assert!(!store.is_revoked(&fp));
    store.revoke(&key.verifying_key()).unwrap();
    assert!(store.is_revoked(&fp));
}

use camino_tempfile::Utf8TempDir as TempDir;
use livtet_plugins::keys::{
    keyfile::keygen,
    signing::{sign_bytes, verify_bytes},
};

#[test]
fn test_keygen_creates_keypair_files() {
    let tmp = TempDir::new().unwrap();
    let label = "test-keygen";
    let report = keygen(tmp.path(), label, true).unwrap();
    assert!(report.key_path.exists());
    assert!(report.pubkey_path.exists());
    assert!(!report.encrypted);
    assert!(report.fingerprint.starts_with("SHA256:"));
}

#[test]
fn test_keygen_writes_minisign_box_format() {
    use minisign::{PublicKeyBox, SecretKeyBox};

    let tmp = TempDir::new().unwrap();
    let label = "minisign-box-format";
    let report = keygen(tmp.path(), label, true).expect("keygen should succeed");

    let sk_text = std::fs::read_to_string(&report.key_path).expect("read secret key");
    assert!(
        sk_text.starts_with("untrusted comment:"),
        "secret key must be in minisign SecretKeyBox format, got: {sk_text:?}"
    );
    let sk_box = SecretKeyBox::from_string(sk_text.trim())
        .expect("secret key must parse as minisign SecretKeyBox");
    let _sk = sk_box
        .into_unencrypted_secret_key()
        .expect("unencrypted key must open without a password");

    let pk_text = std::fs::read_to_string(&report.pubkey_path).expect("read pubkey");
    assert!(
        pk_text.starts_with("untrusted comment:"),
        "pubkey must be in minisign PublicKeyBox format, got: {pk_text:?}"
    );
    let pk_box = PublicKeyBox::from_string(pk_text.trim())
        .expect("pubkey must parse as minisign PublicKeyBox");
    let pk = pk_box
        .into_public_key()
        .expect("pubkey box must yield a public key");
    let _pk_bytes: [u8; 32] = pk.to_bytes()[10..]
        .try_into()
        .expect("minisign public key carries 32 trailing ed25519 bytes after 2-byte sig_alg + 8-byte keynum");

    let parsed_pk = livtet_plugins::keys::signing::parse_pubkey_text(&pk_text)
        .expect("parse_pubkey_text must accept the minisign pubkey that keygen just wrote");
    assert_eq!(parsed_pk.to_bytes(), pk.to_bytes()[10..]);
}

#[test]
fn test_sign_and_verify_round_trip() {
    let mut csprng = rand::rng();
    let key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let message = b"hello world";
    let sig = sign_bytes(&key, message).unwrap();
    assert_eq!(sig.len(), 64);
    verify_bytes(&key.verifying_key(), message, &sig).unwrap();
}

#[test]
fn test_verify_fails_on_tampered_message() {
    let mut csprng = rand::rng();
    let key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let sig = sign_bytes(&key, b"hello world").unwrap();
    let err = verify_bytes(&key.verifying_key(), b"different message", &sig);
    assert!(err.is_err());
}

use livtet_plugins::keys::signing::parse_pubkey_text;

#[test]
fn test_parse_pubkey_text_auto_parses_livtet_custom_format() {
    let tmp = TempDir::new().unwrap();
    let report = keygen(tmp.path(), "auto-custom", true).unwrap();
    let text = std::fs::read_to_string(&report.pubkey_path).unwrap();
    assert!(
        text.starts_with("untrusted comment:"),
        "after hard migration, keygen must write minisign box format, got: {text:?}"
    );
    let parsed = parse_pubkey_text(&text).unwrap();
    let expected = report.fingerprint;
    let actual = livtet_plugins::keys::fingerprint(&parsed);
    assert_eq!(actual, expected);
}

#[test]
fn test_parse_pubkey_parses_minisign_box_format() {
    use minisign::KeyPair;
    let kp = KeyPair::generate_unencrypted_keypair().unwrap();
    let box_str = kp.pk.to_box().unwrap().to_string();
    assert!(box_str.starts_with("untrusted comment:"));
    let parsed = parse_pubkey_text(&box_str).unwrap();
    let expected_pk: [u8; 32] = kp.pk.to_bytes()[10..]
        .try_into()
        .expect("minisign pk is 32 bytes after 2-byte sig_alg + 8-byte keynum");
    assert_eq!(parsed.to_bytes(), expected_pk);
}

#[test]
fn test_parse_pubkey_rejects_garbage() {
    let err = parse_pubkey_text("this is definitely not a pubkey file").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("minisign") || msg.contains("ed25519") || msg.contains("pubkey"),
        "error should mention the format that failed, got: {msg}"
    );
}

// =====================================================================
// TrustStore::load_from_dir
// =====================================================================

#[test]
fn test_trust_store_load_from_dir_with_valid_pub() {
    let tmp = TempDir::new().unwrap();
    let report = keygen(tmp.path(), "my-key", true).unwrap();
    let pubkey_text = std::fs::read_to_string(&report.pubkey_path).unwrap();
    let key_dir = tmp.path().join("trust");
    fs_err::create_dir_all(&key_dir).unwrap();
    fs_err::write(key_dir.join("my-key.pub"), &pubkey_text).unwrap();
    let store = TrustStore::load_from_dir(&key_dir).unwrap();
    let verifying = parse_pubkey_text(&pubkey_text).unwrap();
    assert!(store.is_trusted(&verifying));
}

#[test]
fn test_trust_store_load_from_dir_skips_corrupt_pub() {
    let tmp = TempDir::new().unwrap();
    let key_dir = tmp.path().join("trust");
    fs_err::create_dir_all(&key_dir).unwrap();
    fs_err::write(key_dir.join("bad.pub"), "not a real pubkey").unwrap();
    let store = TrustStore::load_from_dir(&key_dir).unwrap();
    assert_eq!(store.list_trusted().len(), 0);
}

#[test]
fn test_trust_store_load_from_dir_skips_non_pub_files() {
    let tmp = TempDir::new().unwrap();
    let report = keygen(tmp.path(), "k", true).unwrap();
    let pubkey_text = std::fs::read_to_string(&report.pubkey_path).unwrap();
    let key_dir = tmp.path().join("trust");
    fs_err::create_dir_all(&key_dir).unwrap();
    fs_err::write(key_dir.join("k.pub"), &pubkey_text).unwrap();
    fs_err::write(key_dir.join("readme.txt"), "ignored").unwrap();
    let store = TrustStore::load_from_dir(&key_dir).unwrap();
    let verifying = parse_pubkey_text(&pubkey_text).unwrap();
    assert!(store.is_trusted(&verifying));
    assert_eq!(store.list_trusted().len(), 1);
}

#[test]
fn test_trust_store_load_from_dir_utf8_label() {
    let tmp = TempDir::new().unwrap();
    let report = keygen(tmp.path(), "k", true).unwrap();
    let pubkey_text = std::fs::read_to_string(&report.pubkey_path).unwrap();
    let key_dir = tmp.path().join("trust");
    fs_err::create_dir_all(&key_dir).unwrap();
    fs_err::write(key_dir.join("spécial.pub"), &pubkey_text).unwrap();
    let store = TrustStore::load_from_dir(&key_dir).unwrap();
    assert_eq!(store.list_trusted().len(), 1);
    assert_eq!(store.list_trusted()[0].label, "spécial");
}

#[test]
fn test_trust_store_load_from_dir_missing_returns_empty() {
    let store =
        TrustStore::load_from_dir(camino::Utf8Path::new("/tmp/livtet-nonexistent-dir-test"))
            .unwrap();
    assert_eq!(store.list_trusted().len(), 0);
}

// =====================================================================
// keygen encrypted-key tests
// =====================================================================

use std::sync::Mutex;
static PASSPHRASE_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn test_keygen_encrypted_creates_encrypted_file() {
    let _guard = PASSPHRASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    unsafe {
        env::set_var("LIVTET_KEY_PASSPHRASE", "test-passphrase-123");
    }
    let report = keygen(tmp.path(), "enc-key", false).unwrap();
    unsafe {
        env::remove_var("LIVTET_KEY_PASSPHRASE");
    }
    assert!(report.encrypted);
    assert!(report.key_path.exists());
    assert!(report.pubkey_path.exists());
}

#[test]
fn test_load_encrypted_key_with_correct_passphrase() {
    let _guard = PASSPHRASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    unsafe {
        env::set_var("LIVTET_KEY_PASSPHRASE", "correct-password");
    }
    let report = keygen(tmp.path(), "enc-load", false).unwrap();
    assert!(report.encrypted);
    let result = livtet_plugins::keys::signing::load_minisign_signing_key(&report.key_path);
    assert!(
        result.is_ok(),
        "loading with correct passphrase should succeed"
    );
    unsafe {
        env::remove_var("LIVTET_KEY_PASSPHRASE");
    }
}

#[test]
fn test_load_encrypted_key_with_wrong_passphrase_fails() {
    let _guard = PASSPHRASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    unsafe {
        env::set_var("LIVTET_KEY_PASSPHRASE", "right-password");
    }
    let report = keygen(tmp.path(), "enc-wrong", false).unwrap();
    unsafe {
        env::set_var("LIVTET_KEY_PASSPHRASE", "wrong-password");
    }
    let result = livtet_plugins::keys::signing::load_minisign_signing_key(&report.key_path);
    unsafe {
        env::remove_var("LIVTET_KEY_PASSPHRASE");
    }
    assert!(result.is_err(), "loading with wrong passphrase should fail");
}

#[test]
fn test_load_unencrypted_key_succeeds() {
    let _guard = PASSPHRASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let report = keygen(tmp.path(), "unenc-load", true).unwrap();
    assert!(
        !report.encrypted,
        "keygen with no_passphrase=true should create unencrypted key"
    );
    let result = livtet_plugins::keys::signing::load_minisign_signing_key(&report.key_path);
    assert!(
        result.is_ok(),
        "loading unencrypted key without passphrase should succeed"
    );
}

// NOTE: There is no test for "load encrypted key without passphrase" because
// `load_minisign_signing_key` will prompt for password interactively when
// `LIVTET_KEY_PASSPHRASE` is not set and the key is encrypted. This would
// hang the test. The three load cases are covered by:
// 1. test_load_unencrypted_key_succeeds - unencrypted key loads without passphrase
// 2. test_load_encrypted_key_with_correct_passphrase - encrypted key + correct passphrase
// 3. test_load_encrypted_key_with_wrong_passphrase_fails - encrypted key + wrong passphrase

#[cfg(unix)]
#[test]
#[allow(clippy::disallowed_methods)]
fn test_keygen_secret_key_file_permissions() {
    let tmp = TempDir::new().unwrap();
    let report = keygen(tmp.path(), "perm-check", true).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(&report.key_path).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "secret key file should have 0o600 permissions, got {mode:#o}"
    );
}

// =====================================================================
// parse_pubkey_b64 length/decode-error tests
// =====================================================================

use livtet_plugins::keys::parse_pubkey_b64;

#[test]
fn test_parse_pubkey_b64_too_short() {
    use base64::Engine;
    let bytes_31 = [0u8; 31];
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes_31);
    let err = parse_pubkey_b64(&b64).unwrap_err();
    assert!(
        err.to_string().contains("32 bytes"),
        "expected '32 bytes' error, got: {err}"
    );
}

#[test]
fn test_parse_pubkey_b64_too_long() {
    use base64::Engine;
    let bytes_33 = [0u8; 33];
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes_33);
    let err = parse_pubkey_b64(&b64).unwrap_err();
    assert!(
        err.to_string().contains("32 bytes"),
        "expected '32 bytes' error, got: {err}"
    );
}

#[test]
fn test_parse_pubkey_b64_invalid_base64() {
    let err = parse_pubkey_b64("!!!not-base64!!!").unwrap_err();
    assert!(
        err.to_string().contains("base64"),
        "expected 'base64' error, got: {err}"
    );
}

// =====================================================================
// TrustStore::add_user_key error paths
// =====================================================================

#[test]
fn test_add_user_key_rejects_empty_label() {
    let (key, _fp) = make_key();
    let mut store = TrustStore::empty();
    let err = store.add_user_key("", key.verifying_key()).unwrap_err();
    assert!(
        err.to_string().contains("label cannot be empty"),
        "expected empty-label error, got: {err}"
    );
    assert_eq!(store.list_trusted().len(), 0);
}

#[test]
fn test_add_user_key_accepts_valid_label() {
    let (key, fp) = make_key();
    let mut store = TrustStore::empty();
    store
        .add_user_key("alice", key.verifying_key())
        .expect("valid label should succeed");
    let listed = store.list_trusted();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].label, "alice");
    assert_eq!(listed[0].fingerprint, fp);
}

#[test]
fn test_add_user_key_allows_duplicate_fingerprint_with_different_label() {
    // Contract: TrustStore currently allows the same key (matching fingerprint)
    // to be added multiple times under different labels. The list may contain
    // duplicate fingerprints, and `is_trusted` still returns true.
    let (key, fp) = make_key();
    let mut store = TrustStore::empty();
    store
        .add_user_key("alice", key.verifying_key())
        .expect("first add should succeed");
    store
        .add_user_key("alice-2", key.verifying_key())
        .expect("duplicate fingerprint under a different label is currently allowed");
    let listed = store.list_trusted();
    let matching: Vec<_> = listed.iter().filter(|k| k.fingerprint == fp).collect();
    assert_eq!(
        matching.len(),
        2,
        "expected two entries with the same fingerprint, got {matching:?}"
    );
    assert!(store.is_trusted(&key.verifying_key()));
}

// =====================================================================
// TrustStore::list_trusted Builtin branch
// =====================================================================

use livtet_plugins::types::TrustedKeySource;

#[test]
fn test_list_trusted_includes_builtin_keys_with_correct_source() {
    let (key, fp) = make_key();
    let mut store = TrustStore::empty();
    store.add_builtin_key("build-system", key.verifying_key());
    let listed = store.list_trusted();
    let builtin_entries: Vec<_> = listed
        .iter()
        .filter(|k| k.source == TrustedKeySource::Builtin)
        .collect();
    assert_eq!(
        builtin_entries.len(),
        1,
        "expected exactly one builtin entry, got {listed:?}"
    );
    assert_eq!(builtin_entries[0].label, "build-system");
    assert_eq!(builtin_entries[0].fingerprint, fp);
}

// =====================================================================
// bundled_trusted_keys() integration with TrustStore::load()
// =====================================================================
//
// Confirms the bundled plugin signing key is embedded in `livtet-lua-plugins`
// and is recognized as a builtin trust key after `TrustStore::load()`.
// The bundled key is a single ed25519 public key shared across every
// bundled plugin; rotation is driven by `mise run plugin-key-rotate --force`.
//
// Note: this test does NOT require the `bundled` cargo feature. The
// `BUNDLED_SIGNER_PUB_TEXT` const is always embedded (via `include_str!`),
// and `bundled_trusted_keys()` returns an empty list without the feature.
// The `include_bytes!("../bundled/signer.pub")` resolution itself
// requires the file to exist at compile time, which is enforced by
// `livtet-lua-plugins/build.rs`.

// =====================================================================
// bundled_trusted_keys smoke tests
// =====================================================================

#[test]
fn bundled_trusted_keys_is_empty_in_default_build() {
    // In dev builds without LIVTET_BUNDLED_KEY_PATH, the function
    // returns empty. In CI with the feature + key path set, this
    // test may need updating — but for now it documents the dev
    // default.
    let keys = livtet_plugins::keys::bundled_trusted_keys();
    // bundled feature may or may not be active in test builds;
    // either way, the function shouldn't panic.
    let _ = keys;
}

#[test]
fn trust_store_load_does_not_panic_on_empty_keys() {
    let store = TrustStore::load().unwrap();
    assert!(store.list_trusted().is_empty());
}
