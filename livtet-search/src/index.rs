//! [`SearchReader`] / [`SearchIndex`] handles: open, migrate, and accessors.
//!
//! Write paths live in [`crate::write`], read paths in [`crate::search`].

use std::ops::Deref;

use camino::Utf8Path;
use fs_err as fs;
use livtet_data::orm::DatabaseConnection;
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, directory::MmapDirectory, schema::Schema,
};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::{
    label_resolver::LabelResolver,
    schema::{SCHEMA_VERSION, build_schema},
};

/// Sidecar file recording the on-disk schema version.
const META_FILE: &str = "search_schema_version.json";

// ---------------------------------------------------------------------------
// SearchReader
// ---------------------------------------------------------------------------

/// Read-only handle to a Tantivy index directory.
///
/// Opening a `SearchReader` never touches the exclusive lock Tantivy takes
/// for an [`IndexWriter`], so any number of readers can be open on the same
/// directory concurrently — within one process or across processes. Every
/// read path (CLI search, OPDS) should use this type; reach for
/// [`SearchIndex`] only when the caller must write.
pub struct SearchReader {
    pub(crate) index: Index,
    pub(crate) reader: IndexReader,
    pub(crate) schema: Schema,
    /// Pre-resolver for format and language IDs → labels.
    pub label_resolver: LabelResolver,
}

// ---------------------------------------------------------------------------
// SearchIndex
// ---------------------------------------------------------------------------

/// Read-write handle: a [`SearchReader`] plus the exclusive Tantivy
/// [`IndexWriter`].
///
/// Only one of these can exist per index directory at a time — Tantivy
/// enforces that with a directory lock, so a second `SearchIndex::open` on
/// the same path (in this process or another) fails. All read paths are
/// shared through [`Deref`], so a `SearchIndex` can be used wherever a
/// [`SearchReader`] is expected.
pub struct SearchIndex {
    pub(crate) shared: SearchReader,
    /// Serialises writer access. Tantivy's queued operations
    /// (`add_document`, `delete_term`) take `&self`, but `commit` needs
    /// `&mut self`, and every public write path here is `&self` because
    /// handles are shared (FFI/NAPI, async tasks). The mutex also keeps one
    /// upsert's delete → add → commit from interleaving with another's.
    pub(crate) writer: Mutex<IndexWriter>,
}

impl Deref for SearchIndex {
    type Target = SearchReader;

    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}

/// Things that can go wrong inside the search index. We collapse
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

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Read the on-disk schema version from the sidecar. `Ok(None)` means the
/// sidecar is absent; a present-but-unreadable or malformed sidecar is an
/// error rather than a silent fallback.
fn read_schema_version(meta_path: &Utf8Path) -> Result<Option<u32>, SearchError> {
    if !meta_path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(meta_path).map_err(|e| {
        SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
            "cannot read {META_FILE} at {meta_path}: {e}"
        )))
    })?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
            "malformed {META_FILE} at {meta_path}: {e}"
        )))
    })?;
    Ok(Some(
        parsed
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(1),
    ))
}

/// Write the current [`SCHEMA_VERSION`] to the sidecar.
fn write_schema_version(meta_path: &Utf8Path) -> Result<(), SearchError> {
    let payload = serde_json::json!({ "schema_version": SCHEMA_VERSION });
    fs::write(meta_path, payload.to_string() + "\n").map_err(|e| {
        SearchError::Tantivy(tantivy::TantivyError::InvalidArgument(format!(
            "cannot write {META_FILE} at {meta_path}: {e}"
        )))
    })
}

/// Open (or create) the Tantivy directory, its [`Index`], and a reader.
///
/// Deliberately guard-free: [`SearchReader::open`] applies the version
/// check itself, while `migrate_to*` must open a stale index to rebuild it.
fn open_index_parts(index_dir: &Utf8Path) -> Result<(Index, IndexReader, Schema), SearchError> {
    fs::create_dir_all(index_dir).ok();
    let schema = build_schema();
    let dir = MmapDirectory::open(index_dir)
        .map_err(|e| tantivy::TantivyError::InvalidArgument(e.to_string()))?;
    let index = Index::open_or_create(dir, schema.clone())?;
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()?;
    Ok((index, reader, schema))
}

// ---------------------------------------------------------------------------
// SearchReader
// ---------------------------------------------------------------------------

impl SearchReader {
    /// Open (or create) the index at `index_dir` for reading only.
    ///
    /// Takes no writer lock, so this coexists with any number of other
    /// readers — and with a live [`SearchIndex`] — on the same directory.
    /// Reads see commits made elsewhere because every search path reloads
    /// the reader first.
    ///
    /// Reads the sidecar `search_schema_version.json` for the on-disk
    /// schema version. If it doesn't match [`SCHEMA_VERSION`], this is a
    /// hard error — callers MUST call [`SearchIndex::migrate_to`] first.
    pub fn open(index_dir: &Utf8Path) -> Result<Self, SearchError> {
        let meta_path = index_dir.join(META_FILE);
        // Fresh dir with no sidecar: assume the current schema. Callers
        // bootstrapping a brand-new index shouldn't need to run
        // `migrate_to` first; the version file is written below.
        let on_disk_version = read_schema_version(&meta_path)?.unwrap_or(SCHEMA_VERSION);
        if on_disk_version != SCHEMA_VERSION {
            return Err(SearchError::SchemaVersionMismatch {
                found: on_disk_version,
                expected: SCHEMA_VERSION,
            });
        }
        let (index, reader, schema) = open_index_parts(index_dir)?;
        // Fresh dir: record the current schema version so subsequent opens
        // see a matching version and don't trip the mismatch guard.
        // Idempotent: a no-op write of the same value when it already
        // existed.
        if !meta_path.is_file() {
            write_schema_version(&meta_path)?;
        }
        Ok(Self {
            index,
            reader,
            schema,
            label_resolver: LabelResolver::new(),
        })
    }

    /// A reference to the underlying Tantivy [`Index`]. Exposed for
    /// building parsers and queries outside of [`SearchReader`].
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

// ---------------------------------------------------------------------------
// SearchIndex
// ---------------------------------------------------------------------------

impl SearchIndex {
    /// Open (or create) the index at `index_dir` for reading and writing.
    ///
    /// Acquires Tantivy's exclusive index-writer lock, so only one
    /// `SearchIndex` may exist per directory at a time, process-wide or
    /// cross-process. A second open fails with a Tantivy lock error — use
    /// [`SearchReader::open`] for concurrent read access.
    pub fn open(index_dir: &Utf8Path) -> Result<Self, SearchError> {
        let shared = SearchReader::open(index_dir)?;
        let writer = shared.index.writer(50_000_000)?;
        Ok(Self {
            shared,
            writer: Mutex::new(writer),
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
        let meta_path = index_dir.join(META_FILE);
        let prev_version = read_schema_version(&meta_path)?.unwrap_or(1);
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

        let (index, reader, schema) = open_index_parts(index_dir)?;
        let writer = index.writer(50_000_000)?;

        // Rebuild + commit; `reindex` is on the SearchIndex struct
        // so we build a temporary one and call through to its method.
        let temp = Self {
            shared: SearchReader {
                index,
                reader,
                schema,
                label_resolver: LabelResolver::new(),
            },
            writer: Mutex::new(writer),
        };
        temp.reindex(db).await?;

        write_schema_version(&meta_path)?;

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
    /// when newer — but fires
    /// [`ReindexEvent`](crate::write::ReindexEvent)s so the CLI can drive
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

        let meta_path = index_dir.join(META_FILE);
        let prev_version = read_schema_version(&meta_path)?.unwrap_or(1);
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

        let (index, reader, schema) = open_index_parts(index_dir)?;
        let writer = index.writer(50_000_000)?;

        let temp = Self {
            shared: SearchReader {
                index,
                reader,
                schema,
                label_resolver: LabelResolver::new(),
            },
            writer: Mutex::new(writer),
        };
        // `reindex_with_progress` fires the `Loading` event itself.
        temp.reindex_with_progress(db, on_event).await?;

        write_schema_version(&meta_path)?;

        tracing::info!(
            prev_version,
            target = SCHEMA_VERSION,
            "search index schema migration complete"
        );
        Ok(prev_version)
    }
}
