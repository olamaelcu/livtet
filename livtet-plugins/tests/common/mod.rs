//! Shared test helpers for `livtet-plugins` integration tests.
//!
//! Each `tests/*.rs` file is compiled as a separate integration test
//! binary. Functions that are only used by some of those binaries
//! would otherwise trigger `dead_code` warnings in the binaries that
//! don't use them — allow the module-level lint so we don't have to
//! sprinkle `#[allow]` on every helper.
#![allow(dead_code)]

use std::sync::Arc;

use camino::Utf8PathBuf;
use fs_err as fs;
use livtet_plugins::repository::hmac::HmacKey;

/// A deterministic HMAC key for tests (all zeros).
pub fn test_hmac_key() -> Arc<HmacKey> {
    Arc::new(HmacKey::from_bytes([0u8; 32]))
}

/// Resolve a path relative to the crate's `fixtures/` directory.
pub fn fixture_path(relative: &str) -> Utf8PathBuf {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    Utf8PathBuf::from(crate_root)
        .join("fixtures")
        .join(relative)
}

/// Copy a named fixture (livtet.toml + init.lua) into `target`.
pub fn copy_fixture(target: &Utf8PathBuf, name: &str) {
    let dir = target.join(name);
    fs::create_dir_all(&dir).expect("create dir");
    fs::copy(
        fixture_path(&format!("{name}/livtet.toml")),
        dir.join("livtet.toml"),
    )
    .expect("copy livtet.toml");
    fs::copy(
        fixture_path(&format!("{name}/init.lua")),
        dir.join("init.lua"),
    )
    .expect("copy init.lua");
}

/// Copy the `test-provider` fixture (livtet.toml + init.lua) into `target`.
pub fn copy_test_provider(target: &Utf8PathBuf) {
    copy_fixture(target, "test-provider")
}

pub fn verifying_key_from_keygen_report(
    report: &livtet_plugins::types::KeygenReport,
) -> ed25519_dalek::VerifyingKey {
    let text = fs_err::read_to_string(&report.pubkey_path).expect("read pubkey");
    livtet_plugins::keys::signing::parse_pubkey_text(&text).expect("parse_pubkey_text")
}
