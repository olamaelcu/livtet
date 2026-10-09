use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ProgressUnit {
    Percentage,
    Page,
    VirtualPage,
    Timestamp,
    Chapter,
    Cfi,
}

impl ProgressUnit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Percentage => "percentage",
            Self::Page => "page",
            Self::VirtualPage => "virtual_page",
            Self::Timestamp => "timestamp",
            Self::Chapter => "chapter",
            Self::Cfi => "cfi",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Type, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum Progression {
    Percentage(f64),
    Page(u32),
    VirtualPage(u32),
    TimestampSeconds(i64),
    Chapter(f64),
    Cfi(String),
}

#[cfg(test)]
mod tests {
    use super::{ProgressUnit, Progression};

    const ALL_UNITS: [ProgressUnit; 6] = [
        ProgressUnit::Percentage,
        ProgressUnit::Page,
        ProgressUnit::VirtualPage,
        ProgressUnit::Timestamp,
        ProgressUnit::Chapter,
        ProgressUnit::Cfi,
    ];

    /// `as_str` and the serde rename must agree: the same snake_case token is
    /// both the stored discriminant and what `KnownFormats` compares against
    /// in `default_progress_unit_enum`.
    #[test]
    fn as_str_matches_the_serde_representation() {
        for unit in ALL_UNITS {
            let json = serde_json::to_string(&unit).expect("serialize");
            assert_eq!(
                json,
                format!("\"{}\"", unit.as_str()),
                "{unit:?} renders differently through serde than through as_str"
            );
            assert_eq!(
                serde_json::from_str::<ProgressUnit>(&json).expect("deserialize"),
                unit
            );
        }
    }

    #[test]
    fn unit_tokens_are_non_empty_and_unique() {
        let tokens: Vec<&str> = ALL_UNITS.iter().map(|u| u.as_str()).collect();
        assert!(tokens.iter().all(|t| !t.is_empty()));
        let mut deduped = tokens.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), tokens.len(), "two units share a token");
    }

    #[test]
    fn progression_round_trips_through_serde_for_every_variant() {
        for progression in [
            Progression::Percentage(0.5),
            Progression::Page(42),
            Progression::VirtualPage(7),
            Progression::TimestampSeconds(-1),
            Progression::Chapter(3.5),
            Progression::Cfi("epubcfi(/6/4!/4/2)".to_string()),
        ] {
            let json = serde_json::to_string(&progression).expect("serialize");
            assert_eq!(
                serde_json::from_str::<Progression>(&json).expect("deserialize"),
                progression,
                "{json}"
            );
        }
    }

    /// The wire tags are snake_case, which is what the stored JSON uses.
    #[test]
    fn progression_tags_are_snake_case() {
        assert_eq!(
            serde_json::to_string(&Progression::VirtualPage(1)).expect("serialize"),
            r#"{"virtual_page":1}"#
        );
        assert_eq!(
            serde_json::to_string(&Progression::TimestampSeconds(1)).expect("serialize"),
            r#"{"timestamp_seconds":1}"#
        );
    }
}
