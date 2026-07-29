use sea_orm_migration::{prelude::*, schema::*};

use super::schema::ChangeLog;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0001-change_log"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChangeLog::Table)
                    .if_not_exists()
                    .col(pk_auto(ChangeLog::Id))
                    .col(string(ChangeLog::EntityType).not_null())
                    .col(string(ChangeLog::EntityId).not_null())
                    .col(string(ChangeLog::Operation).not_null())
                    .col(integer(ChangeLog::Version).not_null())
                    .col(text(ChangeLog::Payload).not_null())
                    .col(timestamp_with_time_zone(ChangeLog::ChangedAt).not_null())
                    .col(string_null(ChangeLog::DeviceId))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ChangeLog::Table).to_owned())
            .await
    }
}
