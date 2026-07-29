use std::collections::HashMap;

use livtet_backup::{Conflict, ConflictType};

#[test]
fn conflict_new_preserves_data() {
    let mut data = HashMap::new();
    data.insert("id".into(), "42".into());
    data.insert("title".into(), "Test Book".into());

    let conflict = Conflict::new("works".into(), ConflictType::UniqueViolation, data.clone());

    assert_eq!(conflict.table_name, "works");
    assert_eq!(conflict.conflict_type, ConflictType::UniqueViolation);
    assert_eq!(conflict.row_data, data);
}

#[test]
fn conflict_new_empty_data() {
    let conflict = Conflict::new(
        "editions".into(),
        ConflictType::ForeignKeyViolation,
        HashMap::new(),
    );

    assert_eq!(conflict.table_name, "editions");
    assert!(conflict.row_data.is_empty());
}
