//! Livtet database crate — consolidated from multiple source crates.
//!
//! ## Module structure
//!
//! - [`entities`] — SeaORM entity models for the server catalog schema
//! - [`client_entities`] — SeaORM entity models for client-owned tables
//!   (device pairing, plugins, sync change-log)
//! - [`migration`] — Database migrations for the server catalog schema
//! - [`client_migration`] — Database migrations for client-owned tables
//! - [`migrator`] — Migration runner abstraction over business + client schemas
//! - [`state`] — Connection pool, SQLite pragmas, `SharedState` + global init/get
//! - [`seed`] — Database seeding for development/testing (behind `fake` feature)
//! - [`test_db`] — Test-only `TestDb` helper
//! - [`error`] — `CoreError` / `CoreResult`
//!
//! ## Features
//!
//! All DB modules compile unconditionally. The `server` / `client` / `core` /
//! `backup` / `plugin` features are empty compatibility no-ops so existing
//! consumer `features = [...]` lists keep resolving. The `fake` feature enables
//! [`seed`].

pub mod constraint;
pub mod db_error;
pub mod error;
pub mod index;
pub mod migrator;
pub mod primary_key;
pub mod unique_index;
pub mod state;
pub mod test_db;

pub mod client_entities;
pub mod client_migration;
pub mod entities;
pub mod migration;

#[cfg(feature = "fake")]
pub mod seed;

// Re-exports for ergonomic use
pub use constraint::Constraint;
pub use db_error::ConstraintViolation;
pub use entities::*;
pub use error::{CoreError, Result as CoreResult};
pub use index::NamedIndex;
pub use migrator::{Kind, connect_with_migrations, run_kinds};
pub use primary_key::PrimaryKey;
pub use unique_index::UniqueIndex;
#[cfg(feature = "fake")]
pub use seed::{SeedConfig, SeedResult, seed_database};
pub use state::{
    SharedState, apply_optimizations, get_state, init_state, is_initialized, optimize_and_close,
    sqlite_pool_options,
};
pub use test_db::TestDb;

pub use sea_orm as orm;
pub use sqlx as sql;

/// Generate a "vocabulary table" SeaORM entity — a small lookup table
/// keyed by `DbId` with `name`, `value`, and timestamp columns.
///
/// Vocabulary tables are the client-side enum-by-ULID tables that back
/// foreign-key references (e.g. `device_types`, `pairing_statuses`).
/// They share an identical shape, so this macro generates the
/// boilerplate (`Model`, `Relation`, `ActiveModelBehavior`) and lets
/// the caller add entity-specific query helpers in the trailing
/// block.
///
/// # Usage
///
/// ```ignore
/// livtet_data::vocab_table!("device_types", {
///     pub async fn display_name_for(db: &DbConn, fk: livtet_types::DbId) -> Result<String, DbErr> {
///         // ...
///     }
/// });
/// ```
///
/// The trailing block is wrapped in `impl Entity { ... }`. Omit it
/// (pass nothing) for tables that need no extra methods.
#[macro_export]
macro_rules! vocab_table {
    ($table_name:literal) => {
        vocab_table!($table_name, {});
    };
    ($table_name:literal, { $($body:tt)* }) => {
        use ::livtet_types::DbId;
        use ::sea_orm::entity::prelude::*;

        $crate::vocab_table!(@model $table_name);

        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}

        impl Entity {
            $($body)*
        }
    };
    (@model $table_name:literal) => {
        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
        #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
        #[sea_orm(table_name = $table_name)]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: DbId,
            pub name: String,
            pub value: i32,
            pub created_at: ::time::PrimitiveDateTime,
            pub updated_at: Option<::time::PrimitiveDateTime>,
        }
    };
}
