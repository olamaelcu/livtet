pub mod cli;
pub mod error;
pub mod output;

#[cfg(feature = "fake")]
pub mod seed;

use clap::Parser;
pub use error::{CliError, Result};

use crate::cli::Cli;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    cli.command.run()
}
