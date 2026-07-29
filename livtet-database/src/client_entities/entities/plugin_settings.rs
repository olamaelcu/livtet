use livtet_types::DbId;
use sea_orm::entity::prelude::*;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "plugin_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: DbId,
    pub plugin_id: String,
    pub setting_key: String,
    pub value_json: String,
    pub created_at: time::PrimitiveDateTime,
    pub updated_at: Option<time::PrimitiveDateTime>,
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::installed_plugins::Entity",
        from = "Column::PluginId",
        to = "super::installed_plugins::Column::PluginId"
    )]
    InstalledPlugin,
}

impl Related<super::installed_plugins::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::InstalledPlugin.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
