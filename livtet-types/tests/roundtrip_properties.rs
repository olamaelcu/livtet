//! Property-based round-trip and idempotence laws for the core value types.
//!
//! The example-based unit tests pin specific cases; these assert the general
//! invariants — `parse ∘ display` identity, byte/string round-trips, and
//! parse idempotence — across a wide input space via `proptest`. This file
//! is the property-testing baseline; extend it as new value types land.

use std::str::FromStr;

use livtet_types::{DbId, Isbn, PublishedDate};
use proptest::prelude::*;

proptest! {
    /// A `DbId` survives a trip through its 16-byte form, its canonical ULID
    /// string, and its hex form unchanged.
    #[test]
    fn dbid_round_trips_through_bytes_string_and_hex(
        bytes in proptest::array::uniform16(any::<u8>()),
    ) {
        let id = DbId::from_bytes(bytes);
        prop_assert_eq!(id.to_bytes(), bytes);
        prop_assert_eq!(DbId::from_str(&id.to_string()).unwrap(), id);
        prop_assert_eq!(DbId::from_hex(&id.to_hex()).unwrap(), id);
    }

    /// Parsing is idempotent: once a string is accepted as an ISBN, its
    /// canonical form parses back to an equal value.
    #[test]
    fn isbn_parse_is_idempotent(s in ".*") {
        if let Ok(isbn) = Isbn::parse(&s) {
            let reparsed = Isbn::parse(isbn.as_str()).expect("a canonical ISBN must re-parse");
            prop_assert_eq!(reparsed, isbn);
        }
    }

    /// `parse(display(d)) == d` for every valid published date, across all
    /// three precisions (year / year-month / year-month-day).
    #[test]
    fn published_date_display_round_trips(
        year in 1i32..=9999,
        month in 1u8..=12,
        day in 1u8..=28,
        precision in 0u8..=2,
    ) {
        let date = match precision {
            0 => PublishedDate::Year(year),
            1 => PublishedDate::YearMonth { year, month },
            _ => PublishedDate::YearMonthDay { year, month, day },
        };
        let rendered = date.to_string();
        prop_assert_eq!(PublishedDate::parse(&rendered).unwrap(), date);
    }
}
