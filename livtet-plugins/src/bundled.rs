use crate::manifest::PluginManifest;

pub struct BundledPluginEntry {
    pub id: String,
    pub manifest: PluginManifest,
    pub source_bytes: &'static [u8],
}

pub fn bundled_signer_pub() -> &'static str {
    option_env!("LIVTET_BUNDLED_SIGNER_PUB_TEXT").unwrap_or("")
}

pub fn bundled_index() -> Vec<BundledPluginEntry> {
    Vec::new()
}
