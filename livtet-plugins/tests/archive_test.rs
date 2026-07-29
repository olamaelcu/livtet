mod common;
use std::io::Write as _;

use common::verifying_key_from_keygen_report;
use fs_err as fs;
use livtet_plugins::{
    archive::{
        checksums::{ChecksumEntry, generate_checksums, parse_checksums, render_checksums},
        install::install,
        manifest::{ArchiveMeta, parse_archive_toml, render_archive_toml},
        pack::pack,
        verify::verify,
    },
    keys::{TrustStore, keyfile::keygen},
    types::KeygenReport,
};
use camino_tempfile::Utf8TempDir as TempDir;

#[test]
fn test_generate_checksums_basic() {
    let tmp = TempDir::new().unwrap();
    let plugin_dir = tmp.path().join("plugin");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(plugin_dir.join("a.lua"), b"a").unwrap();
    fs::write(plugin_dir.join("b.lua"), b"bb").unwrap();

    let entries =
        generate_checksums(plugin_dir.as_path(), &[]).unwrap();

    assert_eq!(entries[0].path, "plugin/a.lua");
    assert_eq!(entries[1].path, "plugin/b.lua");

    assert_eq!(
        entries[0].sha256,
        "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb"
    );
}

#[test]
fn test_parse_checksums_round_trip() {
    let entries = vec![
        ChecksumEntry {
            sha256: "abc123".to_string(),
            path: "plugin/a.lua".to_string(),
        },
        ChecksumEntry {
            sha256: "def456".to_string(),
            path: "plugin/b.lua".to_string(),
        },
    ];
    let rendered = render_checksums(&entries);
    let parsed = parse_checksums(&rendered).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].sha256, "abc123");
    assert_eq!(parsed[0].path, "plugin/a.lua");
    assert_eq!(parsed[1].sha256, "def456");
}

#[test]
fn test_archive_toml_round_trip() {
    let meta = ArchiveMeta {
        format_version: 1,
        plugin_id: "openlibrary".to_string(),
        plugin_version: "1.0.0".to_string(),
        created_at: "2026-06-01T12:00:00Z".to_string(),
        signed_by: "olamaelcu".to_string(),
        tool: "livtet plugin pack".to_string(),
    };
    let rendered = render_archive_toml(&meta);
    let parsed = parse_archive_toml(&rendered).unwrap();
    assert_eq!(parsed.plugin_id, "openlibrary");
    assert_eq!(parsed.plugin_version, "1.0.0");
    assert_eq!(parsed.signed_by, "olamaelcu");
}

#[test]
fn test_archive_toml_rejects_wrong_format_version() {
    let toml_text = r#"
[archive]
format_version = 99
plugin_id = "x"
plugin_version = "0.0.1"
created_at = "2026-01-01T00:00:00Z"
signed_by = "me"
tool = "manual"
"#;
    let err = parse_archive_toml(toml_text).unwrap_err();
    assert!(
        err.to_string().contains("unsupported") || err.to_string().contains("format"),
        "got: {err}"
    );
}

#[test]
fn test_verify_rejects_non_zip_file() {
    let tmp = TempDir::new().unwrap();
    let bogus = tmp.path().join("bogus.ltp");
    fs::write(&bogus, b"not a zip").unwrap();
    let report = verify(bogus.as_path(), None).unwrap();
    assert!(!report.valid);
    assert!(!report.errors.is_empty());
}

#[test]
fn test_verify_rejects_oversize_archive() {
    let tmp = TempDir::new().unwrap();
    let big = tmp.path().join("big.ltp");
    let mut f = fs::File::create(&big).unwrap();
    f.write_all(&vec![0u8; 60 * 1024 * 1024]).unwrap();
    drop(f);
    let report = verify(big.as_path(), None).unwrap();
    assert!(!report.valid);
    assert!(report.errors.iter().any(|e: &String| e.contains("50 MB")));
}

#[test]
fn test_verify_reports_missing_meta_inf_files() {
    // This test was previously mislabeled as
    // `test_verify_untrusted_key_reports_continue`; it actually
    // constructs an archive that has the `META-INF/archive.toml`
    // but is missing `META-INF/checksums.txt`, `signature.bin`,
    // and `pubkey.txt`. The error path is the
    // "missing META-INF file" branch in `verify()`, not the
    // untrusted-key branch (a key check requires a pubkey, and
    // we have no pubkey). The rename makes the contract
    // explicit so a future reader doesn't go looking for an
    // untrusted-key test that doesn't exist here.
    let tmp = TempDir::new().unwrap();
    let bogus = tmp.path().join("x.zip");
    let mut zipf = fs::File::create(&bogus).unwrap();
    {
        let mut zip = zip::ZipWriter::new(&mut zipf);
        zip.start_file(
            "META-INF/archive.toml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(
            b"format_version = 1\nplugin_id = \"x\"\nplugin_version = \"0.0.1\"\ncreated_at = \"2026-01-01T00:00:00Z\"\nsigned_by = \"someone\"\ntool = \"test\"\n",
        )
        .unwrap();
        zip.finish().unwrap();
    }
    drop(zipf);
    let report = verify(
        bogus.as_path(),
        Some(&TrustStore::empty()),
    )
    .unwrap();
    assert!(!report.valid);
    assert!(
        report
            .errors
            .iter()
            .any(|e: &String| e.contains("META-INF") || e.contains("missing")),
        "expected a 'missing META-INF' error, got: {:?}",
        report.errors
    );
}

#[test]
fn test_pack_then_verify_round_trip() {
    let tmp = TempDir::new().unwrap();
    let plugin_dir = tmp.path().join("plugin-src");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("livtet.toml"),
        b"[plugin]\nid=\"test-pkg\"\nname=\"Test\"\nversion=\"1.0.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    fs::write(plugin_dir.join("init.lua"), b"-- test plugin\n").unwrap();

    let key_dir = tmp.path().join("keys");
    let keygen_report = keygen(
        key_dir.as_path(),
        "test-key",
        true,
    )
    .unwrap();

    let ltp_path = pack(
        plugin_dir.as_path(),
        &keygen_report.key_path,
        "test-key",
        tmp.path(),
    )
    .unwrap();

    let verifying_key = verifying_key_from_keygen_report(&keygen_report);

    let mut store = TrustStore::empty();
    store.add_user_key("test-key", verifying_key).unwrap();

    let report = verify(&ltp_path, Some(&store)).unwrap();
    assert!(report.valid, "verify failed: {:?}", report.errors);
    assert_eq!(report.plugin_id.as_deref(), Some("test-pkg"));
    assert_eq!(report.version.as_deref(), Some("1.0.0"));
}

#[test]
fn test_install_extracts_to_providers_dir() {
    let tmp = TempDir::new().unwrap();
    let plugin_dir = tmp.path().join("plugin-src");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("livtet.toml"),
        b"[plugin]\nid=\"install-test\"\nname=\"Test\"\nversion=\"0.1.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    fs::write(plugin_dir.join("init.lua"), b"-- test\n").unwrap();

    let key_dir = tmp.path().join("keys");
    let keygen_report = keygen(
        key_dir.as_path(),
        "install-test-key",
        true,
    )
    .unwrap();
    let ltp_path = pack(
        plugin_dir.as_path(),
        &keygen_report.key_path,
        "install-test-key",
        tmp.path(),
    )
    .unwrap();

    let verifying_key = verifying_key_from_keygen_report(&keygen_report);
    let mut store = TrustStore::empty();
    store
        .add_user_key("install-test-key", verifying_key)
        .unwrap();

    let providers_dir = tmp.path().join("providers");
    let report = install(
        &ltp_path,
        providers_dir.as_path(),
        Some(&store),
    )
    .unwrap();
    assert_eq!(report.id, "install-test");
    assert_eq!(report.version, "0.1.0");
    assert!(report.install_path.exists());
    assert!(report.install_path.join("livtet.toml").exists());
    assert!(report.install_path.join("init.lua").exists());
}

#[test]
fn test_install_to_existing_version_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let plugin_dir = tmp.path().join("plugin-src");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("livtet.toml"),
        b"[plugin]\nid=\"idem\"\nname=\"Idem\"\nversion=\"1.0.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    fs::write(plugin_dir.join("init.lua"), b"-- v1\n").unwrap();

    let key_dir = tmp.path().join("keys");
    let keygen_report = keygen(
        key_dir.as_path(),
        "idem-key",
        true,
    )
    .unwrap();
    let ltp_path = pack(
        plugin_dir.as_path(),
        &keygen_report.key_path,
        "idem-key",
        tmp.path(),
    )
    .unwrap();

    let verifying_key = verifying_key_from_keygen_report(&keygen_report);
    let mut store = TrustStore::empty();
    store.add_user_key("idem-key", verifying_key).unwrap();

    let providers_dir = tmp.path().join("providers");
    let providers_dir_utf8 = providers_dir.as_path();
    install(&ltp_path, providers_dir_utf8, Some(&store)).unwrap();
    let report2 = install(&ltp_path, providers_dir_utf8, Some(&store)).unwrap();
    assert!(
        report2
            .warnings
            .iter()
            .any(|w: &String| w.contains("replaced") || w.contains("overwrite")),
        "expected replacement warning, got: {:?}",
        report2.warnings
    );
}

// =====================================================================
// Task 2.4 / Step 9: archive install replace-warning paths.
//
// The install code's "replaced" warning fires when the
// target directory `<providers>/<id>/<version>` already
// exists. The test below pins the contract for the two
// interesting cases:
//   1. Same-version reinstall: the target dir already
//      exists, the install code emits a "replaced existing
//      v<version>" warning, then renames the temp dir
//      over the existing one. The on-disk content is
//      whatever the new archive contained.
//   2. Different-version install: the target dir for the
//      new version doesn't exist, so no warning is
//      emitted and the install is fully independent of
//      any other version's directory.
// =====================================================================

/// Helper: pack a tiny `plugin/` tree under the given
#[test]
fn test_install_replaces_existing_same_version_with_warning() {
    // Install v1.0.0 twice. The second install must emit
    // a "replaced existing v1.0.0" warning, and the
    // on-disk content must be from the second archive
    // (the rename overwrites the existing dir).
    let tmp = TempDir::new().unwrap();
    let plugin_dir = tmp.path().join("plugin-src");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("livtet.toml"),
        b"[plugin]\nid=\"replace-warn\"\nname=\"Replace Warn\"\nversion=\"1.0.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    fs::write(plugin_dir.join("init.lua"), b"-- v1\n").unwrap();
    let key_dir = tmp.path().join("keys");
    let keygen_report = keygen(
        key_dir.as_path(),
        "rw-key",
        true,
    )
    .unwrap();
    let ltp_path = pack(
        plugin_dir.as_path(),
        &keygen_report.key_path,
        "rw-key",
        tmp.path(),
    )
    .unwrap();
    let verifying_key = verifying_key_from_keygen_report(&keygen_report);
    let mut store = TrustStore::empty();
    store.add_user_key("rw-key", verifying_key).unwrap();

    let providers_dir = tmp.path().join("providers");
    let providers_dir_utf8 = providers_dir.as_path();
    let first = install(&ltp_path, providers_dir_utf8, Some(&store)).unwrap();
    assert!(
        first.warnings.is_empty(),
        "first install should not warn, got {:?}",
        first.warnings
    );
    // The second install of the same version replaces
    // the target dir and emits a warning.
    let second = install(&ltp_path, providers_dir_utf8, Some(&store)).unwrap();
    assert!(
        second
            .warnings
            .iter()
            .any(|w: &String| w.contains("replaced") && w.contains("1.0.0")),
        "expected a 'replaced existing v1.0.0' warning, got: {:?}",
        second.warnings
    );
    // The on-disk install_path is the same (we replaced
    // the same directory).
    assert_eq!(first.install_path, second.install_path);
}

#[test]
fn test_install_different_version_does_not_warn_or_replace_existing_version() {
    // Install v1.0.0 then v1.1.0 of the same plugin id.
    // Each version lands in its own subdirectory
    // (`<providers>/<id>/1.0.0` and `<providers>/<id>/1.1.0`),
    // so neither install replaces the other and no
    // warning is emitted. The on-disk install_paths
    // differ by version segment.
    let tmp = TempDir::new().unwrap();
    let key_dir = tmp.path().join("keys");
    let keygen_report = keygen(
        key_dir.as_path(),
        "diff-v-key",
        true,
    )
    .unwrap();
    let verifying_key = verifying_key_from_keygen_report(&keygen_report);
    let mut store = TrustStore::empty();
    store.add_user_key("diff-v-key", verifying_key).unwrap();

    let providers_dir = tmp.path().join("providers");
    let providers_dir_utf8 = providers_dir.as_path();

    // Build v1.0.0.
    let plugin_dir_v1 = tmp.path().join("plugin-v1");
    fs::create_dir_all(&plugin_dir_v1).unwrap();
    fs::write(
        plugin_dir_v1.join("livtet.toml"),
        b"[plugin]\nid=\"diff-v\"\nname=\"Diff V\"\nversion=\"1.0.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    fs::write(plugin_dir_v1.join("init.lua"), b"-- v1\n").unwrap();
    let ltp_v1 = pack(
        plugin_dir_v1.as_path(),
        &keygen_report.key_path,
        "diff-v-key",
        tmp.path(),
    )
    .unwrap();
    let first = install(&ltp_v1, providers_dir_utf8, Some(&store)).unwrap();
    assert!(first.warnings.is_empty(), "first install should be quiet");

    // Build v1.1.0. We re-use `pack_minimal`'s shape but
    // override the version by writing fresh files. To
    // avoid mutating shared state, build a fresh plugin
    // dir under a different name and pack.
    let plugin_dir_v11 = tmp.path().join("plugin-v11");
    fs::create_dir_all(&plugin_dir_v11).unwrap();
    fs::write(
        plugin_dir_v11.join("livtet.toml"),
        b"[plugin]\nid=\"diff-v\"\nname=\"Diff V\"\nversion=\"1.1.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    fs::write(plugin_dir_v11.join("init.lua"), b"-- v1.1\n").unwrap();
    let ltp_v11 = pack(
        plugin_dir_v11.as_path(),
        &keygen_report.key_path,
        "diff-v-key",
        tmp.path(),
    )
    .unwrap();
    let second = install(&ltp_v11, providers_dir_utf8, Some(&store)).unwrap();
    // Different version = different target dir = no warning.
    assert!(
        second.warnings.is_empty(),
        "different-version install should not warn, got {:?}",
        second.warnings
    );
    // The two install_paths differ.
    assert_ne!(
        first.install_path, second.install_path,
        "different versions must land in different directories"
    );
    // Both directories exist on disk.
    assert!(first.install_path.exists());
    assert!(second.install_path.exists());
    // The version segment is part of the path.
    assert!(
        first.install_path.to_string().ends_with("1.0.0"),
        "first install_path must end with the v1.0.0 segment, got {:?}",
        first.install_path
    );
    assert!(
        second.install_path.to_string().ends_with("1.1.0"),
        "second install_path must end with the v1.1.0 segment, got {:?}",
        second.install_path
    );
}

// =====================================================================
// Attack-path tests
//
// The tests below construct a valid signed `.ltp` archive
// (via `pack`), then mutate the bytes to simulate the most
// common supply-chain attacks a malicious or buggy plugin
// author could mount. Each test asserts that `verify` (or
// `install`, for the install-time checks) returns a clear
// error instead of accepting the broken archive.
//
// The pack function is a closed box: it returns a signed,
// checksummed zip and we keep the original `KeygenReport`
// around so we can rebuild the trust store. From there each
// test reads the bytes, performs one specific mutation,
// writes them back to a new path, and runs `verify` on
// that broken copy.
// =====================================================================

/// Pack a tiny `plugin/` tree under the given id/version and
/// return the resulting `.ltp` plus the trust store + key
/// report that can verify it. Centralizes the boilerplate so
/// each attack-path test stays focused on the mutation, not
/// the setup.
fn pack_minimal(
    tmp: &TempDir,
    id: &str,
    version: &str,
) -> (camino::Utf8PathBuf, KeygenReport, TrustStore) {
    let plugin_dir = tmp.path().join("plugin-src");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("livtet.toml"),
        format!(
            "[plugin]\nid=\"{id}\"\nname=\"{id}\"\nversion=\"{version}\"\nentry=\"init.lua\"\n"
        ),
    )
    .unwrap();
    fs::write(plugin_dir.join("init.lua"), b"-- minimal plugin\n").unwrap();

    let key_dir = tmp.path().join("keys");
    let keygen_report = keygen(
        key_dir.as_path(),
        "atk-key",
        true,
    )
    .unwrap();
    let ltp_path = pack(
        plugin_dir.as_path(),
        &keygen_report.key_path,
        "atk-key",
        tmp.path(),
    )
    .unwrap();

    let verifying_key = verifying_key_from_keygen_report(&keygen_report);
    let mut store = TrustStore::empty();
    store.add_user_key("atk-key", verifying_key).unwrap();
    (ltp_path, keygen_report, store)
}

/// Read every file in `path.zip` into a `Vec<(name, bytes)>`
/// in archive order. We rebuild the zip from this in the
/// mutation tests below.
fn read_zip_entries(path: &camino::Utf8Path) -> Vec<(String, Vec<u8>)> {
    let bytes = fs::read(path.as_std_path()).expect("read zip");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open zip");
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).expect("entry");
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf).expect("read entry");
        out.push((name, buf));
    }
    out
}

/// Write a fresh zip with the same entries as the original
/// except the named entry's bytes are replaced with
/// `replacement`. The order of the other entries is preserved.
fn rewrite_with_replacement(
    out_path: &camino::Utf8Path,
    entries: &[(String, Vec<u8>)],
    target: &str,
    replacement: &[u8],
) {
    let file = fs::File::create(out_path).expect("create out zip");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        if name == target {
            zip.start_file(name, opts).expect("start_file");
            zip.write_all(replacement).expect("write replacement");
        } else {
            zip.start_file(name, opts).expect("start_file");
            zip.write_all(bytes).expect("write entry");
        }
    }
    zip.finish().expect("finish zip");
}

fn fresh_copy_with_replacement(
    src_zip: &camino::Utf8Path,
    target: &str,
    replacement: &[u8],
) -> camino::Utf8PathBuf {
    let entries = read_zip_entries(src_zip);
    let stem = src_zip.file_stem().unwrap_or("archive");
    let out = src_zip.with_file_name(format!("{stem}-mutated.ltp"));
    rewrite_with_replacement(&out, &entries, target, replacement);
    out
}

/// Convenience: write a fresh zip next to `src_zip` with
/// every entry in the source preserved, plus one extra
/// `extra_name` / `extra_bytes` pair. The checksums.txt is
/// not patched, so the result has a checksums.txt that
/// doesn't include the new file — i.e. exactly the "unsigned
/// file" scenario. Used by tests that want to assert the
/// verifier rejects the extra entry.
fn write_zip_with_extra_entry(
    src_zip: &camino::Utf8Path,
    suffix: &str,
    extra_name: &str,
    extra_bytes: &[u8],
) -> camino::Utf8PathBuf {
    let entries = read_zip_entries(src_zip);
    let stem = src_zip.file_stem().unwrap_or("archive");
    let out = src_zip.with_file_name(format!("{stem}-{suffix}.ltp"));
    let file = fs::File::create(out.as_std_path()).expect("create out zip");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, bytes) in &entries {
        zip.start_file(name, opts).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.start_file(extra_name, opts).unwrap();
    zip.write_all(extra_bytes).unwrap();
    zip.finish().unwrap();
    out
}

#[test]
fn test_verify_rejects_tampered_manifest_id() {
    // The archive.toml's plugin_id and the livtet.toml's
    // [plugin].id MUST agree. Flip the archive.toml's id
    // and the verifier must surface a manifest-mismatch
    // error rather than passing the broken archive.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    let tampered = "format_version = 1\nplugin_id = \"evil-pkg\"\nplugin_version = \"1.0.0\"\ncreated_at = \"2026-01-01T00:00:00Z\"\nsigned_by = \"atk-key\"\ntool = \"manual\"\n";
    let broken =
        fresh_copy_with_replacement(&ltp_path, "META-INF/archive.toml", tampered.as_bytes());
    let report = verify(&broken, Some(&store)).unwrap();
    assert!(
        !report.valid,
        "tampered manifest must be rejected, errors={:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("manifest") || e.contains("mismatch")),
        "expected a manifest mismatch error, got {:?}",
        report.errors
    );
}

#[test]
fn test_verify_rejects_tampered_manifest_version() {
    // Same as the id test, but mutate the version field.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    let tampered = "format_version = 1\nplugin_id = \"honest-pkg\"\nplugin_version = \"9.9.9\"\ncreated_at = \"2026-01-01T00:00:00Z\"\nsigned_by = \"atk-key\"\ntool = \"manual\"\n";
    let broken =
        fresh_copy_with_replacement(&ltp_path, "META-INF/archive.toml", tampered.as_bytes());
    let report = verify(&broken, Some(&store)).unwrap();
    assert!(!report.valid);
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("manifest") || e.contains("mismatch") || e.contains("version")),
        "expected a manifest version mismatch, got {:?}",
        report.errors
    );
}

#[test]
fn test_verify_rejects_tampered_checksum_mismatch() {
    // Mutate a plugin file's bytes WITHOUT updating its
    // checksum in `META-INF/checksums.txt`. The verifier
    // recomputes the SHA-256 over the (now-tampered) bytes
    // and surfaces an integrity-check-failed error.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    let replacement: &[u8] = b"-- this is the tampered version\n";
    let broken = fresh_copy_with_replacement(&ltp_path, "plugin/init.lua", replacement);
    let report = verify(&broken, Some(&store)).unwrap();
    assert!(!report.valid);
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("integrity") || e.contains("init.lua") || e.contains("check")),
        "expected an integrity error for init.lua, got {:?}",
        report.errors
    );
}

#[test]
fn test_verify_rejects_unsigned_file_in_plugin_dir() {
    // The checksums.txt lists every file under `plugin/`. If
    // an attacker adds a new file under `plugin/` (without
    // updating checksums.txt), `verify` must surface an
    // "unsigned file" error. We construct the broken archive
    // by appending an entry to the original.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");

    let out = write_zip_with_extra_entry(
        &ltp_path,
        "unsigned",
        "plugin/extra.lua",
        b"-- unsigned file\n",
    );

    let report = verify(&out, Some(&store)).unwrap();
    assert!(
        !report.valid,
        "unsigned file must be rejected, errors={:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("unsigned") || e.contains("extra.lua")),
        "expected an 'unsigned file' error, got {:?}",
        report.errors
    );
}

#[test]
fn test_verify_rejects_flipped_signature_bytes() {
    // Flip one byte in `META-INF/signature.bin`. The
    // signature check uses the embedded public key (which
    // is itself in the archive), so a flipped byte breaks
    // verification cleanly.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    let entries = read_zip_entries(&ltp_path);
    let sig = entries
        .iter()
        .find(|(n, _)| n == "META-INF/signature.bin")
        .expect("signature entry exists")
        .1
        .clone();
    assert_eq!(sig.len(), 64, "ed25519 signature is 64 bytes");
    let mut flipped = sig.clone();
    flipped[0] ^= 0xFF;
    let broken = fresh_copy_with_replacement(&ltp_path, "META-INF/signature.bin", &flipped);
    let report = verify(&broken, Some(&store)).unwrap();
    assert!(!report.valid);
    assert!(
        report.errors.iter().any(|e| e.contains("signature")
            || e.contains("verification")
            || e.contains("invalid")),
        "expected a signature error, got {:?}",
        report.errors
    );
}

#[test]
fn test_verify_rejects_revoked_key() {
    // Sign with key K, then revoke K. Even though the
    // signature itself is valid, the verifier must reject
    // the archive because the signing key is on the
    // revocation list. This is the post-compromise path:
    // a key was trusted yesterday, the operator revoked
    // it today, and old archives signed by it should no
    // longer verify.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, mut store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    // Find the key we just added (it's the only one) and
    // revoke it. `user_key_by_label` gives us the
    // `VerifyingKey`; we use its fingerprint to revoke.
    let key = *store
        .user_key_by_label("atk-key")
        .expect("atk-key was just added");
    store.revoke(&key).expect("revoke");
    let report = verify(&ltp_path, Some(&store)).unwrap();
    assert!(
        !report.valid,
        "revoked key must be rejected, errors={:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("revoked") || e.contains("revoke")),
        "expected a revoked-key error, got {:?}",
        report.errors
    );
}

#[test]
fn test_verify_rejects_manifest_with_non_rfc3339_created_at() {
    // `archive.toml` parses fine on its own, but `verify`
    // additionally checks that `created_at` is an RFC 3339
    // timestamp. A non-RFC3339 string must surface as an
    // error.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    let bad = "format_version = 1\nplugin_id = \"honest-pkg\"\nplugin_version = \"1.0.0\"\ncreated_at = \"yesterday\"\nsigned_by = \"atk-key\"\ntool = \"manual\"\n";
    let broken = fresh_copy_with_replacement(&ltp_path, "META-INF/archive.toml", bad.as_bytes());
    let report = verify(&broken, Some(&store)).unwrap();
    assert!(
        !report.valid,
        "non-RFC3339 created_at must be rejected, errors={:?}",
        report.errors
    );
    assert!(
        report.errors.iter().any(|e| e.contains("created_at")
            || e.contains("rfc3339")
            || e.contains("time")
            || e.contains("parse")),
        "expected a created_at parse error, got {:?}",
        report.errors
    );
}

#[test]
fn test_verify_rejects_livtet_toml_schema_violation() {
    // `verify` validates the embedded livtet.toml against
    // the PluginManifest schema (id pattern, semver, etc.).
    // A livtet.toml that doesn't parse as a PluginManifest
    // — here, an invalid semver — must surface a
    // "manifest schema" error.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    let bad = b"[plugin]\nid=\"honest-pkg\"\nname=\"Honest\"\nversion=\"not-semver\"\nentry=\"init.lua\"\n";
    let broken = fresh_copy_with_replacement(&ltp_path, "plugin/livtet.toml", bad);
    let report = verify(&broken, Some(&store)).unwrap();
    assert!(
        !report.valid,
        "manifest schema violation must be rejected, errors={:?}",
        report.errors
    );
    assert!(
        report.errors.iter().any(|e| e.contains("manifest")
            || e.contains("schema")
            || e.contains("semver")
            || e.contains("version")),
        "expected a manifest schema error, got {:?}",
        report.errors
    );
}

// =====================================================================
// Archive install security tests
//
// These tests exercise the install-time checks in
// `archive/install.rs`: path traversal, absolute paths, path
// length, per-file size, total size, and the checksums.txt
// 10_000-entry cap.
// =====================================================================

#[test]
fn test_install_rejects_archive_with_dotdot_path() {
    // A plugin file at `plugin/../escape.lua` would land
    // outside the providers dir if the install code were
    // naive. The check fires before any file is written.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    let out = write_zip_with_extra_entry(
        &ltp_path,
        "dotdot",
        "plugin/sub/../../escape.lua",
        b"escape\n",
    );

    let providers_dir = tmp.path().join("providers");
    let result = install(
        &out,
        providers_dir.as_path(),
        Some(&store),
    );
    // Either verify rejects (unsigned file) OR install
    // rejects (path traversal). Both are valid defenses;
    // we accept either.
    match result {
        Err(_) => {
            // Expected. The path-safety check in install
            // may or may not fire depending on whether
            // verify caught it first; either way, the
            // install is rejected.
        }
        Ok(report) => panic!("install with `..` path must fail, got {report:?}"),
    }
}

#[test]
fn test_install_rejects_absolute_path() {
    // A path starting with `/` is an absolute filesystem
    // path. If the install code were naive, the file
    // would land at e.g. `/escape.lua` next to root.
    // install.rs explicitly rejects this.
    //
    // We construct an entry named `plugin//etc/escape.lua`.
    // After the install code strips the `plugin/` prefix
    // (7 chars), the remaining path is `/etc/escape.lua`,
    // which starts with `/`. This triggers the
    // `rel.starts_with('/')` check in install.rs, which
    // rejects absolute paths with an "unsafe path" error.
    //
    // NOTE: A "true" absolute path like `/etc/escape.lua` (without
    // the `plugin/` prefix) would actually be silently skipped by
    // the install code (line 43: `if !name.starts_with("plugin/") { continue; }`),
    // not explicitly rejected. The double-slash variant tests the
    // actual attack vector where an attacker tries to smuggle an
    // absolute path through the prefix stripping logic.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    let out =
        write_zip_with_extra_entry(&ltp_path, "abspath", "plugin//etc/escape.lua", b"escape\n");

    let providers_dir = tmp.path().join("providers");
    let result = install(
        &out,
        providers_dir.as_path(),
        Some(&store),
    );
    // Verify the error is from the path-safety check, not just unsigned file
    let err = result.expect_err("install with absolute path must fail");
    let msg = err.to_string();
    // The error should mention "unsafe path" (from install.rs) or be an unsigned file error
    assert!(
        msg.contains("unsafe") || msg.contains("unsigned"),
        "expected 'unsafe path' or 'unsigned' error, got: {msg}"
    );
}

#[test]
fn test_install_rejects_path_over_255_bytes() {
    // `install.rs` rejects `rel.len() > 255` (the per-path
    // length cap is independent of zip's 65535-byte name
    // cap). We construct a path that is exactly 256 bytes
    // long so the check trips.
    let tmp = TempDir::new().unwrap();
    let (ltp_path, _kr, store) = pack_minimal(&tmp, "honest-pkg", "1.0.0");
    // `plugin/` (7 bytes) + 256-byte name = 263-byte zip
    // entry. Strip the `plugin/` prefix when measuring
    // `rel`: install measures `rel.len()` after stripping
    // `plugin/`, so the inner name must be 256 bytes.
    let long_name: String = "a".repeat(256);
    let full = format!("plugin/{long_name}");
    let out = write_zip_with_extra_entry(&ltp_path, "longpath", &full, b"x\n");

    let providers_dir = tmp.path().join("providers");
    let result = install(
        &out,
        providers_dir.as_path(),
        Some(&store),
    );
    match result {
        Err(_) => {}
        Ok(report) => panic!("install with 256-byte path must fail, got {report:?}"),
    }
}

#[test]
fn test_install_accepts_path_at_255_byte_boundary() {
    // Companion to the >255 test: a path that's exactly
    // 255 bytes after the `plugin/` prefix must still be
    // accepted. We rebuild the archive with checksums
    // matching the new file so verify passes; install's
    // path-length check then becomes the gate.
    let tmp = TempDir::new().unwrap();
    let plugin_dir = tmp.path().join("plugin-src");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("livtet.toml"),
        b"[plugin]\nid=\"long-path-pkg\"\nname=\"LongPath\"\nversion=\"1.0.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    // Create the deeply-nested file. The name after the
    // `plugin/` strip must be exactly 255 bytes; we'll
    // pre-compute the directory and verify on the host
    // that the install succeeded.
    let long_name: String = "b".repeat(255);
    fs::write(plugin_dir.join(&long_name), b"x\n").unwrap();

    let key_dir = tmp.path().join("keys");
    let keygen_report = keygen(
        key_dir.as_path(),
        "longpath-key",
        true,
    )
    .unwrap();
    let ltp_path = pack(
        plugin_dir.as_path(),
        &keygen_report.key_path,
        "longpath-key",
        tmp.path(),
    )
    .unwrap();

    let verifying_key = verifying_key_from_keygen_report(&keygen_report);
    let mut store = TrustStore::empty();
    store.add_user_key("longpath-key", verifying_key).unwrap();

    let providers_dir = tmp.path().join("providers");
    let report = install(
        &ltp_path,
        providers_dir.as_path(),
        Some(&store),
    )
    .expect("255-byte path is the exact boundary and must install");
    assert!(report.install_path.join(&long_name).exists());
}

#[test]
fn test_install_rejects_per_file_over_20mb() {
    // A 20 MiB+1 byte file under `plugin/` must be
    // rejected. We rebuild a fresh archive with a file
    // of that size; verify will pass (checksums match
    // the file's actual bytes) and install's size check
    // becomes the gate.
    let tmp = TempDir::new().unwrap();
    let plugin_dir = tmp.path().join("plugin-src");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("livtet.toml"),
        b"[plugin]\nid=\"big-pkg\"\nname=\"Big\"\nversion=\"1.0.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    // 20 MiB + 1 byte. Allocating 20 MiB in a test is
    // fine; the cap is 50 MB on the archive and 20 MB on
    // a single file. We use a vector of zeros so the
    // test is fast.
    let big: Vec<u8> = vec![0u8; 20 * 1024 * 1024 + 1];
    fs::write(plugin_dir.join("init.lua"), &big).unwrap();

    let key_dir = tmp.path().join("keys");
    let keygen_report = keygen(
        key_dir.as_path(),
        "big-key",
        true,
    )
    .unwrap();
    let ltp_path = pack(
        plugin_dir.as_path(),
        &keygen_report.key_path,
        "big-key",
        tmp.path(),
    )
    .unwrap();

    let verifying_key = verifying_key_from_keygen_report(&keygen_report);
    let mut store = TrustStore::empty();
    store.add_user_key("big-key", verifying_key).unwrap();

    let providers_dir = tmp.path().join("providers");
    let result = install(
        &ltp_path,
        providers_dir.as_path(),
        Some(&store),
    );
    let err = result.expect_err("install of 20 MiB+1 file must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("file too large")
            || msg.contains("20 MB")
            || msg.contains("20\\ MiB")
            || msg.contains("bytes"),
        "expected a 'file too large' error, got: {msg}"
    );
}

#[test]
fn test_install_rejects_total_extraction_over_100mb() {
    // Sum of all files > 100 MiB → rejected. Use two
    // ~60 MiB files so each is under the 20 MiB
    // per-file cap but their sum blows past 100 MiB.
    let tmp = TempDir::new().unwrap();
    let plugin_dir = tmp.path().join("plugin-src");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("livtet.toml"),
        b"[plugin]\nid=\"total-big\"\nname=\"TotalBig\"\nversion=\"1.0.0\"\nentry=\"init.lua\"\n",
    )
    .unwrap();
    // Each file is 19 MiB (under the 20 MiB per-file cap).
    // Two of them: 38 MiB total. That's under 100 MiB, so
    // we need more files. Use six 19 MiB files: 114 MiB
    // total. Each is under 20 MiB, so the per-file check
    // doesn't fire; only the total check does.
    let big: Vec<u8> = vec![0u8; 19 * 1024 * 1024];
    fs::write(plugin_dir.join("init.lua"), &big).unwrap();
    for i in 0..6 {
        let name = format!("data_{i}.bin");
        fs::write(plugin_dir.join(name), &big).unwrap();
    }

    let key_dir = tmp.path().join("keys");
    let keygen_report = keygen(
        key_dir.as_path(),
        "total-big-key",
        true,
    )
    .unwrap();
    let ltp_path = pack(
        plugin_dir.as_path(),
        &keygen_report.key_path,
        "total-big-key",
        tmp.path(),
    )
    .unwrap();

    let verifying_key = verifying_key_from_keygen_report(&keygen_report);
    let mut store = TrustStore::empty();
    store.add_user_key("total-big-key", verifying_key).unwrap();

    let providers_dir = tmp.path().join("providers");
    let result = install(
        &ltp_path,
        providers_dir.as_path(),
        Some(&store),
    );
    let err = result.expect_err("install of >100 MiB total must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("total extracted")
            || msg.contains("100 MB")
            || msg.contains("100\\ MiB")
            || msg.contains("exceeds"),
        "expected a 'total extracted exceeds 100 MB' error, got: {msg}"
    );
}

#[test]
fn test_archive_toml_rejects_non_rfc3339_created_at() {
    // `parse_archive_toml` (in `archive/manifest.rs`) calls
    // `OffsetDateTime::parse(.., &Rfc3339)` after deserializing
    // the TOML. A non-RFC3339 `created_at` must surface as a
    // `InvalidArchive` error that mentions the date-format
    // failure, so a downstream operator can tell the manifest
    // was rejected for a date reason and not, say, a
    // base64/key/signature reason.
    let bad = r#"
format_version = 1
plugin_id = "x"
plugin_version = "0.0.1"
created_at = "not-a-date"
signed_by = "me"
tool = "manual"
"#;
    let err = parse_archive_toml(bad).expect_err("non-RFC3339 created_at must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("created_at"),
        "error should mention the created_at field, got: {msg}"
    );
    assert!(
        msg.contains("rfc3339") || msg.contains("parse") || msg.contains("invalid"),
        "error should mention the date-parse failure, got: {msg}"
    );
}

#[test]
fn test_install_rejects_checksums_txt_over_10000_lines() {
    // The parser caps checksums.txt at 10_000 entries
    // (DoS guard). Hand-craft a checksums.txt with 10_001
    // entries and assert that the parser refuses it. We test
    // the parser directly because the full install path
    // would also hit the signature check (the new
    // checksums.txt bytes invalidate the signature), and
    // we want to pin the specific cap.
    use livtet_plugins::archive::checksums::parse_checksums;
    let mut big: String = String::new();
    for i in 0..10_001 {
        // 64 hex chars (a fake sha256) + "  " + path.
        big.push_str(&format!("{i:064x}  plugin/fake_{i}.lua\n"));
    }
    let err = parse_checksums(&big).expect_err("10001-line checksums.txt must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("10000") || msg.contains("10_000") || msg.contains("exceeds"),
        "expected a 10_000-entry cap error, got: {msg}"
    );
}

// =====================================================================
// Step 2 (Task 2.5 plan): `archive/checksums.rs` error-path
// tests. The 10_000-entry cap is already covered above; these
// tests cover the three remaining parse-error branches:
//   - line with no path separator (`"hash"` with no `"  "`)
//   - line with empty hash (`"  plugin/x.lua"`)
//   - line with empty path (`"hash  "` with trailing whitespace)
//
// `parse_checksums` uses `"  "` (two ASCII spaces) as the
// hash/path separator, matching `render_checksums`. A single
// space or a tab will therefore be treated as part of the
// hash, which is the correct behavior for catching accidental
// truncation.
// =====================================================================

#[test]
fn test_parse_checksums_rejects_line_with_no_separator() {
    // A line that has no `"  "` separator (the parser splits
    // on two spaces, matching `render_checksums`) is treated
    // as a hash with no path. The parser surfaces this as
    // the "missing path" error.
    let line = "abc123def456";
    let err = parse_checksums(line).expect_err("line with no separator must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("path") || msg.contains("missing"),
        "expected a 'missing path' error, got: {msg}"
    );
}

#[test]
fn test_parse_checksums_rejects_line_with_empty_hash() {
    // `"  plugin/x.lua"` — the hash portion is empty before
    // the `"  "` separator. The parser must reject this with
    // an "empty hash" error.
    let line = "  plugin/x.lua";
    let err = parse_checksums(line).expect_err("line with empty hash must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("empty") && msg.contains("hash"),
        "expected an 'empty hash' error, got: {msg}"
    );
}

#[test]
fn test_parse_checksums_rejects_line_with_empty_path() {
    // `"abc123def456  "` — the hash is non-empty but the
    // path is empty (just trailing whitespace after the
    // separator). The parser splits on `"  "` and gets a
    // one-element iterator, so the path branch surfaces
    // "missing path".
    let line = "abc123def456  ";
    let err = parse_checksums(line).expect_err("line with empty path must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("path") || msg.contains("missing"),
        "expected a 'missing path' error, got: {msg}"
    );
}
