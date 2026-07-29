use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};

use crate::Result;

/// How to handle passphrase prompting when generating a new key.
///
/// `Enabled` is the default — the CLI prompts for a passphrase
/// interactively. `Disabled` skips passphrase protection entirely.
/// This enum leaves room for future modes like `FromStdin` or
/// `FromEnv` without breaking the CLI flag.
#[derive(clap::ValueEnum, Clone, Debug, Default, PartialEq, Eq)]
pub enum PassphraseMode {
    #[default]
    Enabled,
    Disabled,
}

#[derive(Parser, Debug)]
#[command(name = "livtet", about = "Livtet plugin manager CLI", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Plugin(PluginArgs),
    Repo(RepoArgs),
    /// Populate the local database with realistic demo data.
    /// Only available in builds with the `fake` feature enabled.
    #[cfg(feature = "fake")]
    Seed(crate::seed::SeedArgs),
    /// Recover access to HMAC-protected state files when the OS keyring
    /// is unavailable (e.g., reinstalled OS, fresh user account, headless
    /// container). Derives a deterministic 32-byte key from a passphrase
    /// using PBKDF2-HMAC-SHA256 and writes the result to a 0600
    /// `passphrase.env` under the livtet config dir so subsequent
    /// `livtet` invocations can read the state files.
    KeyringRecover(KeyringRecoverArgs),
    /// Print canonical app-data directories for the current platform.
    /// Uses the same resolution as every other livtet binary (Tauri
    /// parent, plugin host). Useful in shell scripts and for
    /// debugging path-related issues.
    Path(PathArgs),
}

#[derive(Args, Debug)]
pub struct PathArgs {
    /// Which path to print. Defaults to all of them.
    #[arg(value_name = "KIND", default_value = "all")]
    pub kind: String,
}

impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Command::Plugin(args) => crate::plugin::run(args),
            Command::Repo(args) => crate::repo::run(args),
            #[cfg(feature = "fake")]
            Command::Seed(args) => {
                let rt =
                    tokio::runtime::Runtime::new().map_err(|e| crate::CliError::Operation {
                        message: format!("tokio runtime: {e}"),
                    })?;
                rt.block_on(args.run())
            }
            Command::KeyringRecover(args) => crate::keyring_recover::run(args),
            Command::Path(args) => run_path(args),
        }
    }
}

fn run_path(args: PathArgs) -> Result<()> {
    use livtet_core::paths;
    let kind = args.kind.to_ascii_lowercase();
    let print = |label: &str, p: &str| println!("{label:<8} {p}");
    match kind.as_str() {
        "all" => {
            print("bundle", livtet_core::paths::BUNDLE_ID);
            if let Some(d) = paths::data_dir() {
                print("data", d.as_str());
            }
            if let Some(c) = paths::config_dir() {
                print("config", c.as_str());
            }
            print("logs", paths::logs_dir().as_str());
            print("repos", paths::subdirs::REPOS);
            print("providers", paths::subdirs::PROVIDERS);
            print("permissions", paths::subdirs::PERMISSIONS);
        }
        "bundle" => println!("{}", livtet_core::paths::BUNDLE_ID),
        "data" => {
            if let Some(d) = paths::data_dir() {
                println!("{}", d);
            }
        }
        "config" => {
            if let Some(c) = paths::config_dir() {
                println!("{}", c);
            }
        }
        "logs" => println!("{}", paths::logs_dir()),
        _ => {
            return Err(crate::CliError::Operation {
                message: format!(
                    "unknown path kind `{kind}`; expected one of: \
                     all, bundle, data, config, logs"
                ),
            });
        }
    }
    Ok(())
}

#[derive(Args, Debug)]
pub struct KeyringRecoverArgs {
    /// Read the passphrase from stdin (one line, no echo). Useful for
    /// scripting: `echo "$LIVTET_RECOVERY_PASSPHRASE" | livtet
    /// keyring-recover --passphrase-stdin`.
    #[arg(long, conflicts_with = "passphrase")]
    pub passphrase_stdin: bool,

    /// Passphrase provided on the command line. Avoid in shell history;
    /// prefer `--passphrase-stdin` for any non-interactive flow.
    #[arg(long, conflicts_with = "passphrase_stdin")]
    pub passphrase: Option<String>,

    /// Where to write the derived key. Defaults to
    /// `<config-dir>/passphrase.env` (mode 0600). The file is checked
    /// in to nothing and is consumed by `load_recovery_key` on the
    /// next `livtet` invocation.
    #[arg(long)]
    pub output: Option<camino::Utf8PathBuf>,

    /// Interactive mode: prompt for passphrase if not provided via
    /// --passphrase or --passphrase-stdin.
    #[arg(long, conflicts_with = "passphrase_stdin")]
    pub interactive: bool,
}

#[derive(Args, Debug)]
pub struct PluginArgs {
    #[command(subcommand)]
    pub command: PluginCommand,
}

#[derive(Subcommand, Debug)]
pub enum PluginCommand {
    Keygen {
        /// Human-readable label for the signing key (e.g. `olamaelcu`).
        /// Optional when `--interactive` is set; the value is then
        /// prompted for with `inquire::Text`.
        #[arg(long)]
        label: Option<String>,

        /// How to handle passphrase prompting when generating the key.
        /// `enabled` (default) prompts for a passphrase interactively;
        /// `disabled` stores the key unencrypted. In `--interactive`
        /// mode the user is asked `Use passphrase?`.
        #[arg(long, value_enum, default_value_t = PassphraseMode::default())]
        passphrase: PassphraseMode,

        #[arg(long, default_value_t = crate::plugin::default_trust_dir_string())]
        keys_dir: String,

        /// Interactive mode: prompt for missing fields (label,
        /// passphrase decision) with `inquire`. Non-interactive
        /// callers see no behavioral change.
        #[arg(long)]
        interactive: bool,
    },
    Trust {
        pubkey_path: Utf8PathBuf,
    },
    Search {
        query: String,
        #[arg(long)]
        repo: Option<String>,
    },
    Install {
        archive: String,
        #[arg(long)]
        providers: Option<String>,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
    Uninstall {
        id: String,
        version: String,
        #[arg(long)]
        providers: Option<String>,
        /// Interactive mode: confirm with `inquire::Confirm` before
        /// removing the plugin directory. Non-interactive callers
        /// see no behavioral change.
        #[arg(long)]
        interactive: bool,
    },
    List {
        #[arg(long)]
        providers: Option<String>,
    },
    Unpublish {
        #[arg(long)]
        plugin_id: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        repo_dir: Utf8PathBuf,
        /// Interactive mode: confirm with `inquire::Confirm` before
        /// removing the version from the repo. Non-interactive
        /// callers see no behavioral change.
        #[arg(long)]
        interactive: bool,
    },
    Pack {
        source: Utf8PathBuf,
        #[arg(long, default_value = "olamaelcu")]
        label: String,
        #[arg(long)]
        key: Option<String>,
        #[arg(long, default_value_t = crate::plugin::default_trust_dir_string())]
        key_dir: String,
        #[arg(long)]
        output: Option<Utf8PathBuf>,
    },
}

#[derive(Args, Debug)]
pub struct RepoArgs {
    #[command(subcommand)]
    pub command: RepoCommand,
}

#[derive(Subcommand, Debug)]
pub enum RepoCommand {
    Init {
        /// Directory the new repository will be created in.
        #[arg(long)]
        repo_dir: Utf8PathBuf,
        /// Logical name for the repository (e.g. `olamaelcu`).
        /// Optional when `--interactive` is set.
        #[arg(long)]
        name: Option<String>,
        /// Base URL for the repository.
        /// Optional when `--interactive` is set.
        #[arg(long)]
        url: Option<String>,
        /// SHA-256 fingerprint of the repository's signing key.
        /// Optional when `--interactive` is set.
        #[arg(long)]
        key_fingerprint: Option<String>,
        /// Optional label of the local signing key pair.
        #[arg(long)]
        key_label: Option<String>,
        /// Interactive mode: prompt for missing fields with
        /// `inquire`. Non-interactive callers see no behavioral
        /// change.
        #[arg(long)]
        interactive: bool,
    },
    Add {
        #[arg(long)]
        url: String,
    },
    ConfirmAdd {
        #[arg(long)]
        url: String,
    },
    Remove {
        #[arg(long)]
        name_or_url: String,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    Update {
        #[arg(long)]
        name_or_url: String,
    },
    ConfirmUpdate {
        #[arg(long)]
        name_or_url: String,
    },
    Keygen {
        #[arg(long)]
        name: String,
        /// How to handle passphrase prompting when generating the key.
        /// `enabled` (default) prompts for a passphrase interactively;
        /// `disabled` stores the key unencrypted.
        #[arg(long, value_enum, default_value_t = PassphraseMode::default())]
        passphrase: PassphraseMode,
    },
    Publish {
        #[arg(long)]
        repo_dir: Utf8PathBuf,
        #[arg(long)]
        plugin: Utf8PathBuf,
    },
    Sign {
        #[arg(long)]
        repo_dir: Utf8PathBuf,
    },
    Unpublish {
        #[arg(long)]
        repo_dir: Utf8PathBuf,
        #[arg(long)]
        plugin: String,
        #[arg(long)]
        version: Option<String>,
    },
}
