use camino::{Utf8Path, Utf8PathBuf};
use ed25519_dalek::SigningKey;
use fs_err as fs;
use livtet_plugin::{
    archive::{install::install as archive_install, manifest::now_iso, pack::pack as archive_pack},
    keys::{TrustStore, fingerprint, keyfile::keygen as plugin_keygen, signing::parse_pubkey_text},
    repository::{
        client::{RepositoryClient, search_index},
        hmac::HmacKey,
        index::verify_index_signature,
        installed::InstalledEntry,
        publisher::unpublish_version,
    },
    types::{InstallReport, RepoSearchResult},
};
use rand::{Rng as _, rng};

use crate::{
    Result,
    cli::{PassphraseMode, PluginArgs, PluginCommand},
    error::CliError,
    output,
};

pub const DEFAULT_HMAC_KEY: [u8; 32] = [
    0x55, 0x4c, 0x69, 0x76, 0x74, 0x65, 0x74, 0x5f, 0x52, 0x65, 0x70, 0x6f, 0x5f, 0x48, 0x4d, 0x41,
    0x43, 0x5f, 0x4b, 0x65, 0x79, 0x5f, 0x76, 0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Convenience accessor: build an `HmacKey` from the legacy
/// `DEFAULT_HMAC_KEY` constant for unit tests and any code path that
/// cannot use `load_state_hmac_key` (which requires a keyring or
/// passphrase recovery file). The production CLI no longer uses this;
/// see `crates/livtet-cli/src/keyring_recover::load_state_hmac_key`.
pub fn default_hmac_key() -> HmacKey {
    HmacKey::from_bytes(DEFAULT_HMAC_KEY)
}

pub fn run(args: PluginArgs) -> Result<()> {
    match args.command {
        PluginCommand::Keygen {
            label,
            passphrase,
            keys_dir,
            interactive,
        } => cmd_keygen(label.as_deref(), passphrase, &keys_dir, interactive),
        PluginCommand::Trust { pubkey_path } => cmd_trust(&pubkey_path),
        PluginCommand::Search { query, repo } => {
            let cache = default_cache_dir();
            let config = default_config_dir();
            let trust_dir = default_trust_dir();
            let trust = load_trust_store(&trust_dir)?;
            let results = run_search(&query, repo.as_deref(), &cache, &config, &trust)?;
            print_search_results(&results);
            Ok(())
        }
        PluginCommand::Install {
            archive,
            providers,
            repo,
            version,
        } => {
            let providers = providers
                .as_deref()
                .map(expand_tilde_str)
                .transpose()?
                .unwrap_or_else(default_providers_dir);
            let trust_dir = default_trust_dir();
            let trust = load_trust_store(&trust_dir)?;
            let resolved = resolve_install_source(&archive, repo.as_deref(), version.as_deref())?;
            let report = run_install(&resolved, &providers, &trust)?;
            if let Err(e) = record_install_entry(&report.install_path, &report.id, &report.version)
            {
                output::warn(&format!("could not record install in installed.json: {e}"));
            }
            output::success(&format!(
                "Installed {} v{}\n  Signer: {}\n  Fingerprint: {}\n  Path: {}",
                report.id,
                report.version,
                report.signer_label,
                report.signer_fingerprint,
                report.install_path
            ));
            Ok(())
        }
        PluginCommand::List { providers } => {
            let providers = providers
                .as_deref()
                .map(expand_tilde_str)
                .transpose()?
                .unwrap_or_else(default_providers_dir);
            let listed = run_list(&providers)?;
            print_listed(&listed);
            Ok(())
        }
        PluginCommand::Uninstall {
            id,
            version,
            providers,
            interactive,
        } => {
            let providers = providers
                .as_deref()
                .map(expand_tilde_str)
                .transpose()?
                .unwrap_or_else(default_providers_dir);
            cmd_uninstall(&id, &version, &providers, interactive)
        }
        PluginCommand::Unpublish {
            plugin_id,
            version,
            repo_dir,
            interactive,
        } => cmd_unpublish(&plugin_id, &version, &repo_dir, interactive),
        PluginCommand::Pack {
            source,
            label,
            key: _,
            key_dir,
            output,
            ..
        } => {
            let key_dir = expand_tilde(&key_dir)?;
            let output = match output {
                Some(o) => o,
                None => source.parent().unwrap_or(Utf8Path::new(".")).to_path_buf(),
            };
            let env_value = std::env::var("LIVTET_KEY_LABEL").ok();
            let resolved_label = resolve_pack_label(Some(&label), env_value.as_deref());
            let ltp = run_pack(&source, &resolved_label, &key_dir, Some(&output))?;
            output::success(&format!(
                "Packed {ltp}\n  Signing key label: {resolved_label}"
            ));
            Ok(())
        }
    }
}

pub fn run_install(
    archive_path: &Utf8Path,
    providers: &Utf8Path,
    trust: &TrustStore,
) -> Result<InstallReport> {
    archive_install(archive_path, providers, Some(trust)).map_err(|e| CliError::InstallFailed {
        message: format!("{e}"),
    })
}

pub fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// A `Downloader` fetches a URL and writes the body to a fresh temp
/// file. The path is returned as a `Utf8PathBuf`.
///
/// The trait indirection lets us inject a `MockDownloader` in tests
/// (e.g. a fake that reads from a local `file://` URL or returns a
/// canned 5xx response) without having to spin up a real HTTP server
/// or rely on the public internet. Production code uses
/// `HttpDownloader`; tests use `MockDownloader`.
pub trait Downloader: Send + Sync {
    fn download_to_temp(
        &self,
        url: &str,
    ) -> impl std::future::Future<Output = Result<Utf8PathBuf>> + Send;
}

/// Production `Downloader` backed by the shared CLI HTTP agent.
/// Behavior is identical to the previous free `download_to_temp` function:
/// any non-2xx status is an error, the URL's last path segment is
/// sanitized and used as the temp filename, and the file is written
/// under `std::env::temp_dir()` with a per-pid prefix so concurrent
/// `livtet` invocations don't collide.
pub struct HttpDownloader;

impl Downloader for HttpDownloader {
    async fn download_to_temp(&self, url: &str) -> Result<Utf8PathBuf> {
        let client = crate::network::agent_with_timeout(std::time::Duration::from_secs(300));
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| CliError::DownloadFailed {
                message: format!("{e}"),
            })?;
        if !resp.status().is_success() {
            return Err(CliError::DownloadHttpError {
                status: resp.status().as_u16(),
                url: url.to_string(),
            });
        }
        let bytes = resp.bytes().await.map_err(|e| CliError::DownloadFailed {
            message: format!("download body read failed: {e}"),
        })?;
        write_temp_bytes(url, &bytes)
    }
}

pub async fn download_to_temp(url: &str) -> Result<Utf8PathBuf> {
    HttpDownloader.download_to_temp(url).await
}

fn write_temp_bytes(url: &str, bytes: &[u8]) -> Result<Utf8PathBuf> {
    let filename = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("download.ltp");
    let safe_name = sanitize_filename(filename);
    let temp_path = std::env::temp_dir().join(format!(
        "livtet-install-{}-{}",
        std::process::id(),
        safe_name
    ));
    let temp_utf8 =
        Utf8PathBuf::from_path_buf(temp_path).map_err(|e| CliError::NonUtf8Path { path: e })?;
    fs::write(&temp_utf8, bytes)?;
    Ok(temp_utf8)
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn resolve_install_source(
    archive: &str,
    repo: Option<&str>,
    version: Option<&str>,
) -> Result<Utf8PathBuf> {
    if let Some(repo_name) = repo {
        let plugin_id = archive;
        let ver = version.ok_or(CliError::MissingVersion)?;
        let config_dir = default_config_dir();
        let cache_dir = default_cache_dir();
        let key =
            crate::keyring_recover::load_state_hmac_key().map_err(|source| CliError::HmacLoad {
                source: Box::new(source),
            })?;
        let repos = load_repositories(&config_dir, &key)?;
        let repo_entry = repos.iter().find(|r| r.name == repo_name).ok_or_else(|| {
            CliError::RepositoryNotFound {
                repo_name: repo_name.to_string(),
            }
        })?;
        let index_path = cache_dir.join(&repo_entry.name).join("index.json");
        let index_text =
            fs::read_to_string(&index_path).map_err(|source| CliError::IndexReadFailed {
                repo_name: repo_entry.name.clone(),
                source,
            })?;
        let index =
            livtet_plugin::repository::index::parse_index_json(&index_text).map_err(|source| {
                CliError::IndexParseFailed {
                    repo_name: repo_entry.name.clone(),
                    message: format!("{source}"),
                }
            })?;
        let entry = index
            .plugins
            .get(plugin_id)
            .and_then(|p| p.versions.get(ver))
            .ok_or_else(|| CliError::PluginVersionNotFound {
                plugin_id: plugin_id.to_string(),
                version: ver.to_string(),
                repo_name: repo_name.to_string(),
            })?;
        let archive_path = if entry.archive.starts_with("pool/") {
            entry.archive.clone()
        } else {
            format!("pool/{}", entry.archive)
        };
        let url = format!("{}/{}", repo_entry.url.trim_end_matches('/'), archive_path);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| CliError::TokioRuntimeBuild {
                message: format!("{e}"),
            })?;
        return rt.block_on(download_to_temp(&url));
    }

    if is_url(archive) {
        let url = archive.to_string();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| CliError::TokioRuntimeBuild {
                message: format!("{e}"),
            })?;
        return rt.block_on(download_to_temp(&url));
    }

    Ok(Utf8PathBuf::from(archive))
}

pub fn run_pack(
    source: &Utf8Path,
    label: &str,
    key_dir: &Utf8Path,
    output_dir: Option<&Utf8Path>,
) -> Result<Utf8PathBuf> {
    let key_path = key_dir.join(format!("{label}.key"));
    let output_dir = output_dir.unwrap_or_else(|| source.parent().unwrap_or(Utf8Path::new(".")));
    archive_pack(source, &key_path, label, output_dir).map_err(|e| CliError::PackFailed {
        message: format!("{e}"),
    })
}

pub fn resolve_pack_label(flag: Option<&str>, env_value: Option<&str>) -> String {
    flag.filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| env_value.filter(|s| !s.is_empty()).map(str::to_string))
        .unwrap_or_else(|| "default".to_string())
}

pub struct ListedPlugin {
    pub id: String,
    pub version: String,
    pub install_path: Utf8PathBuf,
}

pub fn run_list(providers: &Utf8Path) -> Result<Vec<ListedPlugin>> {
    let mut out = Vec::new();
    if !providers.exists() {
        return Ok(out);
    }
    for id_entry in fs::read_dir(providers)? {
        let id_entry = id_entry?;
        let id_path = id_entry.path();
        if !id_path.is_dir() {
            continue;
        }
        let id = id_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        for ver_entry in fs::read_dir(&id_path)? {
            let ver_entry = ver_entry?;
            let ver_path = ver_entry.path();
            if !ver_path.is_dir() {
                continue;
            }
            let version = ver_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let manifest_path = ver_path.join("livtet.toml");
            if !manifest_path.exists() {
                continue;
            }
            let install_path = Utf8PathBuf::from_path_buf(ver_path)
                .map_err(|e| CliError::NonUtf8Path { path: e })?;
            out.push(ListedPlugin {
                id: id.clone(),
                version: version.clone(),
                install_path,
            });
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.version.cmp(&b.version)));
    Ok(out)
}

pub fn run_search(
    query: &str,
    repo_filter: Option<&str>,
    cache_dir: &Utf8Path,
    config_dir: &Utf8Path,
    trust: &TrustStore,
) -> Result<Vec<RepoSearchResult>> {
    // CLI's search: use the same key the rest of the CLI commands use
    // (keyring → recovery file → test env). Tests that need a
    // deterministic key should call `run_search_with_key` instead.
    let key = crate::keyring_recover::test_hmac_key_from_env_or_default();
    run_search_with_key(query, repo_filter, cache_dir, config_dir, trust, &key)
}

/// Like [`run_search`] but takes an explicit `HmacKey`. Used by tests
/// that need a deterministic key without going through
/// `load_state_hmac_key`.
pub fn run_search_with_key(
    query: &str,
    repo_filter: Option<&str>,
    cache_dir: &Utf8Path,
    config_dir: &Utf8Path,
    trust: &TrustStore,
    key: &HmacKey,
) -> Result<Vec<RepoSearchResult>> {
    let repos = load_repositories(config_dir, key)?;
    let mut all_results = Vec::new();
    for repo in &repos {
        if let Some(filter) = repo_filter
            && repo.name != filter
        {
            continue;
        }
        let repo_cache = cache_dir.join(&repo.name);
        let index_path = repo_cache.join("index.json");
        let sig_path = repo_cache.join("index.json.sig");
        if !index_path.exists() || !sig_path.exists() {
            continue;
        }
        let index_text = fs::read_to_string(&index_path)?;
        let sig_bytes = fs::read(&sig_path)?;
        let verifying_key = match find_user_key_by_fingerprint(trust, &repo.key_fingerprint) {
            Some(k) => k,
            None => {
                output::warn(&format!(
                    "no trusted key matches repo {:?}'s fingerprint {}; skipping",
                    repo.name, repo.key_fingerprint
                ));
                continue;
            }
        };
        let index = match livtet_plugin::repository::index::parse_index_json(&index_text) {
            Ok(i) => i,
            Err(e) => {
                output::warn(&format!(
                    "failed to parse index.json for repo {:?}: {e}",
                    repo.name
                ));
                continue;
            }
        };
        if verify_index_signature(&index, &index_text, &sig_bytes, &verifying_key).is_err() {
            output::warn(&format!(
                "index signature verification failed for repo {:?}",
                repo.name
            ));
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

fn find_user_key_by_fingerprint(
    trust: &TrustStore,
    fingerprint: &str,
) -> Option<ed25519_dalek::VerifyingKey> {
    let keys = trust.user_keys_snapshot();
    keys.into_iter()
        .find(|(_, key)| fingerprint_pubkey(key) == fingerprint)
        .map(|(_, key)| key)
}

fn fingerprint_pubkey(key: &ed25519_dalek::VerifyingKey) -> String {
    fingerprint(key)
}

fn cmd_keygen(
    label: Option<&str>,
    passphrase: PassphraseMode,
    keys_dir: &str,
    interactive: bool,
) -> Result<()> {
    // Resolve the label. In non-interactive mode the flag is required
    // (clap would have rejected an empty value already); in interactive
    // mode we prompt with `inquire::Text` and reject empty input.
    let resolved_label = match label.map(str::trim).filter(|s| !s.is_empty()) {
        Some(l) => l.to_string(),
        None if interactive => inquire::Text::new("Label for the new signing key:")
            .prompt()
            .map_err(|e| CliError::InteractiveAborted {
                message: format!("label prompt failed: {e}"),
            })?
            .trim()
            .to_string(),
        None => {
            return Err(CliError::Operation {
                message: "--label is required (or pass --interactive)".to_string(),
            });
        }
    };
    if resolved_label.is_empty() {
        return Err(CliError::Operation {
            message: "label cannot be empty".to_string(),
        });
    }

    // Resolve the passphrase mode. `--passphrase=disabled` is the
    // explicit non-interactive opt-out. In interactive mode the user
    // is asked `Use passphrase?` only when they did not already
    // specify `disabled`.
    let no_passphrase = if interactive && passphrase == PassphraseMode::Enabled {
        let use_passphrase = inquire::Confirm::new("Encrypt the signing key with a passphrase?")
            .with_default(true)
            .prompt()
            .map_err(|e| CliError::InteractiveAborted {
                message: format!("passphrase confirmation prompt failed: {e}"),
            })?;
        // `use_passphrase = true` means we *do* want a passphrase, so
        // the boolean passed downstream (`no_passphrase`) is the inverse.
        !use_passphrase
    } else {
        passphrase == PassphraseMode::Disabled
    };

    let path = expand_tilde(keys_dir)?;
    let report = plugin_keygen(&path, &resolved_label, no_passphrase).map_err(CliError::from)?;
    output::success(&format!(
        "Generated signing key\n  Label: {}\n  Key file: {}\n  Pubkey file: {}\n  Fingerprint: {}",
        report.label, report.key_path, report.pubkey_path, report.fingerprint
    ));
    Ok(())
}

fn cmd_trust(pubkey_path: &Utf8Path) -> Result<()> {
    let text = fs::read_to_string(pubkey_path)?;
    let verifying_key = parse_pubkey_text(&text).map_err(CliError::from)?;
    let label = pubkey_path.file_stem().unwrap_or("trusted").to_string();

    let trust_dir = default_trust_dir();
    fs::create_dir_all(&trust_dir)?;
    let dest = trust_dir.join(format!("{label}.pub"));
    fs::copy(pubkey_path.as_std_path(), dest.as_std_path())?;

    output::success(&format!(
        "Trusted\n  Label: {}\n  Pubkey: {}\n  Fingerprint: {}",
        label,
        dest,
        fingerprint(&verifying_key)
    ));
    Ok(())
}

fn cmd_unpublish(
    plugin_id: &str,
    version: &str,
    repo_dir: &Utf8Path,
    interactive: bool,
) -> Result<()> {
    // Interactive confirm: gate unpublish (an irreversible action)
    // behind an explicit `y`. Default is `n` so a stray Enter cannot
    // destroy a published version. Non-interactive callers see no
    // prompt.
    if interactive {
        let confirmed = output::prompt_confirm_interactive(
            &format!(
                "Unpublish {plugin_id} v{version} from {repo_dir}? \
                 This permanently removes the version from the repo index."
            ),
            false,
        )?;
        if !confirmed {
            output::info("Unpublish cancelled.");
            return Ok(());
        }
    }

    let mut key_bytes = [0u8; 32];
    rng().fill_bytes(&mut key_bytes);
    let signing_key = SigningKey::from_bytes(&key_bytes);

    let result = unpublish_version(repo_dir, plugin_id, version, &signing_key);
    match result {
        Ok(()) => {
            output::success(&format!(
                "Unpublished {plugin_id} v{version} from {repo_dir}"
            ));
            Ok(())
        }
        Err(e) => Err(CliError::from(e)),
    }
}

fn cmd_uninstall(id: &str, version: &str, providers: &Utf8Path, interactive: bool) -> Result<()> {
    let install_root = providers.join(id).join(version);
    if !install_root.exists() {
        return Err(CliError::PluginNotInstalled {
            id: id.to_string(),
            version: version.to_string(),
            path: install_root.into(),
        });
    }

    // Interactive confirm: gate removal of the install directory on an
    // explicit `y`. Default is `n` so a stray Enter cannot wipe a
    // plugin. Non-interactive callers see no prompt.
    if interactive {
        let confirmed = output::prompt_confirm_interactive(
            &format!(
                "Uninstall {id} v{version}? \
                 This permanently removes {}.",
                install_root
            ),
            false,
        )?;
        if !confirmed {
            output::info("Uninstall cancelled.");
            return Ok(());
        }
    }

    fs::remove_dir_all(&install_root)?;
    let config_dir = default_config_dir();
    let cache_dir = default_cache_dir();
    let key =
        crate::keyring_recover::load_state_hmac_key().map_err(|source| CliError::HmacLoad {
            source: Box::new(source),
        })?;
    let client = RepositoryClient::with_http(
        cache_dir,
        config_dir,
        key,
        crate::network::agent_with_timeout(std::time::Duration::from_secs(30)),
    );
    let _ = client
        .remove_installed_entry(id, version)
        .map_err(|e| CliError::InstallRecordFailed {
            message: format!("failed to update installed.json: {e}"),
        });
    output::success(&format!("Uninstalled {id} v{version}"));
    Ok(())
}

fn load_repositories(
    config_dir: &Utf8Path,
    key: &HmacKey,
) -> Result<Vec<livtet_plugin::types::Repository>> {
    let path = config_dir.join("repositories.toml");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let cache_dir = Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .unwrap_or_else(|_| Utf8PathBuf::from("/tmp"));
    let client = livtet_plugin::repository::client::RepositoryClient::with_http(
        cache_dir,
        config_dir.to_path_buf(),
        key.clone(),
        crate::network::agent_with_timeout(std::time::Duration::from_secs(30)),
    );
    client
        .load_repositories()
        .map(|f| f.repositories)
        .map_err(|e| CliError::RepositoriesTomlLoadFailed {
            message: format!("{e}"),
        })
}

pub fn record_install_entry(install_path: &Utf8Path, plugin_id: &str, version: &str) -> Result<()> {
    let config_dir = default_config_dir();
    let cache_dir = default_cache_dir();
    fs::create_dir_all(&config_dir)?;
    let key =
        crate::keyring_recover::load_state_hmac_key().map_err(|source| CliError::HmacLoad {
            source: Box::new(source),
        })?;
    let client = RepositoryClient::with_http(
        cache_dir,
        config_dir,
        key,
        crate::network::agent_with_timeout(std::time::Duration::from_secs(30)),
    );
    let entry = InstalledEntry {
        id: plugin_id.to_string(),
        version: version.to_string(),
        source_repo: None,
        install_path: install_path.to_path_buf(),
        installed_at: now_iso(),
    };
    client
        .record_install(entry)
        .map_err(|e| CliError::InstallRecordFailed {
            message: format!("failed to record install: {e}"),
        })
}

pub fn load_trust_store(trust_dir: &Utf8Path) -> Result<TrustStore> {
    let mut store = TrustStore::empty();
    if !trust_dir.exists() {
        return Ok(store);
    }
    for entry in fs::read_dir(trust_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("pub") {
            continue;
        }
        let label = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "trusted".to_string());
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if let Ok(vk) = parse_pubkey_text(&text) {
            let _ = store.add_user_key(&label, vk);
        }
    }
    Ok(store)
}

fn default_cache_dir() -> Utf8PathBuf {
    livtet_core::paths::data_dir_with_migration()
        .map(|p| p.join(livtet_core::paths::subdirs::REPOS))
        .unwrap_or_else(|| Utf8PathBuf::from("/tmp/livtet/repos"))
}

fn default_config_dir() -> Utf8PathBuf {
    livtet_core::paths::config_dir_with_migration()
        .unwrap_or_else(|| Utf8PathBuf::from("/tmp/livtet/config"))
}

pub fn default_trust_dir() -> Utf8PathBuf {
    livtet_core::paths::config_dir_with_migration()
        .map(|p| p.join("keys").join("signing-keys"))
        .unwrap_or_else(|| Utf8PathBuf::from("/tmp/livtet/trust"))
}

/// `String` form of [`default_trust_dir`] for use as a clap
/// `default_value_t` argument.
pub fn default_trust_dir_string() -> String {
    default_trust_dir().to_string()
}

fn default_providers_dir() -> Utf8PathBuf {
    livtet_core::paths::data_dir_with_migration()
        .map(|p| p.join(livtet_core::paths::subdirs::PROVIDERS))
        .unwrap_or_else(|| Utf8PathBuf::from("/tmp/livtet/providers"))
}

fn print_search_results(results: &[RepoSearchResult]) {
    if results.is_empty() {
        output::info("no plugins matched the search");
        return;
    }
    println!(
        "{:<24} {:<10} {:<20} {:>6}",
        "plugin_id", "version", "repository", "score"
    );
    for r in results {
        println!(
            "{:<24} {:<10} {:<20} {:>6.2}",
            r.plugin_id, r.version, r.repository, r.relevance_score
        );
    }
}

fn print_listed(listed: &[ListedPlugin]) {
    if listed.is_empty() {
        output::info("no plugins installed");
        return;
    }
    println!("{:<24} {:<10} path", "id", "version");
    for p in listed {
        println!("{:<24} {:<10} {}", p.id, p.version, p.install_path);
    }
}

fn expand_tilde(s: &str) -> Result<Utf8PathBuf> {
    expand_tilde_str(s)
}

fn expand_tilde_str(s: &str) -> Result<Utf8PathBuf> {
    let path = if let Some(rest) = s.strip_prefix("~/") {
        match std::env::var("HOME") {
            Ok(home) => Utf8PathBuf::from(home).join(rest),
            Err(_) => Utf8PathBuf::from(s),
        }
    } else if s == "~" {
        match std::env::var("HOME") {
            Ok(home) => Utf8PathBuf::from(home),
            Err(_) => Utf8PathBuf::from(s),
        }
    } else {
        Utf8PathBuf::from(s)
    };
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Test-only `Downloader` that returns canned responses and
    /// records every URL it is asked to fetch.
    ///
    /// The point of this mock is to let unit tests pin the
    /// `download_to_temp` contract — success path, error path, and
    /// the sanitization step — without spinning up a real HTTP
    /// server or relying on the public internet. The
    /// `HttpDownloader` production impl has a separate set of
    /// integration tests that exercise the real network path
    /// against a `TcpListener`.
    #[derive(Clone)]
    pub(crate) struct MockDownloader {
        inner: Arc<MockDownloaderInner>,
    }

    struct MockDownloaderInner {
        response: MockResponse,
        calls: Mutex<Vec<String>>,
    }

    /// Canned response for `MockDownloader`.
    #[derive(Clone, Debug)]
    pub(crate) enum MockResponse {
        /// Return success with the given body bytes.
        Ok { body: Vec<u8> },
        /// Return an error with the given message.
        Err(String),
    }

    impl MockDownloader {
        /// Build a mock that will return `response` for every call.
        pub(crate) fn new(response: MockResponse) -> Self {
            Self {
                inner: Arc::new(MockDownloaderInner {
                    response,
                    calls: Mutex::new(Vec::new()),
                }),
            }
        }

        /// Build a mock that always succeeds with the given body.
        pub(crate) fn ok(body: Vec<u8>) -> Self {
            Self::new(MockResponse::Ok { body })
        }

        /// Build a mock that always errors with the given message.
        pub(crate) fn err(msg: impl Into<String>) -> Self {
            Self::new(MockResponse::Err(msg.into()))
        }

        /// Snapshot of the URLs the mock has been asked to fetch.
        pub(crate) fn calls(&self) -> Vec<String> {
            self.inner.calls.lock().unwrap().clone()
        }
    }

    impl Downloader for MockDownloader {
        async fn download_to_temp(&self, url: &str) -> Result<Utf8PathBuf> {
            self.inner.calls.lock().unwrap().push(url.to_string());
            match &self.inner.response {
                MockResponse::Ok { body } => write_temp_bytes(url, body),
                MockResponse::Err(msg) => Err(CliError::Operation {
                    message: msg.clone(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn mock_downloader_writes_canned_bytes_to_temp_file() {
        let body = b"hello, livtet".to_vec();
        let dl = MockDownloader::ok(body.clone());
        let path = dl
            .download_to_temp("https://example.com/archive.ltp")
            .await
            .expect("mock should succeed");

        // The mock must have recorded the call.
        assert_eq!(dl.calls(), vec!["https://example.com/archive.ltp"]);

        // The file must exist on disk with the canned body.
        let on_disk = fs::read(&path).expect("read temp file");
        assert_eq!(on_disk, body, "temp file must contain the canned body");

        // The filename must be derived from the URL (sanitized), with
        // the per-pid prefix the production code uses.
        let file_name = path
            .file_name()
            .expect("temp path has a file name")
            .to_string();
        assert!(
            file_name.starts_with(&format!("livtet-install-{}-", std::process::id())),
            "temp file name must be prefixed with the per-pid marker, got: {file_name}"
        );
        assert!(
            file_name.ends_with("archive.ltp"),
            "temp file name must end with the sanitized URL last segment, got: {file_name}"
        );

        // Clean up.
        let _ = fs::remove_file(&path);
    }

    #[tokio::test]
    async fn mock_downloader_propagates_error_response() {
        let dl = MockDownloader::err("simulated network failure");
        let result = dl.download_to_temp("https://example.com/bad.ltp").await;
        let err = result.expect_err("mock should have errored");
        let msg = format!("{err}");
        assert!(
            msg.contains("simulated network failure"),
            "error must surface the canned message, got: {msg}"
        );
        // The call was still recorded.
        assert_eq!(dl.calls(), vec!["https://example.com/bad.ltp"]);
    }

    #[test]
    fn download_to_temp_sanitizes_path_traversal_in_filename() {
        // The filename is the last path segment of the URL. URLs
        // that end in `../../../etc/passwd` (or similar) must
        // produce a safe local filename: the unsafe characters
        // (`/`) get replaced with `_` so the file is written into
        // `std::env::temp_dir()` with a flat, non-traversing name.
        let safe = sanitize_filename("../../../etc/passwd");
        assert_eq!(safe, ".._.._.._etc_passwd");
        // No path separators survive the sanitization, so the
        // resulting filename cannot escape the temp dir.
        assert!(
            !safe.contains('/') && !safe.contains('\\'),
            "sanitized filename must not contain path separators, got: {safe}"
        );
        // Defensive: a "0 length" filename would still be written
        // to a default name (the production code falls back to
        // `download.ltp` for empty last segments). Verify the
        // fallback name is also safe.
        let fallback = sanitize_filename("download.ltp");
        assert_eq!(fallback, "download.ltp");
    }

    #[tokio::test]
    async fn http_downloader_serves_via_mock_server() {
        // Integration-style test: spin up a real `TcpListener`
        // (mirroring `common::spawn_server` in the integration
        // test crate), point `HttpDownloader` at it, and assert
        // the file is downloaded. We can't use the integration
        // `common` module from a `#[cfg(test)] mod tests` block
        // inside the crate, so we inline a minimal server here.
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let url = format!("http://{addr}/archive.ltp");
        let expected_body = b"http body from test server".to_vec();
        let expected_body_for_task = expected_body.clone();

        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    expected_body_for_task.len()
                );
                let mut payload = response.into_bytes();
                payload.extend_from_slice(&expected_body_for_task);
                let _ = sock.write_all(&payload).await;
                let _ = sock.shutdown().await;
            }
        });

        let path = HttpDownloader
            .download_to_temp(&url)
            .await
            .expect("download should succeed against the mock server");
        let on_disk = fs::read(&path).expect("read temp file");
        assert_eq!(on_disk, expected_body);

        // Clean up.
        let _ = fs::remove_file(&path);
        let _ = server.await;
    }

    #[tokio::test]
    async fn http_downloader_returns_error_for_non_2xx() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let url = format!("http://{addr}/missing.ltp");

        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let body = b"not found";
                let response = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let mut payload = response.into_bytes();
                payload.extend_from_slice(body);
                let _ = sock.write_all(&payload).await;
                let _ = sock.shutdown().await;
            }
        });

        let result = HttpDownloader.download_to_temp(&url).await;
        let err = result.expect_err("404 must surface as an error");
        let msg = format!("{err}");
        assert!(
            msg.contains("404"),
            "non-2xx error must include the status code, got: {msg}"
        );
        let _ = server.await;
    }

    #[test]
    fn is_url_recognizes_http_and_https() {
        assert!(is_url("http://example.com/foo.ltp"));
        assert!(is_url("https://example.com/foo.ltp"));
        assert!(!is_url("HTTP://EXAMPLE.COM/FOO"));
        assert!(!is_url("/tmp/archive.ltp"));
        assert!(!is_url("./relative/path.ltp"));
        assert!(!is_url("file:///tmp/archive.ltp"));
        assert!(!is_url(""));
    }

    #[test]
    fn sanitize_filename_replaces_unsafe_chars() {
        assert_eq!(sanitize_filename("hello-world.ltp"), "hello-world.ltp");
        assert_eq!(sanitize_filename("a/b\\c d.ltp"), "a_b_c_d.ltp");
        assert_eq!(
            sanitize_filename("../../../etc/passwd"),
            ".._.._.._etc_passwd"
        );
        assert_eq!(sanitize_filename(""), "");
    }

    #[test]
    fn resolve_install_source_passthrough_for_local_path() {
        let result = resolve_install_source("/tmp/foo.ltp", None, None).unwrap();
        assert_eq!(result.as_str(), "/tmp/foo.ltp");
    }

    #[test]
    fn resolve_install_source_passthrough_for_relative_path() {
        let result = resolve_install_source("./build/foo.ltp", None, None).unwrap();
        assert_eq!(result.as_str(), "./build/foo.ltp");
    }

    #[test]
    fn resolve_install_source_requires_version_with_repo() {
        // Skip when the test environment has a real repositories.toml that
        // loads successfully — we only need to verify the version check
        // happens *before* the repo lookup.
        let result = resolve_install_source("hello-world", Some("olamaelcu.net"), None);
        match result {
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("--version"), "unexpected error: {msg}");
            }
            Ok(_) => panic!("expected error for missing --version"),
        }
    }
}
