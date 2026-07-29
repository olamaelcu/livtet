use livtet_types::{DbId, WorkStatus};
use sea_orm::entity::prelude::*;

/// Current reading status of a work (singleton per work, primary key = work_id).
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "work_status")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub work_id: DbId,
    pub status: WorkStatus,
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
}

impl Related<super::works::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Work.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
