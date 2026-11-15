//! Audit triggers for `edition_identifiers`.
//!
//! An edition's identifiers are a set of pure link rows, so a pulled edition with
//! no identifier arrives nameless and a note taken on it links to nothing. This
//! migration logs every link so peers can rebuild the set.
//!
//! The payload names the identifier by **value**, never by id: `identifiers.id`
//! is minted locally per `(value, kind)`, so only the value can match across
//! devices. `identifiers.value` is `UNIQUE` on its own, which is what makes that
//! key sound.

use sea_orm_migration::prelude::*;

use super::m0015_sync_reading_progress_annotations::SYNC_APPLY_GUARD;

pub struct Migration;

/// The link table's audit trigger. `edition_identifiers` has no mutable column —
/// a link exists or it does not — so only insert and delete are logged.
pub(crate) fn link_trigger(name: &str, event: &str) -> String {
    let (operation, row) = match name {
        "insert" => ("INSERT", "new"),
        _ => ("DELETE", "old"),
    };
    format!(
        r#"
CREATE TRIGGER IF NOT EXISTS sync_edition_identifier_changelog_{name} {event} ON edition_identifiers
WHEN {guard}
BEGIN
    INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id)
    VALUES (
        'edition_identifier',
        json_object('edition_id', lower(hex({row}.edition_id)), 'value', (SELECT value FROM identifiers WHERE id = {row}.identifier_id)),
        '{operation}',
        (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log),
        json_object(
            'edition_id', lower(hex({row}.edition_id)),
            'kind', (SELECT kind FROM identifiers WHERE id = {row}.identifier_id),
            'value', (SELECT value FROM identifiers WHERE id = {row}.identifier_id)
        ),
        datetime('now'), 'local'
    );
END
"#,
        guard = SYNC_APPLY_GUARD,
    )
}

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "client-0020-sync-edition-identifiers"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        if !manager.has_table("edition_identifiers").await? {
            return Ok(());
        }
        for (name, event) in [("insert", "AFTER INSERT"), ("delete", "AFTER DELETE")] {
            db.execute_unprepared(&format!(
                "DROP TRIGGER IF EXISTS sync_edition_identifier_changelog_{name}"
            ))
            .await?;
            db.execute_unprepared(&link_trigger(name, event)).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for name in ["insert", "delete"] {
            db.execute_unprepared(&format!(
                "DROP TRIGGER IF EXISTS sync_edition_identifier_changelog_{name}"
            ))
            .await?;
        }
        Ok(())
    }
}
