use livtet_cite::{
    CitationContributor, CitationEdition, CitationIdentifier, CitationLanguage, CitationPublisher,
    CitationRows, ContributorRole, CslItem, CslName, to_bibtex, to_bibtex_batch, to_ris,
};

fn full_item() -> CslItem {
    let contributors = [
        CitationContributor {
            role: ContributorRole::Author,
            name: Some("Jane Doe"),
            family: Some("Doe"),
            given: Some("Jane"),
        },
        CitationContributor {
            role: ContributorRole::Author,
            name: Some("John Smith"),
            family: Some("Smith"),
            given: Some("John"),
        },
        CitationContributor {
            role: ContributorRole::Editor,
            name: Some("Alice Editor"),
            family: Some("Editor"),
            given: Some("Alice"),
        },
        CitationContributor {
            role: ContributorRole::Translator,
            name: Some("Ana García"),
            family: Some("García"),
            given: Some("Ana"),
        },
    ];
    let publishers = [
        CitationPublisher {
            name: Some("Acme Press"),
            place: Some("London"),
        },
        CitationPublisher {
            name: Some("Second Press"),
            place: Some("Paris"),
        },
    ];
    let identifiers = [
        CitationIdentifier {
            kind: "isbn",
            value: "urn:isbn:978-3-16-148410-0",
        },
        CitationIdentifier {
            kind: "doi",
            value: "10.1000/xyz123",
        },
        CitationIdentifier {
            kind: "oclc",
            value: "12345",
        },
        CitationIdentifier {
            kind: "lccn",
            value: "2020123456",
        },
    ];
    let languages = [
        CitationLanguage { code: "" },
        CitationLanguage { code: "en" },
    ];
    let rows = CitationRows {
        authors: &contributors,
        edition: CitationEdition {
            title: Some("The Example Book"),
            published_year: Some(2020),
            edition_statement: Some("2nd"),
            number_of_pages: Some(320),
        },
        publishers: &publishers,
        identifiers: &identifiers,
        languages: &languages,
    };
    CslItem::from_rows("example-1", &rows)
}

fn bare_item() -> CslItem {
    let rows = CitationRows {
        edition: CitationEdition {
            title: Some("Bare"),
            ..CitationEdition::default()
        },
        ..CitationRows::default()
    };
    CslItem::from_rows("bare", &rows)
}

fn editor_only_item() -> CslItem {
    let contributors = [CitationContributor {
        role: ContributorRole::Editor,
        name: Some("Jane Doe"),
        family: Some("Doe"),
        given: Some("Jane"),
    }];
    let rows = CitationRows {
        authors: &contributors,
        edition: CitationEdition {
            title: Some("Edited Volume"),
            published_year: Some(2001),
            ..CitationEdition::default()
        },
        ..CitationRows::default()
    };
    CslItem::from_rows("edited", &rows)
}

fn no_date_item() -> CslItem {
    let rows = CitationRows {
        edition: CitationEdition {
            title: Some("Undated"),
            ..CitationEdition::default()
        },
        ..CitationRows::default()
    };
    CslItem::from_rows("undated", &rows)
}

fn same_author_item(id: &str) -> CslItem {
    let contributors = [CitationContributor {
        role: ContributorRole::Author,
        name: Some("Jane Doe"),
        family: Some("Doe"),
        given: Some("Jane"),
    }];
    let rows = CitationRows {
        authors: &contributors,
        edition: CitationEdition {
            title: Some("Same Author"),
            published_year: Some(2020),
            ..CitationEdition::default()
        },
        ..CitationRows::default()
    };
    CslItem::from_rows(id, &rows)
}

#[test]
fn full_field_item_serialises_to_expected_csl_json_and_round_trips() {
    let item = full_item();
    let value = serde_json::to_value(&item).expect("serialise CSL item");

    let expected = serde_json::json!({
        "id": "example-1",
        "type": "book",
        "title": "The Example Book",
        "author": [
            { "family": "Doe", "given": "Jane" },
            { "family": "Smith", "given": "John" }
        ],
        "editor": [{ "family": "Editor", "given": "Alice" }],
        "translator": [{ "family": "García", "given": "Ana" }],
        "publisher": "Acme Press",
        "publisher-place": "London",
        "issued": { "date-parts": [[2020]] },
        "edition": "2nd",
        "language": "en",
        "ISBN": "978-3-16-148410-0",
        "DOI": "10.1000/xyz123",
        "OCLC": "12345",
        "LCCN": "2020123456",
        "number-of-pages": "320"
    });
    assert_eq!(value, expected);

    let json = serde_json::to_string(&item).expect("serialise CSL item");
    assert!(
        !json.contains("null"),
        "CSL-JSON must not contain nulls: {json}"
    );
    let round_tripped: CslItem = serde_json::from_str(&json).expect("deserialise CSL item");
    assert_eq!(round_tripped, item);
}

#[test]
fn absent_fields_are_omitted_from_csl_json() {
    let item = bare_item();
    let value = serde_json::to_value(&item).expect("serialise CSL item");
    let object = value.as_object().expect("CSL item is a JSON object");

    assert_eq!(
        object.len(),
        3,
        "only id, type and title should be present: {object:?}"
    );
    for absent in [
        "author",
        "editor",
        "translator",
        "publisher",
        "publisher-place",
        "issued",
        "edition",
        "language",
        "ISBN",
        "DOI",
        "OCLC",
        "LCCN",
        "number-of-pages",
    ] {
        assert!(!object.contains_key(absent), "{absent} must be omitted");
    }
    assert!(
        !serde_json::to_string(&item)
            .expect("serialise")
            .contains("null")
    );
}

#[test]
fn contributor_without_name_parts_falls_back_to_literal_name() {
    let contributors = [CitationContributor {
        role: ContributorRole::Author,
        name: Some("Plato"),
        family: None,
        given: None,
    }];
    let rows = CitationRows {
        authors: &contributors,
        edition: CitationEdition {
            title: Some("Republic"),
            published_year: Some(-380),
            ..CitationEdition::default()
        },
        ..CitationRows::default()
    };
    let item = CslItem::from_rows("plato", &rows);

    assert_eq!(
        item.author,
        vec![CslName {
            family: None,
            given: None,
            literal: Some("Plato".to_owned()),
        }]
    );
    assert!(to_bibtex(&item).contains("author = {Plato}"));
}

#[test]
fn editor_without_author_renders_in_both_formats() {
    let item = editor_only_item();

    assert!(item.author.is_empty());
    assert_eq!(item.editor.len(), 1);

    let bibtex = to_bibtex(&item);
    assert!(bibtex.contains("editor = {Doe, Jane}"), "{bibtex}");
    assert!(!bibtex.contains("author ="), "{bibtex}");

    let ris = to_ris(&item);
    assert!(ris.contains("A2  - Doe, Jane\n"), "{ris}");
    assert!(!ris.contains("AU  -"), "{ris}");
}

#[test]
fn missing_year_renders_no_date_in_both_formats() {
    let item = no_date_item();

    assert_eq!(item.year_label(), "n.d.");
    assert!(to_bibtex(&item).contains("year = {n.d.}"));
    assert!(to_ris(&item).contains("PY  - n.d.\n"));
}

#[test]
fn bibtex_escapes_special_characters_and_protects_the_title() {
    let title = r"A & B: 100% _C_ {D} $5 #1 ~ ^ \ done";
    let rows = CitationRows {
        edition: CitationEdition {
            title: Some(title),
            ..CitationEdition::default()
        },
        ..CitationRows::default()
    };
    let item = CslItem::from_rows("escaping", &rows);
    let bibtex = to_bibtex(&item);

    assert!(bibtex.contains("title = {{"), "{bibtex}");
    for escaped in [
        r"\&", r"\%", r"\_C\_", r"\{D\}", r"\$5", r"\#1", r"\~", r"\^", r"\\",
    ] {
        assert!(bibtex.contains(escaped), "missing {escaped:?} in {bibtex}");
    }
}

#[test]
fn ris_orders_fields_and_terminates_with_er() {
    let contributors = [CitationContributor {
        role: ContributorRole::Author,
        name: Some("Jane Doe"),
        family: Some("Doe"),
        given: Some("Jane"),
    }];
    let publishers = [CitationPublisher {
        name: Some("Acme Press"),
        place: Some("London"),
    }];
    let identifiers = [
        CitationIdentifier {
            kind: "isbn",
            value: "978-3-16-148410-0",
        },
        CitationIdentifier {
            kind: "doi",
            value: "10.1000/xyz123",
        },
    ];
    let rows = CitationRows {
        authors: &contributors,
        edition: CitationEdition {
            title: Some("Example Book"),
            published_year: Some(2020),
            ..CitationEdition::default()
        },
        publishers: &publishers,
        identifiers: &identifiers,
        ..CitationRows::default()
    };
    let ris = to_ris(&CslItem::from_rows("ris-order", &rows));

    let expected = "\
TY  - BOOK
AU  - Doe, Jane
TI  - Example Book
PB  - Acme Press
CY  - London
PY  - 2020
SN  - 978-3-16-148410-0
DO  - 10.1000/xyz123
ER  -
";
    assert_eq!(ris, expected);
}

#[test]
fn batch_disambiguates_duplicate_base_keys() {
    let batch = to_bibtex_batch(&[same_author_item("id-one"), same_author_item("id-two")]);
    let keys: Vec<&str> = batch
        .lines()
        .filter_map(|line| line.strip_prefix("@book{"))
        .map(|rest| rest.strip_suffix(',').expect("key line ends with a comma"))
        .collect();

    assert_eq!(keys.len(), 2, "{batch}");
    assert_ne!(keys[0], keys[1]);
    for key in &keys {
        assert!(key.starts_with("doe2020-"), "unexpected key {key}");
        assert!(key.len() > "doe2020-".len(), "missing id suffix in {key}");
    }

    // The suffix is id-derived, so re-rendering the batch is byte-identical.
    let repeated = to_bibtex_batch(&[same_author_item("id-one"), same_author_item("id-two")]);
    assert_eq!(batch, repeated);

    // A batch with a unique base keeps the plain key.
    let unique = to_bibtex_batch(&[same_author_item("solo")]);
    assert!(unique.contains("@book{doe2020,"), "{unique}");

    // A single item rendered alone also uses the base key.
    let single = to_bibtex(&same_author_item("id-one"));
    assert!(single.contains("@book{doe2020,"), "{single}");
}
