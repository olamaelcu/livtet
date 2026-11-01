use sea_orm::entity::prelude::*;

/// The `inbound_offers` table — books a paired device has offered this one,
/// waiting on the reader's decision.
///
/// A pull no longer downloads an offer on sight: it records the offer here so
/// the app can ask first. The offer's own catalog rows (`work`, `edition`,
/// `digital_inventory`) are held in `deferred_changes` and applied only once
/// it is accepted, so a declined book never lands in the library.
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "inbound_offers")]
pub struct Model {
    /// The sender's offer id (a ULID string), stable across re-offers.
    #[sea_orm(primary_key, auto_increment = false)]
    pub offer_id: String,
    /// The offering device, as `DbId`'s ULID string form.
    pub from_device_id: String,
    /// Inventory, edition, and work ids in the offer's hex form.
    pub inventory_id: String,
    pub edition_id: String,
    pub work_id: String,
    /// Title and authors travel in the offer payload so the decision dialog
    /// needs no applied catalog row.
    pub title: String,
    /// A JSON array of author names.
    pub authors: String,
    pub file_name: Option<String>,
    pub has_cover: bool,
    /// When the sender recorded the offer.
    pub offered_at: String,
    /// When this daemon first saw it.
    pub received_at: String,
    /// `pending`, `accepted`, or `rejected`.
    pub status: String,
    pub decided_at: Option<String>,
    /// The held `change_log` rows as a JSON array of `SyncChange`, replayed on
    /// acceptance.
    pub deferred_changes: String,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
