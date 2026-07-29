use livtet_backup::BackupError;

#[test]
fn backup_failed_display() {
    let err = BackupError::BackupFailed("disk full".into());
    assert_eq!(format!("{err}"), "backup failed: disk full");
}

#[test]
fn restore_failed_display() {
    let err = BackupError::RestoreFailed("corrupt".into());
    assert_eq!(format!("{err}"), "restore failed: corrupt");
}

#[test]
fn invalid_data_display() {
    let err = BackupError::InvalidData("bad magic".into());
    assert_eq!(format!("{err}"), "invalid backup data: bad magic");
}

#[test]
fn io_error_display() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file gone");
    let err = BackupError::Io(io_err);
    let msg = format!("{err}");
    assert!(msg.starts_with("IO error:"), "got: {msg}");
}

#[test]
fn io_error_from_conversion() {
    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope");
    let err: BackupError = io_err.into();
    let msg = format!("{err}");
    assert!(msg.contains("nope"), "got: {msg}");
}

#[test]
fn database_error_display() {
    let db_err = sea_orm::DbErr::Custom("connection refused".into());
    let err = BackupError::Database(db_err);
    let msg = format!("{err}");
    assert!(msg.starts_with("database error:"), "got: {msg}");
}

#[test]
fn database_error_from_conversion() {
    let db_err = sea_orm::DbErr::RecordNotFound("works".into());
    let err: BackupError = db_err.into();
    let msg = format!("{err}");
    assert!(msg.contains("works"), "got: {msg}");
}
