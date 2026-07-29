use livtet_types::{DeviceType, PairingStatus};
use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::Constraint;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0002-pairing_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_strict_table(
            manager,
            &Table::create()
                .table(DeviceTypes::Table)
                .if_not_exists()
                .col(pk_db_id(DeviceTypes::Id))
                .col(string(DeviceTypes::Name))
                .col(integer(DeviceTypes::Value))
                .col(timestamp(DeviceTypes::CreatedAt))
                .col(timestamp_null(DeviceTypes::UpdatedAt))
                .to_owned(),
        )
        .await?;

        seed_device_types(manager).await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(PairingStatuses::Table)
                .if_not_exists()
                .col(pk_db_id(PairingStatuses::Id))
                .col(string(PairingStatuses::Name))
                .col(integer(PairingStatuses::Value))
                .col(timestamp(PairingStatuses::CreatedAt))
                .col(timestamp_null(PairingStatuses::UpdatedAt))
                .to_owned(),
        )
        .await?;

        seed_pairing_statuses(manager).await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(PairedDevices::Table)
                .if_not_exists()
                .col(pk_db_id(PairedDevices::DeviceId))
                .col(string_null(PairedDevices::Name))
                .col(string_null(PairedDevices::ListenOn))
                .col(db_id_null(PairedDevices::DeviceTypeId))
                .col(timestamp(PairedDevices::PairedAt))
                .col(timestamp_null(PairedDevices::LastSyncAt))
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::PairedDevicesType.to_string())
                        .from(PairedDevices::Table, PairedDevices::DeviceTypeId)
                        .to(DeviceTypes::Table, DeviceTypes::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;

        create_strict_table(
            manager,
            &Table::create()
                .table(PendingPairings::Table)
                .if_not_exists()
                .col(string(PendingPairings::Token).primary_key())
                .col(db_id(PendingPairings::DesktopId))
                .col(string_null(PendingPairings::ListenOn))
                .col(db_id_null(PendingPairings::StatusId))
                .col(string_null(PendingPairings::DeviceName))
                .col(db_id_null(PendingPairings::DeviceTypeId))
                .col(timestamp(PendingPairings::CreatedAt))
                .col(timestamp(PendingPairings::ExpiresAt))
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::PendingPairingsDeviceType.to_string())
                        .from(PendingPairings::Table, PendingPairings::DeviceTypeId)
                        .to(DeviceTypes::Table, DeviceTypes::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::PendingPairingsStatus.to_string())
                        .from(PendingPairings::Table, PendingPairings::StatusId)
                        .to(PairingStatuses::Table, PairingStatuses::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(PendingPairings::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(PairedDevices::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(PairingStatuses::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(DeviceTypes::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

async fn seed_device_types(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for dt in DeviceType::all() {
        let ulid = dt.ulid();
        let stmt = sea_orm::Statement::from_sql_and_values(
            manager.get_database_backend(),
            r#"
            INSERT OR IGNORE INTO device_types (id, name, value, created_at)
            VALUES ($1, $2, $3, $4)
            "#,
            [
                ulid.to_bytes().to_vec().into(),
                dt.name().into(),
                (dt.discriminant() as i64).into(),
                seed_now().into(),
            ],
        );
        manager.get_connection().execute_raw(stmt).await?;
    }
    Ok(())
}

async fn seed_pairing_statuses(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for ps in PairingStatus::all() {
        let ulid = ps.ulid();
        let stmt = sea_orm::Statement::from_sql_and_values(
            manager.get_database_backend(),
            r#"
            INSERT OR IGNORE INTO pairing_statuses (id, name, value, created_at)
            VALUES ($1, $2, $3, $4)
            "#,
            [
                ulid.to_bytes().to_vec().into(),
                ps.name().into(),
                (ps as i32).into(),
                seed_now().into(),
            ],
        );
        manager.get_connection().execute_raw(stmt).await?;
    }
    Ok(())
}

fn seed_now() -> time::PrimitiveDateTime {
    {
        let n = time::OffsetDateTime::now_utc();
        time::PrimitiveDateTime::new(n.date(), n.time())
    }
}
