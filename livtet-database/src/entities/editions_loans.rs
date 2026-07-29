use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "editions_loans")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub edition_id: DbId,
    pub loan_entity_id: DbId,
    pub owned_edition_id: Option<DbId>,
    pub loaned_date: time::Date,
    pub due_date: Option<time::Date>,
    pub returned_date: Option<time::Date>,
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
        belongs_to = "super::loan_entity::Entity",
        from = "Column::LoanEntityId",
        to = "super::loan_entity::Column::Id"
    )]
    LoanEntity,
    #[sea_orm(
        belongs_to = "super::owned_edition::Entity",
        from = "Column::OwnedEditionId",
        to = "super::owned_edition::Column::Id"
    )]
    OwnedEdition,
}

impl ActiveModelBehavior for ActiveModel {}
