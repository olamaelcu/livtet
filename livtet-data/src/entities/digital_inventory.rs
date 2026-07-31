use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "digital_inventory")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    // Schema UNIQUE is driven by m0011, not by this attribute.
    #[sea_orm(unique)]
    pub edition_id: DbId,
    pub file_path: Option<String>,
    pub cover_path: Option<String>,
    pub blurhash: Option<String>,
    pub dominant_color: Option<String>,
    pub file_hash: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub file_format: Option<String>,
    pub notes: Option<String>,
    pub added_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::editions::Entity",
        from = "Column::EditionId",
        to = "super::editions::Column::Id"
    )]
    Edition,
}

impl ActiveModelBehavior for ActiveModel {}
