use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "book_shelf")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub uri: String,
    pub shelf_uri: String,
    pub book_uri: String,
    pub status: String,
    pub position: Option<i32>,
    pub notes: Option<String>,
    pub emoji: Option<String>,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
    pub did: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::shelf::Entity",
        from = "Column::ShelfUri",
        to = "super::shelf::Column::Uri"
    )]
    Shelf,
}

impl ActiveModelBehavior for ActiveModel {}
