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

#[cfg(test)]
mod tests {
    use super::FormatSearch;
    use tantivy_query_grammar::UserInputAst;

    /// Each variant names a different indexed field, and the value is emitted
    /// undelimited -- these are facet-ish equality checks, not phrases.
    #[test]
    fn each_variant_targets_its_own_field() {
        let cases = [
            (
                FormatSearch::ByFormat {
                    label: "epub".into(),
                },
                r#""format":epub"#,
            ),
            (
                FormatSearch::ByLanguage { code: "en".into() },
                r#""language":en"#,
            ),
            (
                FormatSearch::ByItemType {
                    kind: "audiobook".into(),
                },
                r#""item_type":audiobook"#,
            ),
            (
                FormatSearch::ByReadStatus {
                    status: "finished".into(),
                },
                r#""read_status":finished"#,
            ),
        ];
        for (filter, expected) in cases {
            assert_eq!(format!("{:?}", filter.into_ast()), expected);
            // The From impl must delegate rather than build its own tree.
            assert_eq!(
                format!("{:?}", UserInputAst::from(filter.clone())),
                format!("{:?}", filter.into_ast())
            );
        }
    }

    #[test]
    fn serde_tags_the_variant_and_nests_its_value() {
        let filter = FormatSearch::ByFormat {
            label: "epub".into(),
        };
        let json = serde_json::to_string(&filter).expect("serialize");
        assert_eq!(json, r#"{"type":"by_format","value":{"label":"epub"}}"#);
        assert_eq!(
            serde_json::from_str::<FormatSearch>(&json).expect("deserialize"),
            filter
        );
    }
}
