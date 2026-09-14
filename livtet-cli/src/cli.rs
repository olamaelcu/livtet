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
    /// Populate the local database with realistic demo data.
    /// Only available in builds with the `fake` feature enabled.
    #[cfg(feature = "fake")]
    Seed(crate::seed::SeedArgs),
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
            #[cfg(feature = "fake")]
            Command::Seed(args) => {
                let rt =
                    tokio::runtime::Runtime::new().map_err(|e| crate::CliError::Operation {
                        message: format!("tokio runtime: {e}"),
                    })?;
                rt.block_on(args.run())
            }
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
