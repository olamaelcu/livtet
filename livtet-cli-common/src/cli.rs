use clap::{Args, Parser, Subcommand};

use crate::Result;
use crate::context::Context;

#[derive(Parser, Debug)]
#[command(name = "livtet", about = "Livtet CLI", version)]
pub struct Cli {
    /// Profile identifier for scoped database paths.
    /// When set, database and search index live under {data_dir}/{profile_id}/.
    #[arg(long, short = 'p', value_name = "PROFILE", global = true)]
    pub profile: Option<String>,

    #[command(subcommand, next_help_heading = "Commands")]
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
    /// Mutation commands for editions.
    Edition(crate::edition::EditionArgs),
}

#[derive(Args, Debug)]
pub struct PathArgs {
    /// Which path to print. Defaults to all of them.
    #[arg(value_name = "KIND", default_value = "all")]
    pub kind: String,
}

impl Command {
    /// Run without a [`Context`]. Only [`Command::Path`] is supported; every
    /// other variant needs a resolved context and returns an error.
    pub fn run(self) -> Result<()> {
        match self {
            Command::Path(args) => args.run(),
            #[cfg(feature = "fake")]
            Command::Seed(_) => Err(crate::CliError::Operation {
                message: "seed requires a context".to_string(),
            }),
            Command::Reindex(_) | Command::Editions(_) | Command::Edition(_) => {
                Err(crate::CliError::Operation {
                    message: "this command requires a context".to_string(),
                })
            }
        }
    }


    /// Run against an explicit [`Context`] rather than the platform defaults.
    pub fn run_with(self, ctx: &Context) -> Result<()> {
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
                rt.block_on(args.run(ctx))
            }
            Command::Path(args) => args.run(),
            Command::Editions(args) => {
                let rt =
                    tokio::runtime::Runtime::new().map_err(|e| crate::CliError::Operation {
                        message: format!("tokio runtime: {e}"),
                    })?;
                rt.block_on(args.run(ctx))
            }
            Command::Edition(args) => {
                let rt =
                    tokio::runtime::Runtime::new().map_err(|e| crate::CliError::Operation {
                        message: format!("tokio runtime: {e}"),
                    })?;
                rt.block_on(args.run(ctx))
            }
        }
    }
}

#[derive(Debug, tabled::Tabled)]
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

#[cfg(test)]
mod tests {
    use super::{Command, Context, PathArgs};

    fn rows_for(kind: &str) -> Vec<(&'static str, String)> {
        PathArgs {
            kind: kind.to_string(),
        }
        .rows()
        .expect("known kind")
        .into_iter()
        .map(|row| (row.kind, row.path))
        .collect()
    }

    #[test]
    fn each_kind_selects_its_own_row() {
        assert_eq!(rows_for("bundle").len(), 1);
        assert_eq!(rows_for("bundle")[0].0, "bundle");
        assert_eq!(rows_for("logs").len(), 1);
        assert_eq!(rows_for("logs")[0].0, "logs");
        // data and config come from the platform and may be unresolvable, so
        // they yield at most one row rather than always one.
        assert!(rows_for("data").len() <= 1);
        assert!(rows_for("config").len() <= 1);
    }

    #[test]
    fn all_is_the_union_of_the_individual_kinds() {
        let all: Vec<&str> = rows_for("all").iter().map(|(kind, _)| *kind).collect();
        assert_eq!(all.first(), Some(&"bundle"), "bundle leads the listing");
        assert!(all.contains(&"logs"));
        for kind in ["data", "config"] {
            assert_eq!(
                all.contains(&kind),
                !rows_for(kind).is_empty(),
                "`all` must list {kind} exactly when asking for it alone does"
            );
        }
        assert_eq!(
            all.len(),
            1 + rows_for("data").len() + rows_for("config").len() + 1
        );
    }

    #[test]
    fn kind_matching_ignores_case() {
        assert_eq!(rows_for("BUNDLE"), rows_for("bundle"));
        assert_eq!(rows_for("All").len(), rows_for("all").len());
    }

    #[test]
    fn every_reported_path_is_non_empty() {
        for (kind, path) in rows_for("all") {
            assert!(!path.trim().is_empty(), "{kind} reported a blank path");
        }
    }

    #[test]
    fn an_unknown_kind_is_an_error_that_lists_the_valid_ones() {
        let err = PathArgs {
            kind: "elsewhere".to_string(),
        }
        .rows()
        .expect_err("unknown kind must not resolve");
        let message = err.to_string();
        for expected in ["elsewhere", "all", "bundle", "data", "config", "logs"] {
            assert!(
                message.contains(expected),
                "error should mention {expected:?}: {message}"
            );
        }
    }

    /// `Path` is the one command that reads no catalog, so `Command::run_with`
    /// must dispatch it without first resolving a `Context` -- otherwise it
    /// would fail on exactly the machines where it is most useful.
    #[test]
    fn path_command_runs_without_resolving_a_context() {
        let ctx = Context::in_dir("/tmp/livtet-test");
        Command::Path(PathArgs {
            kind: "bundle".to_string(),
        })
        .run_with(&ctx)
        .expect("path must not need a resolvable data directory");
    }
}
