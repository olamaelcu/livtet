//! Pre-resolves `format_id` / `language_id` vectors to text labels
//! with a short-lived TTL cache. Used by Tauri commands and FFI
//! handlers to bridge the [`WorkFilters`](livtet_types::WorkFilters)
//! DbId axes to the tantivy schema's text-label fields.
//!
//! The cache is a single-process `parking_lot::Mutex<HashMap>` with
//! a configurable TTL (default 60 s). Key space: sorted byte-vectors
//! of the raw id bytes. This is acceptable because the number of
//! distinct format+language combinations that a user session hits is
//! bounded by the FilterPanel surface (~200 stable combinations).

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use livtet_data::entities::{formats, languages};
use livtet_types::DbId;
use parking_lot::Mutex;
use livtet_data::orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

type CacheValue = (Vec<String>, Vec<String>, Instant);

/// A TTL-cached resolver for format and language IDs → labels.
///
/// # Cache invalidation
///
/// The default TTL is 60 seconds. If a user renames a format (e.g.
/// "EPUB" → "Electronic publication") mid-session, cached labels
/// stay stale for up to the TTL. Acceptable for a filter dropdown.
type CacheMap = HashMap<(Vec<u8>, Vec<u8>), CacheValue>;

#[derive(Default)]
pub struct LabelResolver {
    cache: Mutex<CacheMap>,
    ttl: Duration,
}

impl LabelResolver {
    /// Create a new resolver with the default TTL (60 s).
    pub fn new() -> Self {
        Self::default().with_ttl(Duration::from_secs(60))
    }

    /// Set a custom TTL for cache entries.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Resolve format and language ID vectors to their text labels.
    ///
    /// Returns `(format_labels, language_labels)` where each label
    /// is the `name` column from the respective table. Empty input
    /// IDs produce empty output vectors (no DB query).
    pub async fn resolve(
        &self,
        db: &DatabaseConnection,
        format_ids: &[DbId],
        language_ids: &[DbId],
    ) -> Result<(Vec<String>, Vec<String>), livtet_data::orm::DbErr> {
        let key = (hash_ids(format_ids), hash_ids(language_ids));

        // Check cache.
        {
            let cache = self.cache.lock();
            if let Some((f, l, ts)) = cache.get(&key)
                && ts.elapsed() < self.ttl
            {
                return Ok((f.clone(), l.clone()));
            }
        }

        // Resolve formats (empty guard).
        let fmts = if format_ids.is_empty() {
            Vec::new()
        } else {
            formats::Entity::find()
                .filter(formats::Column::Id.is_in(format_ids.to_vec()))
                .all(db)
                .await?
                .into_iter()
                .map(|f| f.name)
                .collect()
        };

        // Resolve languages (empty guard).
        let langs = if language_ids.is_empty() {
            Vec::new()
        } else {
            languages::Entity::find()
                .filter(languages::Column::Id.is_in(language_ids.to_vec()))
                .all(db)
                .await?
                .into_iter()
                .map(|l| l.name)
                .collect()
        };

        // Insert into cache.
        {
            let mut cache = self.cache.lock();
            cache.insert(key, (fmts.clone(), langs.clone(), Instant::now()));
        }

        Ok((fmts, langs))
    }
}

/// Produce a sorted byte-vector from a slice of DbIds for use as a
/// cache key. Sorting ensures `[id1, id2]` and `[id2, id1]` map to
/// the same entry.
fn hash_ids(ids: &[DbId]) -> Vec<u8> {
    let mut v: Vec<u8> = ids.iter().flat_map(|i| i.to_bytes().to_vec()).collect();
    v.sort_unstable();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_ids_is_deterministic() {
        let id1 = DbId::new();
        let id2 = DbId::new();
        let a = hash_ids(&[id1, id2]);
        let b = hash_ids(&[id2, id1]);
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn empty_ids_produce_empty_key() {
        let key = hash_ids(&[]);
        assert!(key.is_empty());
    }

    #[test]
    fn label_resolver_ttl_expiry() {
        let resolver = LabelResolver::new().with_ttl(Duration::from_millis(10));

        // The cache is accessed via `resolve` which requires a DB
        // connection. In a unit test we can't call resolve without a
        // DB, but we can verify that the TTL field is set correctly.
        assert_eq!(resolver.ttl, Duration::from_millis(10));
    }

    #[test]
    fn label_resolver_default_ttl() {
        let resolver = LabelResolver::new();
        assert_eq!(resolver.ttl, Duration::from_secs(60));
    }

    #[test]
    fn hash_ids_single_element() {
        let id = DbId::new();
        let key = hash_ids(&[id]);
        assert_eq!(key.len(), 16, "single ULID serialises to 16 bytes");
    }

    #[test]
    fn hash_ids_deduplicates_order() {
        // Sorting the flattened byte view collapses `[a, b]` and
        // `[b, a]` to the same key — but does NOT dedupe repeated IDs.
        // Document that behaviour here so callers don't assume dedup.
        let id_a = DbId::new();
        let id_b = DbId::new();
        let ab = hash_ids(&[id_a, id_b]);
        let ba = hash_ids(&[id_b, id_a]);
        assert_eq!(ab, ba, "sort makes order irrelevant");

        let with_dup = hash_ids(&[id_a, id_b, id_a]);
        assert_eq!(
            with_dup.len(),
            3 * 16,
            "hash_ids does not dedupe; repeated id_a appears twice"
        );
    }
}
