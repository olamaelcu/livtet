pub mod cli;
pub mod error;
pub mod keyring_recover;
pub mod network;
pub mod output;

pub mod plugin;
pub mod repo;
#[cfg(feature = "fake")]
pub mod seed;

use clap::Parser;
pub use error::{CliError, Result};

use crate::cli::Cli;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    cli.command.run()
}
