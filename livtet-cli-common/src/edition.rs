//! `livtet edition` — mutation commands for editions.
//!
//! Currently contains `files` subcommands for managing file associations.

use std::collections::HashMap;

use clap::{Args, Subcommand};

use livtet_data::entities::{digital_inventory, editions, works};
use livtet_data::orm::{
    ActiveModelTrait, ColumnTrait, Database, EntityTrait, QueryFilter, QuerySelect, Set,
};
use livtet_types::now_primitive;

use crate::{CliError, Result};

#[derive(Args, Debug)]
pub struct EditionArgs {
    #[command(subcommand)]
    pub command: EditionCommand,
}

/// Root command for edition mutations.
#[derive(Subcommand, Debug)]
pub enum EditionCommand {
    /// Manage files associated with editions.
    Files(FilesArgs),
}

#[derive(Args, Debug)]
pub struct FilesArgs {
    #[command(subcommand)]
    pub command: FilesCommand,
}

#[derive(Subcommand, Debug)]
pub enum FilesCommand {
    /// Associate a file with an edition.
    Add {
        /// Edition ULID.
        id: String,
        /// Path to the file (relative or absolute).
        path: String,
    },
    /// Remove a file association from an edition.
    Remove {
        /// Edition ULID.
        id: String,
    },
    /// List editions that have associated files.
    List {
        /// Max results (default 20, cap 100).
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(0..=100))]
        limit: u32,
    },
}

impl EditionArgs {
    pub async fn run(self) -> Result<()> {
        match self.command {
            EditionCommand::Files(cmd) => cmd.run().await,
        }
    }
}

impl FilesArgs {
    async fn run(self) -> Result<()> {
        let db_url = {
            let path = default_db_path()?;
            format!("sqlite:{}?mode=rwc", path)
        };
        match self.command {
            FilesCommand::Add { id, path } => add_file(&id, &path, &db_url).await,
            FilesCommand::Remove { id } => remove_file(&id, &db_url).await,
            FilesCommand::List { limit } => list_files(limit, &db_url).await,
        }
    }
}

pub fn default_db_path() -> crate::Result<camino::Utf8PathBuf> {
    let dir = livtet_core::paths::data_dir().ok_or_else(|| CliError::Operation {
        message: "Could not resolve the livtet data directory".to_string(),
    })?;
    Ok(dir.join("livtet.db"))
}

async fn open_db(database_url: &str) -> Result<livtet_data::orm::DatabaseConnection> {
    Database::connect(database_url)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Failed to connect to database: {e}"),
        })
}

async fn add_file(edition_id_str: &str, file_path: &str, database_url: &str) -> Result<()> {
    let db = open_db(database_url).await?;

    let edition_id: livtet_types::DbId =
        edition_id_str
            .parse()
            .map_err(
                |e: <livtet_types::DbId as std::str::FromStr>::Err| CliError::Operation {
                    message: format!("Invalid edition ID: {e}"),
                },
            )?;

    let edition = editions::Entity::find_by_id(edition_id)
        .one(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?
        .ok_or_else(|| CliError::Operation {
            message: format!("edition not found: {edition_id_str}"),
        })?;

    let title = edition.title.as_deref().unwrap_or("<unnamed>");

    let di = digital_inventory::Entity::find_by_id(edition_id)
        .one(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?;

    let current_time = now_primitive();

    match di {
        Some(existing) => {
            let mut model: digital_inventory::ActiveModel = existing.into();
            model.file_path = Set(Some(file_path.to_string()));
            model.updated_at = Set(Some(current_time));
            model.update(&db).await.map_err(|e| CliError::Operation {
                message: format!("Update failed: {e}"),
            })?;
        }
        None => {
            let inventory_id = livtet_types::DbId::new();
            let model = digital_inventory::ActiveModel {
                id: Set(inventory_id),
                edition_id: Set(edition_id),
                file_path: Set(Some(file_path.to_string())),
                cover_path: Set(None),
                blurhash: Set(None),
                dominant_color: Set(None),
                file_hash: Set(None),
                file_size_bytes: Set(None),
                file_format: Set(None),
                notes: Set(None),
                added_at: Set(current_time),
                updated_at: Set(None),
            };
            digital_inventory::Entity::insert(model)
                .exec(&db)
                .await
                .map_err(|e| CliError::Operation {
                    message: format!("Insert failed: {e}"),
                })?;
        }
    }

    println!(
        "Associated file '{}' with edition {} ({})",
        file_path, edition_id_str, title
    );
    Ok(())
}

async fn remove_file(edition_id_str: &str, database_url: &str) -> Result<()> {
    let db = open_db(database_url).await?;

    let edition_id: livtet_types::DbId =
        edition_id_str
            .parse()
            .map_err(
                |e: <livtet_types::DbId as std::str::FromStr>::Err| CliError::Operation {
                    message: format!("Invalid edition ID: {e}"),
                },
            )?;

    let edition = editions::Entity::find_by_id(edition_id)
        .one(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?
        .ok_or_else(|| CliError::Operation {
            message: format!("edition not found: {edition_id_str}"),
        })?;

    let title = edition.title.as_deref().unwrap_or("<unnamed>");

    let count = digital_inventory::Entity::delete_by_id(edition_id)
        .exec(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Delete failed: {e}"),
        })?
        .rows_affected;

    if count == 0 {
        println!("Edition {} has no associated file", edition_id_str);
    } else {
        println!(
            "Removed file association from edition {} ({})",
            edition_id_str, title
        );
    }

    Ok(())
}

async fn list_files(limit: u32, database_url: &str) -> Result<()> {
    let db = open_db(database_url).await?;

    let di: Vec<digital_inventory::Model> = digital_inventory::Entity::find()
        .limit(Some(limit as u64))
        .all(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?;

    if di.is_empty() {
        println!("No editions with associated files");
        return Ok(());
    }

    let di_ids: Vec<livtet_types::DbId> = di.iter().map(|m| m.edition_id).collect();

    let editions_map: HashMap<livtet_types::DbId, editions::Model> = editions::Entity::find()
        .filter(editions::Column::Id.is_in(di_ids))
        .all(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?
        .into_iter()
        .map(|e| (e.id, e))
        .collect();

    let work_ids: Vec<livtet_types::DbId> = editions_map.values().map(|e| e.work_id).collect();

    let works_map: HashMap<livtet_types::DbId, works::Model> = works::Entity::find()
        .filter(works::Column::Id.is_in(work_ids))
        .all(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?
        .into_iter()
        .map(|w| (w.id, w))
        .collect();

    let rows: Vec<SimpleRow> = di
        .iter()
        .map(|d| {
            let edition = editions_map.get(&d.edition_id);
            let work_title = edition
                .and_then(|e| {
                    works_map
                        .get(&e.work_id)
                        .as_ref()
                        .map(|w| w.title.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| "—".to_string());

            let title = edition
                .and_then(|e| e.title.as_deref())
                .unwrap_or("<unnamed>")
                .to_string();

            let file_path = d.file_path.as_deref().unwrap_or("—").to_string();

            let file_format = d.file_format.as_deref().unwrap_or("—").to_string();

            SimpleRow {
                edition_id: d.edition_id.to_string(),
                title,
                work_title,
                file_path,
                file_format,
            }
        })
        .collect();

    use tabled::settings::Style;
    println!("{}", tabled::Table::new(&rows).with(Style::modern()));
    println!("{} edition(s) with files", rows.len());
    Ok(())
}

#[derive(Debug, tabled::Tabled)]
struct SimpleRow {
    #[tabled(rename = "ID")]
    edition_id: String,
    #[tabled(rename = "TITLE")]
    title: String,
    #[tabled(rename = "WORK")]
    work_title: String,
    #[tabled(rename = "FILE")]
    file_path: String,
    #[tabled(rename = "FORMAT")]
    file_format: String,
}
