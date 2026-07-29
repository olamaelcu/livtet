//! Format-flavoured filters — file format, language, item type,
//! reading status. Plain string equality against the matching
//! indexed facet field; no normalisation (we trust the column
//! constraints on the writers).

use serde::{Deserialize, Serialize};
use tantivy_query_grammar::UserInputAst;

use crate::search::dsl::field_term;

/// Format-related filters for the saved-search composer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum FormatSearch {
    /// `format:eq(label)` against the multi-valued TEXT field.
    ByFormat { label: String },
    /// `language:eq(code)` (BCP-47 three-letter or two-letter).
    ByLanguage { code: String },
    /// Coarse item kind ("book", "periodical", "audiobook", …).
    ByItemType { kind: String },
    /// One of `"unread"`, `"reading"`, `"finished"`, `"abandoned"`.
    ByReadStatus { status: String },
}

impl FormatSearch {
    pub fn into_ast(&self) -> UserInputAst {
        match self {
            FormatSearch::ByFormat { label } => field_term("format", label.clone()),
            FormatSearch::ByLanguage { code } => field_term("language", code.clone()),
            FormatSearch::ByItemType { kind } => field_term("item_type", kind.clone()),
            FormatSearch::ByReadStatus { status } => field_term("read_status", status.clone()),
        }
    }
}

impl From<FormatSearch> for UserInputAst {
    fn from(value: FormatSearch) -> Self {
        value.into_ast()
    }
}
