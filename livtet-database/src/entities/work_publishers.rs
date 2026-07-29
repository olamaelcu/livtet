use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "work_publishers")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub work_id: DbId,
    #[sea_orm(primary_key)]
    pub publisher_id: DbId,
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
        belongs_to = "super::publishers::Entity",
        from = "Column::PublisherId",
        to = "super::publishers::Column::Id"
    )]
    Publisher,
}

impl ActiveModelBehavior for ActiveModel {}
