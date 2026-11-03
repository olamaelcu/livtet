use livtet_types::DbId;
use sea_orm::entity::prelude::*;

/// The cover a reader should show for an edition, whether it is a local file or
/// a catalog reference.
///
/// This is a **view**, not a table: `m0018` defines it over
/// `digital_inventory.cover_path` and `editions.cover_image_url`, so it is the
/// one place that knows where a cover can live. `digital_inventory.edition_id`
/// is unique, so an edition appears exactly once.
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "edition_covers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub edition_id: DbId,
    /// Local cover file, when the edition has one on disk.
    pub cover_path: Option<String>,
    /// `inventory` for a file-backed cover, `None` when only a URL exists.
    pub cover_source: Option<String>,
    /// Catalog cover image URL, for editions imported without a file.
    pub cover_url: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
