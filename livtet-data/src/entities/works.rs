use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "works")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub title: String,
    pub description: Option<String>,
    pub sort_title: Option<String>,
    pub series_type: Option<String>,
    pub language_id: Option<DbId>,
    pub preferred_edition_id: Option<DbId>,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::editions::Entity")]
    Editions,
    #[sea_orm(
        belongs_to = "super::editions::Entity",
        from = "Column::PreferredEditionId",
        to = "super::editions::Column::Id"
    )]
    PreferredEdition,
    #[sea_orm(
        belongs_to = "super::languages::Entity",
        from = "Column::LanguageId",
        to = "super::languages::Column::Id"
    )]
    Language,
}

impl Related<super::editions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Editions.def()
    }
}

impl Related<super::languages::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Language.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
