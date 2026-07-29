use livtet_types::{Address, DbId};
use sea_orm::entity::prelude::*;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "pending_pairings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub token: String,
    pub desktop_id: DbId,
    pub listen_on: Option<Address>,
    pub status_id: Option<DbId>,
    pub device_name: Option<String>,
    pub device_type_id: Option<DbId>,
    pub created_at: time::PrimitiveDateTime,
    pub expires_at: time::PrimitiveDateTime,
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
    #[sea_orm(
        belongs_to = "super::pairing_statuses::Entity",
        from = "Column::StatusId",
        to = "super::pairing_statuses::Column::Id"
    )]
    PairingStatus,
}

impl ActiveModelBehavior for ActiveModel {}
