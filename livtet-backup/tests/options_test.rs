use std::assert_matches;

use livtet_backup::*;

#[test]
fn backup_options_default_has_all_five_categories() {
    let opts = BackupOptions::default();
    assert_matches!(opts.backup_type, BackupType::Full);
    assert!(opts.include_metadata);
    assert!(opts.output_dir.is_none());
    assert_eq!(
        opts.categories,
        vec![
            ExportCategory::Works,
            ExportCategory::Reading,
            ExportCategory::Inventory,
            ExportCategory::Settings,
            ExportCategory::Series,
        ]
    );
}

#[test]
fn restore_options_default_has_all_five_categories() {
    let opts = RestoreOptions::default();
    assert_matches!(opts.conflict_resolution, ConflictResolution::Skip);
    assert!(!opts.dry_run);
    assert_eq!(
        opts.categories,
        vec![
            ExportCategory::Works,
            ExportCategory::Reading,
            ExportCategory::Inventory,
            ExportCategory::Settings,
            ExportCategory::Series,
        ]
    );
}

#[test]
fn backup_type_full_and_changeset_are_distinct() {
    let full = BackupType::Full;
    let changeset = BackupType::Changeset;
    assert_ne!(full, changeset);
}

#[test]
fn conflict_resolution_skip_and_replace_are_distinct() {
    let skip = ConflictResolution::Skip;
    let replace = ConflictResolution::Replace;
    assert_ne!(skip, replace);
}

#[test]
fn conflict_type_all_variants_exist() {
    let variants = [
        ConflictType::UniqueViolation,
        ConflictType::ForeignKeyViolation,
        ConflictType::CheckViolation,
        ConflictType::NotNullViolation,
        ConflictType::GenericViolation,
    ];
    for i in 0..variants.len() {
        for j in (i + 1)..variants.len() {
            assert_ne!(variants[i], variants[j]);
        }
    }
}
