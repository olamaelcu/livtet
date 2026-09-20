//! `livtet seed` — populate the local database with realistic demo data.
//!
//! Only available when `livtet-core` was built with the `fake` feature
//! (which is on for debug builds via this CLI's `fake` feature).

use camino::Utf8PathBuf;
use clap::Parser;
use livtet_data::migration::{Migrator, MigratorTrait};

use crate::{CliError, Result, path::default_db_path};

#[derive(tabled::Tabled)]
struct SeedRow {
    #[tabled(rename = "ENTITY")]
    entity: &'static str,
    #[tabled(rename = "COUNT")]
    count: u32,
}

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
            .map_err(|e| CliError::Operation {
                message: format!("confirmation prompt failed: {e}"),
            })?;
            if !confirmed {
                return Err(CliError::Operation {
                    message: "Aborted by user".to_string(),
                });
            }
        }

        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        let sea_conn = livtet_data::orm::Database::connect(&db_url)
            .await
            .map_err(|e| CliError::Operation {
                message: format!("Failed to connect to {db_url}: {e}"),
            })?;

        // Run migrations to ensure tables exist before seeding.
        Migrator::up(&sea_conn, None)
            .await
            .map_err(|e| CliError::Operation {
                message: format!("Failed to migrate {db_path}: {e}"),
            })?;

        let config = livtet_core::seed::SeedConfig {
            num_works: self.works,
            ..Default::default()
        };

        let result = livtet_core::seed::seed_database(&sea_conn, &config)
            .await
            .map_err(|e| CliError::Operation {
                message: format!("Seed failed: {e}"),
            })?;

        use tabled::settings::Style;

        let row = |entity: &'static str, count: u32| SeedRow { entity, count };
        let rows = vec![
            row("works", result.works_created),
            row("editions", result.editions_created),
            row("authors", result.authors_created),
            row("publishers", result.publishers_created),
            row("reading status entries", result.reading_status_count),
            row("annotations", result.annotations_created),
            row("digital inventory", result.digital_inventory_created),
            row("loans", result.loans_created),
            row("reading sessions", result.reading_sessions_created),
            row("saved searches", result.saved_searches_created),
            row("reading lists", result.reading_lists_created),
        ];
        println!("Seeded database at {db_path}:");
        println!("{}", tabled::Table::new(rows).with(Style::modern()));

        Ok(())
    }
}
