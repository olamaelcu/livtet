//! Livtet sync crate — consolidated from livtet-sync-client and livtet-sync-server.
//!
//! # Module structure
//!
//! - `client` — local sync engine and HTTP client (feature: `client`)
//! - `server` — Poem-based HTTP server (feature: `server`)
//! - `types` — wire DTOs, error types, and syncable entity definitions

pub mod client;
pub mod server;
pub mod types;

// Re-export types for convenience
pub use types::{
    ChangeLogEntity, Conflict, ENTITY_DUMP_TYPES, EntityDump, FullDump, PullResponse, PushResponse,
    Result, SyncChange, SyncError, SyncStatus, SyncableEntity, SyncableEntityKind, SyncedEntity,
    entity_type_to_table,
};

/// Supported entity types for sync operations.
pub const SUPPORTED_ENTITY_TYPES: &[&str] = &[
    "work",
    "edition",
    "edition_group",
    "series_entry",
    "annotation",
    "reading_list",
    "reading_progress",
    "digital_inventory",
    "owned_edition",
    "editions_loans",
    "work_author",
    "work_tag",
    "work_genre",
    "work_subject",
    "work_publisher",
    "edition_author",
    "edition_tag",
    "edition_genre",
    "edition_subject",
    "edition_publisher",
    "reading_list_book",
];
