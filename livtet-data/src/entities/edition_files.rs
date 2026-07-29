use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "edition_files")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub edition_id: DbId,
    pub file_path: String,
    pub file_format: String,
    pub file_size_bytes: Option<i64>,
    pub file_last_modified: Option<String>,
    pub file_mode: String,
    pub source_plugin: String,
    pub source_id: Option<String>,
    pub created_at: time::PrimitiveDateTime,
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
