use std::str::FromStr;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use crate::{
    identifier_kind::IdentifierKind,
    urn::{Urn, UrnParseError},
};

/// One identifier returned by a plugin or stored against an
/// edition or work. The URN string is the canonical wire format
/// (e.g. `urn:isbn:978-0-06-112008-4`); `kind` is the typed enum
/// the storage layer uses to filter and join.
///
/// Construct via [`Identifier::new`] or [`Identifier::parse`]. Both
/// forms accept a URN string and infer the `IdentifierKind` from
/// the URN scheme. Unknown schemes are preserved as
/// [`IdentifierKind::Custom`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(try_from = "String", into = "String")]
pub struct Identifier {
    /// The URN scheme (e.g. `"isbn"`, `"wikidata"`,
    /// `"openlibrary"`). Derived from the URN at parse time.
    pub kind: IdentifierKind,
    /// The full URN string.
    pub urn: Urn,
}

impl Identifier {
    /// Build an `Identifier` from a known kind and a value. The
    /// value is used verbatim (no normalization); pass an
    /// already-cleaned ISBN-13, OCLC number, etc.
    pub fn new(kind: IdentifierKind, value: impl Into<String>) -> Self {
        let scheme = kind.as_str().to_string();
        let urn = Urn::new(scheme, value);
        Self { kind, urn }
    }

    /// Parse a URN string into an `Identifier`. The URN's scheme
    /// must match a known kind or fall back to `Custom(scheme)`.
    pub fn parse(s: &str) -> Result<Self, IdentifierParseError> {
        let urn = Urn::parse(s).map_err(IdentifierParseError::Malformed)?;
        let kind = IdentifierKind::parse(urn.scheme())
            .ok_or_else(|| IdentifierParseError::UnknownKind(urn.scheme().to_string()))?;
        Ok(Self { kind, urn })
    }

    /// The URN string. Equivalent to `self.urn.to_urn_string()`.
    pub fn as_urn_string(&self) -> String {
        self.urn.to_urn_string()
    }

    /// The scheme-specific value (the part after `urn:scheme:`).
    pub fn value(&self) -> &str {
        self.urn.value()
    }
}

impl FromStr for Identifier {
    type Err = IdentifierParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_urn_string())
    }
}

impl TryFrom<String> for Identifier {
    type Error = IdentifierParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl From<Identifier> for String {
    fn from(i: Identifier) -> Self {
        i.as_urn_string()
    }
}

use std::fmt;

/// Errors that can occur when parsing an `Identifier`.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum IdentifierParseError {
    #[error("malformed URN: {0}")]
    Malformed(UrnParseError),
    #[error("URN scheme is not a known identifier kind: {0}")]
    UnknownKind(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_parse_isbn() {
        let id = Identifier::parse("urn:isbn:9780061120084").unwrap();
        assert_eq!(id.kind, IdentifierKind::Isbn);
        assert_eq!(id.value(), "9780061120084");
        assert_eq!(id.as_urn_string(), "urn:isbn:9780061120084");
    }

    #[test]
    fn identifier_parse_oclc() {
        let id = Identifier::parse("urn:oclc:12345").unwrap();
        assert_eq!(id.kind, IdentifierKind::Oclc);
        assert_eq!(id.value(), "12345");
    }

    #[test]
    fn identifier_parse_lccn() {
        let id = Identifier::parse("urn:lccn:2024001234").unwrap();
        assert_eq!(id.kind, IdentifierKind::Lccn);
    }

    #[test]
    fn identifier_parse_doi() {
        let id = Identifier::parse("urn:doi:10.1234/abc").unwrap();
        assert_eq!(id.kind, IdentifierKind::Doi);
    }

    #[test]
    fn identifier_parse_unknown_kind_is_custom() {
        let id = Identifier::parse("urn:wikidata:Q42").unwrap();
        assert_eq!(id.kind, IdentifierKind::Custom("wikidata".to_string()));
        assert_eq!(id.value(), "Q42");
    }

    #[test]
    fn identifier_parse_openlibrary_with_slash_value() {
        let id = Identifier::parse("urn:openlibrary:/works/OL2003619W").unwrap();
        assert_eq!(id.kind, IdentifierKind::Custom("openlibrary".to_string()));
        assert_eq!(id.value(), "/works/OL2003619W");
    }

    #[test]
    fn identifier_parse_invalid() {
        assert!(Identifier::parse("not-a-urn").is_err());
        assert!(Identifier::parse("urn:").is_err());
        assert!(Identifier::parse("urn:isbn").is_err());
    }

    #[test]
    fn identifier_new() {
        let id = Identifier::new(IdentifierKind::Isbn, "9780061120084");
        assert_eq!(id.as_urn_string(), "urn:isbn:9780061120084");
    }

    #[test]
    fn identifier_serde_roundtrip() {
        let original = Identifier::parse("urn:isbn:9780061120084").unwrap();
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"urn:isbn:9780061120084\"");
        let parsed: Identifier = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn identifier_display() {
        let id = Identifier::parse("urn:isbn:9780061120084").unwrap();
        assert_eq!(format!("{id}"), "urn:isbn:9780061120084");
    }

    #[test]
    fn identifier_from_str() {
        let id: Identifier = "urn:oclc:12345".parse().unwrap();
        assert_eq!(id.value(), "12345");
    }
}
