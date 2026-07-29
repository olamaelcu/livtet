use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use specta::Type;

/// Canonical display names for the canonical variants. Custom names
/// (e.g. `"KOReader"`) produce their own row in `device_types` and
/// resolve to the canonical category, but the row's `name` column
/// holds the per-device name so the UI can show what was paired.
pub const CANONICAL_DESKTOP: &str = "Desktop";
pub const CANONICAL_MOBILE: &str = "Mobile";
pub const CANONICAL_WEB: &str = "Web";
pub const CANONICAL_EREADER: &str = "E-Reader";

/// Tag strings used by `DeviceType::from_tag` to map user-facing
/// settings strings onto a device-type category. Tags are deliberately
/// broader than canonical names: any e-reader string (`"koreader"`,
/// `"kobo"`, `"kindle"`) resolves to the canonical `Ereader`
/// variant.
pub mod tag {
    pub const DESKTOP: &str = "desktop";
    pub const MOBILE: &str = "mobile";
    pub const WEB: &str = "web";
    pub const EREADER: &str = "ereader";
}

/// A device type paired with a user-facing display name.
///
/// The discriminant identifies the broad category (`Desktop`, `Mobile`,
/// `Web`, `Ereader`). The `String` payload carries the canonical name
/// for seeded rows or a per-device label for custom rows. ULIDs are
/// deterministic for canonical names — `Desktop("Desktop").ulid()` is
/// stable — and xor with a hash of the name for custom variants so
/// they produce distinct rows.
#[derive(Debug, Clone, PartialEq, Eq, Type, Serialize, Deserialize)]
#[serde(tag = "class", content = "label", rename_all = "lowercase")]
pub enum DeviceType {
    /// `"Desktop"` or a custom name like `"Linux Workstation"`.
    Desktop(String),
    /// `"Mobile"` or a custom name like `"iPhone 15 Pro"`.
    Mobile(String),
    /// `"Web"` or a custom name like `"Firefox on macOS"`.
    Web(String),
    /// `"E-Reader"` or a custom name like `"KOReader on Kobo Libra 2"`.
    Ereader(String),
}

impl DeviceType {
    /// Numeric discriminant used for ULID generation and
    /// `display_name_for` fast-paths. Stable across runs because the
    /// value is part of the seed constants the project uses
    /// everywhere.
    pub fn discriminant(&self) -> u16 {
        match self {
            Self::Desktop(_) => 400,
            Self::Mobile(_) => 401,
            Self::Web(_) => 402,
            Self::Ereader(_) => 403,
        }
    }

    /// The human-readable label stored in the `String` payload.
    pub fn name(&self) -> &str {
        match self {
            Self::Desktop(s) | Self::Mobile(s) | Self::Web(s) | Self::Ereader(s) => s.as_str(),
        }
    }

    /// Short machine-readable tag used for serializing and for
    /// matching `from_tag` inputs.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Desktop(_) => tag::DESKTOP,
            Self::Mobile(_) => tag::MOBILE,
            Self::Web(_) => tag::WEB,
            Self::Ereader(_) => tag::EREADER,
        }
    }

    /// True when the `String` payload matches a canonical seeded
    /// name. Custom names return `false`; the canonical name of
    /// each category returns `true`. Used to decide whether a
    /// `DeviceType`'s ULID should use the discriminant directly
    /// (fast-path) or be mixed with the name hash (fall-through to
    /// `device_types` lookup).
    pub fn is_canonical(&self) -> bool {
        match self {
            Self::Desktop(s) => s == CANONICAL_DESKTOP,
            Self::Mobile(s) => s == CANONICAL_MOBILE,
            Self::Web(s) => s == CANONICAL_WEB,
            Self::Ereader(s) => s == CANONICAL_EREADER,
        }
    }

    /// Deterministic ULID for this device type.
    ///
    /// Canonical variants produce a random component equal to their
    /// discriminant so `display_name_for`'s ULID-based fast-path
    /// matches them without a DB query. Custom variants xor the
    /// discriminant with a hash of the label, producing a distinct
    /// ULID per unique label — "E-Reader" with discriminant 403
    /// lands on random-component 403, but "KOReader on Kobo Libra 2"
    /// with the same discriminant lands on a different value and
    /// falls through to the slow path.
    pub fn ulid(&self) -> ulid::Ulid {
        const TIME_MS: u64 = crate::DEVICE_TYPE_TIME_MS;
        let discriminant = self.discriminant() as u128;
        let random = if self.is_canonical() {
            discriminant
        } else {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.name().hash(&mut hasher);
            let label_hash = hasher.finish() as u128;
            discriminant ^ label_hash
        };
        ulid::Ulid::from_parts(TIME_MS, random)
    }

    /// All canonical variants, in deterministic order. Used by the
    /// device-type migration to `INSERT OR IGNORE` the seeded
    /// categories.
    pub fn all() -> Vec<Self> {
        vec![
            Self::Desktop(CANONICAL_DESKTOP.to_string()),
            Self::Mobile(CANONICAL_MOBILE.to_string()),
            Self::Web(CANONICAL_WEB.to_string()),
            Self::Ereader(CANONICAL_EREADER.to_string()),
        ]
    }

    /// Parse a user-provided string into the canonical `DeviceType`
    /// for that category. Unknown strings fall back to the canonical
    /// `Mobile` variant with the canonical display name — the
    /// existing fallback behaviour for unknown `device_type` inputs
    /// on `/sync/pair` POSTs and `pair_device` Tauri commands.
    ///
    /// Recognised tokens (case-insensitive):
    /// - `desktop` family: `desktop`, `windows`, `macos`, `linux`
    /// - `mobile` family: `mobile`, `android`, `ios`, `iphone`, `ipad`
    /// - `web` family: `web`, `browser`
    /// - `ereader` family: `ereader`, `e-reader`, `koreader`, `kobo`,
    ///   `kindle`
    pub fn from_tag(tag_str: &str) -> Self {
        match tag_str.to_lowercase().as_str() {
            "desktop" | "windows" | "macos" | "linux" => {
                Self::Desktop(CANONICAL_DESKTOP.to_string())
            }
            "mobile" | "android" | "ios" | "iphone" | "ipad" => {
                Self::Mobile(CANONICAL_MOBILE.to_string())
            }
            "web" | "browser" => Self::Web(CANONICAL_WEB.to_string()),
            "ereader" | "e-reader" | "koreader" | "kobo" | "kindle" => {
                Self::Ereader(CANONICAL_EREADER.to_string())
            }
            _ => Self::Mobile(CANONICAL_MOBILE.to_string()),
        }
    }
}

impl From<DeviceType> for crate::DbId {
    fn from(value: DeviceType) -> Self {
        crate::DbId(value.ulid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ulids_use_discriminant_as_random() {
        assert_eq!(
            DeviceType::Desktop(CANONICAL_DESKTOP.to_string())
                .ulid()
                .random(),
            400
        );
        assert_eq!(
            DeviceType::Mobile(CANONICAL_MOBILE.to_string())
                .ulid()
                .random(),
            401
        );
        assert_eq!(
            DeviceType::Web(CANONICAL_WEB.to_string()).ulid().random(),
            402
        );
        assert_eq!(
            DeviceType::Ereader(CANONICAL_EREADER.to_string())
                .ulid()
                .random(),
            403
        );
    }

    #[test]
    fn custom_ulids_xor_label_hash() {
        let custom_a = DeviceType::Ereader("KOReader".into()).ulid();
        let custom_b = DeviceType::Ereader("Kobo".into()).ulid();
        let canonical = DeviceType::Ereader(CANONICAL_EREADER.into()).ulid();

        assert_ne!(custom_a.random(), 403);
        assert_ne!(custom_b.random(), 403);
        assert_ne!(custom_a, custom_b);
        assert_eq!(canonical.random(), 403);
    }

    #[test]
    fn from_tag_maps_known_families() {
        assert_eq!(DeviceType::from_tag("desktop").name(), CANONICAL_DESKTOP);
        assert_eq!(DeviceType::from_tag("KOREADER").name(), CANONICAL_EREADER);
        assert_eq!(DeviceType::from_tag("unknown").name(), CANONICAL_MOBILE);
    }

    #[test]
    fn serde_uses_class_label_shape() {
        let value = DeviceType::Ereader("KOReader on Kobo".into());
        let json = serde_json::to_string(&value).expect("serialize");
        assert_eq!(json, r#"{"class":"ereader","label":"KOReader on Kobo"}"#);
    }

    #[test]
    fn discriminant_is_stable() {
        assert_eq!(DeviceType::Desktop("".into()).discriminant(), 400);
        assert_eq!(DeviceType::Mobile("".into()).discriminant(), 401);
        assert_eq!(DeviceType::Web("".into()).discriminant(), 402);
        assert_eq!(DeviceType::Ereader("".into()).discriminant(), 403);
    }

    #[test]
    fn all_returns_four_canonical_variants() {
        let all = DeviceType::all();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].discriminant(), 400);
        assert_eq!(all[1].discriminant(), 401);
        assert_eq!(all[2].discriminant(), 402);
        assert_eq!(all[3].discriminant(), 403);
    }

    #[test]
    fn round_trip_ulid_from_devicetype() {
        let original = DeviceType::Ereader("custom".into());
        let id: crate::DbId = original.clone().into();
        let _ = id;
    }

    #[test]
    fn is_canonical_recognises_seeded_names() {
        assert!(DeviceType::Desktop(CANONICAL_DESKTOP.into()).is_canonical());
        assert!(DeviceType::Mobile(CANONICAL_MOBILE.into()).is_canonical());
        assert!(DeviceType::Web(CANONICAL_WEB.into()).is_canonical());
        assert!(DeviceType::Ereader(CANONICAL_EREADER.into()).is_canonical());
        assert!(!DeviceType::Ereader("KOReader".into()).is_canonical());
    }
}
