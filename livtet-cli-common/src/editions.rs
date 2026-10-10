//! `livtet editions` — query editions (index-backed O(1) list/search, batched SeaORM get).

use std::io::IsTerminal as _;

use clap::{Args, Subcommand};

use crate::CliError;
use crate::context::Context;

use livtet_data::entities::{authors, digital_inventory, edition_authors, editions, formats};
use livtet_data::orm::{ColumnTrait, Database, EntityTrait, QueryFilter};
use livtet_search::{SearchOptions, SearchReader, model::HitKind};

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
    pub async fn run(self, ctx: &Context) -> crate::Result<()> {
        match self.command {
            EditionsCommand::Get { id } => {
                println!("{}", render_detail(&get_detail(ctx, &id).await?));
                Ok(())
            }
            EditionsCommand::List { limit, offset } => {
                print_table(list_rows(ctx, limit, offset).await?);
                Ok(())
            }
            EditionsCommand::Search {
                query,
                limit,
                offset,
            } => {
                print_table(search_rows(ctx, &query, limit, offset).await?);
                Ok(())
            }
        }
    }
}

/// One row of the `editions list` / `editions search` table.
#[derive(Debug, Clone, PartialEq, Eq, tabled::Tabled)]
pub struct Row {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "TITLE")]
    pub title: String,
    #[tabled(rename = "AUTHOR")]
    pub author: String,
    #[tabled(rename = "FORMAT")]
    pub format: String,
    #[tabled(rename = "*")]
    pub on_disk: bool,
}

impl From<LivtetHit> for Row {
    fn from(h: LivtetHit) -> Self {
        Self {
            id: h.edition_id.unwrap_or_default(),
            title: h.title,
            author: if h.authors.is_empty() {
                "—".to_string()
            } else {
                h.authors.join(", ")
            },
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

fn open_index(ctx: &Context) -> crate::Result<SearchReader> {
    let dir = &ctx.index_dir;
    SearchReader::open(dir).map_err(|e| CliError::Operation {
        message: format!("Failed to open index at {dir}: {e}"),
    })
}

/// Shared body of `list` and `search`: the two differ only in the query
/// string they hand the index.
async fn query_rows(
    ctx: &Context,
    query: &str,
    limit: u32,
    offset: u32,
) -> crate::Result<Vec<Row>> {
    // `--limit 0` is accepted by the arg parser (range 0..=100), but
    // tantivy's `TopDocs::with_limit` asserts on a zero limit, so asking the
    // index would panic. Nothing to return is the honest answer.
    if limit == 0 {
        return Ok(Vec::new());
    }
    let index = open_index(ctx)?;
    let opts = SearchOptions {
        offset: offset as i64,
        ..Default::default()
    };
    Ok(index
        .search_with_options(query, limit_u(limit), &opts)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Index search failed: {e}"),
        })?
        .into_iter()
        .map(LivtetHit::from)
        .map(Row::from)
        .collect())
}

// ─── list / search (index-bound, O(1) queries regardless of page) ──────────

/// Every edition the index holds, newest-relevance first.
pub async fn list_rows(ctx: &Context, limit: u32, offset: u32) -> crate::Result<Vec<Row>> {
    query_rows(ctx, "", limit, offset).await
}

/// Editions matching `query`.
pub async fn search_rows(
    ctx: &Context,
    query: &str,
    limit: u32,
    offset: u32,
) -> crate::Result<Vec<Row>> {
    query_rows(ctx, query, limit, offset).await
}

/// Render the table exactly as `run` prints it, trailing count included.
pub fn render_table(rows: &[Row]) -> String {
    use tabled::settings::Style;
    let mut out = String::new();
    if !rows.is_empty() {
        out.push_str(&tabled::Table::new(rows).with(Style::modern()).to_string());
        out.push('\n');
    }
    out.push_str(&format!("{} edition(s)", rows.len()));
    out
}

fn print_table(rows: Vec<Row>) {
    println!("{}", render_table(&rows));
}

// ─── get (SeaORM, batched O(1) queries) ─────────────────────────────────────

/// The `editions get` payload: one edition, with its author names, format,
/// and on-disk file resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditionDetail {
    pub title: String,
    pub authors: Vec<String>,
    pub format: Option<String>,
    pub file_path: Option<String>,
}

/// Fetch one edition by ULID.
///
/// Four batched queries: the edition, its inventory row, its format, and its
/// authors via a single `is_in` over the link table.
pub async fn get_detail(ctx: &Context, id: &str) -> crate::Result<EditionDetail> {
    let db = Database::connect(&ctx.db_url_ro())
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Failed to connect to database: {e}"),
        })?;

    let parsed_id: livtet_types::DbId =
        id.parse()
            .map_err(
                |e: <livtet_types::DbId as std::str::FromStr>::Err| CliError::Operation {
                    message: format!("Invalid edition ID: {e}"),
                },
            )?;
    let edition = editions::Entity::find_by_id(parsed_id)
        .one(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?
        .ok_or_else(|| CliError::Operation {
            message: format!("edition not found: {id}"),
        })?;

    let inv = digital_inventory::Entity::find()
        .filter(digital_inventory::Column::EditionId.eq(edition.id))
        .one(&db)
        .await
        .map_err(|e| CliError::Operation {
            message: format!("Query failed: {e}"),
        })?;

    let format = match edition.format_id {
        Some(fid) => formats::Entity::find_by_id(fid)
            .one(&db)
            .await
            .map_err(|e| CliError::Operation {
                message: format!("Query failed: {e}"),
            })?
            .map(|f| f.name),
        None => None,
    };

    let authors: Vec<String> = {
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
        .unwrap_or_else(|| format!("<edition {id}>"));

    Ok(EditionDetail {
        title,
        authors,
        format,
        file_path: inv.and_then(|di| di.file_path),
    })
}

/// Render a detail block exactly as `run` prints it. The on-disk path is
/// hyperlinked only when stdout is a TTY, so this is stable under capture.
pub fn render_detail(detail: &EditionDetail) -> String {
    render_detail_with(detail, &linkify)
}

/// [`render_detail`] with the path renderer passed in rather than reading the
/// environment through [`linkify`].
///
/// The on-disk line is hyperlinked only on a plain TTY, so rendering through
/// `linkify` makes the output depend on how the process's stdout happens to
/// be attached. That is fine in the binary and useless in a snapshot: the
/// same input yields different bytes under `cargo test` than under a shell
/// with output captured. Tests pass a renderer with the decision already made.
pub fn render_detail_with(detail: &EditionDetail, render_path: &dyn Fn(&str) -> String) -> String {
    let mut out = String::new();
    out.push_str(&format!("  Title:   {}\n", detail.title));
    if detail.authors.is_empty() {
        out.push_str("  Author:  —\n");
    } else {
        out.push_str(&format!("  Author:  {}\n", detail.authors.join(", ")));
    }
    out.push_str(&format!(
        "  Format:  {}\n",
        detail.format.as_deref().unwrap_or("—")
    ));
    match &detail.file_path {
        Some(p) => out.push_str(&format!("  On-disk: * {}", render_path(p))),
        None => out.push_str("  On-disk: —"),
    }
    out
}

// ─── terminal OSC8 hyperlink ───────────────────────────────────────────────

/// Wrap an absolute `file://` URI around `path`, plain text when not a TTY /
/// when a colour/terminal override is set.
pub fn linkify(path: &str) -> String {
    linkify_with(
        path,
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
        std::env::var_os("TERM").is_some_and(|t| t == "dumb"),
    )
}

/// [`linkify`] with the environment passed in rather than read.
///
/// Every suppression path -- not a TTY, `NO_COLOR`, `TERM=dumb` -- is a
/// reason to emit the bare path. Taking them as arguments is what makes
/// those branches reachable from a test without mutating process state.
pub fn linkify_with(path: &str, is_terminal: bool, no_color: bool, term_dumb: bool) -> String {
    if !is_terminal || no_color || term_dumb {
        return path.to_string();
    }
    let uri = if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    };
    format!("\x1b]8;;{uri}\x1b\\{path}\x1b]8;;\x1b\\")
}
