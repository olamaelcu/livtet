use livtet_backup::ExportCategory;

#[test]
fn works_tables() {
    let tables = ExportCategory::Works.tables();
    assert_eq!(
        tables,
        vec![
            "authors",
            "tags",
            "genres",
            "subjects",
            "publishers",
            "formats",
            "languages",
            "identifiers",
            "works",
            "editions",
            "work_authors",
            "work_tags",
            "work_genres",
            "work_subjects",
            "work_publishers",
            "work_identifiers",
            "edition_authors",
            "edition_tags",
            "edition_genres",
            "edition_subjects",
            "edition_publishers",
            "edition_identifiers",
        ]
    );
}

#[test]
fn reading_tables() {
    let tables = ExportCategory::Reading.tables();
    assert_eq!(
        tables,
        vec![
            "annotations",
            "reading_lists",
            "reading_list_book",
            "reading_progress",
        ]
    );
}

#[test]
fn inventory_tables() {
    let tables = ExportCategory::Inventory.tables();
    assert_eq!(
        tables,
        vec![
            "digital_inventory",
            "owned_editions",
            "editions_loans",
            "loan_entities",
            "loan_entity_identifiers",
        ]
    );
}

#[test]
fn settings_tables() {
    let tables = ExportCategory::Settings.tables();
    assert_eq!(tables, vec!["series", "work_status"]);
}

#[test]
fn series_tables() {
    let tables = ExportCategory::Series.tables();
    assert_eq!(tables, vec!["series_entries"]);
}
