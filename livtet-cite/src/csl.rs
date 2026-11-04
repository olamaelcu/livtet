//! CSL-JSON items and the row-based builder that produces them.
//!
//! [`CslItem`] serialises to the Citation Style Language JSON form consumed
//! by Zotero, Pandoc and `citeproc`. It is the canonical representation in
//! this crate: BibTeX and RIS are rendered from it, never from database rows.
//!
//! [`CslItem::from_rows`] maps already-loaded catalog rows onto the item.
//! The input structs are plain borrowed views, so this crate performs no
//! database access and takes no dependency on the ORM or entity crate.

use serde::{Deserialize, Serialize};

/// A CSL name variable (`author`, `editor`, `translator`).
///
/// CSL names are either structured (`family` + `given`) or a single
/// `literal` string. The builder emits the structured form whenever the
/// source row has either name part, and falls back to `literal` when the
/// row only carries a display name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CslName {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub literal: Option<String>,
}

impl CslName {
    /// Human-readable `Family, Given` form, or the literal name.
    ///
    /// Returns `None` when the name carries no non-empty part.
    pub fn display_name(&self) -> Option<String> {
        let family = self.family.as_deref().filter(|part| !part.is_empty());
        let given = self.given.as_deref().filter(|part| !part.is_empty());
        match (family, given) {
            (Some(family), Some(given)) => Some(format!("{family}, {given}")),
            (Some(family), None) => Some(family.to_owned()),
            (None, Some(given)) => Some(given.to_owned()),
            (None, None) => self.literal.clone().filter(|name| !name.is_empty()),
        }
    }
}

/// CSL `issued` date. Only the year is modelled, so `date-parts` always
/// contains a single `[year]` array.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CslDate {
    #[serde(rename = "date-parts")]
    pub date_parts: Vec<Vec<i32>>,
}

impl CslDate {
    /// A year-only CSL date: `{"date-parts": [[year]]}`.
    pub fn from_year(year: i32) -> Self {
        Self {
            date_parts: vec![vec![year]],
        }
    }

    /// The first year in `date-parts`, if any.
    pub fn first_year(&self) -> Option<i32> {
        self.date_parts
            .first()
            .and_then(|parts| parts.first())
            .copied()
    }
}

fn book_type() -> String {
    "book".to_owned()
}

/// A CSL-JSON item with `type: "book"`.
///
/// Every optional field is omitted from the JSON when unset, and empty name
/// vectors are omitted too, so the output contains no `null` values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CslItem {
    pub id: String,
    #[serde(rename = "type", default = "book_type")]
    pub item_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub author: Vec<CslName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub editor: Vec<CslName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub translator: Vec<CslName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(
        rename = "publisher-place",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub publisher_place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued: Option<CslDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(rename = "ISBN", default, skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
    #[serde(rename = "DOI", default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(rename = "OCLC", default, skip_serializing_if = "Option::is_none")]
    pub oclc: Option<String>,
    #[serde(rename = "LCCN", default, skip_serializing_if = "Option::is_none")]
    pub lccn: Option<String>,
    #[serde(
        rename = "number-of-pages",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub number_of_pages: Option<String>,
}

impl CslItem {
    /// Build the canonical CSL item from already-loaded rows.
    ///
    /// `id` should be a stable identifier for the edition (for example its
    /// URN). The caller supplies the rows; this crate never touches a
    /// database:
    ///
    /// - `rows.authors` — every `edition_authors` link with the author's
    ///   `name`/`family`/`given`; `role` routes each row into `author`,
    ///   `editor`, or `translator`.
    /// - `rows.edition` — the effective title (edition title, falling back
    ///   to the work title), publication year, edition statement, and page
    ///   count.
    /// - `rows.publishers` — `edition_publishers` joined with `publishers`;
    ///   the first named publisher wins, and its `place` becomes
    ///   `publisher-place`.
    /// - `rows.identifiers` — joined `edition_identifiers` + `identifiers`
    ///   rows. Values may be raw or `urn:<kind>:<value>`; the URN prefix is
    ///   stripped.
    /// - `rows.languages` — the edition work's languages; the first
    ///   non-empty `code` becomes the CSL `language`.
    pub fn from_rows(id: impl Into<String>, rows: &CitationRows<'_>) -> Self {
        let (publisher, publisher_place) = publisher_fields(rows.publishers);
        Self {
            id: id.into(),
            item_type: book_type(),
            title: non_empty(rows.edition.title).map(str::to_owned),
            author: names_for(rows.authors, ContributorRole::Author),
            editor: names_for(rows.authors, ContributorRole::Editor),
            translator: names_for(rows.authors, ContributorRole::Translator),
            publisher,
            publisher_place,
            issued: rows.edition.published_year.map(CslDate::from_year),
            edition: non_empty(rows.edition.edition_statement).map(str::to_owned),
            language: rows
                .languages
                .iter()
                .find_map(|language| non_empty(Some(language.code)))
                .map(str::to_owned),
            isbn: identifier_value(rows.identifiers, "isbn"),
            doi: identifier_value(rows.identifiers, "doi"),
            oclc: identifier_value(rows.identifiers, "oclc"),
            lccn: identifier_value(rows.identifiers, "lccn"),
            number_of_pages: rows.edition.number_of_pages.map(|pages| pages.to_string()),
        }
    }

    /// Year of `issued`, if known.
    pub fn issued_year(&self) -> Option<i32> {
        self.issued.as_ref().and_then(CslDate::first_year)
    }

    /// The issued year as text, or `"n.d."` (no date) when absent.
    pub fn year_label(&self) -> String {
        self.issued_year()
            .map(|year| year.to_string())
            .unwrap_or_else(|| "n.d.".to_owned())
    }
}

/// All catalog rows needed to build one [`CslItem`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CitationRows<'a> {
    /// `edition_authors` links joined with `authors`.
    pub authors: &'a [CitationContributor<'a>],
    pub edition: CitationEdition<'a>,
    /// `edition_publishers` links joined with `publishers`.
    pub publishers: &'a [CitationPublisher<'a>],
    /// `edition_identifiers` links joined with `identifiers`.
    pub identifiers: &'a [CitationIdentifier<'a>],
    pub languages: &'a [CitationLanguage<'a>],
}

/// One contributor link from `edition_authors`.
#[derive(Clone, Copy, Debug)]
pub struct CitationContributor<'a> {
    pub role: ContributorRole,
    /// `authors.name` — the whole display name.
    pub name: Option<&'a str>,
    /// `authors.family`, populated by `m0017` and later importers.
    pub family: Option<&'a str>,
    /// `authors.given`, populated by `m0017` and later importers.
    pub given: Option<&'a str>,
}

/// How a contributor participates in the edition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContributorRole {
    Author,
    Editor,
    Translator,
    /// Any role that has no CSL name variable in this crate
    /// (illustrator, narrator, foreword author, ...).
    Other,
}

impl ContributorRole {
    /// Classify the `role` string stored in `edition_authors`.
    pub fn from_db_role(role: &str) -> Self {
        match role.trim().to_ascii_lowercase().as_str() {
            "author" => Self::Author,
            "editor" => Self::Editor,
            "translator" => Self::Translator,
            _ => Self::Other,
        }
    }
}

impl From<&str> for ContributorRole {
    fn from(role: &str) -> Self {
        Self::from_db_role(role)
    }
}

/// The edition-level fields of a citation.
///
/// `title` is the effective title: use `editions.title` when present and
/// fall back to `works.title` otherwise. `published_year` is the year of
/// `editions.published_date`. `edition_statement` is the edition
/// designation (`"2nd"`, `"Revised"`, ...) and is separate from the row's
/// own identifier.
#[derive(Clone, Copy, Debug, Default)]
pub struct CitationEdition<'a> {
    pub title: Option<&'a str>,
    pub published_year: Option<i32>,
    pub edition_statement: Option<&'a str>,
    pub number_of_pages: Option<u32>,
}

/// One `edition_publishers` link joined with `publishers`.
#[derive(Clone, Copy, Debug)]
pub struct CitationPublisher<'a> {
    pub name: Option<&'a str>,
    /// `edition_publishers.place`.
    pub place: Option<&'a str>,
}

/// One `edition_identifiers` link joined with `identifiers`.
#[derive(Clone, Copy, Debug)]
pub struct CitationIdentifier<'a> {
    /// `identifiers.kind` (`"isbn"`, `"oclc"`, `"lccn"`, `"doi"`, ...).
    pub kind: &'a str,
    /// `identifiers.value`, raw or in `urn:<kind>:<value>` form.
    pub value: &'a str,
}

/// A language row from `languages`.
#[derive(Clone, Copy, Debug)]
pub struct CitationLanguage<'a> {
    /// `languages.code` (ISO 639, used as the CSL `language`).
    pub code: &'a str,
}

fn names_for(contributors: &[CitationContributor<'_>], role: ContributorRole) -> Vec<CslName> {
    contributors
        .iter()
        .filter(|contributor| contributor.role == role)
        .filter_map(|contributor| {
            let family = non_empty(contributor.family);
            let given = non_empty(contributor.given);
            if family.is_none() && given.is_none() {
                non_empty(contributor.name).map(|literal| CslName {
                    literal: Some(literal.to_owned()),
                    ..CslName::default()
                })
            } else {
                Some(CslName {
                    family: family.map(str::to_owned),
                    given: given.map(str::to_owned),
                    literal: None,
                })
            }
        })
        .collect()
}

fn publisher_fields(publishers: &[CitationPublisher<'_>]) -> (Option<String>, Option<String>) {
    let first = publishers
        .iter()
        .find(|publisher| non_empty(publisher.name).is_some());
    let name = first
        .and_then(|publisher| non_empty(publisher.name))
        .map(str::to_owned);
    let place = first
        .and_then(|publisher| non_empty(publisher.place))
        .or_else(|| {
            publishers
                .iter()
                .find_map(|publisher| non_empty(publisher.place))
        })
        .map(str::to_owned);
    (name, place)
}

fn identifier_value(identifiers: &[CitationIdentifier<'_>], kind: &str) -> Option<String> {
    identifiers
        .iter()
        .find(|identifier| identifier.kind.eq_ignore_ascii_case(kind))
        .map(|identifier| strip_urn(identifier.value, identifier.kind).trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Strip a matching `urn:<kind>:` prefix from an identifier value.
fn strip_urn<'a>(value: &'a str, kind: &str) -> &'a str {
    let Some(rest) = value.strip_prefix("urn:") else {
        return value;
    };
    let Some((scheme, remainder)) = rest.split_once(':') else {
        return value;
    };
    if scheme.eq_ignore_ascii_case(kind.trim()) {
        remainder
    } else {
        value
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.trim().is_empty())
}
