use std::{collections::HashMap, sync::Arc};

use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Convenience alias for `Box<dyn std::error::Error + Send + Sync>`,
/// the trait-object error type shared by `BaselineStore` and its impls.
pub type DynError = Box<dyn std::error::Error + Send + Sync>;

/// Convenience alias for `Arc<dyn BaselineStore>`, the trait-object
/// shape consumed by `BackupDriverImpl::new` and related callers.
pub type SharedBaselineStore = Arc<dyn BaselineStore>;

/// Backup type determining the mechanism used for creating backups
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum BackupType {
    /// Complete database backup via sqlite3_backup_init
    Full,
    /// Incremental changeset via SQLite Session module
    Changeset,
}

/// Conflict resolution strategy for restore operations
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum ConflictResolution {
    /// Skip conflicting rows
    Skip,
    /// Replace conflicting rows
    Replace,
}

/// Category of tables for export/restore operations
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum ExportCategory {
    Works,
    Reading,
    Inventory,
    Settings,
    Series,
}

impl ExportCategory {
    /// Returns the table names for this category
    pub fn tables(&self) -> Vec<&'static str> {
        match self {
            ExportCategory::Works => vec![
                // Core vocabulary
                "authors",
                "tags",
                "genres",
                "subjects",
                "publishers",
                "formats",
                "languages",
                "identifiers",
                // Core entities
                "works",
                "editions",
                // Work junctions
                "work_authors",
                "work_tags",
                "work_genres",
                "work_subjects",
                "work_publishers",
                "work_identifiers",
                // Edition junctions
                "edition_authors",
                "edition_tags",
                "edition_genres",
                "edition_subjects",
                "edition_publishers",
                "edition_identifiers",
            ],
            ExportCategory::Reading => vec![
                "annotations",
                "reading_lists",
                "reading_list_book",
                "reading_progress",
            ],
            ExportCategory::Inventory => vec![
                "digital_inventory",
                "owned_editions",
                "editions_loans",
                "loan_entities",
                "loan_entity_identifiers",
            ],
            ExportCategory::Settings => vec!["series", "work_status"],
            ExportCategory::Series => vec!["series_entries"],
        }
    }
}

/// Options for creating a backup
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Type)]
pub struct BackupOptions {
    pub backup_type: BackupType,
    pub categories: Vec<ExportCategory>,
    pub include_metadata: bool,
    /// Directory the impl should write the backup file into. `None` means
    /// the impl picks a default (typically the current working directory).
    /// The impl always writes to `<output_dir>/backup.db`.
    pub output_dir: Option<camino::Utf8PathBuf>,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            backup_type: BackupType::Full,
            categories: vec![
                ExportCategory::Works,
                ExportCategory::Reading,
                ExportCategory::Inventory,
                ExportCategory::Settings,
                ExportCategory::Series,
            ],
            include_metadata: true,
            output_dir: None,
        }
    }
}

/// Options for restoring from a backup
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Type)]
pub struct RestoreOptions {
    pub conflict_resolution: ConflictResolution,
    pub categories: Vec<ExportCategory>,
    pub dry_run: bool,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            conflict_resolution: ConflictResolution::Skip,
            categories: vec![
                ExportCategory::Works,
                ExportCategory::Reading,
                ExportCategory::Inventory,
                ExportCategory::Settings,
                ExportCategory::Series,
            ],
            dry_run: false,
        }
    }
}

/// Result of a successful backup operation
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Type)]
pub struct BackupResult {
    pub backup_type: BackupType,
    pub tables_backup: HashMap<String, TablePreview>,
    pub total_rows: u64,
    pub file_size_bytes: u64,
    pub duration_ms: u64,
}

/// Result of a successful restore operation
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Type)]
pub struct RestoreResult {
    pub rows_restored: u64,
    pub conflicts: Vec<Conflict>,
    pub duration_ms: u64,
}

/// Represents a conflict that occurred during restore
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Type)]
pub struct Conflict {
    pub table_name: String,
    pub conflict_type: ConflictType,
    pub row_data: HashMap<String, String>,
}

impl Conflict {
    pub fn new(
        table_name: String,
        conflict_type: ConflictType,
        row_data: HashMap<String, String>,
    ) -> Self {
        Self {
            table_name,
            conflict_type,
            row_data,
        }
    }
}

/// Type of conflict encountered during restore
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum ConflictType {
    UniqueViolation,
    ForeignKeyViolation,
    CheckViolation,
    NotNullViolation,
    GenericViolation,
}

/// Preview information for a backup
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Type)]
pub struct BackupPreview {
    pub backup_type: BackupType,
    pub tables: HashMap<String, TablePreview>,
    pub total_rows_estimate: u64,
    pub created_at: time::OffsetDateTime,
}

/// Preview information for a specific table within a backup
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Type)]
pub struct TablePreview {
    pub table_name: String,
    pub row_count: u64,
    pub columns: Vec<String>,
}

impl TablePreview {
    pub fn new(table_name: String, row_count: u64, columns: Vec<String>) -> Self {
        Self {
            table_name,
            row_count,
            columns,
        }
    }
}

/// Errors that can occur during backup operations
#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum BackupError {
    #[error("backup failed: {0}")]
    #[diagnostic(code(livtet_backup::backup_failed))]
    BackupFailed(String),

    #[error("restore failed: {0}")]
    #[diagnostic(code(livtet_backup::restore_failed))]
    RestoreFailed(String),

    #[error("invalid backup data: {0}")]
    #[help("The backup file may be corrupted or from a different version")]
    #[diagnostic(code(livtet_backup::invalid_data))]
    InvalidData(String),

    #[error("IO error: {0}")]
    #[diagnostic(code(livtet_backup::io_error))]
    Io(#[from] std::io::Error),

    #[error("database error: {0}")]
    #[diagnostic(code(livtet_backup::db_error))]
    Database(#[from] sea_orm::DbErr),
}

/// Trait for backup operations - implements the actual backup/restore logic
#[async_trait::async_trait]
pub trait BackupDriver: Send + Sync {
    /// Create a backup with the given options
    async fn create_backup(&self, options: &BackupOptions) -> Result<BackupResult, BackupError>;

    /// Restore from a backup with the given options
    async fn restore(
        &self,
        backup_data: &[u8],
        options: &RestoreOptions,
    ) -> Result<RestoreResult, BackupError>;

    /// Get a preview of a backup without fully restoring it
    async fn get_backup_preview(&self, backup_data: &[u8]) -> Result<BackupPreview, BackupError>;
}

/// Trait for storing and managing baseline snapshots
#[async_trait::async_trait]
pub trait BaselineStore: Send + Sync {
    /// Store a new baseline snapshot
    async fn store_baseline(
        &self,
        name: &str,
        description: Option<&str>,
        backup_data: &[u8],
    ) -> Result<String, DynError>;

    /// Retrieve a baseline by ID
    async fn get_baseline(&self, id: &str) -> Result<Option<Vec<u8>>, DynError>;

    /// Delete a baseline by ID
    async fn delete_baseline(&self, id: &str) -> Result<bool, DynError>;

    /// List all available baselines
    async fn list_baselines(&self) -> Result<Vec<BaselineInfo>, DynError>;
}

/// Information about a stored baseline
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Type)]
pub struct BaselineInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: time::OffsetDateTime,
    pub file_size_bytes: u64,
}
