//! Lower a `tantivy::query_grammar::UserInputAst` (produced by
//! `livtet_search_types::SavedSearches::render`) into a Tantivy
//! [`Query`] that can be executed against the index.
//!
//! The translator exists because the composition layer in
//! `livtet-search-types` is engine-agnostic — it emits
//! `UserInputAst` trees without owning a tantivy schema. Once a
//! search request reaches this crate, the AST has to be reified
//! into a concrete [`Box<dyn Query>`] using the
//! `SearchIndex`-bound [`QueryParser`](tantivy::query::QueryParser).
//!
//! We reach `UserInputAst` through tantivy's `pub use query_grammar`
//! re-export so this crate doesn't need a direct `tantivy-query-grammar`
//! dependency edge (tantivy re-exports the same types).

use tantivy::{query::Query, query_grammar::UserInputAst};

use crate::{SearchError, SearchIndex};

/// Lower a composed `UserInputAst` into a Tantivy `Box<dyn Query>`
/// that can be executed against [`SearchIndex`].
pub fn user_input_ast_to_query(
    index: &SearchIndex,
    ast: UserInputAst,
) -> Result<Box<dyn Query>, SearchError> {
    // Tantivy's `QueryParser` API splits its parse errors into two
    // distinct types: `parse_query(&str)` bubbles a `TantivyError`,
    // but `build_query_from_user_input_ast` returns a
    // `QueryParserError` (which has its own display but
    // tantivy provides `impl From<QueryParserError> for TantivyError`).
    // We funnel both into `SearchError::Tantivy` via an explicit
    // conversion rather than `?` because `SearchError` has no
    // direct `From<QueryParserError>` impl.
    index
        .get_query_parser()
        .build_query_from_user_input_ast(ast)
        .map_err(|e| SearchError::Tantivy(tantivy::TantivyError::from(e)))
}
