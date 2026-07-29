use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{Display, EnumIter, EnumString, IntoEnumIterator, IntoStaticStr};

/// The sealed set of capabilities a plugin can declare in its manifest.
///
/// The manifest's `capabilities` table is deserialized as
/// `BTreeMap<Capability, bool>`; the deserializer maps the TOML
/// string keys (e.g. `search`, `link_resolver`) onto this enum via
/// the explicit `Deserialize` impl below. The enum is sealed to
/// `crates/livtet-plugins` — downstream crates cannot add new
/// variants without changing the plugin host.
///
/// `Probe` is the one exception: it exists so the in-tree
/// `host-probe` test fixture (which declares `probe = true` to mark
/// itself as a host-function exerciser rather than a real plugin)
/// can still be deserialized. It is not part of the public
/// capability surface and is documented as test-only.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Display,
    EnumString,
    EnumIter,
    IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
#[non_exhaustive]
pub enum Capability {
    /// `search` — string-based full-text or filtered search.
    Search,
    /// `lookup` — identifier-based work/edition lookup.
    Lookup,
    /// `enrich` — fill in metadata for a known work.
    Enrich,
    /// `cover` — fetch a cover image for a work/edition.
    Cover,
    /// `watch` — long-lived "new releases" / "updates" stream
    /// (P3 — not yet dispatched by the host, declared by some
    /// bundled plugins for forward compatibility).
    Watch,
    /// `link_resolver` — resolve a URN to a clickable web link.
    LinkResolver,
    /// `reading_progress` — import reading progress from an
    /// external service (KOReader Kosync, Kobo Sync, etc.).
    ReadingProgress,
    /// `annotations` — import highlights/notes from an external
    /// service (Kindle Clippings, etc.).
    Annotations,
    /// `reading_list` — sync reading lists / collections.
    ReadingList,
    /// `series` — detect + order book series.
    Series,
    /// `catalog_resolver` — extract bibliographic metadata from a
    /// library catalog URL (Primo, Koha, Sierra, etc.).
    CatalogResolver,
    /// `import_detect` — examine a source (file path, URL, database)
    /// and return a confidence score for whether this plugin can
    /// import it. Plugins that can't handle the source return nil.
    ImportDetect,
    /// `import_list_items` — list items from a source that a user
    /// can preview before importing. Returns an array of lightweight
    /// preview records.
    ImportListItems,
    /// `import_items` — finalize an import from a source, returning
    /// canonical `ImportRecord` values that the host writes to the
    /// database.
    ImportItems,
    /// `pull` — fetch raw book entries from a configured source
    /// (RSS feed, scraper, etc.). Returns a normalized list of
    /// `RawPullEntry` values that the host stores in the inbox and
    /// passes through the enrichment pipeline.
    Pull,
    /// `probe` — host-probe test fixture only. Not a real
    /// capability; see the type-level docs.
    Probe,
}

impl Capability {
    /// Stable string form used in manifests, IPC payloads, and the
    /// wire protocol.
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Parse a manifest/IPC string back into a `Capability`.
    /// Returns `None` for unknown keys so future manifests with
    /// unrecognised capabilities degrade gracefully (the host
    /// logs a warning and continues) rather than aborting parse.
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        s.parse().ok()
    }

    /// I18n key the web UI uses to localize the human-friendly name.
    /// Convention: `plugin.capability.<snake_case>`. Single literal
    /// lives here; the web side does not duplicate it.
    pub fn i18n_key(self) -> String {
        format!("plugin.capability.{}", self.as_str())
    }
}

impl Serialize for Capability {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Capability {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        // Use `String` rather than `<&str>` here. The `toml` crate
        // processes escape sequences during parse and therefore
        // always hands string values to the visitor as owned
        // `String`s (via `visit_string`), which `<&str>::deserialize`
        // rejects with "expected a borrowed string". `serde_json`
        // is happy with either form because it can borrow directly
        // from the input `&str`. `String` works for both.
        let s = String::deserialize(de)?;
        Self::from_str_lossy(&s).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown capability {s:?}; expected one of {:?}",
                Capability::iter().map(|c| c.as_str()).collect::<Vec<_>>()
            ))
        })
    }
}

/// Per-capability metadata. Only the `enabled` flag is tracked
/// today; the struct is the place to hang future per-capability
/// data (capability-specific config schemas, version pins, etc.).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityMeta {
    pub enabled: bool,
}

impl CapabilityMeta {
    pub const fn enabled() -> Self {
        Self { enabled: true }
    }

    pub const fn disabled() -> Self {
        Self { enabled: false }
    }
}

/// Registry of capabilities exposed by the plugins currently loaded
/// into a host. Replaces the previous `HashMap<String,
/// Box<dyn PluginCapability>>` name-tag store with a typed
/// `BTreeMap<Capability, CapabilityMeta>`. The name-tag trait is
/// gone — capability markers now expose a `const CAPABILITY:
/// Capability` and the registry stores plain data, not trait
/// objects.
#[derive(Debug, Default, Clone)]
pub struct CapabilityRegistry {
    capabilities: BTreeMap<Capability, CapabilityMeta>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self {
            capabilities: BTreeMap::new(),
        }
    }

    /// Register a capability. Replaces any prior metadata for the
    /// same key.
    pub fn register(&mut self, cap: Capability, meta: CapabilityMeta) {
        self.capabilities.insert(cap, meta);
    }

    /// Look up a capability by its enum variant.
    pub fn get(&self, cap: Capability) -> Option<&CapabilityMeta> {
        self.capabilities.get(&cap)
    }

    /// Stable, sorted list of registered capability names (in
    /// `BTreeMap` order — declaration order, see
    /// [`Capability::iter`]).
    pub fn names(&self) -> Vec<&'static str> {
        self.capabilities.keys().map(|c| c.as_str()).collect()
    }

    /// All registered (capability, meta) pairs, in stable order.
    pub fn iter(&self) -> impl Iterator<Item = (Capability, &CapabilityMeta)> {
        self.capabilities.iter().map(|(c, m)| (*c, m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_has_no_names() {
        let registry = CapabilityRegistry::new();
        assert!(registry.names().is_empty());
        assert!(registry.get(Capability::Search).is_none());
    }

    #[test]
    fn register_makes_capability_retrievable() {
        let mut registry = CapabilityRegistry::new();
        registry.register(Capability::LinkResolver, CapabilityMeta::enabled());
        let meta = registry.get(Capability::LinkResolver);
        assert_eq!(meta, Some(&CapabilityMeta::enabled()));
    }

    #[test]
    fn names_returns_all_registered_in_btreemap_order() {
        // BTreeMap orders by key, not insertion order. Capability's
        // `Ord` follows declaration order in the enum, and `Cover`
        // is declared before `LinkResolver` (see `Capability::iter`).
        let mut registry = CapabilityRegistry::new();
        registry.register(Capability::LinkResolver, CapabilityMeta::enabled());
        registry.register(Capability::Cover, CapabilityMeta::enabled());
        let names = registry.names();
        assert_eq!(names, vec!["cover", "link_resolver"]);
    }

    #[test]
    fn as_str_round_trips_via_from_str_lossy() {
        for cap in Capability::iter() {
            assert_eq!(Capability::from_str_lossy(cap.as_str()), Some(cap));
        }
    }

    #[test]
    fn from_str_lossy_rejects_unknown_key() {
        assert_eq!(Capability::from_str_lossy("nope"), None);
        assert_eq!(Capability::from_str_lossy(""), None);
        assert_eq!(Capability::from_str_lossy("Search"), None);
    }

    #[test]
    fn serde_string_round_trip() {
        let original = Capability::ReadingProgress;
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"reading_progress\"");
        let back: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn serde_rejects_unknown_string() {
        let result: Result<Capability, _> = serde_json::from_str("\"telepathy\"");
        assert!(result.is_err());
    }

    #[test]
    fn serde_round_trips_btreemap_of_capabilities() {
        let original: BTreeMap<Capability, bool> = [
            (Capability::Search, true),
            (Capability::Lookup, false),
            (Capability::Enrich, true),
        ]
        .into_iter()
        .collect();

        let json = serde_json::to_string(&original).unwrap();
        let back: BTreeMap<Capability, bool> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn toml_round_trip_matches_manifest_string_form() {
        let original: BTreeMap<Capability, bool> = [
            (Capability::Search, true),
            (Capability::Lookup, true),
            (Capability::Enrich, true),
            (Capability::Cover, true),
        ]
        .into_iter()
        .collect();

        let toml_str = toml::to_string(&original).unwrap();
        let back: BTreeMap<Capability, bool> = toml::from_str(&toml_str).unwrap();
        assert_eq!(back, original);
    }

    // =====================================================================
    // Task 2.4 / Step 8: `Capability::from_str` (the `FromStr` impl)
    //
    // `from_str_lossy` returns `Option<Capability>` and is
    // used everywhere a Capability is decoded from a
    // string. The strum-derived `FromStr` impl backs that so
    // `str::parse::<Capability>()` works. The tests below pin the
    // contract:
    //   - every known string in `Capability::iter()` parses
    //     to its `as_str()` counterpart;
    //   - unknown strings (including the empty string and
    //     case variants of known names) return `Err(_)`.
    // =====================================================================

    #[test]
    fn from_str_accepts_every_known_capability_string() {
        // Every variant in the public surface must
        // round-trip through FromStr. We iterate
        // Capability::iter() so a new variant added in the
        // future is automatically covered.
        for cap in Capability::iter() {
            let s = cap.as_str();
            let parsed: Capability = s
                .parse()
                .unwrap_or_else(|_| panic!("known capability string {s:?} failed to parse"));
            assert_eq!(parsed, cap, "from_str({s:?}) must produce the same variant");
        }
    }

    #[test]
    fn from_str_rejects_unknown_string() {
        // "telepathy" isn't a known capability. FromStr
        // returns Err(strum::ParseError) — we just check the
        // parse fails.
        let r: Result<Capability, strum::ParseError> = "telepathy".parse();
        assert!(r.is_err(), "unknown capability must be rejected");
    }

    #[test]
    fn from_str_rejects_empty_string() {
        let r: Result<Capability, strum::ParseError> = "".parse();
        assert!(r.is_err(), "empty string must be rejected");
    }

    #[test]
    fn from_str_is_case_sensitive() {
        // "Search" (capital S) must NOT match Capability::Search
        // because the manifest wire format uses snake_case
        // ("search"). A case-insensitive parse would let
        // "Search" through, which would then fail to
        // match in the BTreeMap key check or the
        // dispatcher's `capability: String` lookup. The
        // current contract is: case-sensitive snake_case.
        for cap in Capability::iter() {
            let lower = cap.as_str();
            let upper = lower.to_uppercase();
            // Skip strings that don't change with
            // to_uppercase (e.g. all-letters, all-digits).
            if upper == lower {
                continue;
            }
            let r: Result<Capability, strum::ParseError> = upper.parse();
            assert!(
                r.is_err(),
                "from_str({upper:?}) must reject the upper-case form (capability is {lower:?}); got {r:?}"
            );
        }
    }

    #[test]
    fn from_str_rejects_string_with_trailing_whitespace() {
        // The contract is exact-match. A trailing
        // newline or space would be a common
        // copy-paste error; the parser must reject it
        // rather than silently truncate.
        for cap in Capability::iter().take(3) {
            let with_newline = format!("{}\n", cap.as_str());
            let r: Result<Capability, strum::ParseError> = with_newline.parse();
            assert!(
                r.is_err(),
                "from_str({with_newline:?}) must reject trailing newline; got {r:?}"
            );
            let with_space = format!("{} ", cap.as_str());
            let r: Result<Capability, strum::ParseError> = with_space.parse();
            assert!(
                r.is_err(),
                "from_str({with_space:?}) must reject trailing space; got {r:?}"
            );
        }
    }

    #[test]
    fn i18n_key_format_is_stable() {
        assert_eq!(Capability::Search.i18n_key(), "plugin.capability.search");
        assert_eq!(
            Capability::ReadingProgress.i18n_key(),
            "plugin.capability.reading_progress"
        );
        assert_eq!(
            Capability::LinkResolver.i18n_key(),
            "plugin.capability.link_resolver"
        );
        assert_eq!(
            Capability::ImportDetect.i18n_key(),
            "plugin.capability.import_detect"
        );
        assert_eq!(
            Capability::ImportListItems.i18n_key(),
            "plugin.capability.import_list_items"
        );
        assert_eq!(
            Capability::ImportItems.i18n_key(),
            "plugin.capability.import_items"
        );
    }
}
