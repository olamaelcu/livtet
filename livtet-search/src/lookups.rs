//! Lookup traits for categorical IDs alongside the index.

use std::collections::HashMap;

use livtet_data::orm::DatabaseConnection;

pub use livtet_types::search::stale::ResourceKind;

// ---------------------------------------------------------------------------
// Lookup traits
// ---------------------------------------------------------------------------

/// Lookup an individual work or a batch of works by id.
#[async_trait::async_trait]
pub trait WorkLookup: Send + Sync {
    async fn find(
        &self,
        conn: &DatabaseConnection,
        id: livtet_types::DbId,
    ) -> Result<Option<livtet_data::entities::works::Model>, livtet_data::orm::DbErr>;

    async fn find_many(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<Vec<livtet_data::entities::works::Model>, livtet_data::orm::DbErr>;
}

/// Lookup an individual edition or a batch by id, plus the ISBN
/// batch helper used at hit-build time.
#[async_trait::async_trait]
pub trait EditionLookup: Send + Sync {
    async fn find(
        &self,
        conn: &DatabaseConnection,
        id: livtet_types::DbId,
    ) -> Result<Option<livtet_data::entities::editions::Model>, livtet_data::orm::DbErr>;

    async fn find_many(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<Vec<livtet_data::entities::editions::Model>, livtet_data::orm::DbErr>;

    /// Resolve ISBNs for a batch of editions by joining
    /// `edition_identifiers` → `identifiers` where `kind = 'isbn'`.
    /// Each ISBN value is canonicalised to ISBN-13 via
    /// [`livtet_types::Isbn::parse`]; rows that fail to parse are
    /// kept verbatim so they still surface in the result.
    async fn get_edition_isbns(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<HashMap<livtet_types::DbId, Vec<String>>, livtet_data::orm::DbErr>;
}

/// Lookup an individual author or a batch by id.
#[async_trait::async_trait]
pub trait AuthorLookup: Send + Sync {
    async fn find(
        &self,
        conn: &DatabaseConnection,
        id: livtet_types::DbId,
    ) -> Result<Option<livtet_data::entities::authors::Model>, livtet_data::orm::DbErr>;

    async fn find_many(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<Vec<livtet_data::entities::authors::Model>, livtet_data::orm::DbErr>;
}



/// Per-axis existence and name lookup. The SeaORM implementation
/// issues one typed `Entity::find().filter(Column::Id.is_in(...))`
/// query per call — six known tables, no union scans.
#[async_trait::async_trait]
pub trait ResourceLookup: Send + Sync {
    /// Does the given id exist in this axis?
    async fn exists(
        &self,
        conn: &DatabaseConnection,
        kind: ResourceKind,
        id: livtet_types::DbId,
    ) -> Result<bool, livtet_data::orm::DbErr>;

    /// Resolve a batch of ids under a single axis to their display
    /// names. Missing ids are omitted from the result.
    async fn names(
        &self,
        conn: &DatabaseConnection,
        kind: ResourceKind,
        ids: &[livtet_types::DbId],
    ) -> Result<HashMap<livtet_types::DbId, String>, livtet_data::orm::DbErr>;
}
