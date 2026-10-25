use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "shelf")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
