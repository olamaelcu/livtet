use std::collections::HashSet;

use camino::Utf8Path;
use fs_err as fs;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::{
    archive::{
        checksums::parse_checksums,
        error::ArchiveError,
        manifest::{ArchiveMeta, parse_archive_toml},
    },
    keys::{
        TrustStore, fingerprint,
        signing::{parse_pubkey_text, verify_bytes},
    },
    types::VerifyReport,
};

pub fn verify(
    archive_path: &Utf8Path,
    trust_store: Option<&TrustStore>,
) -> Result<VerifyReport, ArchiveError> {
    let mut report = VerifyReport {
        valid: false,
        plugin_id: None,
        version: None,
        signer_key_id: None,
        signer_label: None,
        trusted: None,
        file_count: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
    };

    let archive_meta = match fs::metadata(archive_path) {
        Ok(m) => m,
        Err(e) => {
            report.errors.push(format!("stat archive: {e}"));
            return Ok(report);
        }
    };
    if archive_meta.len() > 50 * 1024 * 1024 {
        report.errors.push(
            ArchiveError::InvalidArchive(format!(
                "archive exceeds 50 MB limit ({} bytes)",
                archive_meta.len()
            ))
            .to_string(),
        );
        return Ok(report);
    }

    let file = match fs::File::open(archive_path) {
        Ok(f) => f,
        Err(e) => {
            report.errors.push(format!("open archive: {e}"));
            return Ok(report);
        }
    };
    let mut zip = match ZipArchive::new(file) {
        Ok(z) => z,
        Err(e) => {
            report.errors.push(format!("read zip: {e}"));
            return Ok(report);
        }
    };

    let meta = match read_archive_meta(&mut zip) {
        Ok(m) => m,
        Err(e) => {
            report.errors.push(e.to_string());
            return Ok(report);
        }
    };
    report.plugin_id = Some(meta.plugin_id.clone());
    report.version = Some(meta.plugin_version.clone());

    let checksums_text = match read_file_from_zip(&mut zip, "META-INF/checksums.txt") {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(e.to_string());
            return Ok(report);
        }
    };
    let signature_bytes = match read_bytes_from_zip(&mut zip, "META-INF/signature.bin") {
        Ok(b) => b,
        Err(e) => {
            report.errors.push(e.to_string());
            return Ok(report);
        }
    };
    let pubkey_text = match read_file_from_zip(&mut zip, "META-INF/pubkey.txt") {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(e.to_string());
            return Ok(report);
        }
    };

    let pubkey = match parse_pubkey_text(&pubkey_text) {
        Ok(k) => k,
        Err(e) => {
            report.errors.push(e.to_string());
            return Ok(report);
        }
    };
    let signer_fp = fingerprint(&pubkey);
    report.signer_key_id = Some(signer_fp.clone());
    report.signer_label = Some(meta.signed_by.clone());

    let store = trust_store.cloned().unwrap_or_else(TrustStore::empty);
    report.trusted = Some(store.is_trusted(&pubkey));
    if store.is_revoked(&signer_fp) {
        report.errors.push(ArchiveError::RevokedKey.to_string());
        return Ok(report);
    }
    if !store.is_trusted(&pubkey) {
        report.errors.push(
            ArchiveError::UntrustedKey {
                fingerprint: signer_fp.clone(),
            }
            .to_string(),
        );
    }

    if let Err(e) = verify_bytes(&pubkey, checksums_text.as_bytes(), &signature_bytes) {
        report.errors.push(e.to_string());
        return Ok(report);
    }

    let parsed = match parse_checksums(&checksums_text) {
        Ok(p) => p,
        Err(e) => {
            report.errors.push(e.to_string());
            return Ok(report);
        }
    };
    report.file_count = parsed.len();

    let declared_paths: HashSet<String> = parsed.iter().map(|e| e.path.clone()).collect();
    for entry in parsed {
        let bytes = match read_bytes_from_zip(&mut zip, &entry.path) {
            Ok(b) => b,
            Err(e) => {
                report.errors.push(format!("{}: {e}", entry.path));
                continue;
            }
        };
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if actual != entry.sha256 {
            report.errors.push(
                ArchiveError::IntegrityCheckFailed {
                    path: entry.path.clone(),
                }
                .to_string(),
            );
        }
    }

    let mut unsigned: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let e = zip.by_index(i)?;
        let name = e.name().to_string();
        if name.starts_with("plugin/") && !declared_paths.contains(&name) {
            unsigned.push(name);
        }
    }
    for u in &unsigned {
        report
            .errors
            .push(ArchiveError::UnsignedFile(u.clone()).to_string());
    }

    let livtet_toml = match read_file_from_zip(&mut zip, "plugin/livtet.toml") {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(e.to_string());
            return Ok(report);
        }
    };
    let plugin_manifest: toml::Value = toml::from_str(&livtet_toml)
        .map_err(|e| ArchiveError::InvalidArchive(format!("plugin/livtet.toml: {e}")))?;
    let id_in_manifest = plugin_manifest
        .get("plugin")
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ver_in_manifest = plugin_manifest
        .get("plugin")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id_in_manifest != meta.plugin_id {
        report.errors.push(
            ArchiveError::ManifestMismatch {
                field: "id".to_string(),
            }
            .to_string(),
        );
    }
    if ver_in_manifest != meta.plugin_version {
        report.errors.push(
            ArchiveError::ManifestMismatch {
                field: "version".to_string(),
            }
            .to_string(),
        );
    }

    if let Err(e) = crate::manifest::PluginManifest::from_toml(&livtet_toml) {
        report.errors.push(format!("manifest schema: {e}"));
    }

    report.valid = report.errors.is_empty();
    Ok(report)
}

fn read_archive_meta(zip: &mut ZipArchive<fs::File>) -> Result<ArchiveMeta, ArchiveError> {
    let text = read_file_from_zip(zip, "META-INF/archive.toml")?;
    parse_archive_toml(&text)
}

fn read_file_from_zip(zip: &mut ZipArchive<fs::File>, name: &str) -> Result<String, ArchiveError> {
    let mut entry = zip
        .by_name(name)
        .map_err(|_| ArchiveError::MissingMetadata(name.to_string()))?;
    let mut s = String::new();
    use std::io::Read;
    entry.read_to_string(&mut s)?;
    Ok(s)
}

fn read_bytes_from_zip(
    zip: &mut ZipArchive<fs::File>,
    name: &str,
) -> Result<Vec<u8>, ArchiveError> {
    let mut entry = zip
        .by_name(name)
        .map_err(|_| ArchiveError::MissingMetadata(name.to_string()))?;
    let mut buf = Vec::new();
    use std::io::Read;
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}
