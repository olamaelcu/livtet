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

/// Entity types whose wire identity is a *value*, not a locally minted id.
///
/// `edition_identifiers` is a link table keyed by the identifier's value: every
/// device mints its own `identifiers.id`, so the trigger keys the change log by
/// `{edition_id, value}`. `SyncableEntityKind::entity_id` builds the id from the
/// model, and the model holds only the locally minted `identifier_id`, so these
/// types have no variant -- a variant's `entity_id` would name a row no peer has
/// ever seen. `SyncEngine::apply` gives them their own branch; the
/// [`SUPPORTED_ENTITY_TYPES`] entry plus [`entity_type_to_table`] is what pins
/// them to a table and its audit triggers.
pub const VALUE_KEYED_ENTITY_TYPES: &[&str] = &["edition_identifier"];

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
    "edition_identifier",
    "reading_list_book",
];
