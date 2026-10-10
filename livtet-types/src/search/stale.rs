//! Per-axis resource lookup abstraction used by saved-search composition.
//!
//! The composition engine needs to know whether an ID-backed filter
//! still has a live target in the database — otherwise the rendered
//! query would silently match nothing. We deliberately don't accept a
//! "search every table" trait; instead we dispatch per [`ResourceKind`]
//! so each implementation can hit a single typed query (six `Entity`
//! types, not a UNION fan-out).

use std::collections::HashMap;

use async_trait::async_trait;
use sea_orm::DbErr;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::DbId;

/// Which categorical axis a `DbId` is being looked up on.
///
/// Kept deliberately small: each variant maps 1:1 to a known
/// SeaORM entity (`authors`, `genres`, `subjects`, `series`,
/// `publishers`, `tags`). Adding a new axis means adding a
/// new variant here, and a new arm in the adapter.
///
/// Canonical definition lives here so both `livtet-types`
/// (saved-search composition) and `livtet-search` (the SeaORM
/// resource lookup adapter) agree on the same enum and wire
/// form. `livtet-search` re-exports this type verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub enum ResourceKind {
    Author,
    Genre,
    Subject,
    Series,
    Publisher,
    Tag,
}

impl std::fmt::Display for ResourceKind {
    /// Stable wire form used by remote adapters and persisted JSON.
    /// Matches the identifier column values stored next to each
    /// `kind = "..."` discriminator.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceKind::Author => write!(f, "author"),
            ResourceKind::Genre => write!(f, "genre"),
            ResourceKind::Subject => write!(f, "subject"),
            ResourceKind::Series => write!(f, "series"),
            ResourceKind::Publisher => write!(f, "publisher"),
            ResourceKind::Tag => write!(f, "tag"),
        }
    }
}
#[test]
fn resource_kind_as_str() {
    assert_eq!(ResourceKind::Author.to_string(), "author");
    assert_eq!(ResourceKind::Genre.to_string(), "genre");
    assert_eq!(ResourceKind::Subject.to_string(), "subject");
    assert_eq!(ResourceKind::Series.to_string(), "series");
    assert_eq!(ResourceKind::Publisher.to_string(), "publisher");
    assert_eq!(ResourceKind::Tag.to_string(), "tag");
}

impl std::str::FromStr for ResourceKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "author" => Ok(ResourceKind::Author),
            "genre" => Ok(ResourceKind::Genre),
            "subject" => Ok(ResourceKind::Subject),
            "series" => Ok(ResourceKind::Series),
            "publisher" => Ok(ResourceKind::Publisher),
            "tag" => Ok(ResourceKind::Tag),
            _ => Err(()),
        }
    }
}

#[test]
fn resource_kind_from_str() {
    use std::str::FromStr;

    assert_eq!(Ok(ResourceKind::Author), ResourceKind::from_str("author"));
    assert_eq!(Ok(ResourceKind::Genre), ResourceKind::from_str("genre"));
    assert_eq!(Ok(ResourceKind::Subject), ResourceKind::from_str("subject"));
    assert_eq!(Ok(ResourceKind::Series), ResourceKind::from_str("series"));
    assert_eq!(
        Ok(ResourceKind::Publisher),
        ResourceKind::from_str("publisher")
    );
    assert_eq!(Ok(ResourceKind::Tag), ResourceKind::from_str("tag"));
    assert_eq!(Err(()), ResourceKind::from_str("not-an-axis"));
}

/// The trait every per-axis lookup must satisfy. Implementations
/// should issue one query per call (and one query for `names`)
/// without resorting to a cross-table UNION.
#[async_trait]
pub trait ResourceLookup: Send + Sync {
    /// `true` iff the row identified by `(kind, id)` is still
    /// present in the database.
    async fn exists(&self, kind: ResourceKind, id: DbId) -> Result<bool, DbErr>;

    /// Resolve the human-readable names for a batch of `DbId`s on a
    /// single axis. Missing IDs are simply omitted from the result
    /// map. Returning an empty map for a non-empty input is valid.
    async fn names(&self, kind: ResourceKind, ids: &[DbId])
    -> Result<HashMap<DbId, String>, DbErr>;
}
