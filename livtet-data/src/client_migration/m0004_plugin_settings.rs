use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0004-plugin_settings"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Installed plugins table
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(InstalledPlugins::Table)
                    .if_not_exists()
                    .col(pk_db_id(InstalledPlugins::Id))
                    .col(string(InstalledPlugins::PluginId).unique_key())
                    .col(string(InstalledPlugins::Name))
                    .col(string(InstalledPlugins::Version))
                    .col(string_null(InstalledPlugins::Description))
                    .col(boolean(InstalledPlugins::Enabled))
                    .col(text(InstalledPlugins::ManifestJson))
                    .col(string(InstalledPlugins::SourcePath))
                    .col(timestamp(InstalledPlugins::InstalledAt))
                    .take(),
            ),
        )
        .await?;

        // Plugin settings table
        // PluginId is a raw string matching installed_plugins.plugin_id — not a FK
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(PluginSettings::Table)
                    .if_not_exists()
                    .col(pk_db_id(PluginSettings::Id))
                    .col(string(PluginSettings::PluginId))
                    .col(string(PluginSettings::SettingKey))
                    .col(text(PluginSettings::ValueJson))
                    .take(),
            ),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(PluginSettings::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(InstalledPlugins::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
