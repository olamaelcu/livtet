//! One book placed on one shelf, for one account.
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "book_shelf")]
pub struct Model {
    /// Shelving record URI (e.g. `at://did:plc:…/net.olamaelcu.livtet.biblio.bookShelving/ol.OL997340M`).
    #[sea_orm(primary_key, auto_increment = false)]
    pub uri: String,
    pub shelf_uri: String,
    /// The edition this row shelves: the local `editions.id` as text. The
    /// record's `book.ref` is the edition's AT Protocol rkey, resolved from
    /// the book index when the shelving record is published.
    pub book_uri: String,
    /// `to-read`, `reading`, `read`, or `dnf` — the shelf record's rkey.
    pub status: String,
    pub position: Option<i32>,
    pub notes: Option<String>,
    pub emoji: Option<String>,
    /// Account DID that owns the row.
    pub did: String,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::shelf::Entity",
        from = "Column::ShelfUri",
        to = "super::shelf::Column::Uri"
    )]
    Shelf,
}

impl Related<super::shelf::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Shelf.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
