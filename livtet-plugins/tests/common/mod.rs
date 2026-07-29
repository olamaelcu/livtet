use std::sync::Arc;

use fs_err as fs;
use livtet_plugins::repository::hmac::HmacKey;

/// A deterministic HMAC key for tests (all zeros).
pub fn test_hmac_key() -> Arc<HmacKey> {
    Arc::new(HmacKey::from_bytes([0u8; 32]))
}

/// Resolve a path relative to the crate's `fixtures/` directory.
pub fn fixture_path(relative: &str) -> Utf8PathBuf {
    let crate_root = env!("CARGO_MANIFEST_DIR");
    Utf8PathBuf::from(crate_root)
        .join("fixtures")
        .join(relative)
}

/// Copy a named fixture (livtet.toml + init.lua) into `target`.
pub fn copy_fixture(target: &Utf8PathBuf, name: &str) {
    let dir = target.join(name);
    fs::create_dir_all(&dir).expect("create dir");
    fs::copy(
        fixture_path(&format!("{name}/livtet.toml")),
        dir.join("livtet.toml"),
    )
    .expect("copy livtet.toml");
    fs::copy(
        fixture_path(&format!("{name}/init.lua")),
        dir.join("init.lua"),
    )
    .expect("copy init.lua");
}

/// Copy the `test-provider` fixture (livtet.toml + init.lua) into `target`.
pub fn copy_test_provider(target: &Utf8PathBuf) {
    copy_fixture(target, "test-provider")
}

#[allow(clippy::disallowed_types)]
pub fn as_utf8(p: &std::path::Path) -> &camino::Utf8Path {
    camino::Utf8Path::from_path(p).expect("temp path must be valid UTF-8")
}

pub fn verifying_key_from_keygen_report(
    report: &livtet_plugins::types::KeygenReport,
) -> ed25519_dalek::VerifyingKey {
    let text = fs_err::read_to_string(&report.pubkey_path).expect("read pubkey");
    livtet_plugins::keys::signing::parse_pubkey_text(&text).expect("parse_pubkey_text")
}

use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use livtet_plugins::host_manager::PluginHostManager;
use miette::{IntoDiagnostic, miette};
use camino_tempfile::Utf8TempDir as TempDir;
use tokio::time::timeout;

pub struct TestContext {
    _temp: TempDir,
    pub temp_path: Utf8PathBuf,
    pub manager: PluginHostManager,
}

impl TestContext {
    pub async fn new(fixture: &str) -> miette::Result<Self> {
        let temp = camino_tempfile::Utf8TempDir::new().into_diagnostic()?;
        let temp_path = temp.path().to_path_buf();
        copy_fixture(&temp_path, fixture);
        let binary = Utf8Path::new(env!("CARGO_BIN_EXE_livtet-plugins-host-lua"));
        let manager = timeout(
            Duration::from_secs(10),
            PluginHostManager::spawn(binary, temp_path.clone(), test_hmac_key()),
        )
        .await
        .map_err(|_| miette!("spawn timed out"))?
        .map_err(|e| miette!("spawn failed: {e}"))?;
        Ok(Self {
            _temp: temp,
            temp_path,
            manager,
        })
    }

    pub async fn load_plugin(&mut self, id: &str) -> miette::Result<()> {
        timeout(
            Duration::from_secs(5),
            self.manager.load_plugin(id, "1.0.0"),
        )
        .await
        .map_err(|_| miette!("load timed out"))?
        .map_err(|e| miette!("load failed: {e}"))
    }

    pub async fn call(
        &mut self,
        plugin: &str,
        cap: &str,
        args: Vec<serde_json::Value>,
    ) -> miette::Result<serde_json::Value> {
        timeout(Duration::from_secs(5), self.manager.call(plugin, cap, args))
            .await
            .map_err(|_| miette!("call timed out"))?
            .map_err(|e| miette!("call failed: {e}"))
    }

    pub async fn shutdown(mut self) {
        let _ = timeout(Duration::from_secs(5), self.manager.shutdown()).await;
    }
}
