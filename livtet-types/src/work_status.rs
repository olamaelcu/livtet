use serde::{Deserialize, Serialize};
use specta::Type;

/// Reading status for a work.
///
/// The DB column stores the URN string (e.g. `"urn:livtet:work/status/300"`)
/// as TEXT. This enum is the Rust-side view: parse from a string via
/// [`std::str::FromStr`], render back to a URN via `to_urn`, or use the
/// deterministic ULID form to round-trip through [`DbId`].
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[repr(u16)]
pub enum WorkStatus {
    ToRead = 300,
    Reading = 301,
    Finished = 302,
    Abandoned = 303,
    Queued = 304,
    Active = 305,
}

const WORK_STATUS_TIME_MS: u64 = 1735689600300u64;

urn_enum!(
    WorkStatus,
    WORK_STATUS_TIME_MS,
    "urn:livtet:work/status/";
    (ToRead = 300, "to-read", "To Read"),
    (Reading = 301, "reading", "Reading"),
    (Finished = 302, "finished", "Finished"),
    (Abandoned = 303, "abandoned", "Abandoned"),
    (Queued = 304, "queued", "Queued"),
    (Active = 305, "active", "Active"),
    all: [ToRead, Reading, Finished, Abandoned, Queued, Active],
    sea_orm { }
);

#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[derive(Debug)]
pub struct UnknownWorkStatus(pub String);

impl std::fmt::Display for UnknownWorkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown work status: {}", self.0)
    }
}

impl std::error::Error for UnknownWorkStatus {}

impl std::str::FromStr for WorkStatus {
    type Err = UnknownWorkStatus;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "to-read" => Ok(Self::ToRead),
            "reading" => Ok(Self::Reading),
            "finished" => Ok(Self::Finished),
            "abandoned" => Ok(Self::Abandoned),
            "queued" => Ok(Self::Queued),
            "active" => Ok(Self::Active),
            other => Err(UnknownWorkStatus(other.to_owned())),
        }
    }
}

impl std::fmt::Display for WorkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
