//! `livtet-cli` — thin front-end over [`livtet_cli_common`].
//!
//! All command definitions and implementations live in
//! `livtet-cli-common` so they can be shared with other livtet
//! binaries (e.g. `livtet-tui`).

pub use livtet_cli_common::{Cli, CliError, Context, Result};

pub fn run() -> Result<()> {
    use clap::Parser;
    let cli = Cli::parse();
    let ctx = Context::from_defaults(cli.profile.as_deref())?;
    cli.command.run_with(&ctx)
}
