//! Shared building blocks for livtet binary front-ends (`livtet-cli`,
//! and future binaries such as `livtet-tui`).
//!
//! This crate owns the pieces that are not specific to any one
//! front-end: the clap command tree, the query/mutation command
//! implementations, error types, and default path resolution.

pub mod cli;
pub mod edition;
pub mod editions;
pub mod error;
pub mod path;
pub mod reindex;

#[cfg(feature = "fake")]
pub mod seed;

pub use cli::Cli;
pub use error::{CliError, Result};
pub use path::{default_db_path, default_index_dir};
pub use reindex::ReindexArgs;

#[cfg(feature = "fake")]
pub use seed::SeedArgs;
