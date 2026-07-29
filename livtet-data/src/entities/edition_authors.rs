use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "edition_authors")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub edition_id: DbId,
    #[sea_orm(primary_key)]
    pub author_id: DbId,
    #[sea_orm(primary_key)]
    pub role: String,
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
    #[sea_orm(
        belongs_to = "super::authors::Entity",
        from = "Column::AuthorId",
        to = "super::authors::Column::Id"
    )]
    Author,
}

impl ActiveModelBehavior for ActiveModel {}
