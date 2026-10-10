#![no_main]
//! Fuzz the scalar value-type parsers: ISBN, published date, and the
//! saved-search placeholder grammar. Each takes an arbitrary user- or
//! catalog-supplied string and must only ever return a value or a typed
//! error — never panic. The property tests in
//! `livtet-types/tests/roundtrip_properties.rs` assert the matching
//! round-trip/idempotence laws on the accepted inputs this finds.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = livtet_types::Isbn::parse(s);
        let _ = livtet_types::PublishedDate::parse(s);
        let _ = livtet_types::search::placeholder::parse_placeholder(s);
    }
});
