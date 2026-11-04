//! Publication place on an edition's publisher link.
//!
//! Citation styles print the place of publication beside the publisher
//! (`London: Tor, 2011`), and CSL-JSON carries it as `publisher-place`. The
//! place belongs to the *edition* — the 2011 Tor printing is London even
//! though the publisher's head office is New York — which is why it lives on
//! `edition_publishers` rather than on `publishers`.
//!
//! The column is nullable and deliberately has no backfill: no existing table
//! holds a place, so there is nothing to derive it from. It is filled by the
//! importers that write edition/publisher links; links written before this
//! migration read back `NULL` until their import is rerun.

use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "core-0020-publisher-place"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // `TEXT` is STRICT-legal; existing links read back as NULL.
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE edition_publishers ADD COLUMN place TEXT")
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Nothing else references the column, and the place is re-importable.
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE edition_publishers DROP COLUMN place")
            .await
            .map(|_| ())
    }
}
