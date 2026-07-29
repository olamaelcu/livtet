//! Bundle of Lua stdlib modules the embedded Lua host can fall back
//! to when the system Lua paths do not expose them (e.g. on mobile,
//! where there is no on-disk `.so` to dlopen and no LuaRocks tree).
//!
//! Stub: the `resolve` lookup always returns `None`, so the host's
//! `host.require` falls through to the system Lua stdlib. Once real
//! bundled sources are vendored in, replace the stub with the
//! bundled `include_bytes!` table.

/// In-process index of bundled Lua stdlib sources keyed by module
/// name (e.g. `dkjson`, `socket`). The lookup is intentionally
/// infallible here — the real implementation will be backed by a
/// `Lazy<HashMap<&'static str, &'static [u8]>>` populated from
/// `include_bytes!` at compile time.
pub struct EmbeddedIndex;

impl EmbeddedIndex {
    /// Resolve a module name to its bundled source bytes. Returns
    /// `None` when the module is not bundled (the host then falls
    /// through to its system stdlib path).
    pub fn resolve(&self, _target: &str) -> Option<&'static [u8]> {
        None
    }
}

/// Return the singleton index. Cheap (no allocation, no IO) so the
/// host can call this on every `require` miss without a perf hit.
pub fn embedded_index() -> EmbeddedIndex {
    EmbeddedIndex
}
