//! [`SearchIndex`] handle: open, migrate, and accessors.
//!
//! Write paths live in [`crate::write`], read paths in [`crate::search`].

use std::sync::Arc;

use camino::Utf8Path;
use fs_err as fs;
use livtet_data::orm::DatabaseConnection;
use tantivy::{Index, IndexWriter, ReloadPolicy, directory::MmapDirectory, schema::Schema};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::{
    label_resolver::LabelResolver,
    schema::{SCHEMA_VERSION, build_schema},
};

// ---------------------------------------------------------------------------
// SearchIndex
// ---------------------------------------------------------------------------

/// Tantivy-backed search index.
pub struct SearchIndex {
    pub(crate) index: Index,
    pub(crate) reader: tantivy::IndexReader,
    pub(crate) writer: Arc<RwLock<IndexWriter>>,
    pub(crate) schema: Schema,
    /// Pre-resolver for format and language IDs → labels.
    pub label_resolver: LabelResolver,
}

/// Things that can go wrong inside `SearchIndex`. We collapse
/// tantivy + sea-orm errors into a single error type so callers
/// only have to handle one enum.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
    #[error("database error: {0}")]
    Db(#[from] livtet_data::orm::DbErr),
    #[error("snippet generation failed: {0}")]
    Snippet(String),
    #[error(
        "index schema version {found} does not match build version {expected}; rebuild index with the original build or run migrate_to first"
    )]
    SchemaVersionMismatch { found: u32, expected: u32 },
}

impl SearchIndex {
    /// Open (or create) an index at `index_dir`. Reads the sidecar
    /// `search_schema_version.json` for the on-disk schema version. If the version
    /// doesn't match `SCHEMA_VERSION`, this is a hard error —
    /// callers MUST call `migrate_to` first.
    pub fn open(index_dir: &Utf8Path) -> Result<Self, SearchError> {
        let schema = build_schema();
        fs::create_dir_all(index_dir).ok();
        let meta_path = index_dir.join("search_schema_version.json");
        let on_disk_version = if meta_path.is_file() {
            let raw = fs::read_to_string(&meta_path).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "cannot read search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "malformed search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            parsed
                .get("schema_version")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(1)
        } else {
            // Fresh index dir with no sidecar: assume the current schema.
            // Callers bootstrapping a brand-new index shouldn't need to
            // run `migrate_to` first. The version file is written below.
            SCHEMA_VERSION
        };
        if on_disk_version > SCHEMA_VERSION {
            return Err(SearchError::SchemaVersionMismatch {
                found: on_disk_version,
                expected: SCHEMA_VERSION,
            });
        }
        if on_disk_version < SCHEMA_VERSION {
            return Err(SearchError::SchemaVersionMismatch {
                found: on_disk_version,
                expected: SCHEMA_VERSION,
            });
        }
        let dir = MmapDirectory::open(index_dir)
            .map_err(|e| tantivy::TantivyError::InvalidArgument(e.to_string()))?;
        let index = Index::open_or_create(dir, schema.clone())?;
        let writer = index.writer(50_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        // If we just opened a fresh dir (no sidecar yet), write the
        // current schema version so subsequent opens see a matching
        // version and don't trip the mismatch guard. Idempotent: if
        // the sidecar already existed above, this is a no-op write of
        // the same value.
        if !meta_path.is_file() {
            let payload = serde_json::json!({ "schema_version": SCHEMA_VERSION });
            fs::write(&meta_path, payload.to_string()).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "cannot write search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
        }

        Ok(Self {
            index,
            reader,
            writer: Arc::new(RwLock::new(writer)),
            schema,
            label_resolver: LabelResolver::new(),
        })
    }

    /// Idempotent migration. Reads the stored `search_schema_version.json`,
    /// compares to `SCHEMA_VERSION`, and:
    /// - If the on-disk version is older: drops the index dir and
    ///   calls `reindex(db)` from scratch, then writes the new
    ///   `search_schema_version.json`.
    /// - If equal: no-op.
    /// - If newer: returns `SearchError::SchemaVersionMismatch`.
    /// Returns the previous schema version.
    #[tracing::instrument(level = "info", name = "search.migrate", skip_all)]
    pub async fn migrate_to(
        index_dir: &Utf8Path,
        db: &DatabaseConnection,
    ) -> Result<u32, SearchError> {
        fs::create_dir_all(index_dir).ok();
        let meta_path = index_dir.join("search_schema_version.json");
        let prev_version = if meta_path.is_file() {
            let raw = fs::read_to_string(&meta_path).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "cannot read search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "malformed search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            parsed
                .get("schema_version")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(1)
        } else {
            1
        };
        if prev_version > SCHEMA_VERSION {
            return Err(SearchError::SchemaVersionMismatch {
                found: prev_version,
                expected: SCHEMA_VERSION,
            });
        }
        if prev_version == SCHEMA_VERSION {
            tracing::debug!(
                prev_version,
                "search index schema is current, nothing to migrate"
            );
            return Ok(prev_version);
        }

        tracing::info!(
            prev_version,
            target = SCHEMA_VERSION,
            "migrating search index schema"
        );

        let schema = build_schema();
        let dir = MmapDirectory::open(index_dir)
            .map_err(|e| tantivy::TantivyError::InvalidArgument(e.to_string()))?;
        let index = Index::open_or_create(dir, schema.clone())?;
        let writer = index.writer(50_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let label_resolver = LabelResolver::new();

        // Rebuild + commit; `reindex` is on the SearchIndex struct
        // so we build a temporary one and call through to its method.
        let temp = Self {
            index,
            reader,
            writer: Arc::new(RwLock::new(writer)),
            schema,
            label_resolver,
        };
        temp.reindex(db).await?;

        let meta = serde_json::json!({ "schema_version": SCHEMA_VERSION });
        fs::write(&meta_path, meta.to_string() + "\n").map_err(|e| {
            SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                "cannot write search_schema_version.json at {meta_path}: {e}"
            )))
        })?;

        tracing::info!(
            prev_version,
            target = SCHEMA_VERSION,
            "search index schema migration complete"
        );
        Ok(prev_version)
    }

    /// Progress-aware variant of [`SearchIndex::migrate_to`].
    ///
    /// Same semantics — no-op when on-disk version already equals
    /// [`SCHEMA_VERSION`], rebuild from scratch when older, hard-error
    /// when newer — but fires [`ReindexEvent`]s so the CLI can drive
    /// an `indicatif` bar.
    pub async fn migrate_to_with_progress<F>(
        index_dir: &Utf8Path,
        db: &DatabaseConnection,
        on_event: &mut F,
    ) -> Result<u32, SearchError>
    where
        F: FnMut(crate::write::ReindexEvent) + Send + Sync,
    {
        use crate::write::ReindexEvent;
        use fs_err as fs;

        fs::create_dir_all(index_dir).ok();
        let meta_path = index_dir.join("search_schema_version.json");
        let prev_version = if meta_path.is_file() {
            let raw = fs::read_to_string(&meta_path).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "cannot read search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                    "malformed search_schema_version.json at {meta_path}: {e}"
                )))
            })?;
            parsed
                .get("schema_version")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(1)
        } else {
            1
        };
        if prev_version > SCHEMA_VERSION {
            return Err(SearchError::SchemaVersionMismatch {
                found: prev_version,
                expected: SCHEMA_VERSION,
            });
        }

        if prev_version == SCHEMA_VERSION {
            on_event(ReindexEvent::Indexing { done: 0, total: 0 });
            tracing::debug!(
                prev_version,
                "search index schema is current, nothing to migrate"
            );
            return Ok(prev_version);
        }

        tracing::info!(
            prev_version,
            target = SCHEMA_VERSION,
            "migrating search index schema"
        );

        let schema = crate::schema::build_schema();
        let dir = tantivy::directory::MmapDirectory::open(index_dir)
            .map_err(|e| tantivy::TantivyError::InvalidArgument(e.to_string()))?;
        let index = tantivy::Index::open_or_create(dir, schema.clone())?;
        let writer = index.writer(50_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(tantivy::ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let label_resolver = crate::label_resolver::LabelResolver::new();

        let temp = Self {
            index,
            reader,
            writer: std::sync::Arc::new(tokio::sync::RwLock::new(writer)),
            schema,
            label_resolver,
        };
        on_event(ReindexEvent::Loading);
        temp.reindex_with_progress(db, on_event).await?;

        let meta = serde_json::json!({ "schema_version": SCHEMA_VERSION });
        fs::write(&meta_path, meta.to_string() + "\n").map_err(|e| {
            SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
                "cannot write search_schema_version.json at {meta_path}: {e}"
            )))
        })?;

        tracing::info!(
            prev_version,
            target = SCHEMA_VERSION,
            "search index schema migration complete"
        );
        Ok(prev_version)
    }

    /// A reference to the underlying Tantivy [`Index`]. Exposed for
    /// building parsers and queries outside of [`SearchIndex`].
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// The Tantivy [`Schema`] backing this index. Exposed so the
    /// lookup traits and snippet generators can resolve field
    /// handles without rebuilding the schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }
}
