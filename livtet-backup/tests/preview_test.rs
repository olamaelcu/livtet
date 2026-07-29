use std::collections::HashMap;

use livtet_backup::{BackupPreview, BackupType, TablePreview};

#[test]
fn table_preview_new_preserves_fields() {
    let columns = vec!["id".into(), "title".into(), "created_at".into()];
    let preview = TablePreview::new("works".into(), 99, columns.clone());

    assert_eq!(preview.table_name, "works");
    assert_eq!(preview.row_count, 99);
    assert_eq!(preview.columns, columns);
}

#[test]
fn table_preview_new_empty_columns() {
    let preview = TablePreview::new("tags".into(), 0, vec![]);
    assert_eq!(preview.table_name, "tags");
    assert_eq!(preview.row_count, 0);
    assert!(preview.columns.is_empty());
}

#[test]
fn backup_preview_is_constructable() {
    let mut tables = HashMap::new();
    tables.insert(
        "works".into(),
        TablePreview::new("works".into(), 10, vec!["id".into(), "title".into()]),
    );
    tables.insert(
        "editions".into(),
        TablePreview::new("editions".into(), 20, vec!["id".into()]),
    );

    let preview = BackupPreview {
        backup_type: BackupType::Full,
        tables: tables.clone(),
        total_rows_estimate: 30,
        created_at: time::OffsetDateTime::now_utc(),
    };

    assert_eq!(preview.backup_type, BackupType::Full);
    assert_eq!(preview.tables.len(), 2);
    assert_eq!(preview.total_rows_estimate, 30);
}
