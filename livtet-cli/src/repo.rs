use camino::{Utf8Path, Utf8PathBuf};
use ed25519_dalek::SigningKey;
use fs_err as fs;
use livtet_plugin::{
    archive::verify::verify,
    keys::{keyfile::keygen as plugin_keygen, signing::load_minisign_signing_key},
    repository::{client::RepositoryClient, hmac::HmacKey, index::parse_index_json, publisher},
    types::{Repository, RepositoryAddResult, RepositoryUpdateResult},
};

use crate::{
    Result,
    cli::{PassphraseMode, RepoArgs, RepoCommand},
    error::CliError,
    keyring_recover::load_state_hmac_key,
    output,
    plugin::load_trust_store,
};

pub fn run(args: RepoArgs) -> Result<()> {
    match args.command {
        RepoCommand::Init {
            repo_dir,
            name,
            url,
            key_fingerprint,
            key_label,
            interactive,
        } => cmd_init(
            &repo_dir,
            name.as_deref(),
            url.as_deref(),
            key_fingerprint.as_deref(),
            key_label.as_deref(),
            interactive,
        ),
        RepoCommand::Add { url } => cmd_add(&url),
        RepoCommand::ConfirmAdd { url } => cmd_confirm_add(&url),
        RepoCommand::Remove { name_or_url } => cmd_remove(&name_or_url),
        RepoCommand::List { json } => cmd_list(json),
        RepoCommand::Update { name_or_url } => cmd_update(&name_or_url),
        RepoCommand::ConfirmUpdate { name_or_url } => cmd_confirm_update(&name_or_url),
        RepoCommand::Keygen { name, passphrase } => cmd_keygen(&name, passphrase),
        RepoCommand::Publish { repo_dir, plugin } => cmd_publish(&repo_dir, &plugin),
        RepoCommand::Sign { repo_dir } => cmd_sign(&repo_dir),
        RepoCommand::Unpublish {
            repo_dir,
            plugin,
            version,
        } => cmd_unpublish(&repo_dir, &plugin, version.as_deref()),
    }
}

fn cmd_init(
    repo_dir: &Utf8Path,
    name: Option<&str>,
    url: Option<&str>,
    key_fingerprint: Option<&str>,
    key_label: Option<&str>,
    interactive: bool,
) -> Result<()> {
    // In interactive mode, prompt for any field the caller did not
    // supply on the command line. In non-interactive mode every
    // required field must be present (clap would normally enforce
    // this, but we accept `Option` to keep one signature for both
    // modes — so we re-validate explicitly here).
    let resolved_name = resolve_required("repository name", name, interactive, |q| {
        inquire::Text::new(q)
            .prompt()
            .map_err(|e| CliError::InteractiveAborted {
                message: format!("{q}: {e}"),
            })
    })?;
    let resolved_url = resolve_required(
        "repository URL (e.g. https://example.com/repo)",
        url,
        interactive,
        |q| {
            inquire::Text::new(q)
                .prompt()
                .map_err(|e| CliError::InteractiveAborted {
                    message: format!("{q}: {e}"),
                })
        },
    )?;
    let resolved_fp = resolve_required(
        "signing key fingerprint (SHA-256 hex)",
        key_fingerprint,
        interactive,
        |q| {
            inquire::Text::new(q)
                .prompt()
                .map_err(|e| CliError::InteractiveAborted {
                    message: format!("{q}: {e}"),
                })
        },
    )?;

    publisher::init_repo(
        repo_dir,
        &resolved_name,
        &resolved_url,
        &resolved_fp,
        key_label,
    )
    .map_err(CliError::from)?;
    let label_msg = match key_label {
        Some(l) => format!("\n  Key label: {l}"),
        None => String::new(),
    };
    output::success(&format!(
        "Initialized repository at {}\n  Name: {}\n  URL: {}\n  Key fingerprint: {}{label_msg}",
        repo_dir, resolved_name, resolved_url, resolved_fp
    ));
    Ok(())
}

/// Resolve a required CLI argument. If the caller supplied a
/// non-empty value it is returned unchanged. Otherwise, when
/// `--interactive` is set, the user is prompted via `inquire::Text`.
/// Without `--interactive`, an empty value is an explicit error.
///
/// `prompt_fn` lets the caller inject the actual `inquire::Text`
/// builder (so we don't take a hard dependency on a specific
/// renderer here and so tests can swap the prompt for a stub).
fn resolve_required<F>(
    label: &str,
    provided: Option<&str>,
    interactive: bool,
    prompt_fn: F,
) -> Result<String>
where
    F: FnOnce(&str) -> Result<String>,
{
    if let Some(v) = provided.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(v.to_string());
    }
    if interactive {
        let value = prompt_fn(&format!("{label}:")).map_err(|e| CliError::InteractiveAborted {
            message: format!("{label} prompt failed: {e}"),
        })?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(CliError::Operation {
                message: format!("{label} cannot be empty"),
            });
        }
        return Ok(trimmed.to_string());
    }
    Err(CliError::Operation {
        message: format!("{label} is required (or pass --interactive)"),
    })
}

fn cmd_add(url: &str) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::TokioRuntimeBuild {
            message: format!("{e}"),
        })?;
    let (client, _) = build_client()?;
    let result = rt.block_on(client.add(url)).map_err(CliError::from)?;
    match result {
        RepositoryAddResult::NeedsTofuConfirmation {
            name,
            url: repo_url,
            fingerprint,
        } => {
            println!("Resolving {url}...");
            println!("Repo name: {name}");
            println!("Signing key fingerprint (SHA256): {fingerprint}");
            println!("To trust this key, run:");
            println!("  livtet plugin trust <path-to-pubkey>");
            println!("  livtet repo confirm-add {repo_url}");
            Ok(())
        }
        RepositoryAddResult::Ok { name, plugin_count } => {
            output::success(&render_repository_add_ok_message(&name, plugin_count));
            Ok(())
        }
    }
}

/// Render the "already trusted" message printed by the
/// `RepositoryAddResult::Ok` branch of `cmd_add`.
///
/// Exposed (not dead) so the contract — the exact wording that
/// ends up in `output::success` — can be pinned by an integration
/// test without depending on a network round-trip to a server
/// whose `client.add` happens to return the `Ok` variant. The
/// production `RepositoryClient::add` always returns
/// `NeedsTofuConfirmation` today, so this branch is currently
/// dead in `cmd_add`; a future client-side shortcut (TOFU-skip on
/// already-trusted) would light it up.
pub fn render_repository_add_ok_message(name: &str, plugin_count: usize) -> String {
    format!("Added {name} (already trusted; {plugin_count} plugins)")
}

fn cmd_confirm_add(url: &str) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::TokioRuntimeBuild {
            message: format!("{e}"),
        })?;
    let (client, _) = build_client()?;
    let trust_dir = crate::plugin::default_trust_dir();
    let trust = load_trust_store(&trust_dir)?;
    let cache_dir = default_repo_cache_dir();
    rt.block_on(client.confirm_add(url, &trust))
        .map_err(CliError::from)?;

    let repo_name = client
        .load_repositories()
        .ok()
        .and_then(|f| {
            f.repositories
                .iter()
                .find(|r| r.url == url)
                .map(|r| r.name.clone())
        })
        .unwrap_or_default();
    let plugin_count = if repo_name.is_empty() {
        0
    } else {
        let index_path = cache_dir.join(&repo_name).join("index.json");
        match fs::read_to_string(index_path.as_std_path()) {
            Ok(text) => parse_index_json(&text)
                .map(|index| index.plugins.values().map(|p| p.versions.len()).sum())
                .unwrap_or(0),
            Err(_) => 0,
        }
    };
    output::success(&format!(
        "Added. Fetched index.json: {plugin_count} plugin versions."
    ));
    Ok(())
}

fn cmd_remove(name_or_url: &str) -> Result<()> {
    let (client, _) = build_client()?;
    client.remove(name_or_url).map_err(CliError::from)?;
    output::success(&format!("Removed {name_or_url}"));
    Ok(())
}

fn cmd_list(json: bool) -> Result<()> {
    let (client, _) = build_client()?;
    let repos = client.list().map_err(CliError::from)?;
    if json {
        let payload = serde_json::to_string_pretty(&repos)?;
        println!("{payload}");
    } else {
        for repo in &repos {
            print_repo_row(repo);
        }
        if repos.is_empty() {
            output::info("No repositories configured.");
        }
    }
    Ok(())
}

fn print_repo_row(repo: &Repository) {
    println!("{}\t{}\t{}", repo.name, repo.url, repo.key_fingerprint);
}

fn cmd_update(name_or_url: &str) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::TokioRuntimeBuild {
            message: format!("{e}"),
        })?;
    let (client, _) = build_client()?;
    let trust_dir = crate::plugin::default_trust_dir();
    let trust = load_trust_store(&trust_dir)?;
    let result = rt
        .block_on(client.update(name_or_url, &trust))
        .map_err(CliError::from)?;
    match result {
        RepositoryUpdateResult::Ok { plugin_count } => {
            output::success(&format!(
                "Updated. Fetched index.json: {plugin_count} plugin versions."
            ));
            Ok(())
        }
        RepositoryUpdateResult::KeyChanged {
            name,
            old_fingerprint,
            new_fingerprint,
        } => {
            output::error(&format!(
                "Signing key changed for {name} (TOFU). Old: {old_fingerprint}; new: {new_fingerprint}."
            ));
            println!("To trust the new key:");
            println!("  livtet plugin trust <path-to-pubkey>");
            println!("Then re-run:");
            println!("  livtet repo confirm-update {name}");
            // Non-zero exit so callers (CI, scripts) can detect the rollover.
            Err(CliError::SigningKeyChanged)
        }
    }
}

fn cmd_confirm_update(name_or_url: &str) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::TokioRuntimeBuild {
            message: format!("{e}"),
        })?;
    let (client, _) = build_client()?;
    let trust_dir = crate::plugin::default_trust_dir();
    let trust = load_trust_store(&trust_dir)?;
    rt.block_on(client.confirm_update(name_or_url, &trust))
        .map_err(CliError::from)?;
    output::success(&format!(
        "Confirmed update for {name_or_url}; cached repo.toml, index.json and index.json.sig."
    ));
    Ok(())
}

fn cmd_keygen(name: &str, passphrase: PassphraseMode) -> Result<()> {
    let no_passphrase = matches!(passphrase, PassphraseMode::Disabled);
    let base_dir = default_repo_keys_dir();
    fs::create_dir_all(&base_dir)?;
    let report = plugin_keygen(&base_dir, name, no_passphrase).map_err(CliError::from)?;
    output::success(&format!(
        "Created {} (encrypted: {})",
        report.key_path, report.encrypted
    ));
    output::success(&format!("Created {}", report.pubkey_path));
    output::success(&format!("Fingerprint: {}", report.fingerprint));
    Ok(())
}

fn cmd_publish(repo_dir: &Utf8Path, plugin: &Utf8Path) -> Result<()> {
    let trust_dir = crate::plugin::default_trust_dir();
    let trust = load_trust_store(&trust_dir)?;
    let report = verify(plugin, Some(&trust)).map_err(CliError::from)?;
    if !report.valid {
        return Err(CliError::ArchiveVerificationFailed {
            errors: report.errors,
        });
    }
    let id = report
        .plugin_id
        .clone()
        .ok_or(CliError::VerifyReportMissingPluginId)?;
    let version = report
        .version
        .clone()
        .ok_or(CliError::VerifyReportMissingVersion)?;

    let key_path = default_repo_keys_dir().join("repo.key");
    let signing_key = load_repo_signing_key(&key_path)?;

    publisher::publish_archive(
        repo_dir,
        plugin,
        &id,
        &version,
        "init.lua",
        "0.5.0",
        &signing_key,
    )
    .map_err(CliError::from)?;
    output::success(&format!(
        "Packed {id}-{version}.ltp to pool/, appended to index.json, re-signed"
    ));
    Ok(())
}

fn cmd_sign(repo_dir: &Utf8Path) -> Result<()> {
    let key_path = default_repo_keys_dir().join("repo.key");
    let signing_key = load_repo_signing_key(&key_path)?;
    publisher::sign_index(repo_dir, &signing_key).map_err(CliError::from)?;
    output::success(&format!("Signed {repo_dir}/index.json"));
    Ok(())
}

fn cmd_unpublish(repo_dir: &Utf8Path, plugin: &str, version: Option<&str>) -> Result<()> {
    let key_path = default_repo_keys_dir().join("repo.key");
    let signing_key = load_repo_signing_key(&key_path)?;
    match version {
        Some(ver) => {
            publisher::unpublish_version(repo_dir, plugin, ver, &signing_key)
                .map_err(CliError::from)?;
            output::success(&format!("Unpublished {plugin} v{ver}"));
        }
        None => {
            let index_path = repo_dir.join("index.json");
            let text = fs::read_to_string(&index_path)?;
            let index = parse_index_json(&text).map_err(CliError::from)?;
            if let Some(p) = index.plugins.get(plugin) {
                let versions: Vec<String> = p.versions.keys().cloned().collect();
                for v in versions {
                    publisher::unpublish_version(repo_dir, plugin, &v, &signing_key)
                        .map_err(CliError::from)?;
                    output::success(&format!("Unpublished {plugin} v{v}"));
                }
            } else {
                output::info(&format!("No versions found for {plugin}"));
            }
        }
    }
    Ok(())
}

fn load_repo_signing_key(key_path: &Utf8Path) -> Result<SigningKey> {
    let (_, signing_key) = load_minisign_signing_key(key_path).map_err(CliError::from)?;
    Ok(signing_key)
}

fn build_client() -> Result<(RepositoryClient, HmacKey)> {
    let cache_dir = default_repo_cache_dir();
    let config_dir = default_config_dir();
    let key = load_state_hmac_key().map_err(|source| CliError::HmacLoad {
        source: Box::new(source),
    })?;
    let client = RepositoryClient::with_http(
        cache_dir,
        config_dir,
        key.clone(),
        crate::network::agent_with_timeout(std::time::Duration::from_secs(30)),
    );
    Ok((client, key))
}

pub fn default_config_dir() -> Utf8PathBuf {
    livtet_core::paths::config_dir_with_migration().unwrap_or_else(fallback_config_dir)
}

fn default_repo_cache_dir() -> Utf8PathBuf {
    livtet_core::paths::data_dir_with_migration()
        .map(|p| p.join(livtet_core::paths::subdirs::REPOS))
        .unwrap_or_else(|| fallback_data_dir().join(livtet_core::paths::subdirs::REPOS))
}

fn fallback_config_dir() -> Utf8PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .and_then(into_utf8_path)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").and_then(into_utf8_string);
            let mut dir = Utf8PathBuf::from(".config");
            if let Some(home) = home {
                dir = Utf8PathBuf::from(home).join(".config");
            }
            dir
        });
    base.join(livtet_core::paths::BUNDLE_ID)
}

fn fallback_data_dir() -> Utf8PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .and_then(into_utf8_path)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").and_then(into_utf8_string);
            let mut dir = Utf8PathBuf::from(".local/share");
            if let Some(home) = home {
                dir = Utf8PathBuf::from(home).join(".local").join("share");
            }
            dir
        });
    base.join(livtet_core::paths::BUNDLE_ID)
}

fn into_utf8_string(s: std::ffi::OsString) -> Option<String> {
    s.into_string().ok()
}

fn into_utf8_path(s: std::ffi::OsString) -> Option<Utf8PathBuf> {
    into_utf8_string(s).map(Utf8PathBuf::from)
}

fn default_repo_keys_dir() -> Utf8PathBuf {
    default_config_dir().join("keys").join("repo-keys")
}
