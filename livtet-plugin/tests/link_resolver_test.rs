use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use common::{copy_test_provider, test_hmac_key};
use livtet_plugin::{
    host_manager::PluginHostManager,
    link_resolver::{LinkCategory, ResolveLinksOptions},
};
use camino_tempfile::Utf8TempDir as TempDir;
use tokio::time::timeout;

mod common;

#[tokio::test]
async fn test_resolve_links_end_to_end() {
    let temp = TempDir::new().expect("tempdir");
    let temp_path = temp.path().to_path_buf();
    copy_test_provider(&temp_path);

    let binary = Utf8Path::new(env!("CARGO_BIN_EXE_livtet-plugin-host-lua"));
    let mut manager = timeout(
        Duration::from_secs(10),
        PluginHostManager::spawn(binary, temp_path.clone(), test_hmac_key()),
    )
    .await
    .expect("spawn timed out")
    .expect("spawn failed");

    timeout(
        Duration::from_secs(5),
        manager.load_plugin("test-provider", "1.0.0"),
    )
    .await
    .expect("load timed out")
    .expect("load failed");

    let result = timeout(
        Duration::from_secs(5),
        manager.resolve_links(
            "test-provider",
            "urn:isbn:9780441013593",
            Default::default(),
        ),
    )
    .await
    .expect("resolve timed out")
    .expect("resolve failed");

    assert_eq!(result.links.len(), 1);
    assert_eq!(result.links[0].label, "Test Link");
    assert_eq!(result.links[0].category, LinkCategory::Reference);
    assert_eq!(
        result.links[0].url,
        "https://example.com/book?urn=urn:isbn:9780441013593"
    );
    assert_eq!(result.links[0].sort_hint, 100);
    assert!(!result.links[0].affiliate);

    let _ = timeout(Duration::from_secs(5), manager.shutdown()).await;
}

#[tokio::test]
async fn test_resolve_links_different_urn() {
    let temp = TempDir::new().expect("tempdir");
    let temp_path = temp.path().to_path_buf();
    copy_test_provider(&temp_path);

    let binary = Utf8Path::new(env!("CARGO_BIN_EXE_livtet-plugin-host-lua"));
    let mut manager = timeout(
        Duration::from_secs(10),
        PluginHostManager::spawn(binary, temp_path.clone(), test_hmac_key()),
    )
    .await
    .expect("spawn timed out")
    .expect("spawn failed");

    timeout(
        Duration::from_secs(5),
        manager.load_plugin("test-provider", "1.0.0"),
    )
    .await
    .expect("load timed out")
    .expect("load failed");

    let result = timeout(
        Duration::from_secs(5),
        manager.resolve_links(
            "test-provider",
            "urn:isbn:9780261103573",
            ResolveLinksOptions::default(),
        ),
    )
    .await
    .expect("resolve timed out")
    .expect("resolve failed");

    assert_eq!(result.links.len(), 1);
    assert_eq!(
        result.links[0].url,
        "https://example.com/book?urn=urn:isbn:9780261103573"
    );

    let _ = timeout(Duration::from_secs(5), manager.shutdown()).await;
}

#[tokio::test]
async fn test_resolve_links_unknown_plugin() {
    let temp = TempDir::new().expect("tempdir");
    let temp_path = temp.path().to_path_buf();
    copy_test_provider(&temp_path);

    let binary = Utf8Path::new(env!("CARGO_BIN_EXE_livtet-plugin-host-lua"));
    let mut manager = timeout(
        Duration::from_secs(10),
        PluginHostManager::spawn(binary, temp_path.clone(), test_hmac_key()),
    )
    .await
    .expect("spawn timed out")
    .expect("spawn failed");

    let result = manager
        .resolve_links("does-not-exist", "urn:isbn:123", Default::default())
        .await;
    assert!(
        matches!(result, Err(livtet_plugin::PluginError::PluginNotFound(_))),
        "expected PluginNotFound, got {result:?}"
    );

    let _ = timeout(Duration::from_secs(5), manager.shutdown()).await;
}
