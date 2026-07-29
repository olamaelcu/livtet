use livtet_types::DbId;
use sea_orm::entity::prelude::*;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "work_identifiers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub work_id: DbId,
    #[sea_orm(primary_key, auto_increment = false)]
    pub identifier_id: DbId,
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
        belongs_to = "super::identifiers::Entity",
        from = "Column::IdentifierId",
        to = "super::identifiers::Column::Id"
    )]
    Identifier,
}

impl ActiveModelBehavior for ActiveModel {}
