use std::io::Write;

use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use sha2::{Digest, Sha256};
use zip::{CompressionMethod, write::SimpleFileOptions};

use crate::{
    archive::{
        checksums::{ChecksumEntry, render_checksums},
        error::ArchiveError,
        manifest::{ArchiveMeta, now_iso, render_archive_toml},
        verify::verify,
    },
    keys::{
        TrustStore,
        signing::{load_minisign_signing_key, sign_bytes},
    },
};

pub fn pack(
    plugin_dir: &Utf8Path,
    key_path: &Utf8Path,
    label: &str,
    output_dir: &Utf8Path,
) -> Result<Utf8PathBuf, ArchiveError> {
    let manifest_text = fs::read_to_string(plugin_dir.join("livtet.toml"))
        .map_err(|e| ArchiveError::InvalidArchive(format!("plugin/livtet.toml: {e}")))?;
    let manifest: toml::Value = toml::from_str(&manifest_text)
        .map_err(|e| ArchiveError::InvalidArchive(format!("plugin/livtet.toml parse: {e}")))?;
    let id = manifest
        .get("plugin")
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| ArchiveError::InvalidArchive("missing [plugin].id".to_string()))?
        .to_string();
    let version = manifest
        .get("plugin")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| ArchiveError::InvalidArchive("missing [plugin].version".to_string()))?
        .to_string();

    let (sk, signing_key) = load_minisign_signing_key(key_path)?;
    let verifying_key = signing_key.verifying_key();

    let pubkey = minisign::PublicKey::from_secret_key(&sk)
        .map_err(|e| ArchiveError::Key(format!("derive minisign pubkey: {e}")))?;
    let pubkey_box = pubkey
        .to_box()
        .map_err(|e| ArchiveError::Key(format!("minisign pk to_box: {e}")))?;
    let pubkey_text = pubkey_box.to_string();

    fs::create_dir_all(output_dir).map_err(ArchiveError::Io)?;
    let ltp_path = output_dir.join(format!("{id}-{version}.ltp"));
    let file = fs::File::create(&ltp_path).map_err(ArchiveError::Io)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let archive_meta = ArchiveMeta {
        format_version: 1,
        plugin_id: id.clone(),
        plugin_version: version.clone(),
        created_at: now_iso(),
        signed_by: label.to_string(),
        tool: "livtet plugin pack".to_string(),
    };
    let archive_toml_text = render_archive_toml(&archive_meta);
    zip.start_file("META-INF/archive.toml", opts)
        .map_err(ArchiveError::Zip)?;
    zip.write_all(archive_toml_text.as_bytes())
        .map_err(ArchiveError::Io)?;

    let mut plugin_files: Vec<(String, Vec<u8>)> = Vec::new();
    walk_plugin_files(plugin_dir, "plugin", &mut plugin_files)?;
    for (archive_path, bytes) in &plugin_files {
        zip.start_file(archive_path, opts)
            .map_err(ArchiveError::Zip)?;
        zip.write_all(bytes).map_err(ArchiveError::Io)?;
    }

    let mut entries: Vec<ChecksumEntry> = plugin_files
        .iter()
        .map(|(path, bytes)| ChecksumEntry {
            sha256: sha256_hex(bytes),
            path: path.clone(),
        })
        .collect();
    entries.push(ChecksumEntry {
        sha256: sha256_hex(archive_toml_text.as_bytes()),
        path: "META-INF/archive.toml".to_string(),
    });
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let checksums_text = render_checksums(&entries);
    zip.start_file("META-INF/checksums.txt", opts)
        .map_err(ArchiveError::Zip)?;
    zip.write_all(checksums_text.as_bytes())
        .map_err(ArchiveError::Io)?;

    let signature = sign_bytes(&signing_key, checksums_text.as_bytes())?;
    zip.start_file("META-INF/signature.bin", opts)
        .map_err(ArchiveError::Zip)?;
    zip.write_all(&signature).map_err(ArchiveError::Io)?;

    zip.start_file("META-INF/pubkey.txt", opts)
        .map_err(ArchiveError::Zip)?;
    zip.write_all(pubkey_text.as_bytes())
        .map_err(ArchiveError::Io)?;

    zip.finish().map_err(ArchiveError::Zip)?;

    let mut self_trust = TrustStore::empty();
    self_trust.add_user_key(label, verifying_key)?;

    let report = verify(&ltp_path, Some(&self_trust))?;
    if !report.valid {
        let _ = fs::remove_file(&ltp_path);
        return Err(ArchiveError::InvalidArchive(format!(
            "self-verification failed: {:?}",
            report.errors
        )));
    }

    Ok(ltp_path)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn walk_plugin_files(
    dir: &Utf8Path,
    prefix: &str,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), ArchiveError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let utf8 = Utf8Path::from_path(&path)
            .ok_or_else(|| ArchiveError::InvalidArchive(format!("non-utf8 path: {path:?}")))?;
        if utf8.is_dir() {
            let name = utf8.file_name().unwrap_or_default();
            walk_plugin_files(utf8, &format!("{prefix}/{name}"), files)?;
        } else {
            let name = utf8.file_name().unwrap_or_default();
            let archive_path = format!("{prefix}/{name}");
            let bytes = fs::read(utf8)?;
            files.push((archive_path, bytes));
        }
    }
    Ok(())
}
