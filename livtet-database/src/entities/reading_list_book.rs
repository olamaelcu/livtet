use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "reading_list_book")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub reading_list_id: DbId,
    #[sea_orm(primary_key)]
    pub edition_id: DbId,
    pub position: i32,
    pub added_at: time::PrimitiveDateTime,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::reading_lists::Entity",
        from = "Column::ReadingListId",
        to = "super::reading_lists::Column::Id"
    )]
    ReadingList,
    #[sea_orm(
        belongs_to = "super::editions::Entity",
        from = "Column::EditionId",
        to = "super::editions::Column::Id"
    )]
    Edition,
}

impl ActiveModelBehavior for ActiveModel {}
