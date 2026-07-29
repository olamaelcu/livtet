use async_trait::async_trait;
use camino::Utf8PathBuf;
use livtet_types::DbId;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::CoverResult;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CachedCover {
    pub key: String,
    pub provider: String,
    pub size: String,
    pub ext: String,
    pub bytes: Vec<u8>,
    pub inventory_id: DbId,
    pub display_path: Option<String>,
}

#[async_trait]
pub trait CoverStorage: Send + Sync {
    async fn list_cached(&self, inventory_id: DbId) -> CoverResult<Vec<CachedCover>>;
    async fn store(&mut self, key: &str, bytes: &[u8]) -> CoverResult<()>;
    async fn copy_to_permanent(
        &mut self,
        cache_key: &str,
        inventory_id: DbId,
    ) -> CoverResult<String>;
    fn permanent_path(&self, inventory_id: DbId, ext: &str) -> Utf8PathBuf;
}
