//! A named shelf (reading-status bucket) owned by one account.
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "shelf")]
pub struct Model {
    /// Shelf record URI (e.g. `at://did:plc:…/net.olamaelcu.livtet.biblio.shelf/to-read`).
    #[sea_orm(primary_key, auto_increment = false)]
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::book_shelf::Entity")]
    BookShelves,
}

impl Related<super::book_shelf::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::BookShelves.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
