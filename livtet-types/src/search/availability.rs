//! Predicate-on-presence filters. Every variant checks the EXISTENCE
//! of a column-shaped field on the indexed document. We never
//! conflate these with "the value is empty" — empty is its own
//! state and stays a feature of the data layer.

use serde::{Deserialize, Serialize};
use tantivy_query_grammar::UserInputAst;

use crate::search::dsl::{field_exists, must_not_clause};

/// "Does the edition have a `cover` field populated?", etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AvailabilitySearch {
    /// Cover image present.
    HasCover { value: bool },
    /// User-authored notes present (non-empty).
    HasNotes { value: bool },
    /// At least one attached file (PDF companion, scan, etc.).
    HasAttachments { value: bool },
    /// OCR / extracted full-text body present.
    HasFullText { value: bool },
    /// Inventory rows attached (copies, locations).
    HasInventory { value: bool },
}

impl AvailabilitySearch {
    pub fn into_ast(&self) -> UserInputAst {
        match self {
            AvailabilitySearch::HasCover { value } => arm("cover", *value),
            AvailabilitySearch::HasNotes { value } => arm("notes", *value),
            AvailabilitySearch::HasAttachments { value } => arm("attachments", *value),
            AvailabilitySearch::HasFullText { value } => arm("full_text", *value),
            AvailabilitySearch::HasInventory { value } => arm("inventory_rows", *value),
        }
    }
}

fn arm(field: &'static str, value: bool) -> UserInputAst {
    if value {
        field_exists(field)
    } else {
        must_not_clause(field_exists(field))
    }
}

impl From<AvailabilitySearch> for UserInputAst {
    fn from(value: AvailabilitySearch) -> Self {
        value.into_ast()
    }
}

#[cfg(test)]
mod tests {
    use super::AvailabilitySearch;
    use tantivy_query_grammar::UserInputAst;

    /// These are presence predicates, so `value: false` has to negate the
    /// existence check rather than test for an empty value -- the module doc
    /// is explicit that empty is its own state.
    #[test]
    fn each_variant_checks_existence_of_its_own_field() {
        let cases = [
            (AvailabilitySearch::HasCover { value: true }, "cover"),
            (AvailabilitySearch::HasNotes { value: true }, "notes"),
            (
                AvailabilitySearch::HasAttachments { value: true },
                "attachments",
            ),
            (AvailabilitySearch::HasFullText { value: true }, "full_text"),
            (
                AvailabilitySearch::HasInventory { value: true },
                "inventory_rows",
            ),
        ];
        for (filter, field) in cases {
            assert_eq!(
                format!("{:?}", filter.into_ast()),
                format!("$exists({field:?})")
            );
            assert_eq!(
                format!("{:?}", UserInputAst::from(filter.clone())),
                format!("{:?}", filter.into_ast())
            );
        }
    }

    #[test]
    fn a_false_value_negates_the_existence_check() {
        for (yes, no) in [
            (
                AvailabilitySearch::HasCover { value: true },
                AvailabilitySearch::HasCover { value: false },
            ),
            (
                AvailabilitySearch::HasInventory { value: true },
                AvailabilitySearch::HasInventory { value: false },
            ),
        ] {
            let positive = format!("{:?}", yes.into_ast());
            assert_eq!(format!("{:?}", no.into_ast()), format!("(-{positive})"));
        }
    }

    #[test]
    fn serde_tags_the_variant_and_nests_its_flag() {
        let filter = AvailabilitySearch::HasCover { value: false };
        let json = serde_json::to_string(&filter).expect("serialize");
        assert_eq!(json, r#"{"type":"has_cover","value":{"value":false}}"#);
        assert_eq!(
            serde_json::from_str::<AvailabilitySearch>(&json).expect("deserialize"),
            filter
        );
    }
}
