//! SeaORM-backed implementation of the lookup traits in [`crate`].
//!
//! The design plan mandates six typed queries per `ResourceKind` (one
//! per axis), not a generic UNION scan. This file provides:
//!
//! - [`SeaOrmResourceLookup`] — per-axis existence and name
//!   resolution against the matching SeaORM entity.
//! - `SeaOrmWorkLookup` / `SeaOrmEditionLookup` /
//!   `SeaOrmAuthorLookup` — direct `find_by_id` lookups for the
//!   three primary entities.
//!
//! All identifiers are stored as 16-byte ULIDs in the underlying
//! `BINARY(16)` columns (see `livtet_types::DbId`), so the
//! `is_in(ids)` filters pass the values straight through.

use std::collections::HashMap;

use livtet_data::entities::{
    authors::Entity as Authors, editions::Entity as Editions, genres::Entity as Genres,
    publishers::Entity as Publishers, series::Entity as Series, subjects::Entity as Subjects,
    tags::Entity as Tags, works::Entity as Works,
};
use livtet_data::orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tracing::debug;

use crate::{AuthorLookup, EditionLookup, ResourceKind, ResourceLookup, WorkLookup};

// ---------------------------------------------------------------------------
// Work / Edition / Author — direct find_by_id.
// ---------------------------------------------------------------------------

/// Default work lookup. Calls `Works::find_by_id` for both single
/// and batched lookups.
#[derive(Default, Clone)]
pub struct SeaOrmWorkLookup;

#[async_trait::async_trait]
impl WorkLookup for SeaOrmWorkLookup {
    async fn find(
        &self,
        conn: &DatabaseConnection,
        id: livtet_types::DbId,
    ) -> Result<Option<<Works as EntityTrait>::Model>, livtet_data::orm::DbErr> {
        Works::find_by_id(id).one(conn).await
    }

    async fn find_many(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<Vec<<Works as EntityTrait>::Model>, livtet_data::orm::DbErr> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        Works::find()
            .filter(<Works as EntityTrait>::Column::Id.is_in(ids.to_vec()))
            .all(conn)
            .await
    }
}

/// Default edition lookup.
#[derive(Default, Clone)]
pub struct SeaOrmEditionLookup;

#[async_trait::async_trait]
impl EditionLookup for SeaOrmEditionLookup {
    async fn find(
        &self,
        conn: &DatabaseConnection,
        id: livtet_types::DbId,
    ) -> Result<Option<<Editions as EntityTrait>::Model>, livtet_data::orm::DbErr> {
        Editions::find_by_id(id).one(conn).await
    }

    async fn find_many(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<Vec<<Editions as EntityTrait>::Model>, livtet_data::orm::DbErr> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        Editions::find()
            .filter(<Editions as EntityTrait>::Column::Id.is_in(ids.to_vec()))
            .all(conn)
            .await
    }

    async fn get_edition_isbns(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<HashMap<livtet_types::DbId, Vec<String>>, livtet_data::orm::DbErr> {
        get_edition_isbns_impl(conn, ids).await
    }
}

/// Default author lookup.
#[derive(Default, Clone)]
pub struct SeaOrmAuthorLookup;

#[async_trait::async_trait]
impl AuthorLookup for SeaOrmAuthorLookup {
    async fn find(
        &self,
        conn: &DatabaseConnection,
        id: livtet_types::DbId,
    ) -> Result<Option<<Authors as EntityTrait>::Model>, livtet_data::orm::DbErr> {
        Authors::find_by_id(id).one(conn).await
    }

    async fn find_many(
        &self,
        conn: &DatabaseConnection,
        ids: &[livtet_types::DbId],
    ) -> Result<Vec<<Authors as EntityTrait>::Model>, livtet_data::orm::DbErr> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        Authors::find()
            .filter(<Authors as EntityTrait>::Column::Id.is_in(ids.to_vec()))
            .all(conn)
            .await
    }
}

// ---------------------------------------------------------------------------
// Resource lookup — per-axis typed queries.
// ---------------------------------------------------------------------------

/// Default resource lookup. Each `ResourceKind` dispatches to the
/// matching SeaORM entity via `Column::Id.is_in(ids.to_vec())`. There
/// is no shared UNION path — six known tables, six typed queries.
#[derive(Default, Clone)]
pub struct SeaOrmResourceLookup;

#[async_trait::async_trait]
impl ResourceLookup for SeaOrmResourceLookup {
    async fn exists(
        &self,
        conn: &DatabaseConnection,
        kind: ResourceKind,
        id: livtet_types::DbId,
    ) -> Result<bool, livtet_data::orm::DbErr> {
        let found = match kind {
            ResourceKind::Author => Authors::find_by_id(id).one(conn).await?.is_some(),
            ResourceKind::Genre => Genres::find_by_id(id).one(conn).await?.is_some(),
            ResourceKind::Subject => Subjects::find_by_id(id).one(conn).await?.is_some(),
            ResourceKind::Series => Series::find_by_id(id).one(conn).await?.is_some(),
            ResourceKind::Publisher => Publishers::find_by_id(id).one(conn).await?.is_some(),
            ResourceKind::Tag => Tags::find_by_id(id).one(conn).await?.is_some(),
        };
        Ok(found)
    }

    async fn names(
        &self,
        conn: &DatabaseConnection,
        kind: ResourceKind,
        ids: &[livtet_types::DbId],
    ) -> Result<HashMap<livtet_types::DbId, String>, livtet_data::orm::DbErr> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let pairs: Vec<(livtet_types::DbId, String)> = match kind {
            ResourceKind::Author => Authors::find()
                .filter(<Authors as EntityTrait>::Column::Id.is_in(ids.to_vec()))
                .all(conn)
                .await?
                .into_iter()
                .map(|a| (a.id, a.name))
                .collect(),
            ResourceKind::Genre => Genres::find()
                .filter(<Genres as EntityTrait>::Column::Id.is_in(ids.to_vec()))
                .all(conn)
                .await?
                .into_iter()
                .map(|g| (g.id, g.name))
                .collect(),
            ResourceKind::Subject => Subjects::find()
                .filter(<Subjects as EntityTrait>::Column::Id.is_in(ids.to_vec()))
                .all(conn)
                .await?
                .into_iter()
                .map(|s| (s.id, s.name))
                .collect(),
            ResourceKind::Series => Series::find()
                .filter(<Series as EntityTrait>::Column::Id.is_in(ids.to_vec()))
                .all(conn)
                .await?
                .into_iter()
                .map(|s| (s.id, s.name))
                .collect(),
            ResourceKind::Publisher => Publishers::find()
                .filter(<Publishers as EntityTrait>::Column::Id.is_in(ids.to_vec()))
                .all(conn)
                .await?
                .into_iter()
                .map(|p| (p.id, p.name))
                .collect(),
            ResourceKind::Tag => Tags::find()
                .filter(<Tags as EntityTrait>::Column::Id.is_in(ids.to_vec()))
                .all(conn)
                .await?
                .into_iter()
                .map(|t| (t.id, t.name))
                .collect(),
        };
        debug!(kind = ?kind, count = pairs.len(), "resolved names");
        Ok(pairs.into_iter().collect())
    }
}

// ---------------------------------------------------------------------------
// Free helpers (also exposed to the integration test).
// ---------------------------------------------------------------------------

/// Look up ISBNs for a batch of editions by joining
/// `edition_identifiers` → `identifiers` where `kind = 'isbn'`.
///
/// Each ISBN value is canonicalised to ISBN-13 via
/// [`livtet_types::Isbn::parse`]. Rows whose value fails to parse
/// (malformed checksum, weird length, …) are kept verbatim so
/// the call site still sees the underlying row — the only
/// difference is the canonical form is preferred when the input
/// was a valid ISBN-10 or pre-normalised ISBN-13.
pub async fn get_edition_isbns_impl(
    conn: &DatabaseConnection,
    ids: &[livtet_types::DbId],
) -> Result<HashMap<livtet_types::DbId, Vec<String>>, livtet_data::orm::DbErr> {
    use livtet_data::entities::{
        edition_identifiers::Entity as EditionIds, identifiers::Entity as Identifiers,
    };

    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let junction_rows = EditionIds::find()
        .filter(<EditionIds as EntityTrait>::Column::EditionId.is_in(ids.to_vec()))
        .all(conn)
        .await?;

    if junction_rows.is_empty() {
        return Ok(HashMap::new());
    }

    let identifier_ids: Vec<livtet_types::DbId> =
        junction_rows.iter().map(|r| r.identifier_id).collect();

    let identifier_rows = Identifiers::find()
        .filter(<Identifiers as EntityTrait>::Column::Id.is_in(identifier_ids))
        .all(conn)
        .await?;

    let identifier_by_id: HashMap<livtet_types::DbId, &<Identifiers as EntityTrait>::Model> =
        identifier_rows.iter().map(|i| (i.id, i)).collect();

    let mut out: HashMap<livtet_types::DbId, Vec<String>> = HashMap::new();
    for row in &junction_rows {
        let Some(ident) = identifier_by_id.get(&row.identifier_id) else {
            continue;
        };
        if ident.kind != "isbn" {
            continue;
        }
        let value = match livtet_types::Isbn::parse(&ident.value) {
            Ok(canonical) => canonical.to_string(),
            // Keep the raw value so unparseable ISBN rows are still
            // discoverable through this seam.
            Err(_) => ident.value.clone(),
        };
        out.entry(row.edition_id).or_default().push(value);
    }

    Ok(out)
}
