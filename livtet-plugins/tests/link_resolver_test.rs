use std::time::Duration;

use common::spawn_test_provider_manager;
use livtet_plugins::link_resolver::{LinkCategory, ResolveLinksOptions};
use tokio::time::timeout;

mod common;

#[tokio::test]
async fn test_resolve_links_end_to_end() {
    let (_temp, mut manager) = spawn_test_provider_manager().await;

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
    let (_temp, mut manager) = spawn_test_provider_manager().await;

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
    let (_temp, mut manager) = spawn_test_provider_manager().await;

    let result = manager
        .resolve_links("does-not-exist", "urn:isbn:123", Default::default())
        .await;
    assert!(
        matches!(result, Err(livtet_plugins::PluginError::PluginNotFound(_))),
        "expected PluginNotFound, got {result:?}"
    );

    let _ = timeout(Duration::from_secs(5), manager.shutdown()).await;
}