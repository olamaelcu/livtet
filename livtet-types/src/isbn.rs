use std::{
    fmt::{self, Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

/// A validated ISBN in canonical ISBN-13 form.
///
/// Construct via [`Isbn::parse`] or `"…".parse::<Isbn>()` for any
/// untrusted source. Both forms accept ISBN-10 and ISBN-13 input with
/// hyphens, spaces, mixed whitespace, and `ISBN:` / `ISBN-10:` /
/// `ISBN-13:` prefixes; the resulting value is always normalized to a
/// 13-digit string with a valid ISBN-13 check digit.
///
/// Use [`Isbn::new_unchecked`] only when the caller has already
/// validated the string (e.g. a freshly parsed form field).
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct Isbn(String);

impl Isbn {
    pub fn parse(s: &str) -> Result<Self, IsbnError> {
        let cleaned = clean(s);

        if cleaned.is_empty() {
            return Err(IsbnError::Empty);
        }

        if let Some(bad) = cleaned.chars().find(|c| !is_isbn_char(*c)) {
            return Err(IsbnError::InvalidCharacter(bad));
        }

        let len = cleaned.chars().count();
        match len {
            10 => {
                if !is_valid_isbn10(&cleaned) {
                    return Err(IsbnError::InvalidChecksum10);
                }
                Ok(Self(upgrade_isbn10_to_13(&cleaned)))
            }
            13 => {
                if !is_valid_isbn13(&cleaned) {
                    return Err(IsbnError::InvalidChecksum13);
                }
                Ok(Self(cleaned))
            }
            other => Err(IsbnError::InvalidLength(other)),
        }
    }

    pub fn new_unchecked(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_13(&self) -> bool {
        self.0.chars().count() == 13
    }
}

impl Display for Isbn {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Isbn {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for Isbn {
    type Err = IsbnError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum IsbnError {
    #[error("ISBN is empty")]
    Empty,
    #[error("ISBN has invalid length: {0} digits (expected 10 or 13)")]
    InvalidLength(usize),
    #[error("ISBN contains non-digit characters: {0}")]
    InvalidCharacter(char),
    #[error("ISBN-10 checksum is invalid")]
    InvalidChecksum10,
    #[error("ISBN-13 checksum is invalid")]
    InvalidChecksum13,
}

fn clean(s: &str) -> String {
    let trimmed = s.trim();
    let stripped = strip_isbn_prefix(trimmed);
    stripped
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect()
}

fn strip_isbn_prefix(s: &str) -> &str {
    const PREFIXES: &[&str] = &["isbn-13:", "isbn-10:", "isbn:"];
    let lower = s.to_ascii_lowercase();
    for prefix in PREFIXES {
        if lower.starts_with(prefix) {
            return &s[prefix.len()..];
        }
    }
    s
}

fn is_isbn_char(c: char) -> bool {
    c.is_ascii_digit() || c == 'X' || c == 'x'
}

fn is_valid_isbn10(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() != 10 {
        return false;
    }
    if !chars.iter().take(9).all(|c| c.is_ascii_digit()) {
        return false;
    }
    let last = chars[9];
    if !(last.is_ascii_digit() || last == 'X' || last == 'x') {
        return false;
    }

    let sum: u32 = chars
        .iter()
        .take(9)
        .enumerate()
        .map(|(i, c)| c.to_digit(10).unwrap_or(0) * (10 - i as u32))
        .sum();
    let last_value = if last == 'X' || last == 'x' {
        10
    } else {
        last.to_digit(10).unwrap_or(0)
    };
    (sum + last_value).is_multiple_of(11)
}

fn is_valid_isbn13(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() != 13 {
        return false;
    }
    if !chars.iter().all(|c| c.is_ascii_digit()) {
        return false;
    }

    let sum: u32 = chars
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let digit = c.to_digit(10).unwrap_or(0);
            let weight = if i % 2 == 0 { 1 } else { 3 };
            digit * weight
        })
        .sum();
    sum.is_multiple_of(10)
}

fn upgrade_isbn10_to_13(isbn10: &str) -> String {
    let chars: Vec<char> = isbn10.chars().collect();
    let core: String = chars.iter().take(9).collect();
    let twelve = format!("978{core}");
    let check = isbn13_check_digit(&twelve);
    format!("{twelve}{check}")
}

fn isbn13_check_digit(twelve: &str) -> char {
    let chars: Vec<char> = twelve.chars().collect();
    let sum: u32 = chars
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let digit = c.to_digit(10).unwrap_or(0);
            let weight = if i % 2 == 0 { 1 } else { 3 };
            digit * weight
        })
        .sum();
    let check = (10 - (sum % 10)) % 10;
    char::from_digit(check, 10).unwrap_or('0')
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn parse_isbn13_with_hyphens() {
        let isbn = Isbn::parse("978-0-06-112008-4").unwrap();
        assert_eq!(isbn.as_str(), "9780061120084");
    }

    #[test]
    fn parse_isbn13_with_prefix() {
        let isbn = Isbn::parse("ISBN-13: 978-0-06-112008-4").unwrap();
        assert_eq!(isbn.as_str(), "9780061120084");
    }

    #[test]
    fn parse_isbn10_canonicalized_to_13() {
        let isbn = Isbn::parse("0-306-40615-2").unwrap();
        assert_eq!(isbn.as_str(), "9780306406157");
    }

    #[test]
    fn parse_isbn13_bad_checksum() {
        let err = Isbn::parse("978-0-06-112008-0").unwrap_err();
        assert_eq!(err, IsbnError::InvalidChecksum13);
    }

    #[test]
    fn parse_too_short() {
        let err = Isbn::parse("123").unwrap_err();
        assert_eq!(err, IsbnError::InvalidLength(3));
    }

    #[test]
    fn parse_invalid_character() {
        let err = Isbn::parse("978-ABC-DEFGHI-J").unwrap_err();
        assert_matches!(err, IsbnError::InvalidCharacter(_));
    }

    #[test]
    fn parse_isbn10_with_x_check_digit() {
        // Programming Perl, ISBN-10 "020161622X"
        let isbn = Isbn::parse("0-201-61622-X").unwrap();
        assert_eq!(isbn.as_str(), "9780201616224");
    }

    #[test]
    fn parse_isbn10_with_lowercase_x() {
        let isbn = Isbn::parse("0-201-61622-x").unwrap();
        assert_eq!(isbn.as_str(), "9780201616224");
    }

    #[test]
    fn parse_isbn10_bad_checksum() {
        let err = Isbn::parse("0-306-40615-0").unwrap_err();
        assert_eq!(err, IsbnError::InvalidChecksum10);
    }

    #[test]
    fn parse_isbn13_with_spaces() {
        let isbn = Isbn::parse("978 0 06 112008 4").unwrap();
        assert_eq!(isbn.as_str(), "9780061120084");
    }

    #[test]
    fn parse_isbn13_with_isbn_prefix() {
        let isbn = Isbn::parse("ISBN:9780061120084").unwrap();
        assert_eq!(isbn.as_str(), "9780061120084");
    }

    #[test]
    fn parse_isbn13_with_isbn10_prefix() {
        let isbn = Isbn::parse("ISBN-10: 0-306-40615-2").unwrap();
        assert_eq!(isbn.as_str(), "9780306406157");
    }

    #[test]
    fn parse_empty_string() {
        assert_eq!(Isbn::parse(""), Err(IsbnError::Empty));
    }

    #[test]
    fn parse_only_whitespace() {
        assert_eq!(Isbn::parse("   "), Err(IsbnError::Empty));
    }

    #[test]
    fn parse_leading_trailing_whitespace() {
        let isbn = Isbn::parse("  9780061120084  ").unwrap();
        assert_eq!(isbn.as_str(), "9780061120084");
    }

    #[test]
    fn from_str_impl() {
        let isbn: Isbn = "9780061120084".parse().unwrap();
        assert_eq!(isbn.as_str(), "9780061120084");
    }

    #[test]
    fn new_unchecked_skips_validation() {
        let isbn = Isbn::new_unchecked("anything");
        assert_eq!(isbn.as_str(), "anything");
    }

    #[test]
    fn display() {
        let isbn = Isbn::new_unchecked("9780061120084");
        assert_eq!(format!("{isbn}"), "9780061120084");
    }

    #[test]
    fn as_ref_str() {
        let isbn = Isbn::new_unchecked("9780061120084");
        let s: &str = isbn.as_ref();
        assert_eq!(s, "9780061120084");
    }

    #[test]
    fn is_13_returns_true_for_13_digit() {
        let isbn = Isbn::parse("9780061120084").unwrap();
        assert!(isbn.is_13());
    }

    #[test]
    fn serde_json_roundtrip() {
        let isbn = Isbn::new_unchecked("9780061120084");
        let json = serde_json::to_string(&isbn).unwrap();
        assert_eq!(json, "\"9780061120084\"");
        let parsed: Isbn = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, isbn);
    }

    #[test]
    fn serde_json_transparent_deserialize() {
        let parsed: Isbn = serde_json::from_str("\"9780061120084\"").unwrap();
        assert_eq!(parsed.as_str(), "9780061120084");
    }

    #[test]
    fn parse_invalid_isbn13_length_12() {
        let err = Isbn::parse("978006112008").unwrap_err();
        assert_eq!(err, IsbnError::InvalidLength(12));
    }

    #[test]
    fn parse_invalid_isbn13_length_14() {
        let err = Isbn::parse("97800611200840").unwrap_err();
        assert_eq!(err, IsbnError::InvalidLength(14));
    }

    #[test]
    fn is_valid_isbn10_known_good() {
        assert!(is_valid_isbn10("0306406152"));
    }

    #[test]
    fn is_valid_isbn10_with_x() {
        assert!(is_valid_isbn10("155404295X"));
    }

    #[test]
    fn is_valid_isbn10_rejects_bad_check() {
        assert!(!is_valid_isbn10("0306406150"));
    }

    #[test]
    fn is_valid_isbn13_known_good() {
        assert!(is_valid_isbn13("9780061120084"));
    }

    #[test]
    fn is_valid_isbn13_rejects_bad_check() {
        assert!(!is_valid_isbn13("9780061120080"));
    }
}
