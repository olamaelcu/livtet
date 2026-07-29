use camino::Utf8Path;
use fs_err as fs;
use sha2::{Digest, Sha256};

use crate::archive::error::ArchiveError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumEntry {
    pub sha256: String,
    pub path: String,
}

pub fn generate_checksums(
    plugin_dir: &Utf8Path,
    extra_entries: &[(String, Vec<u8>)],
) -> Result<Vec<ChecksumEntry>, ArchiveError> {
    let root = plugin_dir
        .parent()
        .filter(|p| !p.as_str().is_empty())
        .unwrap_or(plugin_dir);
    let mut entries = Vec::new();
    collect_checksums(root, plugin_dir, &mut entries)?;

    for (path, bytes) in extra_entries {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let sha = hex::encode(hasher.finalize());
        entries.push(ChecksumEntry {
            sha256: sha,
            path: path.clone(),
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn collect_checksums(
    root: &Utf8Path,
    dir: &Utf8Path,
    out: &mut Vec<ChecksumEntry>,
) -> Result<(), ArchiveError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let utf8 = camino::Utf8Path::from_path(&path)
                .ok_or_else(|| ArchiveError::InvalidArchive(format!("non-utf8 path: {path:?}")))?;
            collect_checksums(root, utf8, out)?;
        } else {
            let bytes = fs::read(&path)?;
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let sha = hex::encode(hasher.finalize());
            let utf8 = camino::Utf8Path::from_path(&path)
                .ok_or_else(|| ArchiveError::InvalidArchive(format!("non-utf8 path: {path:?}")))?;
            let rel = utf8
                .strip_prefix(root)
                .map_err(|e| ArchiveError::InvalidArchive(format!("strip_prefix: {e}")))?
                .to_string()
                .replace('\\', "/");
            out.push(ChecksumEntry {
                sha256: sha,
                path: rel,
            });
        }
    }
    Ok(())
}

pub fn render_checksums(entries: &[ChecksumEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!("{}  {}\n", e.sha256, e.path));
    }
    out
}

pub fn parse_checksums(text: &str) -> Result<Vec<ChecksumEntry>, ArchiveError> {
    if text.lines().count() > 10_000 {
        return Err(ArchiveError::InvalidArchive(
            "checksums.txt exceeds 10000 entries".to_string(),
        ));
    }
    let mut out = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, "  ");
        let sha = parts.next().ok_or_else(|| {
            ArchiveError::InvalidArchive("checksums.txt line missing hash".to_string())
        })?;
        let path = parts
            .next()
            .ok_or_else(|| {
                ArchiveError::InvalidArchive("checksums.txt line missing path".to_string())
            })?
            .to_string();
        if path.is_empty() {
            return Err(ArchiveError::InvalidArchive(
                "checksums.txt line has empty path".to_string(),
            ));
        }
        if sha.is_empty() {
            return Err(ArchiveError::InvalidArchive(
                "checksums.txt line has empty hash".to_string(),
            ));
        }
        out.push(ChecksumEntry {
            sha256: sha.to_string(),
            path,
        });
    }
    Ok(out)
}
