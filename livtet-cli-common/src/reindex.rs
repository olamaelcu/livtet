//! `livtet reindex` — rebuild the Tantivy search index from the database.
//!
//! By default this goes through `SearchIndex::migrate_to`, which is a
//! no-op when the on-disk schema is already current. Pass `--force` to
//! wipe and rebuild unconditionally.

use camino::Utf8PathBuf;
use clap::Parser;
use fs_err as fs;
use livtet_data::migration::{Migrator, MigratorTrait};

use crate::{
    CliError, Result,
    path::{default_db_path, default_index_dir},
};

#[derive(Parser, Debug)]
pub struct ReindexArgs {
    /// Path to the SQLite database file. Defaults to the platform's
    /// livtet data directory.
    #[arg(long)]
    pub database: Option<Utf8PathBuf>,

    /// Directory holding the Tantivy index. Defaults to
    /// `<data-dir>/search-index`.
    #[arg(long)]
    pub index_dir: Option<Utf8PathBuf>,

    /// Skip the confirmation prompt before rebuilding the index.
    #[arg(long)]
    pub yes: bool,

    /// Rebuild even when the on-disk schema is already current.
    #[arg(long)]
    pub force: bool,
}

impl ReindexArgs {
    pub async fn run(&self) -> Result<()> {
        let db_path = match &self.database {
            Some(p) => p.clone(),
            None => default_db_path()?,
        };
        let index_dir = match &self.index_dir {
            Some(p) => p.clone(),
            None => default_index_dir()?,
        };

        if !self.yes {
            let confirmed = inquire::Confirm::new(&format!(
                "This will rebuild the search index at {index_dir} \
                 from the database at {db_path}. Continue?",
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

        let db_url = format!("sqlite:{db_path}?mode=rwc");
        let conn = livtet_data::orm::Database::connect(&db_url)
            .await
            .map_err(|e| CliError::Operation {
                message: format!("Failed to connect to {db_url}: {e}"),
            })?;
        // A fresh database file has no tables yet; migrate so the
        // reindex below sees a schema (and yields an empty index)
        // instead of failing with "no such table".
        Migrator::up(&conn, None)
            .await
            .map_err(|e| CliError::Operation {
                message: format!("Failed to migrate {db_path}: {e}"),
            })?;

        let started = std::time::Instant::now();
        let spinner = indicatif::ProgressBar::new_spinner();
        spinner.set_message("Loading catalog rows");
        spinner.enable_steady_tick(std::time::Duration::from_millis(80));

        // `indexed` mirrors the last Indexing event so the summary
        // below can report document counts.
        let mut indexed = 0u64;
        let mut bar: Option<indicatif::ProgressBar> = None;
        let mut on_event = |event: livtet_core::search::ReindexEvent| match event {
            livtet_core::search::ReindexEvent::Loading => {}
            livtet_core::search::ReindexEvent::Indexing { done, total } => {
                indexed = done;
                let bar = bar.get_or_insert_with(|| {
                    spinner.finish_and_clear();
                    let bar = indicatif::ProgressBar::new(total);
                    bar.set_style(
                        indicatif::ProgressStyle::with_template("[{bar:40.cyan/blue}] {pos}/{len}")
                            .expect("valid progress template")
                            .progress_chars("#>-"),
                    );
                    bar.set_message("Indexing");
                    bar
                });
                if bar.length() != Some(total) {
                    bar.set_length(total);
                }
                bar.set_position(done);
            }
        };

        // First try schema-aware migration (no-op when current).
        let prev = livtet_core::search::SearchIndex::migrate_to_with_progress(
            index_dir.as_path(),
            &conn,
            &mut on_event,
        )
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Reindex failed: {e}"),
        })?;

        let rebuilt = prev != livtet_core::search::SCHEMA_VERSION || self.force;

        if rebuilt {
            // Either the schema was stale (migrate_to already rebuilt)
            // or --force was passed and we need to wipe + rebuild
            // unconditionally since migrate_to was a no-op.
            if self.force && prev == livtet_core::search::SCHEMA_VERSION {
                if index_dir.exists() {
                    fs::remove_dir_all(&index_dir).map_err(|e| CliError::Operation {
                        message: format!("Failed to clear {index_dir}: {e}"),
                    })?;
                }
                spinner.set_message("Rebuilding index");
                let index =
                    livtet_core::search::SearchIndex::open(index_dir.as_path()).map_err(|e| {
                        CliError::Operation {
                            message: format!("Failed to open {index_dir}: {e}"),
                        }
                    })?;
                index
                    .reindex_with_progress(&conn, &mut on_event)
                    .await
                    .map_err(|e| CliError::Operation {
                        message: format!("Reindex failed: {e}"),
                    })?;
            }
        }

        finish_bar(&bar);
        if rebuilt {
            println!(
                "Indexed {indexed} documents into {index_dir} in {:.1?}",
                started.elapsed()
            );
        } else {
            println!(
                "Search index at {index_dir} is already current \
                 (schema v{}); nothing to do",
                livtet_core::search::SCHEMA_VERSION
            );
        }
        Ok(())
    }
}

fn finish_bar(bar: &Option<indicatif::ProgressBar>) {
    if let Some(bar) = bar {
        bar.finish_and_clear();
    }
}
