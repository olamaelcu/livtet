use async_trait::async_trait;
use blake3;
use camino::Utf8PathBuf;
use fs_err::tokio as fs;
use livtet_data::digital_inventory;
use livtet_data::orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use livtet_types::DbId;
use serde::{Deserialize, Serialize};
use specta::Type;
use tracing::warn;

use crate::error::{CoverError, CoverResult};

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct CachedCover {
    pub content_key: String,
    pub edition_id: DbId,
    pub blurhash: Option<String>,
    pub dominant_color: Option<String>,
    pub size: Option<u64>,
}

#[async_trait]
pub trait CoverStorage: Send + Sync {
    async fn store(&self, key: &super::fetcher::CacheKey, bytes: &[u8])
    -> CoverResult<CachedCover>;

    async fn copy_to_permanent(
        &self,
        key: &super::fetcher::CacheKey,
        edition_id: DbId,
        ext: &str,
    ) -> CoverResult<()>;

    async fn list_cached(
        &self,
        edition_id: DbId,
        db: &DatabaseConnection,
    ) -> CoverResult<Vec<CachedCover>>;

    fn permanent_path(&self, edition_id: DbId, ext: &str) -> CoverResult<Utf8PathBuf>;

    async fn remove(&self, edition_id: DbId) -> CoverResult<()>;
}

#[derive(Clone, Debug)]
pub struct FileCoverStorage {
    cache_dir: Utf8PathBuf,
    permanent_dir: Utf8PathBuf,
}

impl FileCoverStorage {
    pub async fn new(cache_dir: Utf8PathBuf, permanent_dir: Utf8PathBuf) -> CoverResult<Self> {
        fs::create_dir_all(cache_dir.join("entries")).await?;
        fs::create_dir_all(cache_dir.join("markers")).await?;
        fs::create_dir_all(&permanent_dir).await?;
        Ok(Self {
            cache_dir,
            permanent_dir,
        })
    }

    fn entries_dir(&self) -> Utf8PathBuf {
        self.cache_dir.join("entries")
    }

    fn markers_dir(&self) -> Utf8PathBuf {
        self.cache_dir.join("markers")
    }

    fn edition_markers_dir(&self, edition_id: DbId) -> Utf8PathBuf {
        self.markers_dir().join(edition_id.to_string())
    }

    fn edition_permanent_dir(&self, edition_id: DbId) -> Utf8PathBuf {
        self.permanent_dir.join(edition_id.to_string())
    }

    fn content_hash(&self, content_key: &str) -> String {
        let hash = blake3::hash(content_key.as_bytes());
        hash.to_hex().to_string()
    }

    fn entry_path(&self, content_key: &str) -> Utf8PathBuf {
        let hash = self.content_hash(content_key);
        self.entries_dir().join(hash)
    }
}

#[async_trait]
impl CoverStorage for FileCoverStorage {
    async fn store(
        &self,
        key: &super::fetcher::CacheKey,
        bytes: &[u8],
    ) -> CoverResult<CachedCover> {
        let content_key = key.content_key();
        let entry_path = self.entry_path(&content_key);

        if let Some(parent) = entry_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&entry_path, bytes).await?;

        let marker_dir =
            self.edition_markers_dir(key.identifier_value.parse::<DbId>().map_err(|_| {
                CoverError::Cache(format!(
                    "invalid edition_id in key: {}",
                    key.identifier_value
                ))
            })?);
        fs::create_dir_all(&marker_dir).await?;
        let marker_path = marker_dir.join(self.content_hash(&content_key));
        fs::write(&marker_path, content_key.as_bytes()).await?;

        Ok(CachedCover {
            content_key,
            edition_id: key.identifier_value.parse().map_err(|_| {
                CoverError::Cache(format!(
                    "invalid edition_id in key: {}",
                    key.identifier_value
                ))
            })?,
            blurhash: None,
            dominant_color: None,
            size: Some(bytes.len() as u64),
        })
    }

    async fn copy_to_permanent(
        &self,
        key: &super::fetcher::CacheKey,
        edition_id: DbId,
        ext: &str,
    ) -> CoverResult<()> {
        let content_key = key.content_key();
        let entry_path = self.entry_path(&content_key);

        let bytes = fs::read(&entry_path)
            .await
            .map_err(|e| CoverError::Cache(format!("cached entry not found: {}", e)))?;

        let perm_dir = self.edition_permanent_dir(edition_id);
        fs::create_dir_all(&perm_dir).await?;

        let ext_sanitized = ext
            .trim_start_matches('.')
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "");
        if ext_sanitized.is_empty() {
            return Err(CoverError::Cache(
                "empty extension after sanitization".into(),
            ));
        }

        let permanent_path = perm_dir.join(format!("cover.{}", ext_sanitized));
        fs::write(&permanent_path, &bytes).await?;

        Ok(())
    }

    async fn list_cached(
        &self,
        edition_id: DbId,
        db: &DatabaseConnection,
    ) -> CoverResult<Vec<CachedCover>> {
        let marker_dir = self.edition_markers_dir(edition_id);

        let mut entries = Vec::new();

        if !marker_dir.exists() {
            return Ok(entries);
        }

        let mut dir = fs::read_dir(&marker_dir).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|_| CoverError::Cache("non-UTF8 marker path".into()))?;

            let content_key = match fs::read_to_string(&path).await {
                Ok(s) => s.trim().to_string(),
                Err(e) => {
                    warn!("failed to read marker {}: {}", path, e);
                    continue;
                }
            };

            if content_key.is_empty() {
                warn!("empty marker file: {}", path);
                continue;
            }

            let hash = path
                .file_name()
                .and_then(|s| s.split('.').next())
                .unwrap_or("");

            let size = fs::metadata(self.entries_dir().join(hash))
                .await
                .ok()
                .map(|m| m.len());

            let db_row = digital_inventory::Entity::find()
                .filter(digital_inventory::Column::EditionId.eq(edition_id))
                .one(db)
                .await?;

            let (blurhash, dominant_color) = db_row
                .map(|m| (m.blurhash, m.dominant_color))
                .unwrap_or((None, None));

            entries.push(CachedCover {
                content_key,
                edition_id,
                blurhash,
                dominant_color,
                size,
            });
        }

        Ok(entries)
    }

    fn permanent_path(&self, edition_id: DbId, ext: &str) -> CoverResult<Utf8PathBuf> {
        let ext_sanitized = ext
            .trim_start_matches('.')
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "");
        if ext_sanitized.is_empty() {
            return Err(CoverError::Cache(
                "empty extension after sanitization".into(),
            ));
        }
        let perm_dir = self.edition_permanent_dir(edition_id);
        Ok(perm_dir.join(format!("cover.{}", ext_sanitized)))
    }

    async fn remove(&self, edition_id: DbId) -> CoverResult<()> {
        let perm_dir = self.edition_permanent_dir(edition_id);
        if perm_dir.exists() {
            fs::remove_dir_all(&perm_dir).await?;
        }

        let marker_dir = self.edition_markers_dir(edition_id);
        if marker_dir.exists() {
            fs::remove_dir_all(&marker_dir).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::CacheKey;

    fn key(edition_id: DbId) -> CacheKey {
        CacheKey {
            provider: "test".into(),
            identifier_type: "edition".into(),
            identifier_value: edition_id.to_string(),
            size: "orig".into(),
            ext: "png".into(),
        }
    }

    #[tokio::test]
    async fn store_copy_list_remove_roundtrip() {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let storage = FileCoverStorage::new(dir.path().join("cache"), dir.path().join("perm"))
            .await
            .unwrap();

        let state =
            livtet_data::SharedState::connect("sqlite::memory:", &[livtet_data::Kind::Business])
                .await
                .unwrap();
        let db = state.db_conn();

        let edition_id = DbId::new();
        let key = key(edition_id);
        let bytes = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3];

        let cached = storage.store(&key, &bytes).await.unwrap();
        assert_eq!(cached.edition_id, edition_id);
        assert_eq!(cached.size, Some(bytes.len() as u64));

        let listed = storage.list_cached(edition_id, &db).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].content_key, key.content_key());

        storage
            .copy_to_permanent(&key, edition_id, "png")
            .await
            .unwrap();
        let perm = storage.permanent_path(edition_id, "png").unwrap();
        assert!(perm.exists());

        storage.remove(edition_id).await.unwrap();
        assert!(!storage.edition_permanent_dir(edition_id).exists());
        assert!(!storage.edition_markers_dir(edition_id).exists());
    }

    #[tokio::test]
    async fn list_cached_empty_without_markers() {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let storage = FileCoverStorage::new(dir.path().join("cache"), dir.path().join("perm"))
            .await
            .unwrap();

        let state =
            livtet_data::SharedState::connect("sqlite::memory:", &[livtet_data::Kind::Business])
                .await
                .unwrap();
        let db = state.db_conn();

        let listed = storage.list_cached(DbId::new(), &db).await.unwrap();
        assert!(listed.is_empty());
    }
}
