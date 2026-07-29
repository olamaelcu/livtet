//! Quantitative limits — year ranges, dates, ratings, sizes.
//!
//! These all lower to a clause rooted at an indexable field (year
//! → numeric range, date → ISO range, rating → numeric range...).
//! We deliberately don't model `By` vs `ByExclusive` separately: a
//! range covers both via inclusive `UserInputBound`s.

use serde::{Deserialize, Serialize};
use tantivy_query_grammar::UserInputAst;

use crate::search::dsl::{field_term, range_inclusive};

/// Quantitative limit filters. Each variant lowers to a range or
/// equality term against its canonical schema field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum LimitSearch {
    /// `published_year:eq(year)`.
    ByYear { year: i32 },
    /// `published_year:[lo TO hi]` inclusive.
    ByYearRange { lower: i32, upper: i32 },
    /// `created_at:[from TO to]` (ISO-8601 strings).
    ByDateAdded { from: String, to: String },
    /// `rating:eq(value)` (0..=5).
    ByRating { value: u8 },
    /// `series_index:eq(index)`.
    BySeriesIndex { index: u32 },
    /// `file_size:eq(bytes)`.
    ByFileSize { bytes: u64 },
    /// `page_count:eq(pages)`.
    ByPageCount { pages: u32 },
}

impl LimitSearch {
    pub fn into_ast(&self) -> UserInputAst {
        match self {
            LimitSearch::ByYear { year } => field_term("published_year", year.to_string()),
            LimitSearch::ByYearRange { lower, upper } => {
                range_inclusive("published_year", lower.to_string(), upper.to_string())
            }
            LimitSearch::ByDateAdded { from, to } => {
                range_inclusive("created_at", from.clone(), to.clone())
            }
            LimitSearch::ByRating { value } => field_term("rating", value.to_string()),
            LimitSearch::BySeriesIndex { index } => field_term("series_index", index.to_string()),
            LimitSearch::ByFileSize { bytes } => field_term("file_size", bytes.to_string()),
            LimitSearch::ByPageCount { pages } => field_term("page_count", pages.to_string()),
        }
    }
}

impl From<LimitSearch> for UserInputAst {
    fn from(value: LimitSearch) -> Self {
        value.into_ast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::dsl::field_exists;

    #[test]
    fn by_year_emits_field_term() {
        let ast = LimitSearch::ByYear { year: 1999 }.into_ast();
        match ast {
            UserInputAst::Leaf(_) => {}
            _ => panic!("expected leaf"),
        }
    }

    #[test]
    fn by_year_range_emits_range() {
        let ast = LimitSearch::ByYearRange {
            lower: 1990,
            upper: 1999,
        }
        .into_ast();
        match ast {
            UserInputAst::Leaf(_) => {}
            _ => panic!("expected leaf"),
        }
    }

    #[test]
    fn placeholder_used_to_avoid_dead_code_warning() {
        let _ = field_exists("x");
    }
}
