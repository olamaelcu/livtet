use sea_orm_migration::prelude::*;

use super::schema::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0001-core_entities"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Authors::Table)
                    .if_not_exists()
                    .col(pk_db_id(Authors::Id))
                    .col(string(Authors::Name))
                    .to_owned(),
            ),
        )
        .await?;

        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Tags::Table)
                    .if_not_exists()
                    .col(pk_db_id(Tags::Id))
                    .col(string(Tags::Name).unique_key())
                    .to_owned(),
            ),
        )
        .await?;

        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Genres::Table)
                    .if_not_exists()
                    .col(pk_db_id(Genres::Id))
                    .col(string(Genres::Name))
                    .to_owned(),
            ),
        )
        .await?;

        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Subjects::Table)
                    .if_not_exists()
                    .col(pk_db_id(Subjects::Id))
                    .col(string(Subjects::Name))
                    .to_owned(),
            ),
        )
        .await?;

        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Publishers::Table)
                    .if_not_exists()
                    .col(pk_db_id(Publishers::Id))
                    .col(string(Publishers::Name))
                    .col(string_null(Publishers::Website))
                    .col(string_null(Publishers::LogoUrl))
                    .to_owned(),
            ),
        )
        .await?;

        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Series::Table)
                    .if_not_exists()
                    .col(pk_db_id(Series::Id))
                    .col(string(Series::Name))
                    .col(string_null(Series::SortTitle))
                    .col(string_null(Series::SeriesType))
                    .to_owned(),
            ),
        )
        .await?;

        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Formats::Table)
                    .if_not_exists()
                    .col(pk_db_id(Formats::Id))
                    .col(string(Formats::Name))
                    .col(json(Formats::MetadataSchema))
                    .to_owned(),
            ),
        )
        .await?;

        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Languages::Table)
                    .if_not_exists()
                    .col(pk_db_id(Languages::Id))
                    .col(string(Languages::Name))
                    .col(string(Languages::Code))
                    .col(string_null(Languages::FlagEmoji))
                    .to_owned(),
            ),
        )
        .await?;

        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(Identifiers::Table)
                    .if_not_exists()
                    .col(pk_db_id(Identifiers::Id))
                    .col(string(Identifiers::Value).unique_key())
                    .col(string(Identifiers::Kind))
                    .to_owned(),
            ),
        )
        .await?;

        create_strict_table(
            manager,
            &timestamps(
                Table::create()
                    .table(BookConditions::Table)
                    .if_not_exists()
                    .col(pk_db_id(BookConditions::Id))
                    .col(string(BookConditions::Name))
                    .col(integer(BookConditions::Value))
                    .to_owned(),
            ),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(BookConditions::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(Identifiers::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Languages::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Formats::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Series::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(Publishers::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Subjects::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Genres::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Tags::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Authors::Table).if_exists().to_owned())
            .await?;

        Ok(())
    }
}
