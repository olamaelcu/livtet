use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "reading_progress")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub edition_id: DbId,
    pub format_id: DbId,
    pub progress: f64,
    pub progress_unit: Option<String>,
    pub last_location: Option<String>,
    pub total_reading_time_secs: i64,
    pub created_at: time::PrimitiveDateTime,
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
        belongs_to = "super::formats::Entity",
        from = "Column::FormatId",
        to = "super::formats::Column::Id"
    )]
    Format,
}

impl ActiveModelBehavior for ActiveModel {}
