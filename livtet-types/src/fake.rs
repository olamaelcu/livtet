#![cfg(feature = "fake")]

//! Manual `Dummy` implementations for types the derive macro can't handle.
//!
//! `fake`'s `time` and `ulid` features now provide built-in `Dummy<Faker>`
//! impls for `time::PrimitiveDateTime`, `time::Date`, `time::OffsetDateTime`,
//! and `ulid::Ulid`. We only need manual impls for:
//!
//! - `std::net::SocketAddr` — not covered by `fake`'s features; the orphan
//!   rule still requires a local config type for this foreign-type impl.
//! - Local wrapper types (`DbId`, `Address`, `DiskPath`, `FormatMetadataSchema`, `Urn`)
//!   that forward to their inner types' `Dummy` impls.

use fake::{Dummy, Fake, Faker, RngExt};

/// Local config type satisfying the orphan rule for `Dummy<LivtetFaker>` impls
/// on foreign types that `fake`'s built-in features don't cover.
pub struct LivtetFaker;

impl Dummy<LivtetFaker> for std::net::SocketAddr {
    fn dummy_with_rng<R: RngExt + ?Sized>(_: &LivtetFaker, rng: &mut R) -> Self {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let port: u16 = rng.random_range(1024..65535);
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }
}

impl Dummy<Faker> for crate::DbId {
    fn dummy_with_rng<R: RngExt + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        // Generate a random ULID manually since fake's ulid feature depends on ulid v1.
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let random: u128 = rng.random();
        crate::DbId(ulid::Ulid::from_parts(timestamp_ms, random))
    }
}

impl Dummy<Faker> for crate::Address {
    fn dummy_with_rng<R: RngExt + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        let inner: std::net::SocketAddr = Dummy::dummy_with_rng(&LivtetFaker, rng);
        crate::Address(inner)
    }
}

impl Dummy<Faker> for crate::DiskPath {
    fn dummy_with_rng<R: RngExt + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        let s: String = Faker.fake_with_rng(rng);
        crate::DiskPath::from_path(s)
    }
}

impl Dummy<Faker> for crate::FormatMetadataSchema {
    fn dummy_with_rng<R: RngExt + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        // Pick a random known variant; Custom would require serde_json::Value
        // which doesn't implement Dummy.
        match rng.random_range(0..3) {
            0 => crate::FormatMetadataSchema::PhysicalBook,
            1 => crate::FormatMetadataSchema::Ebook,
            _ => crate::FormatMetadataSchema::Audiobook,
        }
    }
}

impl Dummy<Faker> for crate::Urn {
    fn dummy_with_rng<R: RngExt + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        let schemes = ["isbn", "wikidata", "url", "goodreads", "oclc"];
        let scheme = schemes[rng.random_range(0..schemes.len())];
        let value: String = Faker.fake_with_rng(rng);
        crate::Urn::new(scheme, value)
    }
}
