//! Note convergence by id (ADR 0010, Stage B).
//!
//! Notes live as a JSON array on `reading_progress.annotations`. Sync applies
//! the whole row, so two writers can otherwise overwrite each other's notes.
//! Every writer and the sync engine use these helpers:
//!
//! - [`merge_notes`] unions two arrays by id; a tombstone is final, otherwise
//!   the greatest `(updatedAt, updatedBy)` wins.
//! - [`replace_notes`] takes the visible list a reader just saved and turns
//!   removed notes into tombstones, so a stale peer cannot resurrect them.
//! - [`visible_notes`] hides tombstones from readers.
//!
//! Note metadata is camelCase inside the JSON: `updatedAt`, `updatedBy`,
//! `deletedAt`. Arrays are sorted by note id so equal states serialize equal.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde_json::Value;

use crate::orm::{ConnectionTrait, DatabaseBackend, Statement};

/// Timestamp given to notes written before Stage B introduced metadata.
pub const EPOCH: &str = "1970-01-01T00:00:00Z";

/// Device id that owns local writes, or `"local"` when the setting is absent.
///
/// The note tie-breaker needs a stable per-device value; the sync daemon stores
/// it in `client_settings` under `device_id`.
pub async fn local_device_id(db: &impl ConnectionTrait) -> String {
    let statement = Statement::from_string(
        DatabaseBackend::Sqlite,
        "SELECT value FROM client_settings WHERE key = 'device_id'",
    );
    db.query_one_raw(statement)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<String>("", "value").ok())
        .unwrap_or_else(|| "local".to_string())
}

/// Decode the column, tolerating absent or malformed data.
pub fn parse_notes(raw: Option<&str>) -> Vec<Value> {
    raw.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

/// Notes a reader should see: tombstones stay in the array but are hidden.
pub fn visible_notes(notes: &[Value]) -> Vec<Value> {
    notes
        .iter()
        .filter(|note| !is_tombstone(note))
        .cloned()
        .collect()
}

/// Serialize a note list for storage.
pub fn notes_json(notes: &[Value]) -> String {
    Value::Array(notes.to_vec()).to_string()
}

/// Union two note arrays by id.
pub fn merge_notes(left: &[Value], right: &[Value]) -> Vec<Value> {
    let mut merged: BTreeMap<String, Value> = BTreeMap::new();
    for note in left.iter().chain(right.iter()) {
        let key = note_key(note);
        match merged.get(&key) {
            Some(existing) => {
                merged.insert(key, pick_winner(existing.clone(), note.clone()));
            }
            None => {
                merged.insert(key, note.clone());
            }
        }
    }
    merged.into_values().collect()
}

/// Apply a reader's visible note list to the stored one.
///
/// - New or edited notes get `updatedAt`/`updatedBy` stamped.
/// - Omitted notes become tombstones when `delete_omitted` is true; callers
///   that receive a partial list (KOReader) pass false.
/// - A stored tombstone is final: an incoming edit cannot resurrect it.
pub fn replace_notes(
    stored: &[Value],
    incoming: &[Value],
    now: &str,
    device: &str,
    delete_omitted: bool,
) -> Vec<Value> {
    let mut result: BTreeMap<String, Value> = stored
        .iter()
        .map(|note| (note_key(note), note.clone()))
        .collect();
    let mut incoming_ids = BTreeMap::new();

    for note in incoming {
        let key = note_key(note);
        incoming_ids.insert(key.clone(), ());
        let Some(existing) = result.get(&key).cloned() else {
            result.insert(key, stamp(note, now, device));
            continue;
        };
        if is_tombstone(&existing) {
            // Deletion wins over a stale edit that arrives with the list.
            continue;
        }
        if content(&existing) == content(note) {
            // A position save re-sends unchanged notes: keep their history.
            continue;
        }
        result.insert(key, stamp(note, now, device));
    }

    if delete_omitted {
        for (key, note) in result.clone() {
            if incoming_ids.contains_key(&key) || is_tombstone(&note) {
                continue;
            }
            result.insert(key, tombstone(note, now, device));
        }
    }

    result.into_values().collect()
}

fn pick_winner(existing: Value, incoming: Value) -> Value {
    let existing_tombstone = is_tombstone(&existing);
    let incoming_tombstone = is_tombstone(&incoming);
    match (existing_tombstone, incoming_tombstone) {
        // A tombstone is final, even when the other side is newer.
        (true, false) => existing,
        (false, true) => incoming,
        (true, true) | (false, false) => {
            if metadata_order(&incoming, &existing) == Ordering::Greater {
                incoming
            } else {
                existing
            }
        }
    }
}

/// Compare `(updatedAt, updatedBy, canonical content)` so the winner is stable.
fn metadata_order(left: &Value, right: &Value) -> Ordering {
    updated_at(left)
        .cmp(updated_at(right))
        .then_with(|| updated_by(left).cmp(updated_by(right)))
        .then_with(|| content(left).cmp(&content(right)))
}

fn stamp(note: &Value, now: &str, device: &str) -> Value {
    let mut note = note.clone();
    if let Some(object) = note.as_object_mut() {
        object.insert("updatedAt".to_string(), Value::String(now.to_string()));
        object.insert("updatedBy".to_string(), Value::String(device.to_string()));
        object.remove("deletedAt");
    }
    note
}

fn tombstone(mut note: Value, now: &str, device: &str) -> Value {
    if let Some(object) = note.as_object_mut() {
        // Keep the full original entry; the tombstone is additional metadata.
        object.insert("updatedAt".to_string(), Value::String(now.to_string()));
        object.insert("updatedBy".to_string(), Value::String(device.to_string()));
        object.insert("deletedAt".to_string(), Value::String(now.to_string()));
    }
    note
}

fn is_tombstone(note: &Value) -> bool {
    note.get("deletedAt").is_some_and(|value| !value.is_null())
}

fn updated_at(note: &Value) -> &str {
    note.get("updatedAt")
        .and_then(Value::as_str)
        .unwrap_or(EPOCH)
}

fn updated_by(note: &Value) -> &str {
    note.get("updatedBy").and_then(Value::as_str).unwrap_or("")
}

/// The note without the sync-only metadata, for change detection.
fn content(note: &Value) -> String {
    let mut value = note.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("updatedAt");
        object.remove("updatedBy");
        object.remove("deletedAt");
    }
    value.to_string()
}

fn note_key(note: &Value) -> String {
    note.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        // An id-less note cannot merge by identity; keep it after keyed notes
        // and dedupe identical entries rather than dropping them.
        .unwrap_or_else(|| format!("\u{0}{}", content(note)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn merge_unions_by_id_and_keeps_the_latest_edit() {
        let local = vec![
            json!({"id": "a", "text": "local", "updatedAt": "2026-01-01T00:00:00Z", "updatedBy": "l"}),
            json!({"id": "b", "text": "only local", "updatedAt": EPOCH}),
        ];
        let remote = vec![
            json!({"id": "a", "text": "remote", "updatedAt": "2026-01-02T00:00:00Z", "updatedBy": "r"}),
            json!({"id": "c", "text": "only remote"}),
        ];

        let merged = merge_notes(&local, &remote);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0]["id"], "a");
        assert_eq!(merged[0]["text"], "remote");
        assert_eq!(merged[1]["id"], "b");
        assert_eq!(merged[2]["id"], "c");
    }

    #[test]
    fn a_tombstone_is_final_even_against_a_newer_edit() {
        let tombstone = json!({
            "id": "a",
            "text": "original",
            "updatedAt": "2026-01-01T00:00:00Z",
            "updatedBy": "l",
            "deletedAt": "2026-01-01T00:00:00Z"
        });
        let newer_edit = json!({
            "id": "a",
            "text": "resurrected?",
            "updatedAt": "2026-12-01T00:00:00Z",
            "updatedBy": "r"
        });

        let merged = merge_notes(&[tombstone.clone()], &[newer_edit]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["text"], "original");
        assert!(merged[0]["deletedAt"].is_string());
    }

    #[test]
    fn replace_stamps_new_and_edited_notes_and_buries_removed_ones() {
        let stored = vec![
            json!({"id": "a", "text": "same", "color": "yellow"}),
            json!({"id": "b", "text": "edit me", "color": "yellow"}),
            json!({"id": "c", "text": "remove me"}),
            json!({"id": "d", "text": "already gone", "deletedAt": "2026-01-01T00:00:00Z"}),
        ];
        let incoming = vec![
            json!({"id": "a", "text": "same", "color": "yellow"}),
            json!({"id": "b", "text": "edited", "color": "blue"}),
            json!({"id": "e", "text": "new"}),
            json!({"id": "d", "text": "try to resurrect"}),
        ];

        let result = replace_notes(&stored, &incoming, "2026-02-01T00:00:00Z", "dev", true);

        let by_id = |id: &str| result.iter().find(|note| note["id"] == id).unwrap();
        // Unchanged note keeps no metadata invented by the writer.
        assert!(by_id("a").get("updatedAt").is_none());
        assert_eq!(by_id("b")["text"], "edited");
        assert_eq!(by_id("b")["updatedAt"], "2026-02-01T00:00:00Z");
        assert_eq!(by_id("b")["updatedBy"], "dev");
        // Omitted note is a tombstone, not gone from the array.
        assert_eq!(by_id("c")["deletedAt"], "2026-02-01T00:00:00Z");
        assert!(by_id("e").get("updatedAt").is_some());
        // A stale edit cannot resurrect the tombstoned note.
        assert_eq!(by_id("d")["deletedAt"], "2026-01-01T00:00:00Z");
        assert_eq!(by_id("d")["text"], "already gone");
    }

    #[test]
    fn replace_can_keep_notes_a_partial_list_omits() {
        let stored = vec![json!({"id": "a", "text": "keep"})];
        let incoming = vec![json!({"id": "b", "text": "new"})];
        let result = replace_notes(&stored, &incoming, "2026-01-01T00:00:00Z", "ko", false);
        assert_eq!(result.len(), 2);
        assert!(
            visible_notes(&result)
                .iter()
                .all(|note| note.get("deletedAt").is_none())
        );
    }

    #[test]
    fn visible_notes_hides_tombstones_only() {
        let notes = vec![
            json!({"id": "a", "text": "shown"}),
            json!({"id": "b", "text": "hidden", "deletedAt": "2026-01-01T00:00:00Z"}),
            json!({"id": "c", "text": "null deletedAt", "deletedAt": null}),
        ];
        let visible = visible_notes(&notes);
        assert_eq!(visible.len(), 2);
        assert!(visible.iter().all(|note| note["id"] != "b"));
    }
}
