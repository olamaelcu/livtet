use crate::types::db_id::DbId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CacheKey {
    pub key: String,
    pub provider: String,
    pub identifier_type: String,
    pub identifier_value: String,
    pub size: String,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum FetchError {
    #[error("Network error reaching {url}: {message}")]
    #[diagnostic(code(livtet_core::cover::network_error))]
    Network { url: String, message: String },

    #[error("Provider has no cover for this key")]
    #[help("The cover provider doesn't have a cover for this ISBN/identifier")]
    #[diagnostic(code(livtet_core::cover::provider_not_found))]
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
        edition: &livtet_data::entities::edtions::Model,
        db: &livtet_data::orm::DatabaseConnection,
    ) -> std::result::Result<Vec<CacheKey>, livtet_data::orm::DbErr>;
    async fn fetch(&self, key: &CacheKey) -> std::result::Result<FetchedCover, FetchError>;
}
