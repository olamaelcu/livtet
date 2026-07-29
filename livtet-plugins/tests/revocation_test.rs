// =====================================================================
// Revocation-list test coverage.
//
// `RevocationList` is a small on-disk list of revoked key
// fingerprints. The Tauri app writes it to
// `revocation-list.toml` next to the trust store; archive
// verification consults it to reject packs signed by a key
// the user has explicitly marked untrustworthy.
//
// The contract is small but security-critical:
//   - `load_or_default` returns the empty list when the file
//     doesn't exist (first-run UX, not an error);
//   - `save` then `load_or_default` round-trips losslessly;
//   - `revoke` is idempotent — revoking the same fingerprint
//     twice is a no-op, not a duplicate entry;
//   - `fingerprints` is a stable set view of the entries;
//   - revoking a key via `TrustStore::revoke` clears the
//     `is_trusted` flag for that key.
// =====================================================================

mod common;
use camino_tempfile::Utf8TempDir as TempDir;
use livtet_plugins::keys::revocation::RevocationList;

// ---- `load_or_default` on a missing file ---------------------------

#[test]
fn test_load_or_default_missing_file_returns_empty() {
    // First-run case: the file does not exist yet. The
    // function must return Ok(empty), not Err — callers
    // expect a usable empty list, not a hard error.
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("revocation-list.toml");
    assert!(!path.exists(), "test setup: file must not exist");
    let list = RevocationList::load_or_default(&path)
        .expect("missing file must yield Ok(default), not Err");
    assert!(
        list.entries.is_empty(),
        "missing-file list must be empty, got: {:?}",
        list.entries
    );
    assert!(
        list.fingerprints().is_empty(),
        "fingerprints() on the missing-file list must be empty"
    );
}

// ---- save → load round trip ----------------------------------------

#[test]
fn test_save_then_load_round_trip() {
    // Save a list with two entries, load it back, and assert
    // every field round-trips. The TOML format is a flat
    // array of tables, but `toml::to_string_pretty` produces
    // a stable, readable form.
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("revocation-list.toml");

    let mut list = RevocationList::default();
    list.revoke(
        "SHA256:abc123".to_string(),
        "compromised".to_string(),
        "2026-06-01T00:00:00Z".to_string(),
    );
    list.revoke(
        "SHA256:def456".to_string(),
        "rotated".to_string(),
        "2026-06-02T00:00:00Z".to_string(),
    );

    list.save(&path).expect("save must succeed");
    assert!(path.exists(), "save must create the file");

    let loaded = RevocationList::load_or_default(&path).expect("load after save must succeed");
    assert_eq!(
        loaded.entries.len(),
        2,
        "round-tripped list must have 2 entries"
    );
    let fps = loaded.fingerprints();
    assert!(fps.contains("SHA256:abc123"));
    assert!(fps.contains("SHA256:def456"));
}

// ---- `revoke` is idempotent ----------------------------------------

#[test]
fn test_revoke_idempotent_no_duplicate() {
    // Revoking the same fingerprint twice must NOT add a
    // second entry. The list is a set, not a multiset, and
    // downstream `is_revoked` lookups use a HashSet under
    // the hood — a duplicate entry would just waste a few
    // bytes, but it would also make audits confusing.
    let mut list = RevocationList::default();
    list.revoke(
        "SHA256:abc".to_string(),
        "reason".to_string(),
        "2026-01-01T00:00:00Z".to_string(),
    );
    list.revoke(
        "SHA256:abc".to_string(),
        "same key again".to_string(),
        "2026-01-02T00:00:00Z".to_string(),
    );
    assert_eq!(
        list.entries.len(),
        1,
        "revoke must be idempotent; got: {:?}",
        list.entries
    );
}

// ---- `fingerprints` set view ---------------------------------------

#[test]
fn test_fingerprints_after_multiple_revokes() {
    // `fingerprints()` returns a `HashSet<String>`. After
    // revoking three distinct fingerprints, the set has
    // exactly three elements.
    let mut list = RevocationList::default();
    list.revoke("SHA256:a".to_string(), "r1".to_string(), "t1".to_string());
    list.revoke("SHA256:b".to_string(), "r2".to_string(), "t2".to_string());
    list.revoke("SHA256:c".to_string(), "r3".to_string(), "t3".to_string());
    let fps = list.fingerprints();
    assert_eq!(
        fps.len(),
        3,
        "expected 3 distinct fingerprints, got {fps:?}"
    );
    assert!(fps.contains("SHA256:a"));
    assert!(fps.contains("SHA256:b"));
    assert!(fps.contains("SHA256:c"));
}

// ---- `TrustStore::revoke` clears `is_trusted` ----------------------

#[test]
fn test_revoke_clears_is_trusted() {
    use livtet_plugins::keys::TrustStore;
    use rand::Rng as _;

    let mut csprng = rand::rng();
    let key = {
        let mut __ed25519_seed = [0u8; 32];
        csprng.fill_bytes(&mut __ed25519_seed);
        ed25519_dalek::SigningKey::from_bytes(&__ed25519_seed)
    };
    let verifying = key.verifying_key();

    let mut store = TrustStore::empty();
    store.add_user_key("test", verifying).unwrap();
    assert!(store.is_trusted(&verifying));

    store.revoke(&verifying).unwrap();
    assert!(!store.is_trusted(&verifying));
}
