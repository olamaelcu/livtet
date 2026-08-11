//! Project-specific schema helpers for client migrations.
//!
//! Re-exports all helpers from `sea_orm_migration::schema` and adds
//! `DbId`-aware column builders. Contains only the iden enums for
/// client-owned tables.
// Re-export all upstream schema helpers so migrations only need one import.
use sea_orm_migration::prelude::*;
pub use sea_orm_migration::schema::*;

// Re-export STRICT-table helpers and `DbId` column builders from the
// canonical `migration::schema` module so client migrations share a
// single source of truth with server migrations.
pub use crate::migration::schema::{
    create_strict_table, db_id, db_id_null, pk_db_id, timestamps,
};

// ── Table and Column Identifiers (Client-owned only) ───────────────────

#[derive(DeriveIden)]
pub enum DeviceTypes {
    Table,
    Id,
    Name,
    Value,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum PairingStatuses {
    Table,
    Id,
    Name,
    Value,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum PairedDevices {
    Table,
    DeviceId,
    Name,
    ListenOn,
    DeviceTypeId,
    PairedAt,
    LastSyncAt,
    SessionToken,
}

#[derive(DeriveIden)]
pub enum PendingPairings {
    Table,
    Token,
    DesktopId,
    ListenOn,
    StatusId,
    DeviceName,
    DeviceTypeId,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
pub enum InstalledPlugins {
    Table,
    Id,
    PluginId,
    Name,
    Version,
    Description,
    Enabled,
    ManifestJson,
    SourcePath,
    InstalledAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum PluginSettings {
    Table,
    Id,
    PluginId,
    SettingKey,
    ValueJson,
}

#[derive(DeriveIden)]
pub enum EditionPluginMetadata {
    Table,
    Id,
    EditionId,
    PluginId,
    Key,
    Value,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum ChangeLog {
    Table,
    Id,
    EntityType,
    EntityId,
    Operation,
    Version,
    Payload,
    ChangedAt,
    DeviceId,
}

#[derive(DeriveIden)]
pub enum Editions {
    Table,
    Id,
}

#[derive(DeriveIden)]
pub enum EditionEmbeddings {
    Table,
    Id,
    EditionId,
    Model,
    Dimensions,
    Vector,
}