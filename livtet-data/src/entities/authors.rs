use livtet_types::DbId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "authors")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub name: String,
    /// Family (last) name, split out of `name` by `m0017` for citation
    /// styles. `NULL` until the split has run or an import supplies it.
    pub family: Option<String>,
    /// Given (fore) names, split out of `name` by `m0017`. `NULL` for a
    /// mononym or until an import supplies the structure.
    pub given: Option<String>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
