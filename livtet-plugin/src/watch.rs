//! Typed structures for the `watch` capability.
//!
//! Polling-based change notifications: the host asks the plugin
//! "what changed since X?" and the plugin returns a batch of
//! change items. Tracks P3 §"Watch capability" of
//! `docs/plans/2026-06-07-plugin-roadmap.md`.
//!
//! Plugins return these as Lua tables; the host dispatches
//! `provider.watch(since)` through the same IPC round-trip as
//! any other capability and the dispatcher decodes the JSON
//! payload into the typed shape below.
//!
//! The `changes` field is intentionally typed as
//! `Vec<serde_json::Value>` rather than a fixed struct: the
//! change items are plugin-defined (e.g. an "availability"
//! change for a library-hold plugin, a "new release" change
//! for a series-tracking plugin). The host and frontend can
//! discriminate on `changes[i]["kind"]` to pick a renderer.

use serde::{Deserialize, Deserializer, Serialize};

/// Deserialize a `Vec<T>` from a JSON array, an absent
/// field, or `null` — folding both the "absent" and
/// "explicit null" cases into an empty vec. Plain
/// `#[serde(default)]` only covers the absent-field case.
fn deserialize_null_to_default_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt: Option<Vec<T>> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// Full response shape for `provider.watch(since)`.
///
/// `since` is the opaque cursor the host received from a prior
/// call (or `None` for the first poll). The plugin responds
/// with the changes it knows about whose `observed_at` is
/// newer than `since` (or simply "all current state" when
/// `since` is `None` and the host has no prior view).
///
/// `has_more` signals whether the host should keep polling.
/// `next_cursor` is the opaque token the host passes as
/// `since` on the next call; the plugin is free to use any
/// scheme (timestamp, server-side bookmark, etc.) so the
/// host must treat it as opaque.
///
/// Not derived with `specta::Type` — the Tauri command layer
/// stringifies the result and re-parses on the frontend, the
/// same way `plugin_search` / `plugin_lookup` do. Keeping
/// `changes: Vec<serde_json::Value>` opaque avoids forcing
/// `specta` to model every possible plugin-defined change
/// shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatchResult {
    /// Defaults to an empty vec when the plugin omits the
    /// field, returns `null`, or returns `nil` from Lua.
    /// This is the natural Lua idiom for "no changes" — an
    /// empty Lua table serializes to a JSON object, which
    /// serde can't auto-rewrap into a `Vec`. Letting the
    /// plugin send `nil` (or omit the field) and folding
    /// that into an empty vec is the simplest path that
    /// matches the Lua side.
    #[serde(default, deserialize_with = "deserialize_null_to_default_vec")]
    pub changes: Vec<serde_json::Value>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn watch_result_round_trips_minimal() {
        // A plugin can return just `changes` — `has_more`
        // defaults to `false` and `next_cursor` to `None`.
        let result = json!({
            "changes": [
                { "kind": "availability", "identifier": "urn:isbn:111" },
            ],
        });
        let parsed: WatchResult = serde_json::from_value(result).unwrap();
        assert_eq!(parsed.changes.len(), 1);
        assert!(!parsed.has_more);
        assert!(parsed.next_cursor.is_none());
        assert_eq!(parsed.changes[0]["kind"], "availability");
    }

    #[test]
    fn watch_result_round_trips_with_cursor() {
        let result = json!({
            "changes": [],
            "has_more": true,
            "next_cursor": "cursor:abc-123",
        });
        let parsed: WatchResult = serde_json::from_value(result).unwrap();
        assert!(parsed.changes.is_empty());
        assert!(parsed.has_more);
        assert_eq!(parsed.next_cursor.as_deref(), Some("cursor:abc-123"));
    }

    #[test]
    fn watch_result_defaults_changes_to_empty_when_omitted() {
        // Plugins that "have nothing to report" can return
        // either an explicit empty array, an omitted
        // `changes` field, or `nil` from Lua — all three
        // should fold into an empty vec. This matches the
        // Lua-side convention where `{}` is ambiguous
        // (mlua serializes it as a JSON object, which serde
        // can't unwrap to a Vec) but `nil` is unambiguous.
        let result = json!({
            "has_more": false,
            "next_cursor": null,
        });
        let parsed: WatchResult = serde_json::from_value(result).unwrap();
        assert!(parsed.changes.is_empty());
        assert!(!parsed.has_more);
        assert!(parsed.next_cursor.is_none());
    }

    #[test]
    fn watch_result_defaults_changes_to_empty_on_null() {
        // Lua `nil` round-trips to JSON `null` — the
        // dispatcher should treat that the same as an
        // omitted field.
        let result = json!({
            "changes": null,
            "has_more": false,
        });
        let parsed: WatchResult = serde_json::from_value(result).unwrap();
        assert!(parsed.changes.is_empty());
    }
}
