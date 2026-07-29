use serde::{Deserialize, Serialize};
use specta::Type;
use strum::{Display, EnumIter, EnumString, IntoStaticStr, VariantNames};

use crate::DbId;

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct WorkFilters {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_ids: Vec<DbId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genre_ids: Vec<DbId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subject_ids: Vec<DbId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub publisher_ids: Vec<DbId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub author_ids: Vec<DbId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub format_ids: Vec<DbId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub language_ids: Vec<DbId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<WorkSortBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_direction: Option<SortDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

impl WorkFilters {
    /// Resolve the SQL `LIMIT` clause for this filter set.
    ///
    /// Behaviour:
    /// - An explicit `limit` always wins (even when `NewestCap` is set).
    /// - `sort_by = NewestCap` implies a hard ceiling of 100 rows.
    /// - Any other combination returns `None` (no cap).
    ///
    /// The default of 100 mirrors the front-end's "newest 100" cap and
    /// is the single source of truth for that number on the Rust side.
    pub fn effective_limit(&self) -> Option<u64> {
        if self.limit.is_some() {
            return self.limit;
        }
        match self.sort_by {
            Some(WorkSortBy::NewestCap) => Some(100),
            _ => None,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Display,
    EnumString,
    EnumIter,
    IntoStaticStr,
    VariantNames,
    Serialize,
    Deserialize,
    Type,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum WorkSortBy {
    #[default]
    CreatedAt,
    Title,
    UpdatedAt,
    /// Selects the newest works.
    ///
    /// `sort_direction` is silently ignored when this variant is
    /// selected — the name `NewestCap` carries the ordering semantic,
    /// so callers that pass a `sort_direction` should expect their
    /// value to be discarded. `ORDER BY created_at DESC` always wins.
    ///
    /// The 100-row cap is enforced by callers honouring
    /// [`WorkFilters::effective_limit`], which returns `Some(100)`
    /// when `sort_by = NewestCap` and no explicit `limit` was supplied.
    NewestCap,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Display,
    EnumString,
    EnumIter,
    IntoStaticStr,
    VariantNames,
    Serialize,
    Deserialize,
    Type,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    #[default]
    Desc,
    Asc,
}

/// Which field to sort search results by.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Display,
    EnumString,
    EnumIter,
    IntoStaticStr,
    VariantNames,
    Serialize,
    Deserialize,
    Type,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    /// Sort by title (alphabetical via `title_sort` fast field).
    Title,
    /// Sort by creation date (`created_at` fast field).
    #[default]
    CreatedAt,
    /// Sort by last update date (`updated_at` fast field).
    UpdatedAt,
    /// Sort by BM25 relevance score.
    Score,
}

/// A fully-resolved sort specification produced by
/// [`WorkFiltersQuery::build_sort`].
/// Shared across the Tauri command, FFI, and saved-searches paths.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SortSpec {
    pub field: SortField,
    pub direction: SortDirection,
    pub limit: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_cap_with_desc_implies_limit_100() {
        let f = WorkFilters {
            sort_by: Some(WorkSortBy::NewestCap),
            sort_direction: Some(SortDirection::Desc),
            ..WorkFilters::default()
        };
        assert_eq!(f.effective_limit(), Some(100));
    }

    #[test]
    fn explicit_limit_overrides_newest_cap_default() {
        let f = WorkFilters {
            sort_by: Some(WorkSortBy::NewestCap),
            sort_direction: Some(SortDirection::Desc),
            limit: Some(50),
            ..WorkFilters::default()
        };
        assert_eq!(f.effective_limit(), Some(50));
    }

    #[test]
    fn no_sort_no_limit_means_no_cap() {
        let f = WorkFilters::default();
        assert_eq!(f.effective_limit(), None);
    }

    #[test]
    fn newest_cap_without_direction_still_implies_limit() {
        let f = WorkFilters {
            sort_by: Some(WorkSortBy::NewestCap),
            limit: None,
            ..WorkFilters::default()
        };
        assert_eq!(f.effective_limit(), Some(100));
    }

    #[test]
    fn newest_cap_strum_round_trip() {
        // strum round-trip: IntoStaticStr → &str → EnumString parses back
        let v = WorkSortBy::NewestCap;
        let s: &'static str = v.into();
        assert_eq!(s, "newest_cap");
        let parsed: WorkSortBy = s.parse().unwrap();
        assert_eq!(parsed, WorkSortBy::NewestCap);
    }
}
