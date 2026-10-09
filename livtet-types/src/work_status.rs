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
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
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

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use sea_orm::sea_query::{ArrayType, ColumnType, Nullable, ValueType};
    use sea_orm::{TryFromU64, Value};

    use super::WorkStatus;

    /// `WorkStatus` is the only `urn_enum!` type that asks for the `sea_orm`
    /// block, so it is the only place these impls exist to be exercised.
    #[test]
    fn value_round_trips_through_try_from() {
        for status in WorkStatus::all() {
            let value = Value::from(status);
            assert_eq!(
                <WorkStatus as TryFrom<Value>>::try_from(value.clone()).expect("round trip"),
                status
            );
            assert_eq!(
                <WorkStatus as ValueType>::try_from(value).expect("ValueType round trip"),
                status
            );
        }
    }

    #[test]
    fn try_from_value_rejects_malformed_urns() {
        // Each arm of the parse: wrong prefix, unparseable discriminant,
        // in-range-looking but unknown discriminant, and a non-string Value.
        let cases = [
            Value::String(Some("urn:livtet:book-cond/700".to_string())),
            Value::String(Some("urn:livtet:work/status/not-a-number".to_string())),
            Value::String(Some("urn:livtet:work/status/999".to_string())),
            Value::BigInt(Some(300)),
        ];
        for value in cases {
            let err = <WorkStatus as TryFrom<Value>>::try_from(value.clone())
                .expect_err(&format!("{value:?} must not parse"));
            assert!(!err.is_empty(), "error message must say what went wrong");
            assert!(
                <WorkStatus as ValueType>::try_from(value).is_err(),
                "ValueType must reject whatever TryFrom rejects"
            );
        }
    }

    #[test]
    fn value_type_metadata_describes_a_text_column() {
        assert_eq!(<WorkStatus as ValueType>::type_name(), "WorkStatus");
        assert_eq!(<WorkStatus as ValueType>::array_type(), ArrayType::String);
        assert_eq!(<WorkStatus as ValueType>::column_type(), ColumnType::Text);
        assert_eq!(
            <WorkStatus as Nullable>::null(),
            Value::String(None),
            "a null work status is a null TEXT, not a null integer"
        );
    }

    /// The URN is the stored form, so there is no integer identity to build
    /// from -- `try_from_u64` exists only to satisfy the trait and must fail.
    #[test]
    fn cannot_be_built_from_a_u64() {
        let err = WorkStatus::try_from_u64(300).expect_err("must not build from u64");
        assert!(err.to_string().contains("WorkStatus"), "got: {err}");
    }

    #[test]
    fn snake_form_round_trips_through_from_str() {
        for status in WorkStatus::all() {
            assert_eq!(
                WorkStatus::from_str(status.as_str()).expect("round trip"),
                status
            );
            assert_eq!(
                status.to_string(),
                status.as_str(),
                "Display is the snake form"
            );
        }
    }

    #[test]
    fn from_str_rejects_an_unknown_label_and_names_it() {
        let err = WorkStatus::from_str("to-reed").expect_err("must not parse");
        assert_eq!(err.0, "to-reed");
        assert!(
            err.to_string().contains("to-reed"),
            "the error must echo the input: {err}"
        );
        // The display form, not the snake form, is not accepted.
        assert!(WorkStatus::from_str("To Read").is_err());
    }
}
