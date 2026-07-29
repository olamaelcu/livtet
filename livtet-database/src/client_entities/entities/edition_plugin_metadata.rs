use livtet_types::DbId;
use sea_orm::entity::prelude::*;

use crate::entities::editions;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "edition_plugin_metadata")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub edition_id: DbId,
    pub plugin_id: String,
    pub key: String,
    pub value: String,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "editions::Entity",
        from = "Column::EditionId",
        to = "editions::Column::Id"
    )]
    Edition,
}

impl Related<editions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Edition.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
