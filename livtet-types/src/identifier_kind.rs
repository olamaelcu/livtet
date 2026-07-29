//! The URN scheme stored alongside each identifier.
//!
//! Each variant maps to the lowercase string used in the URN itself
//! (`urn:isbn:...` → `kind = "isbn"`). Known schemes are first-class
//! variants so the IPC contract with the frontend's `IdentifierKind`
//! specta type stays narrow. Unknown schemes are preserved as
//! [`IdentifierKind::Custom`] rather than rejected — `parse`
//! round-trips any non-empty string so an old `kind` written by hand
//! (`"wikidata"`, `"openlibrary"`, ...) still loads cleanly into an
//! existing DB.
//!
//! Originally defined in `livtet-core::identifiers`; moved here so the
//! search crate (and any other downstream crate that wants to talk
//! about identifier kinds without pulling in the full `livtet-core`
//! graph) can depend only on `livtet-types`. `livtet-core` re-exports
//! the type for backcompat.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use specta::Type;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierKind {
    Isbn,
    Oclc,
    Lccn,
    Doi,
    /// HTTP/HTTPS identifier. Both `as_str` and the JSON wire
    /// format use the canonical `"http"` form; `parse` accepts
    /// both `"http"` and `"https"` so HTTPS-only URNs round-trip
    /// without the caller having to rewrite them.
    #[serde(rename = "http")]
    Web,
    Opds,
    /// Any other URN scheme (e.g. `"wikidata"`, `"openlibrary"`).
    /// The string is stored verbatim in the `identifiers.kind`
    /// column and round-trips through `as_str` / `parse`.
    Custom(String),
}

impl IdentifierKind {
    /// The string stored in the `identifiers.kind` column. For
    /// [`IdentifierKind::Custom`] this returns the inner scheme
    /// verbatim (so `Custom("wikidata".into()).as_str() == "wikidata"`).
    pub fn as_str(&self) -> &str {
        match self {
            IdentifierKind::Isbn => "isbn",
            IdentifierKind::Oclc => "oclc",
            IdentifierKind::Lccn => "lccn",
            IdentifierKind::Doi => "doi",
            IdentifierKind::Web => "http",
            IdentifierKind::Opds => "opds",
            IdentifierKind::Custom(s) => s.as_str(),
        }
    }

    /// The DSL prefix used when a saved-search references this kind.
    /// Stable strings so user-defined saved searches round-trip even
    /// if the enum gains new variants. `Custom` falls back to its
    /// scheme string verbatim, and unknown reserved schemes (`web`,
    /// `opds`) get a dedicated prefix so they can still be targeted
    /// by name in a composed query.
    pub fn dsl_prefix(&self) -> &str {
        match self {
            IdentifierKind::Isbn => "isbn",
            IdentifierKind::Oclc => "oclc",
            IdentifierKind::Lccn => "lccn",
            IdentifierKind::Doi => "doi",
            IdentifierKind::Web => "web",
            IdentifierKind::Opds => "opds",
            IdentifierKind::Custom(s) => s.as_str(),
        }
    }

    /// Inverse of [`Self::as_str`]. Any non-empty string that isn't
    /// a known scheme is captured as [`IdentifierKind::Custom`] so
    /// existing DB rows load cleanly even after the schema narrows.
    /// The empty string returns `None` (refuse to confuse callers).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "isbn" => Some(IdentifierKind::Isbn),
            "oclc" => Some(IdentifierKind::Oclc),
            "lccn" => Some(IdentifierKind::Lccn),
            "doi" => Some(IdentifierKind::Doi),
            "http" | "https" => Some(IdentifierKind::Web),
            "opds" => Some(IdentifierKind::Opds),
            "custom" => Some(IdentifierKind::Custom("custom".to_string())),
            s if !s.is_empty() => Some(IdentifierKind::Custom(s.to_string())),
            _ => None,
        }
    }
}

impl FromStr for IdentifierKind {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsl_prefix_for_known_kinds() {
        assert_eq!(IdentifierKind::Isbn.dsl_prefix(), "isbn");
        assert_eq!(IdentifierKind::Web.dsl_prefix(), "web");
        assert_eq!(IdentifierKind::Opds.dsl_prefix(), "opds");
    }

    #[test]
    fn dsl_prefix_for_custom_is_the_inner_string() {
        assert_eq!(
            IdentifierKind::Custom("wikidata".into()).dsl_prefix(),
            "wikidata"
        );
    }

    #[test]
    fn parse_roundtrip_via_as_str() {
        for k in [
            IdentifierKind::Isbn,
            IdentifierKind::Oclc,
            IdentifierKind::Lccn,
            IdentifierKind::Doi,
            IdentifierKind::Web,
            IdentifierKind::Opds,
            IdentifierKind::Custom("custom".into()),
        ] {
            assert_eq!(IdentifierKind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn parse_unknown_is_custom() {
        assert_eq!(
            IdentifierKind::parse("not-a-kind"),
            Some(IdentifierKind::Custom("not-a-kind".into()))
        );
        assert!(IdentifierKind::parse("").is_none());
    }

    #[test]
    fn parse_http_https_are_web() {
        assert_eq!(IdentifierKind::parse("http"), Some(IdentifierKind::Web));
        assert_eq!(IdentifierKind::parse("https"), Some(IdentifierKind::Web));
    }
}
