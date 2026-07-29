use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use livtet_plugin::repository::{
    hmac::HmacKey,
    installed::{InstalledEntry, InstalledFile},
};
use camino_tempfile::Utf8TempDir as TempDir;

fn test_key() -> HmacKey {
    HmacKey::from_bytes([0x44u8; 32])
}

#[test]
fn test_installed_file_round_trip() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path()
        .join("installed.json");

    let mut file = InstalledFile::default();
    file.entries.push(InstalledEntry {
        id: "openlibrary".to_string(),
        version: "1.0.0".to_string(),
        source_repo: Some("olamaelcu".to_string()),
        install_path: Utf8PathBuf::from("/providers/openlibrary/1.0.0"),
        installed_at: "2026-06-01T00:00:00Z".to_string(),
    });

    file.save(&path, &test_key()).unwrap();
    let loaded = InstalledFile::load(&path, &test_key()).unwrap();
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].id, "openlibrary");
    assert_eq!(loaded.entries[0].version, "1.0.0");
    assert_eq!(loaded.entries[0].source_repo.as_deref(), Some("olamaelcu"));
    assert_eq!(
        loaded.entries[0].install_path,
        Utf8PathBuf::from("/providers/openlibrary/1.0.0")
    );
    assert_eq!(loaded.entries[0].installed_at, "2026-06-01T00:00:00Z");
}

#[test]
fn test_installed_file_tamper_detected() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path()
        .join("installed.json");

    let mut file = InstalledFile::default();
    file.entries.push(InstalledEntry {
        id: "x".to_string(),
        version: "1.0.0".to_string(),
        source_repo: None,
        install_path: Utf8PathBuf::from("/p/x/1.0.0"),
        installed_at: "2026-06-01T00:00:00Z".to_string(),
    });
    file.save(&path, &test_key()).unwrap();

    fs_err::write(&path, b"{\"entries\":[{\"tampered\":true}]}").unwrap();
    let result = InstalledFile::load(&path, &test_key());
    assert!(result.is_err(), "expected HMAC mismatch error");
}

#[test]
fn test_installed_file_missing_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path()
        .join("installed.json");
    let loaded = InstalledFile::load(&path, &test_key()).unwrap();
    assert_eq!(loaded.entries.len(), 0);
}

// =====================================================================
// Step 3 (Task 2.5 plan): `InstalledFile::load` with a
// non-UTF-8 file body.
//
// The HMAC sidecar is verified first; a sidecar computed
// over arbitrary bytes can still verify, so the only
// remaining defense against a non-UTF-8 file is the
// `std::str::from_utf8` conversion in `load`. If that
// conversion fails, the loader must surface an error
// rather than panic or accept the data.
//
// We construct the scenario by:
//   1. Writing a non-UTF-8 byte sequence (containing 0xFF
//      which is not a valid UTF-8 lead byte) to
//      `installed.json`.
//   2. Computing the sidecar over those exact bytes and
//      writing it next to the file, so the HMAC check
//      passes.
//   3. Calling `load` and asserting an error.
// =====================================================================

#[test]
fn test_installed_file_rejects_invalid_utf8() {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    let tmp = TempDir::new().unwrap();
    let path = tmp.path()
        .join("installed.json");
    let key = test_key();

    // Build a "JSON-looking" byte buffer that is NOT valid
    // UTF-8 because it contains 0xFF (which is never legal
    // inside a UTF-8 codepoint — a single 0xFF byte is
    // unambiguously invalid). The HMAC sidecar is computed
    // over these exact bytes, so the HMAC check passes and
    // the load function reaches `std::str::from_utf8`.
    let mut data: Vec<u8> = b"{\"entries\":[{\"id\":\"\xff".to_vec();
    data.extend_from_slice(b"\",\"version\":\"1.0.0\"");
    data.extend_from_slice(b",\"install_path\":\"/p/x\"");
    data.extend_from_slice(b",\"installed_at\":\"2026\"}]}");

    fs::write(&path, &data).expect("write raw data");

    // Compute the sidecar HMAC-SHA256 the same way
    // `hmac::write_protected` does: HMAC(key, data) → 32
    // raw bytes, hex-encoded.
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key");
    mac.update(&data);
    let tag = mac.finalize().into_bytes();
    let sidecar_hex: String = tag.iter().map(|b| format!("{b:02x}")).collect();
    let sidecar_path = path.with_extension("json.hmac");
    fs::write(sidecar_path.as_std_path(), sidecar_hex.as_bytes()).expect("write sidecar");

    // The HMAC matches the bytes, so the load function
    // reaches `from_utf8` and surfaces the UTF-8 error.
    let result = InstalledFile::load(&path, &key);
    let err = result.expect_err("invalid UTF-8 in installed.json must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("installed.json") && (msg.contains("utf") || msg.contains("invalid")),
        "expected an installed.json UTF-8 error, got: {msg}"
    );
}
