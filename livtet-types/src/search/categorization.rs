//! Categorical filters by ID — authors, genres, subjects, series,
//! publishers, tags, collections.
//!
//! Each variant that takes a `DbId` renders as either a pure
//! `field:<id>` term or as a hybrid (id OR resolved-name) clause
//! depending on whether the caller has looked up names in advance.
//! Pure name-based queries (e.g. `ByAnyTag`, `ByAllTags`,
//! `InAnyCollection`) are intentionally **not** modelled here —
//! those belong in the free-text side of the lexicon and would
//! shadow the typed-id path.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tantivy_query_grammar::UserInputAst;

use crate::{
    DbId,
    search::{
        dsl::{field_term_exact, field_term_undelimited, or_clause},
        stale::ResourceKind,
    },
};

/// One categorical axis filter. Exactly seven variants per the
/// design plan; deliberately no `ByAnyTag`, `ByAllTags`, or
/// `InAnyCollection` (those would mask the typed-id path and force
/// the search to fall back to free text on every shelf).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CategorizationSearch {
    ByAuthor(ByAuthorAxis),
    ByGenre(ByGenreAxis),
    BySubject(BySubjectAxis),
    BySeries(BySeriesAxis),
    ByPublisher(ByPublisherAxis),
    ByTag(DbId),
    InCollection(DbId),
}

/// Six axis-specific newtype wrappers keep the typed `author_id` /
/// `genre_id` field name visible at the call site and stop serde
/// from collapsing both `ByAuthor(id)` and `ByGenre(id)` into the
/// same tag-value pair.
macro_rules! axis_newtype {
    ($wrapper:ident, $field:literal, $text_field:literal, $kind:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[cfg_attr(feature = "fake", derive(fake::Dummy))]
        #[serde(transparent)]
        pub struct $wrapper(pub DbId);

        impl From<$wrapper> for UserInputAst {
            fn from(value: $wrapper) -> Self {
                hybrid($kind, value.0, &HashMap::new(), $field, $text_field)
            }
        }
    };
}

axis_newtype!(ByAuthorAxis, "author_id", "authors", ResourceKind::Author);
axis_newtype!(ByGenreAxis, "genre_id", "genres", ResourceKind::Genre);
axis_newtype!(
    BySubjectAxis,
    "subject_id",
    "subjects",
    ResourceKind::Subject
);
axis_newtype!(BySeriesAxis, "series_id", "series", ResourceKind::Series);
axis_newtype!(
    ByPublisherAxis,
    "publisher_id",
    "publishers",
    ResourceKind::Publisher
);

/// Build the hybrid (id-OR-resolved-name) clause for any axis.
///
/// If the name is missing for the requested `(kind, id)` pair we fall
/// back to the id-only arm so the query still has *some* narrowing.
/// The caller pre-computes the name map with
/// [`crate::search::stale::ResourceLookup::names`] to fill `names`.
fn hybrid(
    kind: ResourceKind,
    id: DbId,
    names: &HashMap<(ResourceKind, DbId), String>,
    id_field: &'static str,
    text_field: &'static str,
) -> UserInputAst {
    let key = (kind, id);
    let id_arm = field_term_exact(id_field, id.to_string());
    let Some(name) = names.get(&key) else {
        return id_arm;
    };
    let text_arm = field_term_undelimited(text_field, name.clone());
    or_clause(vec![id_arm, text_arm])
}

impl CategorizationSearch {
    /// Render the filter against a precomputed name map. Used by
    /// `SavedSearchKind::into_ast_with_names` and the composer.
    pub fn into_ast_with_names(
        &self,
        names: &HashMap<(ResourceKind, DbId), String>,
    ) -> UserInputAst {
        match self {
            CategorizationSearch::ByAuthor(axis) => {
                hybrid(ResourceKind::Author, axis.0, names, "author_id", "authors")
            }
            CategorizationSearch::ByGenre(axis) => {
                hybrid(ResourceKind::Genre, axis.0, names, "genre_id", "genres")
            }
            CategorizationSearch::BySubject(axis) => hybrid(
                ResourceKind::Subject,
                axis.0,
                names,
                "subject_id",
                "subjects",
            ),
            CategorizationSearch::BySeries(axis) => {
                hybrid(ResourceKind::Series, axis.0, names, "series_id", "series")
            }
            CategorizationSearch::ByPublisher(axis) => hybrid(
                ResourceKind::Publisher,
                axis.0,
                names,
                "publisher_id",
                "publishers",
            ),
            CategorizationSearch::ByTag(id) => {
                hybrid(ResourceKind::Tag, *id, names, "tag_id", "tags")
            }
            CategorizationSearch::InCollection(id) => {
                field_term_exact("collection_id", id.to_string())
            }
        }
    }

    /// Collect every `DbId` referenced by this filter, paired with
    /// the axis it belongs to. The composer iterates the result to
    /// bulk-resolve names and check for stale references.
    pub fn referenced_ids(&self) -> Vec<(ResourceKind, DbId)> {
        match self {
            CategorizationSearch::ByAuthor(axis) => {
                vec![(ResourceKind::Author, axis.0)]
            }
            CategorizationSearch::ByGenre(axis) => {
                vec![(ResourceKind::Genre, axis.0)]
            }
            CategorizationSearch::BySubject(axis) => {
                vec![(ResourceKind::Subject, axis.0)]
            }
            CategorizationSearch::BySeries(axis) => {
                vec![(ResourceKind::Series, axis.0)]
            }
            CategorizationSearch::ByPublisher(axis) => {
                vec![(ResourceKind::Publisher, axis.0)]
            }
            CategorizationSearch::ByTag(id) => vec![(ResourceKind::Tag, *id)],
            CategorizationSearch::InCollection(_) => Vec::new(),
        }
    }
}

impl From<CategorizationSearch> for UserInputAst {
    fn from(value: CategorizationSearch) -> Self {
        value.into_ast_with_names(&HashMap::new())
    }
}

/// Placeholder for the planned flat-categorisation representation.
/// The conversion from a flat category list to a `CategorizationSearch`
/// AST is the composer's job — see `SavedSearchKind::From<FlatCategorization>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub struct FlatCategorization;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::dsl::field_term_exact;

    #[test]
    fn empty_names_falls_back_to_id_arm() {
        let f = CategorizationSearch::ByAuthor(ByAuthorAxis(DbId::new()));
        let ast = f.into_ast_with_names(&HashMap::new());
        // Should serialise to `author_id:"<id>"` — exact literal.
        match ast {
            UserInputAst::Leaf(_) => {}
            _ => panic!("hybrid() with no name must produce a leaf"),
        }
        let _ = field_term_exact("x", "y"); // silence dead-code on imported helper
    }

    #[test]
    fn hybrid_with_name_returns_disjunction() {
        let id = DbId::new();
        let mut names = HashMap::new();
        names.insert((ResourceKind::Author, id), "Ursula".to_string());
        let ast = CategorizationSearch::ByAuthor(ByAuthorAxis(id)).into_ast_with_names(&names);
        match ast {
            UserInputAst::Clause(parts) => {
                assert_eq!(parts.len(), 2);
            }
            _ => panic!("hybrid() with a name must return a clause"),
        }
    }

    #[test]
    fn referenced_ids_covers_each_axis() {
        let id = DbId::new();
        let cases = [
            (
                CategorizationSearch::ByAuthor(ByAuthorAxis(id)),
                ResourceKind::Author,
            ),
            (
                CategorizationSearch::ByGenre(ByGenreAxis(id)),
                ResourceKind::Genre,
            ),
            (
                CategorizationSearch::BySubject(BySubjectAxis(id)),
                ResourceKind::Subject,
            ),
            (
                CategorizationSearch::BySeries(BySeriesAxis(id)),
                ResourceKind::Series,
            ),
            (
                CategorizationSearch::ByPublisher(ByPublisherAxis(id)),
                ResourceKind::Publisher,
            ),
            (CategorizationSearch::ByTag(id), ResourceKind::Tag),
        ];
        for (filter, expected_kind) in cases {
            let refs = filter.referenced_ids();
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].0, expected_kind);
            assert_eq!(refs[0].1, id);
        }
    }
}
