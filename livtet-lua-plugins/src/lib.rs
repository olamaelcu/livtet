//! Bundle of pre-vendored Lua plugins the host can load directly from
//! memory without touching the filesystem. The bundled plugins are
//! produced by the workspace build script and embedded as
//! `include_bytes!` blobs; the host consults the index from
//! `embedded_index()` to resolve a `<id>` to its `BundledPlugin`
//! entry, then `read_entry` to extract a script by relative path.
//!
//! Stub: the embedded index is empty. Once real plugin sources are
//! vendored in, replace this with the generated index.

/// One plugin entry in the embedded index. `manifest_bytes` is the
/// raw `livtet.toml` source the host parses to discover the
/// plugin's capabilities and metadata; `files` is the in-memory
/// file tree the host extracts from when the plugin is loaded.
pub struct BundledPlugin {
    pub id: &'static str,
    pub version: &'static str,
    pub manifest_bytes: &'static [u8],
}

/// Singleton index of every bundled plugin. `iter()` walks all known
/// plugins in id order; `get(id)` resolves a single id.
pub struct EmbeddedIndex;

impl EmbeddedIndex {
    /// Every bundled plugin, in id order. Empty in the stub.
    pub fn plugins(&self) -> &'static [BundledPlugin] {
        &[]
    }

    /// Iterate every bundled plugin in id order. Equivalent to
    /// `self.plugins().iter()`; provided as a method so callers
    /// can use `embedded_index().iter()` ergonomically.
    pub fn iter(&self) -> core::slice::Iter<'static, BundledPlugin> {
        self.plugins().iter()
    }

    /// Look up a bundled plugin by id. Returns `None` when the
    /// plugin is not bundled.
    pub fn get(&self, _id: &str) -> Option<&'static BundledPlugin> {
        None
    }
}

/// Return the singleton index. Cheap (no allocation, no IO).
pub fn embedded_index() -> EmbeddedIndex {
    EmbeddedIndex
}

/// Extract the source text of a single file inside a bundled plugin.
/// Stub: returns `None` for any path. The real implementation
/// resolves `entry_path` against the in-memory file tree of the
/// `BundledPlugin`.
pub fn read_entry(_p: &BundledPlugin, _entry_path: &str) -> Option<String> {
    None
}

/// Canonical "well-known" signer public key text used by the
/// permission grant system. Stub: empty string. The real
/// implementation embeds the production pubkey at compile time.
pub const BUNDLED_SIGNER_PUB_TEXT: &str = "";

/// Path under the data directory that the bundled plugin loader
/// uses when it needs to materialise a plugin from memory to disk
/// (rare — most plugins run entirely from memory).
pub fn synthetic_entry_path(plugin_id: &str) -> camino::Utf8PathBuf {
    camino::Utf8PathBuf::from(format!("bundled/{plugin_id}"))
}
