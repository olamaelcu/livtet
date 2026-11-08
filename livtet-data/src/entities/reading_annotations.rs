use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// One row per note, plucked from the `reading_progress.annotations` JSON by
/// the `reading_annotations` view. Read-only: writes go through the row.
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "reading_annotations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub edition_id: DbId,
    pub format_id: DbId,
    pub content: String,
    pub location: Option<String>,
    pub quote: Option<String>,
    pub page_number: Option<i32>,
    pub color: Option<String>,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
