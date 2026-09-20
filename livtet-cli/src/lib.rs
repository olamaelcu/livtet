//! `livtet-cli` — thin front-end over [`livtet_cli_common`].
//!
//! All command definitions and implementations live in
//! `livtet-cli-common` so they can be shared with other livtet
//! binaries (e.g. `livtet-tui`).

pub use livtet_cli_common::{CliError, Result};

pub fn run() -> Result<()> {
    use clap::Parser;
    livtet_cli_common::Cli::parse().command.run()
}
