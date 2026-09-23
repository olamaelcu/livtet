//! Client-owned table identifiers.
//!
//! Contains only the `DeriveIden` enums the client migrations use. The
//! STRICT-table and `DbId`-aware column helpers are shared with the
//! business schema, so they are re-exported from [`crate::migration::schema`]
//! instead of being duplicated here.

use sea_orm_migration::prelude::*;

// Shared helpers: STRICT-table creation and `DbId` column builders.
pub use crate::migration::schema::{create_strict_table, db_id, db_id_null, pk_db_id};
// Upstream column builders (`string`, `text`, `integer`, `timestamp`, …).
pub use sea_orm_migration::schema::*;

// ── Table and Column Identifiers (client-owned only) ───────────────────

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
    DeviceId,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
pub enum ClientSettings {
    Table,
    Key,
    Value,
    UpdatedAt,
}

// ── Existence helpers ─────────────────────────────────────────────────
//
// Migrations sometimes need to inspect the live schema before issuing DDL
// (e.g. `ALTER TABLE ... ADD COLUMN` is not idempotent). These helpers
// answer "does X already exist?" straight from SQLite's catalog, so the
// answer reflects what is actually present rather than what the migration
// bookkeeping believes.

/// True when a table named `table` exists in the database schema.
pub async fn table_exists(manager: &SchemaManager<'_>, table: &str) -> Result<bool, DbErr> {
    let stmt = sea_orm::Statement::from_sql_and_values(
        manager.get_database_backend(),
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
        [sea_orm::Value::from(table)],
    );
    Ok(manager
        .get_connection()
        .query_one_raw(stmt)
        .await?
        .is_some())
}

/// True when an index named `index` exists in the database schema.
pub async fn index_exists(manager: &SchemaManager<'_>, index: &str) -> Result<bool, DbErr> {
    let stmt = sea_orm::Statement::from_sql_and_values(
        manager.get_database_backend(),
        "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?",
        [sea_orm::Value::from(index)],
    );
    Ok(manager
        .get_connection()
        .query_one_raw(stmt)
        .await?
        .is_some())
}

/// True when `column` exists on `table`.
pub async fn column_exists(
    manager: &SchemaManager<'_>,
    table: &str,
    column: &str,
) -> Result<bool, DbErr> {
    let stmt = sea_orm::Statement::from_sql_and_values(
        manager.get_database_backend(),
        "SELECT 1 FROM pragma_table_info(?) WHERE name = ?",
        [sea_orm::Value::from(table), sea_orm::Value::from(column)],
    );
    Ok(manager
        .get_connection()
        .query_one_raw(stmt)
        .await?
        .is_some())
}
