use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoverError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Db(#[from] livtet_data::orm::DbErr),

    #[error("Cache key error: {0}")]
    Cache(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Fetch error: {0}")]
    Fetch(String),

    #[error("Encode error: {0}")]
    Encode(String),
}

pub type CoverResult<T> = Result<T, CoverError>;