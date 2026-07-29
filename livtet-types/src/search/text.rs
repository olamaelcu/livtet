//! Free-text filters. Each variant emits a phrase literal against
//! the named full-text field; tantivy's normalizer does the
//! tokenisation. We don't include stemming here — the parser
//! configuration in `livtet-search` chooses per-field analyzers.
//!
//! Phrase fields use the double-quoted delimiter so a query like
//! `ByTitle { phrase: "lord of the rings" }` requires the whole
//! phrase to appear, not any of the words.

use serde::{Deserialize, Serialize};
use tantivy_query_grammar::UserInputAst;

use crate::search::dsl::field_term_exact;

/// Free-text field filters. Each is a phrase literal against the
/// named schema field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "fake", derive(fake::Dummy))]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum TextSearch {
    /// `title:"<phrase>"`.
    ByTitle { phrase: String },
    /// `edition_description:"<phrase>"`.
    ByDescription { phrase: String },
    /// `notes:"<phrase>"`.
    ByNotes { phrase: String },
    /// `body:"<phrase>"` (full-text indexed body).
    FullText { phrase: String },
}

impl TextSearch {
    pub fn into_ast(&self) -> UserInputAst {
        match self {
            TextSearch::ByTitle { phrase } => field_term_exact("title", phrase.clone()),
            TextSearch::ByDescription { phrase } => {
                field_term_exact("edition_description", phrase.clone())
            }
            TextSearch::ByNotes { phrase } => field_term_exact("notes", phrase.clone()),
            TextSearch::FullText { phrase } => field_term_exact("body", phrase.clone()),
        }
    }
}

impl From<TextSearch> for UserInputAst {
    fn from(value: TextSearch) -> Self {
        value.into_ast()
    }
}
