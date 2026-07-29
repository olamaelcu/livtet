use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use time::{Date, Month};

/// A publication date with three precision levels:
///
///   * year-only — `1989` (displayed as `"1989"`)
///   * year-month — `1989-09` (displayed as `"1989-09"`)
///   * year-month-day — `1989-09-03` (displayed as `"1989-09-03"`)
///
/// Plugins emit a string in any of these forms. The wire format is
/// the canonical ISO-style string. Round-tripping is via
/// [`PublishedDate::parse`] and [`Display`].
///
/// Conversion to a `time::Date` (used by the storage layer) is
/// only possible for the full year-month-day form. Year-only and
/// year-month forms convert to `None` — the DB column is a
/// `date_text` but the save path treats the partial forms as
/// "no specific date", which mirrors the existing behaviour for
/// works where only the publication year is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(try_from = "String", into = "String")]
pub enum PublishedDate {
    /// `1989` — only the year is known.
    Year(i32),
    /// `1989-09` — the year and month are known.
    YearMonth { year: i32, month: u8 },
    /// `1989-09-03` — the full calendar date is known.
    YearMonthDay { year: i32, month: u8, day: u8 },
}

impl PublishedDate {
    /// Parse a publication date string. Accepts:
    ///
    ///   * `YYYY` — 4-digit year
    ///   * `YYYY-MM` — 4-digit year and 1-or-2-digit month
    ///   * `YYYY-MM-DD` — full ISO 8601 calendar date
    pub fn parse(s: &str) -> Result<Self, PublishedDateError> {
        let s = s.trim();
        // Try YYYY-MM-DD first (most specific).
        if let Some((y, rest)) = s.split_once('-') {
            // YYYY only has no dashes; we get here for YYYY-MM and YYYY-MM-DD.
            let year = parse_year(y).map_err(PublishedDateError::BadYear)?;
            if let Some((m, d)) = rest.split_once('-') {
                // YYYY-MM-DD
                let month = parse_month(m).map_err(PublishedDateError::BadMonth)?;
                let day = parse_day(d).map_err(PublishedDateError::BadDay)?;
                if !is_valid_ymd(year, month, day) {
                    return Err(PublishedDateError::InvalidDate { year, month, day });
                }
                return Ok(PublishedDate::YearMonthDay { year, month, day });
            } else {
                // YYYY-MM
                let month = parse_month(rest).map_err(PublishedDateError::BadMonth)?;
                if !(1..=12).contains(&month) {
                    return Err(PublishedDateError::InvalidMonth(month));
                }
                return Ok(PublishedDate::YearMonth { year, month });
            }
        }
        // YYYY only
        let year = parse_year(s).map_err(PublishedDateError::BadYear)?;
        Ok(PublishedDate::Year(year))
    }

    /// The four-digit year. Always populated.
    pub fn year(&self) -> i32 {
        match *self {
            PublishedDate::Year(y) => y,
            PublishedDate::YearMonth { year, .. } => year,
            PublishedDate::YearMonthDay { year, .. } => year,
        }
    }

    /// The 1-based month, or `None` for year-only.
    pub fn month(&self) -> Option<u8> {
        match *self {
            PublishedDate::Year(_) => None,
            PublishedDate::YearMonth { month, .. } => Some(month),
            PublishedDate::YearMonthDay { month, .. } => Some(month),
        }
    }

    /// The day-of-month, or `None` for year-only and year-month.
    pub fn day(&self) -> Option<u8> {
        match *self {
            PublishedDate::Year(_) | PublishedDate::YearMonth { .. } => None,
            PublishedDate::YearMonthDay { day, .. } => Some(day),
        }
    }

    /// Convert to a `time::Date`. Returns `None` for year-only and
    /// year-month forms (the underlying DB column is a `date_text`,
    /// but the save flow treats partial dates as "no specific
    /// date" — see ADR 0019 for the rationale).
    pub fn to_time_date(&self) -> Option<Date> {
        match *self {
            PublishedDate::YearMonthDay { year, month, day } => Month::try_from(month)
                .ok()
                .and_then(|m| Date::from_calendar_date(year, m, day).ok()),
            _ => None,
        }
    }
}

fn parse_year(s: &str) -> Result<i32, String> {
    if s.len() != 4 || !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(s.to_string());
    }
    s.parse::<i32>().map_err(|_| s.to_string())
}

fn parse_month(s: &str) -> Result<u8, String> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(s.to_string());
    }
    let n: u8 = s.parse().map_err(|_| s.to_string())?;
    if !(1..=12).contains(&n) {
        return Err(s.to_string());
    }
    Ok(n)
}

fn parse_day(s: &str) -> Result<u8, String> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(s.to_string());
    }
    let n: u8 = s.parse().map_err(|_| s.to_string())?;
    if !(1..=31).contains(&n) {
        return Err(s.to_string());
    }
    Ok(n)
}

fn is_valid_ymd(year: i32, month: u8, day: u8) -> bool {
    let Some(m) = Month::try_from(month).ok() else {
        return false;
    };
    Date::from_calendar_date(year, m, day).is_ok()
}

impl fmt::Display for PublishedDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            PublishedDate::Year(y) => write!(f, "{y:04}"),
            PublishedDate::YearMonth { year, month } => write!(f, "{year:04}-{month:02}"),
            PublishedDate::YearMonthDay { year, month, day } => {
                write!(f, "{year:04}-{month:02}-{day:02}")
            }
        }
    }
}

impl FromStr for PublishedDate {
    type Err = PublishedDateError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for PublishedDate {
    type Error = PublishedDateError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl From<PublishedDate> for String {
    fn from(d: PublishedDate) -> Self {
        d.to_string()
    }
}

/// Errors that can occur when parsing a `PublishedDate`.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum PublishedDateError {
    #[error("invalid year (expected 4 digits): {0}")]
    BadYear(String),
    #[error("invalid month: {0}")]
    BadMonth(String),
    #[error("invalid day: {0}")]
    BadDay(String),
    #[error("invalid month number: {0}")]
    InvalidMonth(u8),
    #[error("invalid date: {year:04}-{month:02}-{day:02}")]
    InvalidDate { year: i32, month: u8, day: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_year_only() {
        let d = PublishedDate::parse("1989").unwrap();
        assert_eq!(d, PublishedDate::Year(1989));
        assert_eq!(d.year(), 1989);
        assert_eq!(d.month(), None);
        assert_eq!(d.day(), None);
        assert_eq!(d.to_string(), "1989");
        assert_eq!(d.to_time_date(), None);
    }

    #[test]
    fn parse_year_month() {
        let d = PublishedDate::parse("1989-09").unwrap();
        assert_eq!(
            d,
            PublishedDate::YearMonth {
                year: 1989,
                month: 9
            }
        );
        assert_eq!(d.year(), 1989);
        assert_eq!(d.month(), Some(9));
        assert_eq!(d.day(), None);
        assert_eq!(d.to_string(), "1989-09");
        assert_eq!(d.to_time_date(), None);
    }

    #[test]
    fn parse_year_month_day() {
        let d = PublishedDate::parse("1989-09-03").unwrap();
        assert_eq!(
            d,
            PublishedDate::YearMonthDay {
                year: 1989,
                month: 9,
                day: 3
            }
        );
        assert_eq!(d.year(), 1989);
        assert_eq!(d.month(), Some(9));
        assert_eq!(d.day(), Some(3));
        assert_eq!(d.to_string(), "1989-09-03");
        let t = d.to_time_date().unwrap();
        assert_eq!(t.year(), 1989);
        assert_eq!(t.month() as u8, 9);
        assert_eq!(t.day(), 3);
    }

    #[test]
    fn parse_year_month_day_single_digit_components() {
        let d = PublishedDate::parse("1989-9-3").unwrap();
        assert_eq!(
            d,
            PublishedDate::YearMonthDay {
                year: 1989,
                month: 9,
                day: 3
            }
        );
        assert_eq!(d.to_string(), "1989-09-03");
    }

    #[test]
    fn parse_invalid_year() {
        assert!(matches!(
            PublishedDate::parse("89"),
            Err(PublishedDateError::BadYear(_))
        ));
        assert!(matches!(
            PublishedDate::parse(""),
            Err(PublishedDateError::BadYear(_))
        ));
        assert!(matches!(
            PublishedDate::parse("abcd"),
            Err(PublishedDateError::BadYear(_))
        ));
        assert!(matches!(
            PublishedDate::parse("1989a"),
            Err(PublishedDateError::BadYear(_))
        ));
    }

    #[test]
    fn parse_invalid_month() {
        assert!(matches!(
            PublishedDate::parse("1989-13"),
            Err(PublishedDateError::BadMonth(_))
        ));
        assert!(matches!(
            PublishedDate::parse("1989-00"),
            Err(PublishedDateError::BadMonth(_))
        ));
        assert!(matches!(
            PublishedDate::parse("1989-ab"),
            Err(PublishedDateError::BadMonth(_))
        ));
    }

    #[test]
    fn parse_invalid_day() {
        assert!(matches!(
            PublishedDate::parse("1989-09-00"),
            Err(PublishedDateError::BadDay(_))
        ));
        assert!(matches!(
            PublishedDate::parse("1989-09-32"),
            Err(PublishedDateError::BadDay(_))
        ));
        // Feb 30
        assert!(matches!(
            PublishedDate::parse("1989-02-30"),
            Err(PublishedDateError::InvalidDate { .. })
        ));
    }

    #[test]
    fn parse_leap_day_valid() {
        let d = PublishedDate::parse("2024-02-29").unwrap();
        assert_eq!(d.day(), Some(29));
        assert!(d.to_time_date().is_some());
    }

    #[test]
    fn parse_non_leap_year_rejects_feb_29() {
        assert!(matches!(
            PublishedDate::parse("2023-02-29"),
            Err(PublishedDateError::InvalidDate { .. })
        ));
    }

    #[test]
    fn serde_json_roundtrip_year_only() {
        let original = PublishedDate::Year(1989);
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"1989\"");
        let parsed: PublishedDate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn serde_json_roundtrip_year_month() {
        let original = PublishedDate::YearMonth {
            year: 1989,
            month: 9,
        };
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"1989-09\"");
        let parsed: PublishedDate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn serde_json_roundtrip_year_month_day() {
        let original = PublishedDate::YearMonthDay {
            year: 1989,
            month: 9,
            day: 3,
        };
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"1989-09-03\"");
        let parsed: PublishedDate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn serde_rejects_malformed() {
        let result: Result<PublishedDate, _> = serde_json::from_str("\"1989-13-01\"");
        assert!(result.is_err());
    }

    #[test]
    fn display_format() {
        assert_eq!(PublishedDate::Year(1989).to_string(), "1989");
        assert_eq!(
            PublishedDate::YearMonth {
                year: 1989,
                month: 9
            }
            .to_string(),
            "1989-09"
        );
        assert_eq!(
            PublishedDate::YearMonthDay {
                year: 1989,
                month: 9,
                day: 3
            }
            .to_string(),
            "1989-09-03"
        );
    }
}
