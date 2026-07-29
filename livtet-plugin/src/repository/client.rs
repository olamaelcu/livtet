use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use ed25519_dalek::VerifyingKey;
use fs_err as fs;

use crate::{
    keys::{TrustStore, fingerprint, signing::verify_bytes},
    repository::{
        config::RepositoriesFile,
        error::RepositoryError,
        hmac::HmacKey,
        index::{
            Index, IndexVersionEntry, find_version as index_find_version, parse_index_json,
            verify_index_signature,
        },
        installed::{InstalledEntry, InstalledFile},
        repo_toml::{RepoToml, parse_repo_toml},
    },
    types::{RepoSearchResult, Repository, RepositoryAddResult, RepositoryUpdateResult},
};

pub struct RepositoryClient {
    pub cache_dir: Utf8PathBuf,
    pub config_dir: Utf8PathBuf,
    pub hmac_key: HmacKey,
    pub http: reqwest::Client,
}

impl RepositoryClient {
    pub fn new(cache_dir: Utf8PathBuf, config_dir: Utf8PathBuf, hmac_key: HmacKey) -> Self {
        Self::with_http(
            cache_dir,
            config_dir,
            hmac_key,
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        )
    }

    /// Build a client with an externally supplied `reqwest::Client`.
    pub fn with_http(
        cache_dir: Utf8PathBuf,
        config_dir: Utf8PathBuf,
        hmac_key: HmacKey,
        http: reqwest::Client,
    ) -> Self {
        Self {
            cache_dir,
            config_dir,
            hmac_key,
            http,
        }
    }

    pub fn repositories_path(&self) -> Utf8PathBuf {
        self.config_dir.join("repositories.toml")
    }

    pub fn installed_path(&self) -> Utf8PathBuf {
        self.config_dir.join("installed.json")
    }

    pub fn load_repositories(&self) -> Result<RepositoriesFile, RepositoryError> {
        RepositoriesFile::load(&self.repositories_path(), &self.hmac_key)
    }

    pub fn save_repositories(&self, file: &RepositoriesFile) -> Result<(), RepositoryError> {
        file.save(&self.repositories_path(), &self.hmac_key)
    }

    pub fn load_installed(&self) -> Result<InstalledFile, RepositoryError> {
        InstalledFile::load(&self.installed_path(), &self.hmac_key)
    }

    pub fn save_installed(&self, file: &InstalledFile) -> Result<(), RepositoryError> {
        file.save(&self.installed_path(), &self.hmac_key)
    }

    pub fn record_install(&self, entry: InstalledEntry) -> Result<(), RepositoryError> {
        let mut file = self.load_installed()?;
        file.entries
            .retain(|e| !(e.id == entry.id && e.version == entry.version));
        file.entries.push(entry);
        self.save_installed(&file)
    }

    pub fn remove_installed_entry(&self, id: &str, version: &str) -> Result<bool, RepositoryError> {
        let mut file = self.load_installed()?;
        let before = file.entries.len();
        file.entries
            .retain(|e| !(e.id == id && e.version == version));
        let removed = file.entries.len() != before;
        if removed {
            self.save_installed(&file)?;
        }
        Ok(removed)
    }

    pub fn add_offline(
        &self,
        name: &str,
        url: &str,
        signing_key: &VerifyingKey,
    ) -> RepositoryAddResult {
        RepositoryAddResult::NeedsTofuConfirmation {
            name: name.to_string(),
            url: url.to_string(),
            fingerprint: fingerprint(signing_key),
        }
    }

    pub async fn add(&self, url: &str) -> Result<RepositoryAddResult, RepositoryError> {
        let (repo_toml, _raw) = self.fetch_repo_toml(url).await?;
        Ok(RepositoryAddResult::NeedsTofuConfirmation {
            name: repo_toml.repo.name,
            url: repo_toml.repo.url,
            fingerprint: repo_toml.signing.key_fingerprint,
        })
    }

    pub async fn fetch_repo_toml(&self, url: &str) -> Result<(RepoToml, String), RepositoryError> {
        let toml_url = format!("{}/repo.toml", url.trim_end_matches('/'));
        let resp = self
            .http
            .get(&toml_url)
            .send()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(RepositoryError::Http {
                status: status.as_u16(),
                url: toml_url,
            });
        }
        let text = resp
            .text()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;
        let parsed = parse_repo_toml(&text)?;
        Ok((parsed, text))
    }

    pub async fn fetch_index(
        &self,
        base_url: &str,
        verifying_key: &VerifyingKey,
    ) -> Result<(Index, String), RepositoryError> {
        let base = base_url.trim_end_matches('/');
        let index_url = format!("{base}/index.json");
        let sig_url = format!("{base}/index.json.sig");

        let resp = self
            .http
            .get(&index_url)
            .send()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(RepositoryError::Http {
                status: status.as_u16(),
                url: index_url,
            });
        }
        let text = resp
            .text()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;

        let sig_resp = self
            .http
            .get(&sig_url)
            .send()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;
        let sig_status = sig_resp.status();
        if !sig_status.is_success() {
            return Err(RepositoryError::Http {
                status: sig_status.as_u16(),
                url: sig_url,
            });
        }
        let sig_bytes = sig_resp
            .bytes()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;

        let index = parse_index_json(&text)?;
        verify_bytes(verifying_key, text.as_bytes(), &sig_bytes)
            .map_err(|_| RepositoryError::BadIndexSignature)?;
        Ok((index, text))
    }

    pub async fn download_archive(
        &self,
        base_url: &str,
        archive_name: &str,
        expected_size: u64,
        expected_sha256: &str,
        dest: &Utf8Path,
    ) -> Result<(), RepositoryError> {
        let base = base_url.trim_end_matches('/');
        let url = format!("{base}/pool/{archive_name}");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(RepositoryError::Http {
                status: status.as_u16(),
                url,
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;
        if bytes.len() as u64 != expected_size {
            return Err(RepositoryError::Network(format!(
                "size mismatch for {archive_name}: expected {expected_size}, got {}",
                bytes.len()
            )));
        }
        let actual_sha256 = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        };
        if actual_sha256 != expected_sha256 {
            return Err(RepositoryError::IndexParse(format!(
                "sha256 mismatch for {archive_name}: expected {expected_sha256}, got {actual_sha256}"
            )));
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(dest, &bytes)?;
        Ok(())
    }

    pub fn search(
        &self,
        query: &str,
        trust: &TrustStore,
    ) -> Result<Vec<RepoSearchResult>, RepositoryError> {
        let mut all_results = Vec::new();
        let repos = self.load_repositories()?.repositories;
        for repo in &repos {
            let repo_cache = self.cache_dir.join(&repo.name);
            let index_path = repo_cache.join("index.json");
            let sig_path = repo_cache.join("index.json.sig");
            if !index_path.exists() || !sig_path.exists() {
                continue;
            }
            let index_text = match fs::read_to_string(&index_path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let sig_bytes = match fs::read(&sig_path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let verifying_key = match trust.user_key_by_label(&repo.name) {
                Some(k) => *k,
                None => match find_user_key_by_fingerprint(trust, &repo.key_fingerprint) {
                    Some(k) => k,
                    None => continue,
                },
            };
            let index = match parse_index_json(&index_text) {
                Ok(i) => i,
                Err(_) => continue,
            };
            if verify_index_signature(&index, &index_text, &sig_bytes, &verifying_key).is_err() {
                continue;
            }
            let results = search_index(&index, query, &repo.name);
            all_results.extend(results);
        }
        all_results.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.repository.cmp(&b.repository))
                .then_with(|| a.plugin_id.cmp(&b.plugin_id))
                .then_with(|| a.version.cmp(&b.version))
        });
        Ok(all_results)
    }

    pub async fn confirm_add(
        &self,
        url: &str,
        trust_store: &TrustStore,
    ) -> Result<(), RepositoryError> {
        let (repo_toml, repo_toml_text) = self.fetch_repo_toml(url).await?;
        let verifying_key = match trust_store.user_key_by_label(&repo_toml.signing.key_label) {
            Some(k) => *k,
            None => {
                match find_user_key_by_fingerprint(trust_store, &repo_toml.signing.key_fingerprint)
                {
                    Some(k) => k,
                    None => {
                        return Err(RepositoryError::Keyring(format!(
                            "trusted key with label {:?} or fingerprint {} not found",
                            repo_toml.signing.key_label, repo_toml.signing.key_fingerprint
                        )));
                    }
                }
            }
        };
        let (_index, index_text) = self.fetch_index(url, &verifying_key).await?;
        let base = url.trim_end_matches('/');
        let index_sig_url = format!("{base}/index.json.sig");
        let index_sig_bytes = self
            .http
            .get(index_sig_url)
            .send()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;

        let mut file = self.load_repositories()?;
        if file.repositories.iter().any(|r| r.url == url) {
            return Err(RepositoryError::AlreadyAdded(url.to_string()));
        }

        let cache_subdir = self.cache_dir.join(&repo_toml.repo.name);
        fs::create_dir_all(&cache_subdir)?;
        fs::write(cache_subdir.join("repo.toml"), &repo_toml_text)?;
        fs::write(cache_subdir.join("index.json"), &index_text)?;
        fs::write(cache_subdir.join("index.json.sig"), &index_sig_bytes)?;

        file.repositories.push(Repository {
            name: repo_toml.repo.name,
            url: repo_toml.repo.url,
            description: repo_toml.repo.description,
            maintainer: repo_toml.repo.maintainer,
            added_at: crate::archive::manifest::now_iso(),
            last_index_update: Some(crate::archive::manifest::now_iso()),
            key_fingerprint: repo_toml.signing.key_fingerprint,
        });
        self.save_repositories(&file)?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Repository>, RepositoryError> {
        Ok(self.load_repositories()?.repositories)
    }

    pub fn remove(&self, name_or_url: &str) -> Result<(), RepositoryError> {
        let mut file = self.load_repositories()?;
        let before = file.repositories.len();
        file.repositories
            .retain(|r| r.name != name_or_url && r.url != name_or_url);
        if file.repositories.len() == before {
            return Err(RepositoryError::NotFound(name_or_url.to_string()));
        }
        self.save_repositories(&file)
    }

    pub fn detect_key_change(
        &self,
        name: &str,
        old_fingerprint: &str,
        new_fingerprint: &str,
    ) -> RepositoryUpdateResult {
        if old_fingerprint == new_fingerprint {
            RepositoryUpdateResult::Ok { plugin_count: 0 }
        } else {
            RepositoryUpdateResult::KeyChanged {
                name: name.to_string(),
                old_fingerprint: old_fingerprint.to_string(),
                new_fingerprint: new_fingerprint.to_string(),
            }
        }
    }

    pub async fn update(
        &self,
        name_or_url: &str,
        trust_store: &TrustStore,
    ) -> Result<RepositoryUpdateResult, RepositoryError> {
        let mut file = self.load_repositories()?;
        let repo = file
            .repositories
            .iter()
            .find(|r| r.name == name_or_url || r.url == name_or_url)
            .ok_or_else(|| RepositoryError::NotFound(name_or_url.to_string()))?
            .clone();
        let previous_fp = repo.key_fingerprint.clone();

        let (fresh_toml, repo_toml_text) = self.fetch_repo_toml(&repo.url).await?;
        let fresh_fp = fresh_toml.signing.key_fingerprint.clone();

        if previous_fp != fresh_fp {
            return Ok(self.detect_key_change(&repo.name, &previous_fp, &fresh_fp));
        }

        // Same key — fetch + verify the index, then refresh the cached files
        // and bump `last_index_update`. Mirrors the lookup pattern in
        // `confirm_add` (label first, fingerprint fallback) so a key trusted
        // under a different label still verifies the index.
        let by_label = trust_store.user_key_by_label(&fresh_toml.signing.key_label);
        let by_fp = find_user_key_by_fingerprint(trust_store, &fresh_toml.signing.key_fingerprint);
        let verifying_key = by_label.or(by_fp.as_ref()).ok_or_else(|| {
            RepositoryError::Keyring(format!(
                "trusted key with label {:?} or fingerprint {} not found",
                fresh_toml.signing.key_label, fresh_toml.signing.key_fingerprint
            ))
        })?;
        let (index, index_text) = self.fetch_index(&repo.url, verifying_key).await?;

        // `fetch_index` discards the raw sig bytes; re-fetch so we can
        // persist them alongside `index.json`. Same pattern as `confirm_add`.
        let base = repo.url.trim_end_matches('/');
        let index_sig_url = format!("{base}/index.json.sig");
        let index_sig_bytes = self
            .http
            .get(&index_sig_url)
            .send()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;

        let cache_subdir = self.cache_dir.join(&fresh_toml.repo.name);
        fs::create_dir_all(&cache_subdir)?;
        fs::write(cache_subdir.join("repo.toml"), &repo_toml_text)?;
        fs::write(cache_subdir.join("index.json"), &index_text)?;
        fs::write(cache_subdir.join("index.json.sig"), &index_sig_bytes)?;

        if let Some(entry) = file
            .repositories
            .iter_mut()
            .find(|r| r.name == repo.name || r.url == repo.url)
        {
            entry.last_index_update = Some(crate::archive::manifest::now_iso());
        }
        self.save_repositories(&file)?;

        Ok(RepositoryUpdateResult::Ok {
            plugin_count: index.plugins.values().map(|p| p.versions.len()).sum(),
        })
    }

    pub async fn confirm_update(
        &self,
        name_or_url: &str,
        trust_store: &TrustStore,
    ) -> Result<(), RepositoryError> {
        // Locate the repository and grab everything we need to drive the
        // re-fetch without holding an `iter_mut` borrow across an `await`.
        let mut file = self.load_repositories()?;
        let repo_idx = file
            .repositories
            .iter()
            .position(|r| r.name == name_or_url || r.url == name_or_url)
            .ok_or_else(|| RepositoryError::NotFound(name_or_url.to_string()))?;
        let url = file.repositories[repo_idx].url.clone();

        // Re-fetch the freshest repo.toml and refresh cached files. Lookup
        // mirrors `confirm_add` (label → fingerprint fallback) so the new
        // key (now trusted by the user after the prior `update` returned
        // `KeyChanged`) can verify the index.
        let (fresh_toml, repo_toml_text) = self.fetch_repo_toml(&url).await?;
        let fresh_fp = fresh_toml.signing.key_fingerprint.clone();

        let by_label = trust_store.user_key_by_label(&fresh_toml.signing.key_label);
        let by_fp = find_user_key_by_fingerprint(trust_store, &fresh_toml.signing.key_fingerprint);
        let verifying_key = by_label.or(by_fp.as_ref()).ok_or_else(|| {
            RepositoryError::Keyring(format!(
                "trusted key with label {:?} or fingerprint {} not found",
                fresh_toml.signing.key_label, fresh_toml.signing.key_fingerprint
            ))
        })?;
        let (_index, index_text) = self.fetch_index(&url, verifying_key).await?;

        let base = url.trim_end_matches('/');
        let index_sig_url = format!("{base}/index.json.sig");
        let index_sig_bytes = self
            .http
            .get(&index_sig_url)
            .send()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;

        let cache_subdir = self.cache_dir.join(&fresh_toml.repo.name);
        fs::create_dir_all(&cache_subdir)?;
        fs::write(cache_subdir.join("repo.toml"), &repo_toml_text)?;
        fs::write(cache_subdir.join("index.json"), &index_text)?;
        fs::write(cache_subdir.join("index.json.sig"), &index_sig_bytes)?;

        let repo = &mut file.repositories[repo_idx];
        if repo.key_fingerprint != fresh_fp {
            // TOFU path: the user has just accepted the new key. Update the
            // stored fingerprint so subsequent `update` calls match again.
            repo.key_fingerprint = fresh_fp;
        }
        repo.last_index_update = Some(crate::archive::manifest::now_iso());
        self.save_repositories(&file)
    }
}

pub fn search_index(index: &Index, query: &str, repo_name: &str) -> Vec<RepoSearchResult> {
    let q = query.to_lowercase();
    let mut results = Vec::new();
    for (plugin_id, plugin) in &index.plugins {
        let id_lower = plugin_id.to_lowercase();
        let mut score: f64 = 0.0;
        if id_lower.contains(&q) {
            score += 1.0;
            if id_lower == q {
                score += 1.0;
            }
        }
        if score == 0.0 {
            continue;
        }
        for (version, entry) in &plugin.versions {
            if entry.archive.to_lowercase().contains(&q) {
                score += 0.5;
            }
            results.push(RepoSearchResult {
                plugin_id: plugin_id.clone(),
                name: plugin_id.clone(),
                version: version.clone(),
                description: None,
                repository: repo_name.to_string(),
                relevance_score: score,
            });
        }
    }
    results
}

pub fn find_version<'a>(
    index: &'a Index,
    id: &str,
    version: &str,
) -> Option<&'a IndexVersionEntry> {
    index_find_version(index, id, version)
}

fn find_user_key_by_fingerprint(
    trust: &TrustStore,
    fingerprint: &str,
) -> Option<ed25519_dalek::VerifyingKey> {
    trust
        .user_keys_snapshot()
        .into_iter()
        .find(|(_, k)| crate::keys::fingerprint(k) == fingerprint)
        .map(|(_, k)| k)
}
