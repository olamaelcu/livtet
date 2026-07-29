use std::{assert_matches, collections::HashSet};

use livtet_cover::*;
use livtet_types::DbId;

#[test]
fn cache_key_hash_eq_roundtrip() {
    let key = CacheKey {
        key: "cover-l".to_string(),
        provider: "openlibrary".to_string(),
        identifier_type: "isbn".to_string(),
        identifier_value: "978-0-123456-78-9".to_string(),
        size: "L".to_string(),
    };
    let mut set = HashSet::new();
    assert!(set.insert(key.clone()));
    assert!(!set.insert(key.clone()));
    assert!(set.contains(&key));
}

#[test]
fn cache_key_serde_roundtrip() {
    let key = CacheKey {
        key: "cover-m".to_string(),
        provider: "google".to_string(),
        identifier_type: "isbn".to_string(),
        identifier_value: "978-0-987654-32-1".to_string(),
        size: "M".to_string(),
    };
    let json = serde_json::to_string(&key).unwrap();
    let back: CacheKey = serde_json::from_str(&json).unwrap();
    assert_eq!(key, back);
}

#[test]
fn fetch_error_network_display() {
    let err = FetchError::Network("connection refused".to_string());
    assert!(err.to_string().contains("connection refused"));
    assert!(err.to_string().contains("Network error"));
}

#[test]
fn fetch_error_not_found_display() {
    let err = FetchError::NotFound;
    assert!(err.to_string().contains("no cover"));
}

#[test]
fn fetched_cover_constructable() {
    let cover = FetchedCover {
        bytes: vec![0x89, 0x50, 0x4e, 0x47],
        content_type: Some("image/png".to_string()),
    };
    assert_eq!(cover.bytes.len(), 4);
    assert_eq!(cover.content_type.as_deref(), Some("image/png"));
}

#[test]
fn fetched_cover_no_content_type() {
    let cover = FetchedCover {
        bytes: vec![],
        content_type: None,
    };
    assert!(cover.content_type.is_none());
}

#[test]
fn cached_cover_constructable() {
    let cover = CachedCover {
        key: "cover-s".to_string(),
        provider: "openlibrary".to_string(),
        size: "S".to_string(),
        ext: "jpg".to_string(),
        bytes: vec![0xff, 0xd8, 0xff],
        inventory_id: DbId::new(),
        display_path: Some("/covers/abc.jpg".to_string()),
    };
    assert_eq!(cover.ext, "jpg");
    assert!(cover.display_path.is_some());
}

#[test]
fn cached_cover_serde_roundtrip() {
    let cover = CachedCover {
        key: "cover-xl".to_string(),
        provider: "google".to_string(),
        size: "XL".to_string(),
        ext: "png".to_string(),
        bytes: vec![1, 2, 3],
        inventory_id: DbId::new(),
        display_path: None,
    };
    let json = serde_json::to_string(&cover).unwrap();
    let back: CachedCover = serde_json::from_str(&json).unwrap();
    assert_eq!(cover.key, back.key);
    assert_eq!(cover.bytes, back.bytes);
    assert_eq!(cover.inventory_id, back.inventory_id);
}

#[test]
fn cover_error_io_from_conversion() {
    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    let err: CoverError = io_err.into();
    assert_matches!(err, CoverError::Io(_));
    assert!(err.to_string().contains("denied"));
}

#[test]
fn cover_error_db_from_conversion() {
    let db_err = livtet_database::orm::DbErr::Custom("table missing".to_string());
    let err: CoverError = db_err.into();
    assert_matches!(err, CoverError::Db(_));
    assert!(err.to_string().contains("table missing"));
}

#[test]
fn cover_error_fetch_from_conversion() {
    let fetch_err = FetchError::Network("timeout".to_string());
    let err: CoverError = fetch_err.into();
    assert_matches!(err, CoverError::Fetch(_));
    assert!(err.to_string().contains("timeout"));
}

#[test]
fn cover_error_cache_display() {
    let err = CoverError::Cache("overflow".to_string());
    assert!(err.to_string().contains("overflow"));
    assert!(err.to_string().contains("Cache error"));
}

#[test]
fn cover_error_not_found_display() {
    let err = CoverError::NotFound("isbn:123".to_string());
    assert!(err.to_string().contains("isbn:123"));
    assert!(err.to_string().contains("not found"));
}
