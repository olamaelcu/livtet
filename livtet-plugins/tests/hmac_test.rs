use fs_err as fs;
use livtet_plugins::repository::hmac::{HmacKey, read_protected, write_protected};
use camino_tempfile::Utf8TempDir as TempDir;

fn test_key() -> HmacKey {
    HmacKey::from_bytes([0x42u8; 32])
}

#[test]
fn test_write_then_read_round_trip() {
    let tmp = TempDir::new().unwrap();
    let data_path = tmp.path().join("state.toml");
    let original = b"key = \"value\"\nnumber = 42\n";
    write_protected(&data_path, original, &test_key()).unwrap();
    let read_back = read_protected(&data_path, &test_key()).unwrap();
    assert_eq!(read_back, original);
}

#[test]
fn test_tampered_file_fails_verification() {
    let tmp = TempDir::new().unwrap();
    let data_path = tmp.path().join("state.toml");
    let original = b"key = \"value\"\n";
    write_protected(&data_path, original, &test_key()).unwrap();
    fs::write(&data_path, b"key = \"tampered\"\n").unwrap();
    let result = read_protected(&data_path, &test_key());
    assert!(result.is_err(), "expected HMAC mismatch error");
}

#[test]
fn test_missing_sidecar_treated_as_missing() {
    let tmp = TempDir::new().unwrap();
    let data_path = tmp.path().join("state.toml");
    fs::write(&data_path, b"key = \"value\"\n").unwrap();
    let result = read_protected(&data_path, &test_key());
    assert!(result.is_err());
}

#[test]
fn test_hmac_key_from_bytes_is_32_bytes() {
    let key = HmacKey::from_bytes([0u8; 32]);
    assert_eq!(key.as_bytes().len(), 32);
}

#[test]
fn test_different_keys_do_not_verify() {
    let tmp = TempDir::new().unwrap();
    let data_path = tmp.path().join("state.toml");
    write_protected(&data_path, b"x", &test_key()).unwrap();
    let other_key = HmacKey::from_bytes([0x99u8; 32]);
    let result = read_protected(&data_path, &other_key);
    assert!(result.is_err());
}

// =====================================================================
// Step 7 (Task 2.5 plan): `repository/hmac.rs` sidecar
// error paths.
//
// `read_protected` reads the data file and the sidecar,
// then hex-decodes the sidecar and verifies the length
// is exactly 32 bytes (HMAC-SHA256). The two error paths
// the audit flagged are:
//   - sidecar content is not valid hex (e.g. contains
//     non-base16 characters)
//   - sidecar content is hex but the wrong length (e.g.
//     31 bytes / 62 hex chars, instead of the expected
//     32 bytes / 64 hex chars)
//
// We exercise both directly. The sidecar must EXIST and
// contain the wrong payload — a missing sidecar is the
// "missing sidecar" branch which is already covered by
// `test_missing_sidecar_treated_as_missing` above.
// =====================================================================

#[test]
fn test_read_protected_rejects_non_hex_sidecar() {
    // A sidecar that exists but contains characters that
    // are not in the base-16 alphabet must be rejected
    // by `hex::decode` with a "sidecar hex" error.
    let tmp = TempDir::new().unwrap();
    let data_path = tmp.path().join("state.toml");
    fs::write(&data_path, b"some data").unwrap();
    let mut sidecar = data_path.as_std_path().to_path_buf();
    sidecar.set_extension("toml.hmac");
    // 64 ASCII chars but `g`-`z` are not hex.
    fs::write(
        &sidecar,
        b"gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
    )
    .unwrap();
    let result = read_protected(&data_path, &test_key());
    let err = result.expect_err("non-hex sidecar must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("sidecar") && (msg.contains("hex") || msg.contains("decode")),
        "expected a sidecar/hex error, got: {msg}"
    );
}

#[test]
fn test_read_protected_rejects_wrong_length_sidecar() {
    // A sidecar that contains valid hex but with the
    // wrong number of bytes (e.g. 31 bytes / 62 hex chars
    // instead of the expected 32 bytes / 64 hex chars)
    // must be rejected with a "sidecar not 32 bytes"
    // error. We construct exactly 31 bytes worth of hex
    // (62 ASCII chars) to trigger this branch.
    let tmp = TempDir::new().unwrap();
    let data_path = tmp.path().join("state.toml");
    fs::write(&data_path, b"some data").unwrap();
    let mut sidecar = data_path.as_std_path().to_path_buf();
    sidecar.set_extension("toml.hmac");
    // 62 hex chars = 31 bytes (HMAC-SHA256 expects 64 hex
    // chars = 32 bytes). The `hex::decode` succeeds, the
    // length check then trips.
    fs::write(&sidecar, "a".repeat(62).as_bytes()).unwrap();
    let result = read_protected(&data_path, &test_key());
    let err = result.expect_err("wrong-length sidecar must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("32") && msg.contains("sidecar"),
        "expected a 'sidecar not 32 bytes' error, got: {msg}"
    );
}
