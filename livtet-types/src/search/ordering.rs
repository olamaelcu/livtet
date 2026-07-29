//! Built-in ordering shortcuts. These are full-blown leaf
//! expressions — applying one of these to a query narrows the
//! index to the matching documents and implicitly pushes a sort
//! key (the actual tantivy ordering lives in the engine, not the
//! `UserInputAst`).
//!
//! The current set is "presence-only" — each variant picks a
//! timestamps column (`created_at` or `last_read_at`) and asserts
//! that the row has a value on it. Selecting "MostRecentlyAdded"
//! is therefore equivalent to "the row has a `created_at` value";
//! the sort order is supplied by the search engine at query time.

use serde::{Deserialize, Serialize};
use tantivy_query_grammar::{Delimiter, UserInputAst, UserInputLeaf, UserInputLiteral};

use crate::search::dsl::field_exists;

/// Built-in ordering shortcuts keyed by their plain-English name.
/// `as_str` is used to identify the ordering in serialized
/// payloads; the wire form is the same as the JSON tag (`snake_case`
/// of the variant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrderingSearch {
    MostRecentlyAdded,
    MostRecentlyUpdated,
    RecentlyRead,
    RecentlyImported,
    NeverRead,
}

impl OrderingSearch {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderingSearch::MostRecentlyAdded => "most_recently_added",
            OrderingSearch::MostRecentlyUpdated => "most_recently_updated",
            OrderingSearch::RecentlyRead => "recently_read",
            OrderingSearch::RecentlyImported => "recently_imported",
            OrderingSearch::NeverRead => "never_read",
        }
    }

    pub fn into_ast(&self) -> UserInputAst {
        let marker = match self {
            OrderingSearch::MostRecentlyAdded => "ordering:most_recently_added",
            OrderingSearch::MostRecentlyUpdated => "ordering:most_recently_updated",
            OrderingSearch::RecentlyRead => "ordering:recently_read",
            OrderingSearch::RecentlyImported => "ordering:recently_imported",
            OrderingSearch::NeverRead => "ordering:never_read",
        };
        // Force the placeholder helper to be retained as part of
        // the public API; the actual leaf is a synthetic marker
        // the engine recognises.
        let _ = field_exists("created_at");
        UserInputAst::Leaf(Box::new(UserInputLeaf::Literal(UserInputLiteral {
            field_name: Some("ordering".to_string()),
            phrase: marker.to_string(),
            delimiter: Delimiter::None,
            slop: 0,
            prefix: false,
        })))
    }
}

impl From<OrderingSearch> for UserInputAst {
    fn from(value: OrderingSearch) -> Self {
        value.into_ast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_returns_snake_case() {
        assert_eq!(
            OrderingSearch::MostRecentlyAdded.as_str(),
            "most_recently_added"
        );
        assert_eq!(
            OrderingSearch::MostRecentlyUpdated.as_str(),
            "most_recently_updated"
        );
        assert_eq!(OrderingSearch::RecentlyRead.as_str(), "recently_read");
        assert_eq!(
            OrderingSearch::RecentlyImported.as_str(),
            "recently_imported"
        );
        assert_eq!(OrderingSearch::NeverRead.as_str(), "never_read");
    }

    #[test]
    fn into_ast_produces_ordering_marker() {
        let ast = OrderingSearch::MostRecentlyAdded.into_ast();
        match ast {
            UserInputAst::Leaf(leaf) => match *leaf {
                UserInputLeaf::Literal(lit) => {
                    assert_eq!(lit.field_name, Some("ordering".to_string()));
                    assert_eq!(lit.phrase, "ordering:most_recently_added");
                }
                _ => panic!("expected Literal"),
            },
            _ => panic!("expected Leaf"),
        }
    }

    #[test]
    fn from_trait_produces_same_as_into_ast() {
        let via_from: UserInputAst = OrderingSearch::RecentlyRead.into();
        let via_method = OrderingSearch::RecentlyRead.into_ast();
        assert_eq!(via_from, via_method);
    }
}
