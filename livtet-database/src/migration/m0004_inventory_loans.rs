use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::Constraint;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0004-inventory_loans"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. owned_editions
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(OwnedEditions::Table)
                    .if_not_exists()
                    .col(pk_db_id(OwnedEditions::Id))
                    .col(db_id(OwnedEditions::EditionId))
                    .col(date_null(OwnedEditions::AcquiredAt))
                    .col(db_id_null(OwnedEditions::ConditionId))
                    .col(text_null(OwnedEditions::Notes))
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::OwnedEditionsEdition.to_string())
                            .from(OwnedEditions::Table, OwnedEditions::EditionId)
                            .to(Editions::Table, Editions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::OwnedEditionsCondition.to_string())
                            .from(OwnedEditions::Table, OwnedEditions::ConditionId)
                            .to(BookConditions::Table, BookConditions::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        // 2. loan_entities
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(LoanEntity::Table)
                    .if_not_exists()
                    .col(pk_db_id(LoanEntity::Id))
                    .col(string(LoanEntity::Name))
                    .col(text_null(LoanEntity::Notes))
                    .to_owned(),
            ),
        )
        .await?;

        // 3. loan_entity_identifiers
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(LoanEntityIdentifiers::Table)
                    .if_not_exists()
                    .col(pk_db_id(LoanEntityIdentifiers::Id))
                    .col(db_id(LoanEntityIdentifiers::LoanEntityId))
                    .col(string(LoanEntityIdentifiers::Url))
                    .col(string_null(LoanEntityIdentifiers::Label))
                    .foreign_key(
                        ForeignKey::create()
                            .name(Constraint::LoanEntityIdentifiersEntity.to_string())
                            .from(
                                LoanEntityIdentifiers::Table,
                                LoanEntityIdentifiers::LoanEntityId,
                            )
                            .to(LoanEntity::Table, LoanEntity::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            ),
        )
        .await?;

        // 4. editions_loans (no timestamps)
        create_strict_table(
            manager,
            &Table::create()
                .table(EditionsLoans::Table)
                .if_not_exists()
                .col(pk_db_id(EditionsLoans::Id))
                .col(db_id(EditionsLoans::EditionId))
                .col(db_id(EditionsLoans::LoanEntityId))
                .col(db_id_null(EditionsLoans::OwnedEditionId))
                .col(date(EditionsLoans::LoanedDate))
                .col(date_null(EditionsLoans::DueDate))
                .col(date_null(EditionsLoans::ReturnedDate))
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionsLoansEdition.to_string())
                        .from(EditionsLoans::Table, EditionsLoans::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionsLoansLoan.to_string())
                        .from(EditionsLoans::Table, EditionsLoans::LoanEntityId)
                        .to(LoanEntity::Table, LoanEntity::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::EditionsLoansOwned.to_string())
                        .from(EditionsLoans::Table, EditionsLoans::OwnedEditionId)
                        .to(OwnedEditions::Table, OwnedEditions::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;

        // 5. digital_inventory
        create_strict_table(
            manager,
            &Table::create()
                .table(DigitalInventory::Table)
                .if_not_exists()
                .col(pk_db_id(DigitalInventory::Id))
                .col(db_id(DigitalInventory::EditionId))
                .col(text_null(DigitalInventory::FilePath))
                .col(text_null(DigitalInventory::CoverPath))
                .col(text_null(DigitalInventory::FileHash))
                .col(big_integer_null(DigitalInventory::FileSizeBytes))
                .col(text_null(DigitalInventory::Notes))
                .col(timestamp(DigitalInventory::AddedAt).default(Expr::current_timestamp()))
                .col(timestamp_null(DigitalInventory::UpdatedAt))
                .foreign_key(
                    ForeignKey::create()
                        .name(Constraint::DigitalInventoryEdition.to_string())
                        .from(DigitalInventory::Table, DigitalInventory::EditionId)
                        .to(Editions::Table, Editions::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop in reverse creation order
        manager
            .drop_table(
                Table::drop()
                    .table(DigitalInventory::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EditionsLoans::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(LoanEntityIdentifiers::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(LoanEntity::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(OwnedEditions::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
