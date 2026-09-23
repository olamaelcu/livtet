use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// The `change_log` table — records every entity mutation for sync.
///
/// Every INSERT / UPDATE / DELETE on a syncable entity produces a row
/// here (via SQL triggers).  The sync engine reads from this table to
/// deliver incremental changes to paired devices.
///
/// Unlike most entity models this one uses an auto-increment integer
/// primary key (`id`) and a `String` entity_id (the entity's ULID or
/// a JSON blob for compound-key entities).
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "change_log")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub version: i64,
    pub payload: String,
    pub changed_at: String,
    pub device_id: String,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
