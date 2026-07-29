//! `SavedSearchKind`: the recursive top-level definition for
//! user-defined and built-in saved searches.
//!
//! The enum mirrors what the frontend composer edits: an
//! `And` / `Or` / `Not` combinator tree over topical leaves and
//! user-defined raw-DSL leaves. Every variant is serialised with
//! `serde(tag = "type")` so the wire form is `{ "type": "...", "value": ... }`
//! and round-trips losslessly when the composer rewrites a saved
//! search on edit.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tantivy_query_grammar::{UserInputAst, parse_query};
use thiserror::Error;

use crate::{
    DbId,
    search::{
        availability::AvailabilitySearch,
        categorization::{CategorizationSearch, FlatCategorization},
        dsl::field_exists,
        format::FormatSearch,
        identifier_search::IdentifierSearch,
        limit::LimitSearch,
        ordering::OrderingSearch,
        stale::ResourceKind,
        text::TextSearch,
    },
};

/// Recursive saved-search definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SavedSearchKind {
    /// Conjunction. Empty `of` list lowers to a tautology (match
    /// every document).
    And { of: Vec<SavedSearchKind> },
    /// Disjunction. Empty `of` list lowers to a tautology.
    Or { of: Vec<SavedSearchKind> },
    /// Negation of a single child.
    Not { of: Box<SavedSearchKind> },
    /// Categorical axis filter.
    Categorization(CategorizationSearch),
    /// Quantitative limit.
    Limit(LimitSearch),
    /// Format-flavoured filter.
    Format(FormatSearch),
    /// Identifier-based filter.
    Identifier(IdentifierSearch),
    /// Presence predicate.
    Availability(AvailabilitySearch),
    /// Built-in ordering shortcut.
    Ordering(OrderingSearch),
    /// Free-text phrase filter.
    Text(TextSearch),
    /// Raw tantivy query DSL string.
    UserDefined { query_dsl: String },
}

/// Reasons a raw DSL string failed to parse into a `UserInputAst`.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[error("invalid user-defined DSL: {message}")]
pub struct InvalidDslError {
    pub message: String,
    pub position: usize,
}

/// Lower a `UserDefined { query_dsl }` payload into a tantivy AST.
/// Centralised so every caller produces a consistent error.
pub fn parse_user_dsl(query_dsl: &str) -> Result<UserInputAst, InvalidDslError> {
    let (ast, errors) = tantivy_query_grammar::parse_query_lenient(query_dsl);
    if let Some(err) = errors.into_iter().next() {
        return Err(InvalidDslError {
            message: format!("{:?}", err),
            position: 0,
        });
    }
    Ok(ast)
}

impl SavedSearchKind {
    /// Render this saved search against a per-axis name map.
    /// Combinators recurse; topical leaves defer to their own
    /// `into_ast_with_names`; user-defined DSL is parsed lazily
    /// here.
    pub fn into_ast_with_names(
        &self,
        names: &HashMap<(ResourceKind, DbId), String>,
    ) -> Result<UserInputAst, InvalidDslError> {
        match self {
            SavedSearchKind::And { of } => {
                let mut children = Vec::with_capacity(of.len());
                for c in of {
                    children.push(c.into_ast_with_names(names)?);
                }
                // Empty conjunction: tantivy expects at least one
                // child to build a Must clause; we substitute
                // EXISTS on a synthetic field which the engine
                // treats as "match everything".
                if children.is_empty() {
                    return Ok(field_exists("_all"));
                }
                Ok(crate::search::dsl::and_clause(children))
            }
            SavedSearchKind::Or { of } => {
                let mut children = Vec::with_capacity(of.len());
                for c in of {
                    children.push(c.into_ast_with_names(names)?);
                }
                if children.is_empty() {
                    return Ok(field_exists("_all"));
                }
                Ok(crate::search::dsl::or_clause(children))
            }
            SavedSearchKind::Not { of } => {
                let inner = of.into_ast_with_names(names)?;
                Ok(crate::search::dsl::must_not_clause(inner))
            }
            SavedSearchKind::Categorization(c) => Ok(c.into_ast_with_names(names)),
            SavedSearchKind::Limit(l) => Ok(l.into_ast()),
            SavedSearchKind::Format(f) => Ok(f.into_ast()),
            SavedSearchKind::Identifier(i) => Ok(i.clone().into()),
            SavedSearchKind::Availability(a) => Ok(a.into_ast()),
            SavedSearchKind::Ordering(o) => Ok(o.into_ast()),
            SavedSearchKind::Text(t) => Ok(t.into_ast()),
            SavedSearchKind::UserDefined { query_dsl } => parse_user_dsl(query_dsl),
        }
    }

    /// Collect every categorical `DbId` reference, paired with its
    /// axis. Recurses through combinators and stops at
    /// non-categorical leaves.
    pub fn referenced_ids(&self) -> Vec<(ResourceKind, DbId)> {
        let mut out = Vec::new();
        collect_refs(self, &mut out);
        out
    }
}

fn collect_refs(node: &SavedSearchKind, out: &mut Vec<(ResourceKind, DbId)>) {
    match node {
        SavedSearchKind::And { of } | SavedSearchKind::Or { of } => {
            for c in of {
                collect_refs(c, out);
            }
        }
        SavedSearchKind::Not { of } => collect_refs(of, out),
        SavedSearchKind::Categorization(c) => out.extend(c.referenced_ids()),
        SavedSearchKind::Limit(_)
        | SavedSearchKind::Format(_)
        | SavedSearchKind::Identifier(_)
        | SavedSearchKind::Availability(_)
        | SavedSearchKind::Ordering(_)
        | SavedSearchKind::Text(_)
        | SavedSearchKind::UserDefined { .. } => {}
    }
}

// --- `From<Enum> for SavedSearchKind` convenience impls, one per
// topical enum. Keep them co-located with the enum definition so
// every leaf has the obvious wrapping path.

macro_rules! wrap_leaf {
    ($enum:ident, $variant:ident) => {
        impl From<$enum> for SavedSearchKind {
            fn from(value: $enum) -> Self {
                SavedSearchKind::$variant(value)
            }
        }
    };
}

wrap_leaf!(CategorizationSearch, Categorization);
wrap_leaf!(LimitSearch, Limit);
wrap_leaf!(FormatSearch, Format);
wrap_leaf!(IdentifierSearch, Identifier);
wrap_leaf!(AvailabilitySearch, Availability);
wrap_leaf!(OrderingSearch, Ordering);
wrap_leaf!(TextSearch, Text);

impl From<FlatCategorization> for SavedSearchKind {
    fn from(value: FlatCategorization) -> Self {
        let _ = value;
        unimplemented!(
            "FlatCategorization -> CategorizationSearch conversion is the composer's job"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_is_tautology() {
        let k = SavedSearchKind::And { of: vec![] };
        let ast = k.into_ast_with_names(&HashMap::new()).expect("render");
        // Marker exists-check on the synthetic `_all` field.
        match ast {
            UserInputAst::Leaf(_) => {}
            _ => panic!("empty AND should collapse to a leaf exists marker"),
        }
    }

    #[test]
    fn invalid_user_dsl_returns_invalid_dsl_error() {
        let k = SavedSearchKind::UserDefined {
            query_dsl: "title:\"".to_string(), // unbalanced quote
        };
        let err = k.into_ast_with_names(&HashMap::new()).unwrap_err();
        // The exact error depends on tantivy-version specifics;
        // any `InvalidDslError` is fine here.
        assert_eq!(err.message.is_empty(), false);
    }

    #[test]
    fn references_recurse_through_not() {
        let id = DbId::new();
        let inner = SavedSearchKind::Categorization(CategorizationSearch::ByTag(id));
        let outer = SavedSearchKind::Not {
            of: Box::new(inner),
        };
        let refs = outer.referenced_ids();
        assert_eq!(refs, vec![(ResourceKind::Tag, id)]);
    }

    #[test]
    fn references_recurse_through_and() {
        let a_id = DbId::new();
        let g_id = DbId::new();
        let search = SavedSearchKind::And {
            of: vec![
                SavedSearchKind::Categorization(CategorizationSearch::ByTag(a_id)),
                SavedSearchKind::Categorization(CategorizationSearch::ByGenre(
                    crate::search::categorization::ByGenreAxis(g_id),
                )),
            ],
        };
        let refs = search.referenced_ids();
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&(ResourceKind::Tag, a_id)));
        assert!(refs.contains(&(ResourceKind::Genre, g_id)));
    }
}

// `parse_query` is re-exported to silence the unused-import lint
// for the parser entry point. Consumers call `parse_user_dsl`
// above to keep error-path policy in one place.
#[allow(unused_imports)]
use parse_query as _parse_query_silence_unused;
