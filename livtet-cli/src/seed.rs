//! `livtet seed` — populate the local database with realistic demo data.
//!
//! Only available when `livtet-core` was built with the `fake` feature
//! (which is on for debug builds via this CLI's `fake` feature).

#![cfg(feature = "fake")]

use camino::Utf8PathBuf;
use clap::Parser;

use crate::Result;

#[derive(Parser, Debug)]
pub struct SeedArgs {
    /// Number of works to generate. Editions will be 1-3 per work.
    #[arg(long, default_value = "30")]
    pub works: u32,

    /// Path to the SQLite database file. Defaults to the platform's
    /// livtet data directory.
    #[arg(long)]
    pub database: Option<Utf8PathBuf>,

    /// Skip the confirmation prompt before mutating the database.
    #[arg(long)]
    pub yes: bool,
}

impl SeedArgs {
    pub async fn run(&self) -> Result<()> {
        let db_path = match &self.database {
            Some(p) => p.clone(),
            None => default_db_path()?,
        };

        if !self.yes {
            let confirmed = inquire::Confirm::new(&format!(
                "This will populate the database at {} with {} works of test data. \
                 Existing rows may be duplicated. Continue?",
                db_path, self.works
            ))
            .with_default(false)
            .prompt()
            .map_err(|e| crate::CliError::InteractiveAborted {
                message: e.to_string(),
            })?;
            if !confirmed {
                return Err(crate::CliError::Operation {
                    message: "Aborted by user".to_string(),
                });
            }
        }

        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        let sea_conn = livtet_data::orm::Database::connect(&db_url)
            .await
            .map_err(|e| crate::CliError::Operation {
                message: format!("Failed to connect to {db_url}: {e}"),
            })?;

        let config = livtet_core::seed::SeedConfig {
            num_works: self.works,
            ..Default::default()
        };

        let result = livtet_core::seed::seed_database(&sea_conn, &config)
            .await
            .map_err(|e| crate::CliError::Operation {
                message: format!("Seed failed: {e}"),
            })?;

        println!("Seeded database at {db_path}:");
        println!("  Works: {}", result.works_created);
        println!("  Editions: {}", result.editions_created);
        println!("  Authors: {}", result.authors_created);
        println!("  Publishers: {}", result.publishers_created);
        println!("  Reading status entries: {}", result.reading_status_count);
        println!("  Annotations: {}", result.annotations_created);
        println!("  Digital inventory: {}", result.digital_inventory_created);
        println!("  Loans: {}", result.loans_created);
        println!("  Reading sessions: {}", result.reading_sessions_created);
        println!("  Saved searches: {}", result.saved_searches_created);
        println!("  Reading lists: {}", result.reading_lists_created);

        Ok(())
    }
}

fn default_db_path() -> Result<Utf8PathBuf> {
    use livtet_core::paths;
    let dir = paths::data_dir().ok_or_else(|| crate::CliError::Operation {
        message: "Could not resolve the livtet data directory".to_string(),
    })?;
    Ok(dir.join("livtet.db"))
}
