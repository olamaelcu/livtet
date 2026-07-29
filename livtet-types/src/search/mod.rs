//! Type-level definitions for saved-search composition in Livtet.
//!
//! This module provides the enum families that model user-visible
//! search filters (categorical, text, format, limits, etc.), the
//! composition layer that renders them into tantivy
//! [`UserInputAst`] trees, and the DSL primitives for building those
//! trees.
//!
//! The types here are consumed by [`livtet-search`] (the Tantivy
//! indexing + search engine) and by the Tauri frontend commands that
//! serialise/deserialise saved searches.

pub mod availability;
pub mod categorization;
pub mod composition;
pub mod db;
pub mod dsl;
pub mod format;
pub mod identifier_search;
pub mod limit;
pub mod ordering;
pub mod placeholder;
pub mod saved_search;
pub mod stale;
pub mod text;

// Re-exports so the `#[macro_export]` macros in `identifier_search`
// can resolve `$crate::__<name>` paths at the crate root.
#[doc(hidden)]
pub use identifier_search::__field_exists_alias;
#[doc(hidden)]
pub use identifier_search::__identifier_kind_alias;
#[doc(hidden)]
pub use identifier_search::__identifier_search_alias;
#[doc(hidden)]
pub use identifier_search::__isbn_alias;
#[doc(hidden)]
pub use identifier_search::__must_not_alias;
/// Used by `identifier_search_enum!` / `identifier_search_to_ast!` macros to call
/// `paste::paste!` for generating `By<Kind>` / `Has<Kind>` enum variants.
#[doc(hidden)]
pub use identifier_search::__paste_use;
#[doc(hidden)]
pub use identifier_search::__user_input_ast_alias;
