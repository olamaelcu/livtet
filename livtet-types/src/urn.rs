use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

/// Represents a URN identifier (e.g. `urn:isbn:978-0-06-112008-4`).
///
/// The `type_` field is the URN scheme (`"isbn"`, `"wikidata"`,
/// `"openlibrary"`, etc.) and the `value` is the scheme-specific
/// identifier value. The wire format is the canonical URN string;
/// `Urn::parse` and `Urn::to_urn_string` round-trip the two
/// representations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(try_from = "String", into = "String")]
pub struct Urn {
    /// The URN scheme. Lowercase. Examples: `"isbn"`, `"wikidata"`,
    /// `"openlibrary"`.
    pub type_: String,
    /// The identifier value (the part after the scheme and the
    /// `:` separator). Examples: `"978-0-06-112008-4"`, `"Q193359"`,
    /// `"/works/OL2003619W"`.
    pub value: String,
}

impl Urn {
    /// Construct a URN from a scheme and a value.
    pub fn new(type_: impl Into<String>, value: impl Into<String>) -> Self {
        Urn {
            type_: type_.into(),
            value: value.into(),
        }
    }

    /// Parse a URN string into its components.
    pub fn parse(s: &str) -> Result<Self, UrnParseError> {
        let prefix = "urn:";
        if !s.starts_with(prefix) {
            return Err(UrnParseError::MissingPrefix(s.to_string()));
        }
        let rest = &s[prefix.len()..];
        let colon_pos = rest
            .find(':')
            .ok_or_else(|| UrnParseError::MissingSeparator(s.to_string()))?;
        let type_ = rest[..colon_pos].to_string();
        let value = rest[colon_pos + 1..].to_string();
        if type_.is_empty() {
            return Err(UrnParseError::EmptyScheme(s.to_string()));
        }
        if value.is_empty() {
            return Err(UrnParseError::EmptyValue(s.to_string()));
        }
        Ok(Urn { type_, value })
    }

    /// The URN scheme. Lowercase.
    pub fn scheme(&self) -> &str {
        &self.type_
    }

    /// The URN value (scheme-specific).
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Convert back to URN string format.
    pub fn to_urn_string(&self) -> String {
        format!("urn:{}:{}", self.type_, self.value)
    }
}

impl fmt::Display for Urn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_urn_string())
    }
}

impl FromStr for Urn {
    type Err = UrnParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for Urn {
    type Error = UrnParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl From<Urn> for String {
    fn from(u: Urn) -> Self {
        u.to_urn_string()
    }
}

/// Errors that can occur when parsing a URN string.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum UrnParseError {
    #[error("URN must start with 'urn:': {0}")]
    MissingPrefix(String),
    #[error("URN is missing ':' separator after scheme: {0}")]
    MissingSeparator(String),
    #[error("URN scheme is empty: {0}")]
    EmptyScheme(String),
    #[error("URN value is empty: {0}")]
    EmptyValue(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urn_new_and_to_string() {
        let urn = Urn::new("isbn", "978-0-06-112008-4");
        assert_eq!(urn.to_urn_string(), "urn:isbn:978-0-06-112008-4");
    }

    #[test]
    fn urn_parse_valid() {
        let urn = Urn::parse("urn:wikidata:Q193359").unwrap();
        assert_eq!(urn.type_, "wikidata");
        assert_eq!(urn.value, "Q193359");
    }

    #[test]
    fn urn_parse_roundtrip() {
        let original = "urn:isbn:978-0-06-112008-4";
        let parsed = Urn::parse(original).unwrap();
        assert_eq!(parsed.to_urn_string(), original);
    }

    #[test]
    fn urn_parse_invalid() {
        assert!(matches!(
            Urn::parse("not-a-urn"),
            Err(UrnParseError::MissingPrefix(_))
        ));
        assert!(matches!(
            Urn::parse("urn:"),
            Err(UrnParseError::MissingSeparator(_))
        ));
        assert!(matches!(
            Urn::parse("urn:isbn"),
            Err(UrnParseError::MissingSeparator(_))
        ));
        assert!(matches!(
            Urn::parse("urn::value"),
            Err(UrnParseError::EmptyScheme(_))
        ));
        assert!(matches!(
            Urn::parse("urn:isbn:"),
            Err(UrnParseError::EmptyValue(_))
        ));
    }

    #[test]
    fn urn_serde_json_roundtrip() {
        let original = Urn::parse("urn:isbn:978-0-06-112008-4").unwrap();
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"urn:isbn:978-0-06-112008-4\"");
        let parsed: Urn = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn urn_serde_rejects_malformed() {
        let result: Result<Urn, _> = serde_json::from_str("\"not-a-urn\"");
        assert!(result.is_err());
    }

    #[test]
    fn urn_display() {
        let urn = Urn::parse("urn:isbn:9780061120084").unwrap();
        assert_eq!(format!("{urn}"), "urn:isbn:9780061120084");
    }

    #[test]
    fn urn_from_str() {
        let urn: Urn = "urn:openlibrary:/works/OL2003619W".parse().unwrap();
        assert_eq!(urn.scheme(), "openlibrary");
        assert_eq!(urn.value(), "/works/OL2003619W");
    }

    #[test]
    fn urn_with_slash_value() {
        let urn = Urn::parse("urn:openlibrary:/works/OL2003619W").unwrap();
        assert_eq!(urn.type_, "openlibrary");
        assert_eq!(urn.value, "/works/OL2003619W");
    }
}
