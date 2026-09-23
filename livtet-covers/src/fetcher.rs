use async_trait::async_trait;
use livtet_data::orm::{DatabaseConnection, DbErr};
use livtet_types::DbId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub provider: String,
    pub identifier_type: String,
    pub identifier_value: String,
    pub size: String,
    pub ext: String,
}

impl CacheKey {
    pub fn content_key(&self) -> String {
        format!(
            "{}::{}::{}::{}::{}",
            self.provider, self.identifier_type, self.identifier_value, self.size, self.ext
        )
    }
}

#[derive(Clone, Debug)]
pub struct FetchedCover {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Database error: {0}")]
    Db(#[from] DbErr),
}

#[async_trait]
pub trait CoverFetcher: Send + Sync {
    fn priority(&self) -> u8;

    async fn keys_for(
        &self,
        edition_id: DbId,
        db: &DatabaseConnection,
    ) -> Result<Vec<CacheKey>, FetchError>;

    async fn fetch(&self, key: &CacheKey) -> Result<FetchedCover, FetchError>;
}
