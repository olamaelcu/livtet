use async_trait::async_trait;
use livtet_types::DbId;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct CacheKey {
    pub key: String,
    pub provider: String,
    pub identifier_type: String,
    pub identifier_value: String,
    pub size: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("Provider has no cover for this key")]
    NotFound,
}

#[derive(Debug)]
pub struct FetchedCover {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

#[async_trait]
pub trait CoverFetcher: Send + Sync {
    fn priority(&self) -> u8;
    async fn keys_for(
        &self,
        edition_id: DbId,
        db: &livtet_data::orm::DatabaseConnection,
    ) -> std::result::Result<Vec<CacheKey>, livtet_data::orm::DbErr>;
    async fn fetch(&self, key: &CacheKey) -> std::result::Result<FetchedCover, FetchError>;
}
