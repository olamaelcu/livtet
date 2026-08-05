use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;

use crate::{
    error::{PluginError, PluginResult},
    manifest::PluginManifest,
};

pub struct DiscoveredPlugin {
    pub id: String,
    pub path: Utf8PathBuf,
    pub manifest: PluginManifest,
    pub source: PluginSource,
    pub bundled_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluginSource {
    Folder,
    LegacyFile,
    Bundled,
}

pub fn scan_plugins(dir: &Utf8Path) -> PluginResult<Vec<DiscoveredPlugin>> {
    let mut plugins = Vec::new();

    if !dir.exists() {
        return Ok(plugins);
    }

    let entries = fs::read_dir(dir).map_err(|e| PluginError::Discovery(e.to_string()))?;

    for entry in entries {
        let entry = entry.map_err(|e| PluginError::Discovery(e.to_string()))?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| PluginError::Discovery("non-UTF-8 path".into()))?;

        let metadata = entry
            .metadata()
            .map_err(|e| PluginError::Discovery(e.to_string()))?;

        if !metadata.is_dir() {
            if metadata.is_file()
                && let Some(ext) = path.extension()
                && ext == "lua"
            {
                let stem = path.file_stem().unwrap_or("").to_string();
                let manifest = PluginManifest::from_legacy_file(&stem);
                plugins.push(DiscoveredPlugin {
                    id: manifest.plugin.id.clone(),
                    path,
                    manifest,
                    source: PluginSource::LegacyFile,
                    bundled_bytes: None,
                });
            }
            continue;
        }

        let direct_manifest = path.join("livtet.toml");
        if direct_manifest.exists()
            && let Ok(content) = fs::read_to_string(&direct_manifest)
            && let Ok(manifest) = PluginManifest::from_toml(&content)
        {
            plugins.push(DiscoveredPlugin {
                id: manifest.plugin.id.clone(),
                path,
                manifest,
                source: PluginSource::Folder,
                bundled_bytes: None,
            });
            continue;
        }

        let sub_entries = match fs::read_dir(&path) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to read dir {}: {}", path, e);
                continue;
            }
        };
        for sub_entry in sub_entries {
            let sub_entry = match sub_entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Failed to read entry: {}", e);
                    continue;
                }
            };
            let sub_path = Utf8PathBuf::from_path_buf(sub_entry.path())
                .map_err(|_| PluginError::Discovery("non-UTF-8 path".into()))?;
            if !sub_path.is_dir() {
                continue;
            }
            let manifest_path = sub_path.join("livtet.toml");
            if !manifest_path.exists() {
                continue;
            }
            let content = match fs::read_to_string(&manifest_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to read manifest {}: {}", manifest_path, e);
                    continue;
                }
            };
            match PluginManifest::from_toml(&content) {
                Ok(manifest) => {
                    plugins.push(DiscoveredPlugin {
                        id: manifest.plugin.id.clone(),
                        path: sub_path,
                        manifest,
                        source: PluginSource::Folder,
                        bundled_bytes: None,
                    });
                }
                Err(e) => {
                    tracing::warn!("Skipping invalid manifest at {}: {}", manifest_path, e);
                }
            }
        }
    }

    Ok(plugins)
}
