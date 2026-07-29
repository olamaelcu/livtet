use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use camino::Utf8PathBuf;
use livtet_cover::*;
use livtet_types::DbId;

struct TestStorage {
    store: Mutex<HashMap<String, Vec<u8>>>,
    copy_calls: Mutex<Vec<(String, DbId)>>,
}

impl TestStorage {
    fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
            copy_calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl CoverStorage for TestStorage {
    async fn list_cached(&self, _inventory_id: DbId) -> CoverResult<Vec<CachedCover>> {
        Ok(vec![])
    }

    async fn store(&mut self, key: &str, bytes: &[u8]) -> CoverResult<()> {
        self.store
            .lock()
            .unwrap()
            .insert(key.to_string(), bytes.to_vec());
        Ok(())
    }

    async fn copy_to_permanent(
        &mut self,
        cache_key: &str,
        inventory_id: DbId,
    ) -> CoverResult<String> {
        self.copy_calls
            .lock()
            .unwrap()
            .push((cache_key.to_string(), inventory_id));
        let path = self.permanent_path(inventory_id, "jpg");
        Ok(path.to_string())
    }

    fn permanent_path(&self, inventory_id: DbId, ext: &str) -> Utf8PathBuf {
        let safe_ext = ext.replace(['.', '/', '\\'], "");
        Utf8PathBuf::from(format!("/covers/{}/cover.{}", inventory_id, safe_ext))
    }
}

struct TestFetcher {
    priority_val: u8,
    data: Vec<u8>,
}

impl TestFetcher {
    fn new(priority: u8, data: Vec<u8>) -> Self {
        Self {
            priority_val: priority,
            data,
        }
    }
}

#[async_trait]
impl CoverFetcher for TestFetcher {
    fn priority(&self) -> u8 {
        self.priority_val
    }

    async fn keys_for(
        &self,
        _edition_id: DbId,
        _db: &livtet_database::orm::DatabaseConnection,
    ) -> Result<Vec<CacheKey>, livtet_database::orm::DbErr> {
        Ok(vec![CacheKey {
            key: "cover-s".to_string(),
            provider: "test".to_string(),
            identifier_type: "isbn".to_string(),
            identifier_value: "978-0-000-00000-0".to_string(),
            size: "S".to_string(),
        }])
    }

    async fn fetch(&self, _key: &CacheKey) -> Result<FetchedCover, FetchError> {
        Ok(FetchedCover {
            bytes: self.data.clone(),
            content_type: Some("image/jpeg".to_string()),
        })
    }
}

#[tokio::test]
async fn test_fetcher_returns_bytes() {
    let fetcher = TestFetcher::new(10, vec![0xff, 0xd8, 0xff]);
    let key = CacheKey {
        key: "cover-s".to_string(),
        provider: "test".to_string(),
        identifier_type: "isbn".to_string(),
        identifier_value: "978-0-000-00000-0".to_string(),
        size: "S".to_string(),
    };
    let result = fetcher.fetch(&key).await.unwrap();
    assert_eq!(result.bytes, vec![0xff, 0xd8, 0xff]);
    assert_eq!(result.content_type.as_deref(), Some("image/jpeg"));
}

#[tokio::test]
async fn test_fetcher_priority() {
    let fetcher = TestFetcher::new(5, vec![]);
    assert_eq!(fetcher.priority(), 5);
}

#[tokio::test]
async fn test_storage_store_and_get() {
    let mut storage = TestStorage::new();
    storage.store("k1", &[1, 2, 3]).await.unwrap();
    let stored = storage.store.lock().unwrap();
    assert_eq!(stored.get("k1"), Some(&vec![1, 2, 3]));
}

#[tokio::test]
async fn test_storage_permanent_path_uses_inventory_id() {
    let storage = TestStorage::new();
    let id = DbId::new();
    let path = storage.permanent_path(id, "png");
    assert!(path.as_str().contains(&id.to_string()));
    assert!(path.as_str().ends_with(".png"));
}

#[tokio::test]
async fn test_storage_copy_to_permanent_records_call() {
    let mut storage = TestStorage::new();
    let id = DbId::new();
    storage.store("ck", &[0xaa]).await.unwrap();
    let result = storage.copy_to_permanent("ck", id).await.unwrap();
    assert!(result.contains(&id.to_string()));
    let calls = storage.copy_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "ck");
    assert_eq!(calls[0].1, id);
}

#[test]
fn test_permanent_path_rejects_path_traversal() {
    let storage = TestStorage::new();
    let path = storage.permanent_path(DbId::new(), "../../etc/passwd");
    let s = path.as_str();
    assert!(
        !s.contains(".."),
        "permanent_path must reject or sanitize path traversal: got {s}"
    );
}
