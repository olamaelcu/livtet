use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use super::fetcher::FetchError;

#[derive(Error, Debug, Diagnostic, Serialize, Deserialize)]
pub enum CoverError {
    #[error("IO error: {0}")]
    #[diagnostic(code(livtet_core::cover::io_error))]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    #[diagnostic(code(livtet_core::cover::db_error))]
    Db(#[from] livtet_database::orm::DbErr),

    #[error("Cache error: {0}")]
    #[diagnostic(code(livtet_core::cover::cache_error))]
    Cache(String),

    #[error("Cover not found: {key}")]
    #[help("Try refreshing the cover cache or checking the ISBN format")]
    #[diagnostic(code(livtet_core::cover::not_found))]
    NotFound { key: String },

    #[error("Fetch error: {0}")]
    #[diagnostic(code(livtet_core::cover::fetch_error))]
    Fetch(#[from] FetchError),
}

pub type CoverResult<T> = miette::Result<T, CoverError>;
