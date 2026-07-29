use livtet_types::DbId;
use sea_orm::entity::prelude::*;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "installed_plugins")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    #[sea_orm(unique)]
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub manifest_json: String,
    pub source_path: String,
    pub installed_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::plugin_settings::Entity")]
    PluginSettings,
}

impl Related<super::plugin_settings::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PluginSettings.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
