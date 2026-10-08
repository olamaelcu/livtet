//! `livtet edition` — mutation commands for editions.
//!
//! Currently contains `files` subcommands for managing file associations.
//!
//! Each command splits into a function returning what changed and a thin
//! renderer, so tests can assert on the outcome rather than on stdout. See
//! [`crate::context::Context`] for how they are pointed at a database.

use std::collections::HashMap;

use clap::{Args, Subcommand};

use livtet_data::entities::{digital_inventory, editions, works};
use livtet_data::orm::{
    ActiveModelTrait, ColumnTrait, Database, EntityTrait, QueryFilter, QuerySelect, Set,
};
use livtet_types::now_primitive;

use crate::context::Context;
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
    pub async fn run(self, ctx: &Context) -> Result<()> {
        match self.command {
            EditionCommand::Files(cmd) => cmd.run(ctx).await,
        }
    }
}

impl FilesArgs {
    async fn run(self, ctx: &Context) -> Result<()> {
        match self.command {
            FilesCommand::Add { id, path } => {
                println!("{}", render_added(&add_file(ctx, &id, &path).await?));
                Ok(())
            }
            FilesCommand::Remove { id } => {
                println!("{}", render_removed(&remove_file(ctx, &id).await?));
                Ok(())
            }
            FilesCommand::List { limit } => {
                println!("{}", render_file_table(&list_files_rows(ctx, limit).await?));
                Ok(())
            }
        }
    }
}

/// What a file-association change did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub edition_id: String,
    /// The edition's own title, or `<unnamed>` when it has none.
    pub title: String,
    /// The path now associated, or `None` after a removal.
    pub file_path: Option<String>,
    /// False when a removal found no association to remove.
    pub changed: bool,
}

/// One row of the `edition files list` table.
#[derive(Debug, Clone, PartialEq, Eq, tabled::Tabled)]
pub struct FileRow {
    #[tabled(rename = "ID")]
    pub edition_id: String,
    #[tabled(rename = "TITLE")]
    pub title: String,
    #[tabled(rename = "WORK")]
    pub work_title: String,
    #[tabled(rename = "FILE")]
    pub file_path: String,
    #[tabled(rename = "FORMAT")]
    pub file_format: String,
}

async fn open_db(ctx: &Context) -> Result<livtet_data::orm::DatabaseConnection> {
    Database::connect(&ctx.db_url_rwc())
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Failed to connect to database: {e}"),
        })
}

fn parse_edition_id(raw: &str) -> Result<livtet_types::DbId> {
    raw.parse().map_err(
        |e: <livtet_types::DbId as std::str::FromStr>::Err| CliError::Operation {
            message: format!("Invalid edition ID: {e}"),
        },
    )
}

async fn load_edition(
    db: &livtet_data::orm::DatabaseConnection,
    id: livtet_types::DbId,
    raw: &str,
) -> Result<editions::Model> {
    editions::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?
        .ok_or_else(|| CliError::Operation {
            message: format!("edition not found: {raw}"),
        })
}

/// The inventory row for an edition.
async fn inventory_for(
    db: &livtet_data::orm::DatabaseConnection,
    edition_id: livtet_types::DbId,
) -> Result<Option<digital_inventory::Model>> {
    digital_inventory::Entity::find_by_id(edition_id)
        .one(db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })
}

/// Associate `file_path` with an edition, replacing any existing path.
pub async fn add_file(ctx: &Context, edition_id_str: &str, file_path: &str) -> Result<FileChange> {
    let db = open_db(ctx).await?;
    let edition_id = parse_edition_id(edition_id_str)?;
    let edition = load_edition(&db, edition_id, edition_id_str).await?;
    let title = edition.title.as_deref().unwrap_or("<unnamed>").to_string();

    let current_time = now_primitive();
    match inventory_for(&db, edition_id).await? {
        Some(existing) => {
            let mut model: digital_inventory::ActiveModel = existing.into();
            model.file_path = Set(Some(file_path.to_string()));
            model.updated_at = Set(Some(current_time));
            model.update(&db).await.map_err(|e| CliError::Operation {
                message: format!("Update failed: {e}"),
            })?;
        }
        None => {
            let model = digital_inventory::ActiveModel {
                id: Set(livtet_types::DbId::new()),
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

    Ok(FileChange {
        edition_id: edition_id_str.to_string(),
        title,
        file_path: Some(file_path.to_string()),
        changed: true,
    })
}

/// Drop an edition's file association. `changed` is false when there was none.
pub async fn remove_file(ctx: &Context, edition_id_str: &str) -> Result<FileChange> {
    let db = open_db(ctx).await?;
    let edition_id = parse_edition_id(edition_id_str)?;
    let edition = load_edition(&db, edition_id, edition_id_str).await?;
    let title = edition.title.as_deref().unwrap_or("<unnamed>").to_string();

    let changed = digital_inventory::Entity::delete_by_id(edition_id)
        .exec(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Delete failed: {e}"),
        })?
        .rows_affected
        > 0;

    Ok(FileChange {
        edition_id: edition_id_str.to_string(),
        title,
        file_path: None,
        changed,
    })
}

/// Editions that have an associated file, newest inventory first.
pub async fn list_files_rows(ctx: &Context, limit: u32) -> Result<Vec<FileRow>> {
    let db = open_db(ctx).await?;

    let inventory: Vec<digital_inventory::Model> = digital_inventory::Entity::find()
        .limit(Some(limit as u64))
        .all(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?;
    if inventory.is_empty() {
        return Ok(Vec::new());
    }

    let edition_ids: Vec<livtet_types::DbId> = inventory.iter().map(|m| m.edition_id).collect();
    let editions_map: HashMap<livtet_types::DbId, editions::Model> = editions::Entity::find()
        .filter(editions::Column::Id.is_in(edition_ids))
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

    Ok(inventory
        .iter()
        .map(|d| {
            let edition = editions_map.get(&d.edition_id);
            FileRow {
                edition_id: d.edition_id.to_string(),
                title: edition
                    .and_then(|e| e.title.as_deref())
                    .unwrap_or("<unnamed>")
                    .to_string(),
                work_title: edition
                    .and_then(|e| works_map.get(&e.work_id))
                    .map(|w| w.title.clone())
                    .unwrap_or_else(|| "—".to_string()),
                file_path: d.file_path.as_deref().unwrap_or("—").to_string(),
                file_format: d.file_format.as_deref().unwrap_or("—").to_string(),
            }
        })
        .collect())
}

pub fn render_added(change: &FileChange) -> String {
    format!(
        "Associated file '{}' with edition {} ({})",
        change.file_path.as_deref().unwrap_or("—"),
        change.edition_id,
        change.title
    )
}

pub fn render_removed(change: &FileChange) -> String {
    if change.changed {
        format!(
            "Removed file association from edition {} ({})",
            change.edition_id, change.title
        )
    } else {
        format!("Edition {} has no associated file", change.edition_id)
    }
}

pub fn render_file_table(rows: &[FileRow]) -> String {
    use tabled::settings::Style;
    if rows.is_empty() {
        return "No editions with associated files".to_string();
    }
    format!(
        "{}\n{} edition(s) with files",
        tabled::Table::new(rows).with(Style::modern()),
        rows.len()
    )
}
