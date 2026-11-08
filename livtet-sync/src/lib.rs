//! Livtet sync domain layer.
//!
//! Contains the local sync engine (which reads/writes the `change_log`
//! and `conflicts` tables owned by `livtet-data`'s client migrations) and
//! the wire DTOs plus syncable-entity registry shared by every transport.
//!
//! This crate owns **no DDL**: the `change_log` / `conflicts` tables and
//! their audit triggers are created by `livtet-data`'s client migrations,
//! and callers must run those migrations before exercising the engine.

mod apply;
pub mod engine;
pub mod types;

// Re-export types for convenience
pub use engine::{ApplyOutcome, SyncEngine, adopt_cover, adopt_inventory};
pub use types::{
    ChangeLogEntity, Conflict, ENTITY_DUMP_TYPES, EntityDump, FullDump, PairLink, PairLinkError,
    PullResponse, PushResponse, Result, SyncChange, SyncError, SyncStatus, SyncableEntity,
    SyncableEntityKind, SyncedEntity, entity_type_to_table,
};

/// Entity types that live in the library but never travel between devices.
///
/// Their rows name this device's own disk — absolute paths, local hashes — so
/// they get no audit triggers, no place in a full dump, and a peer's rows for
/// them are ignored. Accepting an offer writes the receiver's own row instead.
pub const DEVICE_LOCAL_ENTITY_TYPES: &[&str] = &["digital_inventory"];

/// Supported entity types for sync operations.
pub const SUPPORTED_ENTITY_TYPES: &[&str] = &[
    "work",
    "edition",
    "edition_group",
    "series_entry",
    "reading_list",
    "reading_progress",
    "digital_inventory",
    "owned_edition",
    "edition_loan",
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
