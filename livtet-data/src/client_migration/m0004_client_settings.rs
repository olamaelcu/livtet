use sea_orm_migration::prelude::*;

use super::schema::*;

/// Creates the `client_settings` key/value table.
///
/// A tiny namespaced store for client-side preferences that do not warrant
/// their own table (last-sync cursor, UI toggles, etc.). Both columns are
/// plain `TEXT`; `updated_at` records the last write.
///
/// Note: the column uses the `timestamp(..)` helper rather than
/// `timestamp_with_time_zone(..)`. `create_strict_table` normalizes the
/// SeaORM-emitted SQLite type names, and only `timestamp_text` is mapped to
/// a STRICT-legal type — the SQLite builder renders `TimestampWithTimeZone`
/// as `timestamp_with_timezone_text`, which SQLite STRICT rejects. Both
/// helpers collapse to `TEXT` under STRICT, so the stored DDL is identical.
pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0004-client_settings"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_strict_table(
            manager,
            &Table::create()
                .table(ClientSettings::Table)
                .if_not_exists()
                .col(string(ClientSettings::Key).primary_key())
                .col(text(ClientSettings::Value).not_null())
                .col(timestamp(ClientSettings::UpdatedAt).not_null())
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ClientSettings::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}
