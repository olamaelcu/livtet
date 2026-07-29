use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "editions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub work_id: DbId,
    pub group_id: Option<DbId>,
    pub title: Option<String>,
    pub published_date: Option<time::Date>,
    pub format_id: Option<DbId>,
    pub language_id: Option<DbId>,
    pub notes: Option<String>,
    pub description: Option<String>,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
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
        belongs_to = "super::edition_groups::Entity",
        from = "Column::GroupId",
        to = "super::edition_groups::Column::Id"
    )]
    EditionGroup,
}

impl Related<super::works::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Work.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
