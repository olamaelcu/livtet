use serde::{Deserialize, Serialize};
use specta::Type;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum SeriesType {
    Anthology = 200,
    Omnibus = 201,
    Collection = 202,
}

const SERIES_TYPE_TIME_MS: u64 = 1735689600200u64;

urn_enum!(
    SeriesType,
    SERIES_TYPE_TIME_MS,
    "urn:livtet:series/";
    (Anthology = 200, "anthology", "Anthology"),
    (Omnibus = 201, "omnibus", "Omnibus"),
    (Collection = 202, "collection", "Collection"),
    all: [Anthology, Omnibus, Collection]
);
