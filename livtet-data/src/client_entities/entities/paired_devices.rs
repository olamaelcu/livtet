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
