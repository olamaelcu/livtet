use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// A page-level bookmark: a location a reader flagged without any note text or
/// highlight colour.
///
/// Mirrors [`super::annotations`] in intent (a per-edition marker a reader
/// returns to) but is deliberately its own table: a bookmark carries no
/// content, so it has no `user_id`, `content` or update timestamp.
///
/// The position is encoded exactly like [`super::reading_progress`]:
/// `progress` carries the number, `progress_unit` names its unit
/// (`"ratio"`, `"page"`, `"timestamp"`, `"epub"`, …), and `last_location`
/// carries the opaque locator (a serialized Readium locator for EPUB). One
/// shape therefore covers EPUBs, PDFs, physical page numbers and audiobook
/// positions alike.
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "reading_bookmarks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub edition_id: DbId,
    pub progress: f64,
    pub progress_unit: Option<String>,
    pub last_location: Option<String>,
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
}

impl ActiveModelBehavior for ActiveModel {}
