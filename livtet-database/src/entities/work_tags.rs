use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "work_tags")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub work_id: DbId,
    #[sea_orm(primary_key)]
    pub tag_id: DbId,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::works::Entity",
        from = "Column::WorkId",
        to = "super::works::Column::Id"
    )]
    Work,
    #[sea_orm(
        belongs_to = "super::tags::Entity",
        from = "Column::TagId",
        to = "super::tags::Column::Id"
    )]
    Tag,
}

impl ActiveModelBehavior for ActiveModel {}
