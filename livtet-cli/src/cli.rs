use clap::{Args, Parser, Subcommand};

use crate::Result;

#[derive(Parser, Debug)]
#[command(name = "livtet", about = "Livtet CLI", version)]
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
    /// Rebuild the Tantivy search index from the local SQLite database.
    /// No-op when the on-disk schema is current unless `--force`.
    /// Destructive by design — pair with `--yes` and custom `--database`
    /// / `--index-dir` to target a non-production database safely.
    Reindex(crate::reindex::ReindexArgs),
    /// Print canonical app-data directories for the current platform.
    /// Uses the same resolution as every other livtet binary (Tauri
    /// parent, plugin host). Useful in shell scripts and for
    /// debugging path-related issues.
    Path(PathArgs),
    /// Query editions (index-backed list/search, batched get).
    Editions(crate::editions::EditionsArgs),
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
            Command::Reindex(args) => {
                let rt =
                    tokio::runtime::Runtime::new().map_err(|e| crate::CliError::Operation {
                        message: format!("tokio runtime: {e}"),
                    })?;
                rt.block_on(args.run())
            }
            Command::Path(args) => args.run(),
            Command::Editions(args) => {
                let rt =
                    tokio::runtime::Runtime::new().map_err(|e| crate::CliError::Operation {
                        message: format!("tokio runtime: {e}"),
                    })?;
                rt.block_on(args.run())
            }
        }
    }
}

#[derive(tabled::Tabled)]
struct PathRow {
    #[tabled(rename = "KIND")]
    kind: &'static str,
    #[tabled(rename = "PATH")]
    path: String,
}

impl PathArgs {
    pub fn run(self) -> Result<()> {
        use tabled::settings::Style;
        let rows = self.rows()?;
        if !rows.is_empty() {
            println!("{}", tabled::Table::new(rows).with(Style::modern()));
        }
        Ok(())
    }

    fn rows(self) -> Result<Vec<PathRow>> {
        use livtet_core::paths;
        let kind = self.kind.to_ascii_lowercase();
        let row = |kind: &'static str, path: String| PathRow { kind, path };
        match kind.as_str() {
            "all" => {
                let mut rows = vec![row("bundle", paths::BUNDLE_ID.to_string())];
                if let Some(d) = paths::data_dir() {
                    rows.push(row("data", d.to_string()));
                }
                if let Some(c) = paths::config_dir() {
                    rows.push(row("config", c.to_string()));
                }
                rows.push(row("logs", paths::logs_dir().to_string()));
                Ok(rows)
            }
            "bundle" => Ok(vec![row("bundle", paths::BUNDLE_ID.to_string())]),
            "data" => Ok(paths::data_dir()
                .map(|d| row("data", d.to_string()))
                .into_iter()
                .collect()),
            "config" => Ok(paths::config_dir()
                .map(|c| row("config", c.to_string()))
                .into_iter()
                .collect()),
            "logs" => Ok(vec![row("logs", paths::logs_dir().to_string())]),
            _ => Err(crate::CliError::Operation {
                message: format!(
                    "unknown path kind `{kind}`; expected one of: \
                     all, bundle, data, config, logs"
                ),
            }),
        }
    }
}
