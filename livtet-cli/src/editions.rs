//! `livtet editions` — query editions (index-backed O(1) list/search, batched SeaORM get).

use std::io::IsTerminal as _;

use camino::Utf8PathBuf;
use clap::{Args, Subcommand};

use crate::CliError;

use livtet_data::entities::{editions, formats, digital_inventory, authors, edition_authors};
use livtet_data::orm::{ColumnTrait, Database, EntityTrait, QueryFilter};
use livtet_search::{SearchIndex, SearchOptions, model::HitKind};

/// Resolve the path of the default livtet SQLite database file.
pub fn default_db_path() -> crate::Result<Utf8PathBuf> {
    let dir = livtet_core::paths::data_dir().ok_or_else(|| CliError::Operation {
        message: "Could not resolve the livtet data directory".to_string(),
    })?;
    Ok(dir.join("livtet.db"))
}

/// Resolve the on-disk search index directory (sibling of the DB).
pub fn default_index_dir() -> crate::Result<Utf8PathBuf> {
    let dir = livtet_core::paths::data_dir().ok_or_else(|| CliError::Operation {
        message: "Could not resolve the livtet data directory".to_string(),
    })?;
    Ok(dir.join("search-index"))
}

#[derive(Args, Debug)]
pub struct EditionsArgs {
    #[command(subcommand)]
    pub command: EditionsCommand,
}

#[derive(Subcommand, Debug)]
pub enum EditionsCommand {
    /// Fetch a single edition by ID.
    Get {
        /// Edition ULID.
        id: String,
    },
    /// List all editions (index-backed).
    List {
        /// Max results (default 20, cap 100).
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(0..=100))]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
    /// Full-text search across editions (index-backed).
    Search {
        /// Free-text query.
        query: String,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(0..=100))]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
}

impl EditionsArgs {
    pub async fn run(self) -> crate::Result<()> {
        match self.command {
            EditionsCommand::Get { id } => get(&id).await,
            EditionsCommand::List { limit, offset } => list(limit, offset).await,
            EditionsCommand::Search { query, limit, offset } => search(&query, limit, offset).await,
        }
    }
}

#[derive(tabled::Tabled)]
struct Row {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "TITLE")]
    title: String,
    #[tabled(rename = "AUTHOR")]
    author: String,
    #[tabled(rename = "FORMAT")]
    format: String,
    #[tabled(rename = "*")]
    on_disk: bool,
}

impl From<LivtetHit> for Row {
    fn from(h: LivtetHit) -> Self {
        Self {
            id: h.edition_id.unwrap_or_default(),
            title: h.title,
            author: if h.authors.is_empty() { "—".to_string() } else { h.authors.join(", ") },
            format: h.format.unwrap_or_else(|| "—".to_string()),
            on_disk: h.has_file,
        }
    }
}

/// Projection of `livtet_search::SearchHit` we consume.
struct LivtetHit {
    edition_id: Option<String>,
    title: String,
    authors: Vec<String>,
    format: Option<String>,
    has_file: bool,
}

impl From<livtet_search::model::SearchHit> for LivtetHit {
    fn from(h: livtet_search::model::SearchHit) -> Self {
        match h.kind {
            HitKind::Edition => Self {
                edition_id: h.edition_id,
                title: h.title,
                authors: h.authors,
                format: h.format,
                has_file: h.has_file,
            },
            _ => Self {
                edition_id: None,
                title: h.title,
                authors: h.authors,
                format: h.format,
                has_file: h.has_file,
            },
        }
    }
}

#[inline]
fn limit_u(limit: u32) -> usize {
    limit.min(100) as usize
}

#[inline]
fn offset_u(offset: u32) -> usize {
    offset as usize
}

async fn open_index() -> crate::Result<SearchIndex> {
    let dir = default_index_dir()?;
    SearchIndex::open(&dir).map_err(|e| CliError::Operation {
        message: format!("Failed to open index at {dir}: {e}"),
    })
}

// ─── list / search (index-bound, O(1) queries regardless of page) ──────────

async fn list(limit: u32, offset: u32) -> crate::Result<()> {
    let index = open_index().await?;
    let opts = SearchOptions {
        offset: offset_u(offset),
        ..Default::default()
    };
    let hits: Vec<LivtetHit> = index
        .search_with_options(
            "",
            limit_u(limit),
            &opts,
        )
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Index search failed: {e}"),
        })?
        .into_iter()
        .map(LivtetHit::from)
        .collect();
    print_table(hits);
    Ok(())
}

async fn search(query: &str, limit: u32, offset: u32) -> crate::Result<()> {
    let index = open_index().await?;
    let opts = SearchOptions {
        offset: offset_u(offset),
        ..Default::default()
    };
    let hits: Vec<LivtetHit> = index
        .search_with_options(query, limit_u(limit), &opts)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Index search failed: {e}"),
        })?
        .into_iter()
        .map(LivtetHit::from)
        .collect();
    print_table(hits);
    Ok(())
}

fn print_table(hits: Vec<LivtetHit>) {
    use tabled::settings::Style;
    let rows: Vec<Row> = hits.into_iter().map(Row::from).collect();
    let n = rows.len();
    if !rows.is_empty() {
        println!("{}", tabled::Table::new(&rows).with(Style::modern()));
    }
    println!("{n} edition(s)");
}

// ─── get (SeaORM, batched O(1) queries) ─────────────────────────────────────

async fn get(id: &str) -> crate::Result<()> {
    let db_url = {
        let path = default_db_path()?;
        format!("sqlite:{}?mode=ro", path)
    };
    let db = Database::connect(&db_url)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Failed to connect to database: {e}"),
        })?;

    // Single edition fetch.
    let parsed_id: livtet_types::DbId = id.parse().map_err(|e: <livtet_types::DbId as std::str::FromStr>::Err| CliError::Operation {
        message: format!("Invalid edition ID: {e}"),
    })?;
    let edition = editions::Entity::find_by_id(parsed_id)
    .one(&db)
    .await
    .map_err(|e| CliError::Operation {
        message: format!("Query failed: {e}"),
    })?
    .ok_or_else(|| CliError::Operation {
        message: format!("edition not found: {id}"),
    })?;

    // Single inventory fetch for the on-disk file.
    let inv = digital_inventory::Entity::find_by_id(edition.id)
        .one(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?;

    // Single format fetch.
    let fmt = match edition.format_id {
        Some(fid) => formats::Entity::find_by_id(fid)
            .one(&db)
            .await
            .map_err(|e| CliError::Operation {
                message: format!("Query failed: {e}"),
            })?
            .map(|f| f.name),
        None => None,
    };

    // Single author batch: join edition_authors -> authors via is_in.
    let author_names: Vec<String> = {
        let ea = edition_authors::Entity::find()
            .filter(edition_authors::Column::EditionId.eq(edition.id))
            .all(&db)
            .await
            .map_err(|e| CliError::Operation {
                message: format!("Query failed: {e}"),
            })?;
        if ea.is_empty() {
            Vec::new()
        } else {
            let author_ids: Vec<_> = ea.iter().map(|r| r.author_id).collect();
            authors::Entity::find()
                .filter(authors::Column::Id.is_in(author_ids))
                .all(&db)
                .await
                .map_err(|e| CliError::Operation {
                    message: format!("Query failed: {e}"),
                })?
                .into_iter()
                .map(|a| a.name)
                .collect()
        }
    };

    let title = edition
        .title
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("<edition {}>", id));

    println!("  Title:   {title}");
    if author_names.is_empty() {
        println!("  Author:  —");
    } else {
        println!("  Author:  {}", author_names.join(", "));
    }
    println!("  Format:  {}", fmt.as_deref().unwrap_or("—"));

    match &inv {
        Some(di) if di.file_path.is_some() => {
            let p = di.file_path.as_ref().unwrap();
            println!("  On-disk: * {}", linkify(p));
        }
        _ => println!("  On-disk: —"),
    }

    Ok(())
}

// ─── terminal OSC8 hyperlink ───────────────────────────────────────────────

/// Wrap an absolute `file://` URI around `path`, plain text when not a TTY /
/// when a colour/terminal override is set.
pub fn linkify(path: &str) -> String {
    let stdout = std::io::stdout();
    if !stdout.is_terminal() {
        return path.to_string();
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return path.to_string();
    }
    if std::env::var_os("TERM").is_some_and(|t| t == "dumb") {
        return path.to_string();
    }
    let uri = if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{}", path)
    };
    format!("\x1b]8;;{uri}\x1b\\{path}\x1b]8;;\x1b\\")
}
