//! Wire DTOs and shared error type for the livtet sync protocol.
//!
//! This module holds the serde-serialised request/response shapes used
//! by every `/sync/*` HTTP route plus the `SyncChange`/`Conflict`/
//! `PullResponse`/`PushResponse`/`FullDump`/`EntityDump`/`SyncStatus`
//! types that the FFI consumes, and the DDL that materialises the
//! `change_log`/`conflicts` SQLite tables and their 51 audit triggers.

pub mod change_log;
pub mod error;
pub mod syncable_entity;
pub mod types;

pub use error::{Result, SyncError};
pub use syncable_entity::{
    ChangeLogEntity, ENTITY_DUMP_TYPES, SyncableEntity, SyncableEntityKind, SyncedEntity,
    entity_type_to_table,
};
pub use types::{
    Conflict, EntityDump, FullDump, PullResponse, PushResponse, SyncChange, SyncStatus,
};
