use std::io::{Read, Write};

use camino::Utf8Path;
use fs_err as fs;
use zip::ZipArchive;

use crate::{
    archive::{error::ArchiveError, verify::verify},
    keys::TrustStore,
    types::InstallReport,
};

pub fn install(
    archive_path: &Utf8Path,
    providers_dir: &Utf8Path,
    trust_store: Option<&TrustStore>,
) -> Result<InstallReport, ArchiveError> {
    let report = verify(archive_path, trust_store)?;
    if !report.valid {
        return Err(ArchiveError::InvalidArchive(format!(
            "verification failed: {:?}",
            report.errors
        )));
    }
    let id = report.plugin_id.clone().unwrap();
    let version = report.version.clone().unwrap();

    let target = providers_dir.join(&id).join(&version);
    let temp = providers_dir.join(format!(
        ".{id}-{version}.tmp-{}",
        ulid::Ulid::new().to_string()
    ));

    fs::create_dir_all(&temp)?;
    let file = fs::File::open(archive_path)?;
    let mut zip = ZipArchive::new(file)?;
    let mut warnings = Vec::new();
    let mut total_extracted: u64 = 0;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        if !name.starts_with("plugin/") {
            continue;
        }
        let rel = &name["plugin/".len()..];
        if rel.is_empty() {
            continue;
        }
        if rel.contains("..") || rel.starts_with('/') {
            return Err(ArchiveError::InvalidArchive(format!(
                "unsafe path in archive: {rel}"
            )));
        }
        if rel.len() > 255 {
            return Err(ArchiveError::InvalidArchive(format!(
                "path exceeds 255 bytes: {rel} ({} bytes)",
                rel.len()
            )));
        }
        let out_path = temp.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if entry.size() > 20 * 1024 * 1024 {
                return Err(ArchiveError::InvalidArchive(format!(
                    "file too large: {rel} ({} bytes)",
                    entry.size()
                )));
            }
            let mut out = fs::File::create(&out_path)?;
            let mut buf = [0u8; 8192];
            loop {
                let n = entry.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                total_extracted += n as u64;
                if total_extracted > 100 * 1024 * 1024 {
                    return Err(ArchiveError::InvalidArchive(format!(
                        "total extracted exceeds 100 MB limit ({} bytes)",
                        total_extracted
                    )));
                }
                out.write_all(&buf[..n])?;
            }
        }
    }

    if target.exists() {
        warnings.push(format!("replaced existing v{version}"));
    }
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&temp, &target)?;

    Ok(InstallReport {
        id,
        version,
        signer_label: report.signer_label.unwrap_or_default(),
        signer_fingerprint: report.signer_key_id.unwrap_or_default(),
        trusted: report.trusted.unwrap_or(false),
        replaced_versions: vec![],
        warnings,
        install_path: target,
    })
}
