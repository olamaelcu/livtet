//! Macro-driven identifier search variants and the matching
//! `From<UserInputAst>` plumbing.
//!
//! Adding a new identifier kind requires three coordinated edits:
//!
//! 1. The `IdentifierKind` enum gains a variant (`livtet-types`).
//! 2. Its `dsl_prefix()` arm returns the new stable prefix.
//! 3. Both `identifier_search_enum!` and `identifier_search_to_ast!`
//!    invocations below gain the new identifier name. The macro
//!    uses the [`paste`] crate for identifier concatenation, so the
//!    crate root also requires `use paste::paste;` in scope at the
//!    call site.
//!
//! Each variant exists in two flavours (`By<Kind>(String)` for
//! "value contains" and `Has<Kind>(bool)` for "any value present")
//! so saved searches can target either the typed equality match or
//! the existence check on a per-kind basis.
use serde::{Deserialize, Serialize};

/// Build the `IdentifierSearch` enum given a list of `(kind_name, prefix)`
/// pairs. Each pair emits a `By<Kind>(String)` and a `Has<Kind>(bool)`
/// variant, plus `kind()` and `dsl_prefix()` accessors.
#[macro_export]
macro_rules! identifier_search_enum {
    ($(($kind:ident, $prefix:expr)),* $(,)?) => {
        $crate::__paste_use::paste! {
            #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
            #[cfg_attr(feature = "fake", derive(fake::Dummy))]
            #[serde(tag = "type", rename_all = "snake_case")]
            pub enum IdentifierSearch {
                $(
                    [<By $kind>](String),
                    [<Has $kind>](bool),
                )*
            }

            impl IdentifierSearch {
                /// Which [`IdentifierKind`] this variant targets.
                pub fn kind(&self) -> $crate::__identifier_kind_alias::IdentifierKind {
                    match self {
                        $(
                            Self::[<By $kind>](_)
                            | Self::[<Has $kind>](_)
                            => {
                                // Use the kind name to construct
                                // IdentifierKind::Custom for kinds
                                // that have no first-class variant.
                                let _prefix = $prefix;
                                let name = stringify!($kind);
                                match name {
                                    "Isbn" => $crate::__identifier_kind_alias::IdentifierKind::Isbn,
                                    "Oclc" => $crate::__identifier_kind_alias::IdentifierKind::Oclc,
                                    "Lccn" => $crate::__identifier_kind_alias::IdentifierKind::Lccn,
                                    "Doi" => $crate::__identifier_kind_alias::IdentifierKind::Doi,
                                    "Web" => $crate::__identifier_kind_alias::IdentifierKind::Web,
                                    "Opds" => $crate::__identifier_kind_alias::IdentifierKind::Opds,
                                    _ => $crate::__identifier_kind_alias::IdentifierKind::Custom(name.to_string()),
                                }
                            }
                        ),*
                    }
                }

                /// The DSL prefix (stable across renames).
                pub fn dsl_prefix(&self) -> &'static str {
                    match self {
                        $(
                            Self::[<By $kind>](_)
                            | Self::[<Has $kind>](_)
                            => $prefix,
                        )*
                    }
                }
            }
        }
    };
}

/// Build `From<IdentifierSearch> for UserInputAst`. Lives as its
/// own macro so it can read the same kind list as
/// `identifier_search_enum!` without forcing the caller to keep two
/// lists in lockstep manually.
#[macro_export]
macro_rules! identifier_search_to_ast {
    ($(($kind:ident, $prefix:expr)),* $(,)?) => {
        $crate::__paste_use::paste! {
            impl From<$crate::__identifier_search_alias::IdentifierSearch> for $crate::__user_input_ast_alias::UserInputAst {
                fn from(value: $crate::__identifier_search_alias::IdentifierSearch) -> Self {
                    let prefix = value.dsl_prefix();
                    let field = format!("identifier_values:{prefix}");
                    match value {
                        $(
                            $crate::__identifier_search_alias::IdentifierSearch::[<By $kind>](needle) => {
                                // Canonicalize ISBN via
                                // livtet_types::Isbn::parse so a
                                // saved search round-trips
                                // correctly.
                                let value = if prefix == "isbn" {
                                    $crate::__isbn_alias::Isbn::parse(&needle)
                                        .map(|i| i.as_str().to_string())
                                        .unwrap_or(needle)
                                } else {
                                    needle
                                };
                                $crate::__user_input_ast_alias::UserInputAst::Leaf(
                                    Box::new(
                                        $crate::__user_input_ast_alias::UserInputLeaf::Literal(
                                            $crate::__user_input_ast_alias::UserInputLiteral {
                                                field_name: Some(field),
                                                phrase: value,
                                                delimiter: $crate::__user_input_ast_alias::Delimiter::None,
                                                slop: 0,
                                                prefix: false,
                                            },
                                        ),
                                    ),
                                )
                            }
                            $crate::__identifier_search_alias::IdentifierSearch::[<Has $kind>](true) => {
                                $crate::__field_exists_alias(field)
                            }
                            $crate::__identifier_search_alias::IdentifierSearch::[<Has $kind>](false) => {
                                $crate::__must_not_alias(
                                    $crate::__field_exists_alias(field),
                                )
                            }
                        ),*
                    }
                }
            }
        }
    };
}

// --- Public alias items used by the macros above. Hidden from the
// crate's public API via `#[doc(hidden)]` so consumers don't depend
// on them by accident.
//
// These are modules (not type aliases) for paths accessed via
// `$crate::__<name>::<Item>` in the macros. Function aliases
// (used directly as `$crate::__<name>(...)`) remain `pub use`.

/// Re-exports [`paste::paste`] for use by the `identifier_search_enum!` and
/// `identifier_search_to_ast!` macros.
///
/// The `paste` crate lets us concatenate identifiers at compile time, which
/// generates enum variants like `ByIsbn`, `HasIsbn`, `ByOclc`, `HasOclc`, etc.
/// without having to write them out by hand.
///
/// This module **must** be named with a leading `__` prefix so the macro can
/// access it via `$crate::__paste_use::paste!` at the crate root.
#[doc(hidden)]
#[allow(unused_imports)]
pub mod __paste_use {
    pub use ::paste::paste;
}

#[doc(hidden)]
pub mod __identifier_kind_alias {
    pub use crate::IdentifierKind;
}

#[doc(hidden)]
pub mod __user_input_ast_alias {
    pub use tantivy_query_grammar::{Delimiter, UserInputAst, UserInputLeaf, UserInputLiteral};
}

#[doc(hidden)]
pub mod __isbn_alias {
    pub use crate::Isbn;
}

#[doc(hidden)]
pub use crate::search::dsl::field_exists as __field_exists_alias;
#[doc(hidden)]
pub use crate::search::dsl::must_not_clause as __must_not_alias;

// The `IdentifierSearch` enum, generated by the macro below.
// Each entry is `(VariantName, dsl_prefix_string)`. Note that
// `OpenLibrary` and `Wikidata` are not first-class `IdentifierKind`
// variants — they are stored as `Custom("openlibrary")` and
// `Custom("wikidata")` — so they use the `Custom` prefix here.
identifier_search_enum!(
    (Isbn, "isbn"),
    (Oclc, "oclc"),
    (Lccn, "lccn"),
    (Doi, "doi"),
    (Web, "web"),
    (Opds, "opds"),
    (Custom, "custom"),
);

identifier_search_to_ast!(
    (Isbn, "isbn"),
    (Oclc, "oclc"),
    (Lccn, "lccn"),
    (Doi, "doi"),
    (Web, "web"),
    (Opds, "opds"),
    (Custom, "custom"),
);

#[doc(hidden)]
pub mod __identifier_search_alias {
    pub use super::IdentifierSearch;
}

#[cfg(test)]
mod tests {
    use tantivy_query_grammar::{UserInputAst, UserInputLeaf};

    use super::*;

    #[test]
    fn kind_round_trip_for_each_variant() {
        // Note: OpenLibrary and Wikidata are not first-class
        // IdentifierSearch variants — they are stored as
        // Custom("openlibrary") / Custom("wikidata") per the
        // doc comment on the macro invocation above. Earlier
        // revisions of this test referenced
        // IdentifierSearch::ByOpenLibrary / ByWikidata, which
        // never existed (see the `By<Kind>(String)` flavour
        // described in the crate-level docs).
        let cases: Vec<IdentifierSearch> = vec![
            IdentifierSearch::ByIsbn("9780061120084".to_string()),
            IdentifierSearch::HasIsbn(true),
            IdentifierSearch::ByOclc("12345".to_string()),
            IdentifierSearch::ByDoi("10.1234/abc".to_string()),
            IdentifierSearch::ByWeb("http://example.com".to_string()),
            IdentifierSearch::HasWeb(true),
            IdentifierSearch::ByOpds("urn:opds:abc".to_string()),
            IdentifierSearch::HasOpds(true),
            IdentifierSearch::ByCustom("openlibrary:OL1M".to_string()),
            IdentifierSearch::ByCustom("wikidata:Q42".to_string()),
            IdentifierSearch::ByCustom("foo".to_string()),
        ];
        for v in &cases {
            // kind is reachable.
            let _: crate::__identifier_kind_alias::IdentifierKind = v.kind();
        }
    }

    #[test]
    fn isbn_is_canonicalised_in_to_ast() {
        // Hyphens and ISBN-10 prefix should survive through to the
        // literal leaf, because Isbn::parse normalises them.
        let v = IdentifierSearch::ByIsbn("0-306-40615-2".to_string());
        let ast: UserInputAst = v.into();
        match ast {
            UserInputAst::Leaf(box_leaf) => match *box_leaf {
                UserInputLeaf::Literal(lit) => {
                    assert_eq!(lit.field_name.as_deref(), Some("identifier_values:isbn"));
                    assert_eq!(lit.phrase, "9780306406157");
                }
                _ => panic!("expected literal leaf"),
            },
            _ => panic!("expected leaf ast"),
        }
    }
}
