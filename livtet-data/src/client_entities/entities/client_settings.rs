use sea_orm::entity::prelude::*;

/// The `client_settings` table — a small key/value store for client-side
/// preferences that do not warrant a dedicated table.
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "client_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub value: String,
    pub updated_at: time::PrimitiveDateTime,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
