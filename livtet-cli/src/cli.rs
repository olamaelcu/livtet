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
