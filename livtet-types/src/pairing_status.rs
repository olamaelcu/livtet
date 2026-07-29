use serde::{Deserialize, Serialize};
use specta::Type;

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum PairingStatus {
    Pending = 500,
    Approved = 501,
    Rejected = 502,
}

urn_enum!(
    PairingStatus,
    crate::PAIRING_STATUS_TIME_MS,
    "urn:livtet:pairing/";
    (Pending = 500, "pending", "Pending"),
    (Approved = 501, "approved", "Approved"),
    (Rejected = 502, "rejected", "Rejected"),
    all: [Pending, Approved, Rejected]
);
