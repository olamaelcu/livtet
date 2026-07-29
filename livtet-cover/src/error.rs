use thiserror::Error;

pub use super::fetcher::FetchError;

#[derive(Error, Debug)]
pub enum CoverError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Database error: {0}")]
    Db(#[from] livtet_database::orm::DbErr),
    #[error("Cache error: {0}")]
    Cache(String),
    #[error("Cover not found in cache: {0}")]
    NotFound(String),
    #[error("Fetch error: {0}")]
    Fetch(#[from] FetchError),
    #[error("Cover metadata encoding error: {0}")]
    Cover(String),
}

pub type CoverResult<T> = std::result::Result<T, CoverError>;
