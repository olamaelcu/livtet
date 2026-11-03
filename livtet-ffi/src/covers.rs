//! On-disk cover cache (methods on [`LivtetStore`]).
//!
//! Covers used to come from the network on every render. `livtet-covers`
//! already owns the on-disk layout — blake3-hashed entries under a cache root
//! plus one marker per edition, and a permanent copy per edition — so this
//! module only exposes that layout across the FFI boundary.
//!
//! The foreign side still does its own HTTP: it hands the bytes it downloaded
//! to [`LivtetStore::store_cover`] and asks [`LivtetStore::cover_cache_path`]
//! first next time. Keeping the transfer out of here means the mobile clients
//! use their own networking stack rather than pulling one into core.

use camino::Utf8PathBuf;
use livtet_core::types::{DbId, DiskPath};
use livtet_covers::{CacheKey, CoverStorage, FileCoverStorage};

use crate::error::LivtetError;
use crate::store::LivtetStore;

/// Provider stamped onto every key this module writes. The cache key's other
/// fields describe which image of an edition it is; a catalog import has
/// exactly one, so the edition id plus the extension identify it.
const COVER_PROVIDER: &str = "catalog";
const COVER_IDENTIFIER_TYPE: &str = "edition";
const COVER_SIZE: &str = "orig";

#[uniffi::export(async_runtime = "tokio")]
impl LivtetStore {
    /// Where the cached cover for `edition_id` lives, if it has been stored.
    ///
    /// Returns `None` when the edition has no cached cover, which is the
    /// signal for the caller to download one and call [`Self::store_cover`].
    /// Never touches the network.
    pub async fn cover_cache_path(
        &self,
        edition_id: DbId,
        cache_root: String,
    ) -> Result<Option<DiskPath>, LivtetError> {
        let storage = storage_at(cache_root).await?;
        let db = self.state.db_conn();

        let cached = storage
            .list_cached(edition_id, &db)
            .await
            .map_err(map_cover_error)?;
        let Some(entry) = cached.first() else {
            return Ok(None);
        };

        let Some(ext) = extension_of(&entry.content_key) else {
            return Ok(None);
        };
        let path = storage
            .permanent_path(edition_id, ext)
            .map_err(map_cover_error)?;
        if path.exists() {
            return Ok(Some(DiskPath::from_path(path.as_path())));
        }

        // The entry survived but its permanent copy did not (a cleared
        // app cache keeps one and not the other); rebuild it from the entry.
        let key = key_from_content_key(&entry.content_key, edition_id)?;
        storage
            .copy_to_permanent(&key, edition_id, ext)
            .await
            .map_err(map_cover_error)?;
        let path = storage
            .permanent_path(edition_id, ext)
            .map_err(map_cover_error)?;
        Ok(Some(DiskPath::from_path(path.as_path())))
    }

    /// Store downloaded cover bytes for `edition_id` and return the file the
    /// UI should load.
    ///
    /// Overwrites any earlier cover for the edition, so a corrected image
    /// replaces a wrong one.
    pub async fn store_cover(
        &self,
        edition_id: DbId,
        cache_root: String,
        ext: String,
        bytes: Vec<u8>,
    ) -> Result<DiskPath, LivtetError> {
        if bytes.is_empty() {
            return Err(LivtetError::InvalidInput(
                "cover bytes are empty".to_string(),
            ));
        }
        let ext = clean_extension(&ext)?;
        let storage = storage_at(cache_root).await?;

        let key = CacheKey {
            provider: COVER_PROVIDER.to_string(),
            identifier_type: COVER_IDENTIFIER_TYPE.to_string(),
            identifier_value: edition_id.to_string(),
            size: COVER_SIZE.to_string(),
            ext: ext.clone(),
        };

        storage.store(&key, &bytes).await.map_err(map_cover_error)?;
        storage
            .copy_to_permanent(&key, edition_id, &ext)
            .await
            .map_err(map_cover_error)?;

        let path = storage
            .permanent_path(edition_id, &ext)
            .map_err(map_cover_error)?;
        Ok(DiskPath::from_path(path.as_path()))
    }

    /// Drop every cached cover for an edition.
    pub async fn forget_cover(
        &self,
        edition_id: DbId,
        cache_root: String,
    ) -> Result<(), LivtetError> {
        storage_at(cache_root)
            .await?
            .remove(edition_id)
            .await
            .map_err(map_cover_error)
    }
}

/// Open the storage rooted at `<cache_root>/covers`.
///
/// The crate keeps two trees: `cache` (evictable entries and markers) and
/// `permanent` (the copy the UI reads). Both live under the app's cache
/// directory, which the caller owns.
async fn storage_at(cache_root: String) -> Result<FileCoverStorage, LivtetError> {
    let root = Utf8PathBuf::from(cache_root.trim());
    if root.as_str().is_empty() {
        return Err(LivtetError::InvalidInput(
            "cover cache root must not be empty".to_string(),
        ));
    }
    let base = root.join("covers");
    FileCoverStorage::new(base.join("cache"), base.join("permanent"))
        .await
        .map_err(map_cover_error)
}

/// The extension a content key ends with, or `None` when it has none.
fn extension_of(content_key: &str) -> Option<&str> {
    let ext = content_key.rsplit("::").next()?.trim();
    (!ext.is_empty()).then_some(ext)
}

/// Rebuild the key a stored entry was written with.
fn key_from_content_key(content_key: &str, edition_id: DbId) -> Result<CacheKey, LivtetError> {
    let parts: Vec<&str> = content_key.split("::").collect();
    if parts.len() != 5 {
        return Err(LivtetError::InvalidInput(format!(
            "malformed cover content key: {content_key}"
        )));
    }
    Ok(CacheKey {
        provider: parts[0].to_string(),
        identifier_type: parts[1].to_string(),
        identifier_value: edition_id.to_string(),
        size: parts[3].to_string(),
        ext: parts[4].to_string(),
    })
}

/// Normalise a caller-supplied extension: leading dots dropped, anything that
/// is not alphanumeric removed.
fn clean_extension(ext: &str) -> Result<String, LivtetError> {
    let cleaned: String = ext
        .trim()
        .trim_start_matches('.')
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if cleaned.is_empty() {
        return Err(LivtetError::InvalidInput(
            "cover extension must not be empty".to_string(),
        ));
    }
    Ok(cleaned.to_ascii_lowercase())
}

fn map_cover_error(error: livtet_covers::CoverError) -> LivtetError {
    LivtetError::InvalidInput(format!("cover cache: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    async fn store_with_edition() -> (camino_tempfile::Utf8TempDir, Arc<LivtetStore>, DbId) {
        let tmp = camino_tempfile::tempdir().unwrap();
        let store = LivtetStore::open(
            tmp.path().join("livtet.db").to_string(),
            tmp.path().join("index").to_string(),
        )
        .await
        .expect("open store");

        livtet_core::data::seed::seed_database(
            &store.state.db_conn(),
            &livtet_core::data::seed::SeedConfig {
                num_works: 1,
                ..Default::default()
            },
        )
        .await
        .expect("seed");

        let work = &store.list_works(1, 0, None, None).await.unwrap()[0];
        let edition = store.list_editions(work.id).await.unwrap()[0].id;
        (tmp, store, edition)
    }

    #[tokio::test]
    async fn cover_round_trips_through_the_disk_cache() {
        let (tmp, store, edition) = store_with_edition().await;
        let root = tmp.path().join("app-cache").to_string();

        assert!(
            store
                .cover_cache_path(edition, root.clone())
                .await
                .unwrap()
                .is_none(),
            "nothing cached yet"
        );

        let written = store
            .store_cover(edition, root.clone(), "jpg".to_string(), vec![1, 2, 3, 4])
            .await
            .unwrap();
        assert!(written.to_string().ends_with("cover.jpg"));
        assert_eq!(
            std::fs::read(written.to_string()).unwrap(),
            vec![1, 2, 3, 4]
        );

        // A second lookup answers from disk without any bytes handed in.
        let again = store
            .cover_cache_path(edition, root.clone())
            .await
            .unwrap()
            .expect("cached");
        assert_eq!(again.to_string(), written.to_string());

        store.forget_cover(edition, root.clone()).await.unwrap();
        assert!(
            store
                .cover_cache_path(edition, root)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn stored_cover_survives_a_lost_permanent_copy() {
        let (tmp, store, edition) = store_with_edition().await;
        let root = tmp.path().join("app-cache").to_string();

        let written = store
            .store_cover(edition, root.clone(), ".PNG".to_string(), vec![9, 9])
            .await
            .unwrap();
        assert!(written.to_string().ends_with("cover.png"), "ext normalised");

        // Simulate a cleared permanent tree: the entry remains, the copy does not.
        std::fs::remove_file(written.to_string()).unwrap();
        let rebuilt = store
            .cover_cache_path(edition, root)
            .await
            .unwrap()
            .expect("rebuilt from the entry");
        assert_eq!(std::fs::read(rebuilt.to_string()).unwrap(), vec![9, 9]);
    }

    #[tokio::test]
    async fn cover_requests_validate_their_inputs() {
        let (tmp, store, edition) = store_with_edition().await;
        let root = tmp.path().join("app-cache").to_string();

        let err = store
            .store_cover(edition, root.clone(), "jpg".to_string(), Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::InvalidInput(_)), "{err:?}");

        let err = store
            .store_cover(edition, root.clone(), "..".to_string(), vec![1])
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::InvalidInput(_)), "{err:?}");

        let err = store
            .cover_cache_path(edition, "  ".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, LivtetError::InvalidInput(_)), "{err:?}");
    }
}
