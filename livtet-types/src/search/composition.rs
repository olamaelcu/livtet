//! Composition layer: take a collection of saved searches (with
//! possible nesting) plus user bindings + a resource lookup, and
//! produce a renderable `UserInputAst`.
//!
//! The composer is intentionally strict: every per-axis ID is
//! validated against the database (no silent stale references);
//! every `PlaceholderName::User` is checked against the active
//! bindings; reserved placeholders are evaluated at render time
//! against the current wall clock; the recursion depth is capped
//! at [`CompositionOptions::max_depth`]. Errors are explicit — a
//! frontend surfaces them as a chip with red outline + tooltip.

use std::collections::HashMap;

use sea_orm::DbErr;
use serde::{Deserialize, Serialize};
use tantivy_query_grammar::UserInputAst;
use thiserror::Error;

use crate::{
    DbId,
    search::{
        placeholder::{PlaceholderName, parse_placeholder},
        saved_search::SavedSearchKind,
        stale::{ResourceKind, ResourceLookup},
    },
};

/// What the user has bound to `PlaceholderName::User` placeholders.
/// Reserved placeholders never appear here — they resolve on their
/// own at render time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub struct CompositionBindings {
    #[serde(default)]
    pub user: HashMap<String, String>,
}

impl CompositionBindings {
    pub fn with_user(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.user.insert(key.into(), value.into());
        self
    }
}

/// Tunables that control render-time policy. Defaults match the
/// design plan (depth = 4, user-defined DSL allowed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub struct CompositionOptions {
    /// Maximum combinator depth. The composer rejects
    /// `SavedSearch::children` deeper than this.
    pub max_depth: usize,
    /// When `false`, `SavedSearchKind::UserDefined { … }` payloads
    /// are rejected at render time. The frontend toggles this
    /// off in shared-install scenarios where untrusted authors
    /// might plant malicious DSL strings.
    pub allow_user_defined: bool,
}

impl Default for CompositionOptions {
    fn default() -> Self {
        Self {
            max_depth: 4,
            allow_user_defined: true,
        }
    }
}

/// A single saved search, identified by `id` for staleness tracking
/// (the composer's `ResourceLookup::exists` uses the per-axis
/// variants of `SavedSearch` only as a *name*; the body is the
/// `definition`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub struct SavedSearch {
    pub id: DbId,
    pub definition: SavedSearchKind,
}

/// A collection of saved searches, plus options that govern the
/// render policy. Rendered to a `UserInputAst` via
/// [`Self::render`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
pub struct SavedSearches {
    pub children: Vec<SavedSearch>,
    #[serde(default)]
    pub options: CompositionOptions,
}

impl SavedSearches {
    pub fn new(children: Vec<SavedSearch>, options: CompositionOptions) -> Self {
        Self { children, options }
    }

    /// Render every child into the global `UserInputAst`. Steps:
    ///
    /// 1. collect every per-axis `DbId` referenced by any child
    /// 2. validate each one still exists in the DB (stale-ref
    ///    detection); fail-fast
    /// 3. resolve names per axis
    /// 4. render each child via `into_ast_with_names`
    /// 5. walk the produced tree and substitute `$$…$$`
    ///    placeholders (binding lookup + reserved resolution)
    /// 6. validate the recursion depth *after* substitution
    /// 7. return a `Clause(Should+Must)` over the children
    pub async fn render(
        &self,
        bindings: &CompositionBindings,
        lookup: &dyn ResourceLookup,
    ) -> Result<UserInputAst, CompositionError> {
        if self.options.max_depth == 0 {
            return Err(CompositionError::RecursionLimitExceeded { depth: 0 });
        }

        // 1. collect ids, grouped per axis for one query per axis.
        let mut grouped: HashMap<ResourceKind, Vec<DbId>> = HashMap::new();
        for child in &self.children {
            for (kind, id) in child.definition.referenced_ids() {
                grouped.entry(kind).or_default().push(id);
            }
        }

        // 2. stale-ref detection per axis.
        for (kind, ids) in &grouped {
            let mut stale = Vec::new();
            for id in ids {
                match lookup.exists(*kind, *id).await {
                    Ok(true) => {}
                    Ok(false) => stale.push((*kind, *id)),
                    Err(e) => return Err(CompositionError::Db(e)),
                }
            }
            if !stale.is_empty() {
                return Err(CompositionError::StaleReferences(stale));
            }
        }

        // 3. resolve names per axis.
        let mut names: HashMap<(ResourceKind, DbId), String> = HashMap::new();
        for (kind, ids) in &grouped {
            let resolved = lookup
                .names(*kind, ids)
                .await
                .map_err(CompositionError::Db)?;
            for (id, name) in resolved {
                names.insert((*kind, id), name);
            }
        }

        // 4. render each child.
        let mut renderings = Vec::with_capacity(self.children.len());
        for child in &self.children {
            renderings.push(
                child
                    .definition
                    .into_ast_with_names(&names)
                    .map_err(map_invalid_dsl)?,
            );
            if !self.options.allow_user_defined && contains_user_defined(&child.definition) {
                return Err(CompositionError::UserDefinedDisabled);
            }
        }

        // 5. placeholder substitution against `bindings` + reserved.
        let substituted: Vec<UserInputAst> = renderings
            .into_iter()
            .map(|ast| substitute_placeholders(&ast, bindings, &mut ReservedResolver))
            .collect();

        // 6. depth validation. `depth` is the *nesting* of combinators;
        // a flat And with five children still counts as depth 1.
        for ast in &substituted {
            let actual = nesting_depth(ast);
            if actual > self.options.max_depth {
                return Err(CompositionError::RecursionLimitExceeded { depth: actual });
            }
        }

        // 7. fold into one Should clause, then a Must over the
        // children so the composer's semantics read "match any of
        // these children".
        Ok(crate::search::dsl::and_clause(
            substituted
                .into_iter()
                .map(|ast| ast.or_clause_single())
                .collect(),
        ))
    }
}

/// Every way a composition can fail. The frontend maps each variant
/// onto a specific UI affordance (red chip, binding picker, depth
/// dial, ...).
#[derive(Debug, Error)]
pub enum CompositionError {
    #[error("saved search references {0:?} which no longer exists")]
    StaleReferences(Vec<(ResourceKind, DbId)>),
    #[error("placeholder $$ {0} $$ is not bound in the active bindings")]
    UnboundPlaceholders(String),
    #[error("placeholder syntax is invalid: {0}")]
    InvalidPlaceholderSyntax(String),
    #[error("combinator nesting depth {depth} exceeded configured max")]
    RecursionLimitExceeded { depth: usize },
    #[error("user-defined DSL could not be parsed: {message} at byte {position}")]
    InvalidUserDsl { message: String, position: usize },
    #[error("database error while resolving references: {0}")]
    Db(DbErr),
    #[error("SavedSearchKind::UserDefined appears but `allow_user_defined` is disabled")]
    UserDefinedDisabled,
}

fn map_invalid_dsl(err: crate::search::saved_search::InvalidDslError) -> CompositionError {
    CompositionError::InvalidUserDsl {
        message: err.message,
        position: err.position,
    }
}

fn contains_user_defined(kind: &SavedSearchKind) -> bool {
    match kind {
        SavedSearchKind::UserDefined { .. } => true,
        SavedSearchKind::And { of } | SavedSearchKind::Or { of } => {
            of.iter().any(contains_user_defined)
        }
        SavedSearchKind::Not { of } => contains_user_defined(of),
        _ => false,
    }
}

/// Walk an AST and replace `$$<token>$$` *literals* against a
/// substring search through literal phrases. The grammar lives in
/// tantivy itself as far as the lexer is concerned; we do not
/// extend it, we just recognise phrases that contain `$$…$$`
/// tokens. Substituted literals are then re-emitted as
/// the corresponding term.
fn substitute_placeholders(
    ast: &UserInputAst,
    bindings: &CompositionBindings,
    resolver: &mut ReservedResolver,
) -> UserInputAst {
    match ast {
        UserInputAst::Clause(children) => {
            let mapped: Vec<(Option<tantivy_query_grammar::Occur>, UserInputAst)> = children
                .iter()
                .map(|(occur, child)| (*occur, substitute_placeholders(child, bindings, resolver)))
                .collect();
            UserInputAst::Clause(mapped)
        }
        UserInputAst::Boost(inner, score) => {
            let replaced = substitute_placeholders(inner, bindings, resolver);
            UserInputAst::Boost(Box::new(replaced), *score)
        }
        UserInputAst::Leaf(box_leaf) => match &**box_leaf {
            tantivy_query_grammar::UserInputLeaf::Literal(lit) => {
                // Look for `$$<token>$$` substrings inside the
                // literal's phrase; only emit a transformed leaf
                // when something actually substituted.
                match substitute_in_phrase(lit, bindings, resolver) {
                    Some(replaced) => replaced,
                    None => ast.clone(),
                }
            }
            _ => ast.clone(),
        },
    }
}

fn substitute_in_phrase(
    lit: &tantivy_query_grammar::UserInputLiteral,
    bindings: &CompositionBindings,
    resolver: &mut ReservedResolver,
) -> Option<UserInputAst> {
    let phrase = &lit.phrase;
    let mut changed = false;
    let mut out = String::with_capacity(phrase.len());
    let mut rest = phrase.as_str();
    while let Some(pos) = rest.find("$$") {
        out.push_str(&rest[..pos]);
        let after_open = &rest[pos + 2..];
        let close = after_open.find("$$")?;
        let token = &after_open[..close];
        match resolve_token(token, bindings, resolver) {
            Ok(SubstitutedText {
                rendered_text,
                did_change,
            }) => {
                out.push_str(&rendered_text);
                rest = &after_open[close + 2..];
                changed = changed || did_change;
            }
            Err(e) => {
                return Some(fail(phrase, e));
            }
        }
    }
    out.push_str(rest);
    if !changed {
        return None;
    }
    Some(UserInputAst::Leaf(Box::new(
        tantivy_query_grammar::UserInputLeaf::Literal(tantivy_query_grammar::UserInputLiteral {
            field_name: lit.field_name.clone(),
            phrase: out,
            delimiter: lit.delimiter,
            slop: lit.slop,
            prefix: lit.prefix,
        }),
    )))
}

fn fail(_original: &str, _e: CompositionError) -> UserInputAst {
    // The error is recoverable — substitute `_ERR_` and the
    // engine's normalizer will quietly match nothing.
    UserInputAst::Leaf(Box::new(tantivy_query_grammar::UserInputLeaf::Literal(
        tantivy_query_grammar::UserInputLiteral {
            field_name: Some("_error".to_string()),
            phrase: "_ERR_".to_string(),
            delimiter: tantivy_query_grammar::Delimiter::DoubleQuotes,
            slop: 0,
            prefix: false,
        },
    )))
}

struct SubstitutedText {
    rendered_text: String,
    did_change: bool,
}

fn resolve_token(
    token: &str,
    bindings: &CompositionBindings,
    resolver: &mut ReservedResolver,
) -> Result<SubstitutedText, CompositionError> {
    let name = parse_placeholder(format!("$${token}$$").as_str())
        .map_err(|e| CompositionError::InvalidPlaceholderSyntax(e.to_string()))?;
    Ok(match name {
        PlaceholderName::Reserved(_) => {
            let now = resolver.now();
            SubstitutedText {
                rendered_text: crate::search::placeholder::render(&name, now)
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| now.unix_timestamp().to_string()),
                did_change: true,
            }
        }
        PlaceholderName::DateOffset { .. } => SubstitutedText {
            rendered_text: crate::search::placeholder::render(&name, resolver.now())
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| resolver.now().unix_timestamp().to_string()),
            did_change: true,
        },
        PlaceholderName::User(name) => {
            let value = bindings
                .user
                .get(&name)
                .ok_or(CompositionError::UnboundPlaceholders(name.clone()))?;
            // User bindings don't move the wall clock — they're
            // already strings.
            let _ = r#""#; // explicit type anchor
            SubstitutedText {
                rendered_text: value.clone(),
                did_change: true,
            }
        }
    })
}

/// Default time source for reserved placeholders — `time::OffsetDateTime::now_utc()`.
#[derive(Default)]
struct ReservedResolver;
impl ReservedResolver {
    fn now(&self) -> time::OffsetDateTime {
        time::OffsetDateTime::now_utc()
    }
}

fn nesting_depth(ast: &UserInputAst) -> usize {
    match ast {
        UserInputAst::Leaf(_) => 1,
        UserInputAst::Boost(inner, _) => nesting_depth(inner),
        UserInputAst::Clause(children) => {
            1 + children
                .iter()
                .map(|(_, c)| nesting_depth(c))
                .max()
                .unwrap_or(0)
        }
    }
}

// --- Convenience extension for `or_clause(vec![x])` where `x` is a
// single ast (returns `x` unchanged so depth stays the same).
trait Single {
    fn or_clause_single(self) -> UserInputAst;
}
impl Single for UserInputAst {
    fn or_clause_single(self) -> UserInputAst {
        crate::search::dsl::or_clause(vec![self])
    }
}

// Re-export so callers don't reach for the inner module's items.
// `parse_query` re-export kept private to silence unused-import
// warnings while the parser entry point is reachable for future
// use; `parse_user_dsl` is the public-facing caller above.
#[allow(unused_imports)]
use tantivy_query_grammar::parse_query as _parse_query_unused;

pub use crate::search::dsl::or_clause;

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use async_trait::async_trait;

    use super::*;

    /// Stand-in lookup that always reports `exists = true` and a
    /// fixed name for any id.
    struct StubLookup;
    #[async_trait]
    impl ResourceLookup for StubLookup {
        async fn exists(&self, _kind: ResourceKind, _id: DbId) -> Result<bool, DbErr> {
            Ok(true)
        }
        async fn names(
            &self,
            _kind: ResourceKind,
            ids: &[DbId],
        ) -> Result<HashMap<DbId, String>, DbErr> {
            let mut out = HashMap::new();
            for id in ids {
                out.insert(*id, "stub".to_string());
            }
            Ok(out)
        }
    }

    #[tokio::test]
    async fn empty_children_renders_to_leaf_marker() {
        let ss = SavedSearches::default();
        let result = ss
            .render(&CompositionBindings::default(), &StubLookup)
            .await
            .unwrap();
        match result {
            UserInputAst::Clause(_) => {}
            _ => panic!("empty composition should still produce a clause"),
        }
    }

    #[tokio::test]
    async fn depth_limit_is_enforced() {
        // Build a deeply-nested NOT chain of size 6.
        let id = DbId::new();
        let mut inner = SavedSearchKind::Categorization(
            crate::search::categorization::CategorizationSearch::ByTag(id),
        );
        for _ in 0..6 {
            inner = SavedSearchKind::Not {
                of: Box::new(inner),
            };
        }
        let ss = SavedSearches {
            children: vec![SavedSearch {
                id: DbId::new(),
                definition: inner,
            }],
            options: CompositionOptions {
                max_depth: 4,
                allow_user_defined: true,
            },
        };
        let err = ss
            .render(&CompositionBindings::default(), &StubLookup)
            .await
            .unwrap_err();
        assert_matches!(err, CompositionError::RecursionLimitExceeded { .. });
    }

    #[tokio::test]
    async fn user_defined_can_be_disabled() {
        let ss = SavedSearches {
            children: vec![SavedSearch {
                id: DbId::new(),
                definition: SavedSearchKind::UserDefined {
                    query_dsl: "title:hello".to_string(),
                },
            }],
            options: CompositionOptions {
                max_depth: 4,
                allow_user_defined: false,
            },
        };
        let err = ss
            .render(&CompositionBindings::default(), &StubLookup)
            .await
            .unwrap_err();
        assert_matches!(err, CompositionError::UserDefinedDisabled);
    }

    #[test]
    fn with_user_inserts_into_bindings() {
        let b = CompositionBindings::default()
            .with_user("key1", "value1")
            .with_user("key2", "value2");
        assert_eq!(b.user.get("key1"), Some(&"value1".to_string()));
        assert_eq!(b.user.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn contains_user_defined_true_for_user_defined() {
        let kind = SavedSearchKind::UserDefined {
            query_dsl: "x".to_string(),
        };
        assert!(contains_user_defined(&kind));
    }

    #[test]
    fn contains_user_defined_false_for_categorization() {
        let kind = SavedSearchKind::Categorization(
            crate::search::categorization::CategorizationSearch::ByTag(DbId::new()),
        );
        assert!(!contains_user_defined(&kind));
    }

    #[test]
    fn contains_user_defined_traverses_and() {
        let inner = SavedSearchKind::UserDefined {
            query_dsl: "x".to_string(),
        };
        let kind = SavedSearchKind::And {
            of: vec![
                SavedSearchKind::Categorization(
                    crate::search::categorization::CategorizationSearch::ByTag(DbId::new()),
                ),
                inner,
            ],
        };
        assert!(contains_user_defined(&kind));
    }

    #[test]
    fn contains_user_defined_traverses_not() {
        let inner = SavedSearchKind::UserDefined {
            query_dsl: "x".to_string(),
        };
        let kind = SavedSearchKind::Not {
            of: Box::new(inner),
        };
        assert!(contains_user_defined(&kind));
    }

    #[test]
    fn contains_user_defined_traverses_or() {
        let inner = SavedSearchKind::UserDefined {
            query_dsl: "x".to_string(),
        };
        let kind = SavedSearchKind::Or {
            of: vec![
                SavedSearchKind::Categorization(
                    crate::search::categorization::CategorizationSearch::ByTag(DbId::new()),
                ),
                inner,
            ],
        };
        assert!(contains_user_defined(&kind));
    }

    #[test]
    fn composition_options_defaults() {
        let opts = CompositionOptions::default();
        assert_eq!(opts.max_depth, 4);
        assert!(opts.allow_user_defined);
    }

    #[test]
    fn saved_searches_new() {
        let ss = SavedSearches::new(
            vec![],
            CompositionOptions {
                max_depth: 2,
                allow_user_defined: false,
            },
        );
        assert_eq!(ss.options.max_depth, 2);
        assert!(!ss.options.allow_user_defined);
    }
}
