use livtet_types::{Address, DbId};
use sea_orm::entity::prelude::*;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "paired_devices")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub device_id: DbId,
    pub name: Option<String>,
    pub listen_on: Option<Address>,
    pub device_type_id: Option<DbId>,
    pub paired_at: time::PrimitiveDateTime,
    pub last_sync_at: Option<time::PrimitiveDateTime>,
    /// Whether this device may pull from the peer.
    ///
    /// Set when this device files the claim, and also when it approves one: a
    /// pairing is mutual, so the token both sides store lets either dial the
    /// other by node id. A row whose claim predates node ids stays `false`
    /// because it has no address to dial.
    pub outbound: bool,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::device_types::Entity",
        from = "Column::DeviceTypeId",
        to = "super::device_types::Column::Id"
    )]
    DeviceType,
}

impl ActiveModelBehavior for ActiveModel {}
