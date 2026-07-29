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
