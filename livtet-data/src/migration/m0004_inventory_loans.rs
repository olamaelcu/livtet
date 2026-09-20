use sea_orm_migration::prelude::*;

use super::schema::*;
use crate::{Constraint, NamedIndex};

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
        //
        // Includes the columns that historically arrived via later
        // migrations (m0007 cover metadata, m0012 file_format) and the
        // m0011 UNIQUE index — squashed into the initial creation now
        // that no production databases exist to preserve.
        //
        // `edition_id` is UNIQUE: digital_inventory is 1:1 with
        // editions (see `uq_digital_inventory_edition_id`, which the
        // `UniqueIndex::DigitalInventoryEdition` error-mapping patterns
        // match on).
        create_strict_table(
            manager,
            &Table::create()
                .table(DigitalInventory::Table)
                .if_not_exists()
                .col(pk_db_id(DigitalInventory::Id))
                .col(db_id(DigitalInventory::EditionId))
                .col(text_null(DigitalInventory::FilePath))
                .col(text_null(DigitalInventory::CoverPath))
                .col(text_null(DigitalInventory::Blurhash))
                .col(text_null(DigitalInventory::DominantColor))
                .col(text_null(DigitalInventory::FileHash))
                .col(big_integer_null(DigitalInventory::FileSizeBytes))
                .col(text(DigitalInventory::FileFormat).default(Expr::val("EPUB")))
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

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("uq_digital_inventory_edition_id")
                    .table(DigitalInventory::Table)
                    .col(DigitalInventory::EditionId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // FK child-column indexes (joins + cascade deletes).
        create_named_index(
            manager,
            NamedIndex::OwnedEditionsEditionId,
            OwnedEditions::Table,
            OwnedEditions::EditionId,
        )
        .await?;
        create_named_index(
            manager,
            NamedIndex::OwnedEditionsConditionId,
            OwnedEditions::Table,
            OwnedEditions::ConditionId,
        )
        .await?;
        create_named_index(
            manager,
            NamedIndex::LoanEntityIdentifiersLoanEntityId,
            LoanEntityIdentifiers::Table,
            LoanEntityIdentifiers::LoanEntityId,
        )
        .await?;
        create_named_index(
            manager,
            NamedIndex::EditionsLoansEditionId,
            EditionsLoans::Table,
            EditionsLoans::EditionId,
        )
        .await?;
        create_named_index(
            manager,
            NamedIndex::EditionsLoansLoanEntityId,
            EditionsLoans::Table,
            EditionsLoans::LoanEntityId,
        )
        .await?;
        create_named_index(
            manager,
            NamedIndex::EditionsLoansOwnedEditionId,
            EditionsLoans::Table,
            EditionsLoans::OwnedEditionId,
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
