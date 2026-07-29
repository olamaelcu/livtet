use std::collections::BTreeMap;

use camino::Utf8Path;
use ed25519_dalek::SigningKey;
use fs_err as fs;
use sha2::{Digest, Sha256};

use crate::{
    archive::manifest::now_iso,
    keys::{fingerprint, signing::sign_bytes},
    repository::{
        error::RepositoryError,
        index::{
            Index, IndexPlugin, IndexVersionEntry, SUPPORTED_INDEX_FORMAT_VERSION,
            parse_index_json, render_index_json,
        },
        repo_toml::{RepoSection, RepoToml, SigningSection, render_repo_toml},
    },
};

pub fn init_repo(
    repo_dir: &Utf8Path,
    name: &str,
    url: &str,
    key_fingerprint: &str,
    key_label: Option<&str>,
) -> Result<(), RepositoryError> {
    fs::create_dir_all(repo_dir.join("pool"))?;
    let toml = RepoToml {
        format_version: 1,
        repo: RepoSection {
            name: name.to_string(),
            url: url.to_string(),
            description: Some("Livtet plugins".to_string()),
            maintainer: None,
        },
        signing: SigningSection {
            key_label: key_label.unwrap_or("").to_string(),
            key_fingerprint: key_fingerprint.to_string(),
        },
    };
    fs::write(repo_dir.join("repo.toml"), render_repo_toml(&toml))?;
    let empty = Index {
        format_version: SUPPORTED_INDEX_FORMAT_VERSION,
        generated_at: now_iso(),
        plugins: BTreeMap::new(),
    };
    fs::write(repo_dir.join("index.json"), render_index_json(&empty))?;
    Ok(())
}

pub fn publish_archive(
    repo_dir: &Utf8Path,
    archive_path: &Utf8Path,
    plugin_id: &str,
    version: &str,
    entry: &str,
    min_app_version: &str,
    signing_key: &SigningKey,
) -> Result<(), RepositoryError> {
    let bytes = fs::read(archive_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha = hex::encode(hasher.finalize());

    let archive_name = format!("{plugin_id}-{version}.ltp");
    let pool_dest = repo_dir.join("pool").join(&archive_name);
    fs::create_dir_all(repo_dir.join("pool"))?;
    fs::write(&pool_dest, &bytes)?;

    let index_path = repo_dir.join("index.json");
    let mut index: Index = if index_path.exists() {
        let text = fs::read_to_string(&index_path)?;
        parse_index_json(&text)?
    } else {
        Index {
            format_version: SUPPORTED_INDEX_FORMAT_VERSION,
            generated_at: now_iso(),
            plugins: BTreeMap::new(),
        }
    };

    let entry_struct = IndexVersionEntry {
        entry: entry.to_string(),
        capabilities: BTreeMap::new(),
        dependencies: vec![],
        archive: archive_name,
        archive_size: bytes.len() as u64,
        archive_sha256: sha,
        min_app_version: min_app_version.to_string(),
    };
    index
        .plugins
        .entry(plugin_id.to_string())
        .or_insert_with(|| IndexPlugin {
            versions: BTreeMap::new(),
        })
        .versions
        .insert(version.to_string(), entry_struct);
    index.generated_at = now_iso();

    let rendered = render_index_json(&index);
    fs::write(&index_path, &rendered)?;
    write_signature(&index_path, signing_key)?;
    Ok(())
}

pub fn sign_index(repo_dir: &Utf8Path, signing_key: &SigningKey) -> Result<(), RepositoryError> {
    let index_path = repo_dir.join("index.json");
    if !index_path.exists() {
        return Err(RepositoryError::NotFound("index.json".to_string()));
    }
    write_signature(&index_path, signing_key)
}

pub fn unpublish_version(
    repo_dir: &Utf8Path,
    plugin_id: &str,
    version: &str,
    signing_key: &SigningKey,
) -> Result<(), RepositoryError> {
    let archive_name = format!("{plugin_id}-{version}.ltp");
    let pool_path = repo_dir.join("pool").join(&archive_name);
    if pool_path.exists() {
        fs::remove_file(&pool_path)?;
    }

    let index_path = repo_dir.join("index.json");
    let text = fs::read_to_string(&index_path)?;
    let mut index = parse_index_json(&text)?;
    if let Some(plugin) = index.plugins.get_mut(plugin_id) {
        plugin.versions.remove(version);
        if plugin.versions.is_empty() {
            index.plugins.remove(plugin_id);
        }
    }
    index.generated_at = now_iso();
    fs::write(&index_path, render_index_json(&index))?;
    write_signature(&index_path, signing_key)?;
    Ok(())
}

fn write_signature(index_path: &Utf8Path, signing_key: &SigningKey) -> Result<(), RepositoryError> {
    let text = fs::read_to_string(index_path)?;
    let sig = sign_bytes(signing_key, text.as_bytes())?;
    let sig_path = index_path.with_extension("json.sig");
    fs::write(sig_path, sig)?;
    Ok(())
}

#[allow(dead_code)]
pub fn fingerprint_for_key(signing_key: &SigningKey) -> String {
    fingerprint(&signing_key.verifying_key())
}
