//! Typed representation of a format's metadata schema.
//!
//! Every row in the `formats` table carries a `metadata_schema` column that
//! stores a JSON Schema document describing which fields are meaningful when
//! recording reading progress for editions of that format.
//!
//! The three first-party families and their discriminating `required` field:
//!
//! | Variant        | Required field        | Seeded formats                       |
//! |----------------|-----------------------|--------------------------------------|
//! | `PhysicalBook` | `"page_count"`        | Hardcover, Trade Paperback, MMPB     |
//! | `Ebook`        | `"virtual_page_count"`| eBook, PDF, EPUB, MOBI               |
//! | `Audiobook`    | `"duration_seconds"`  | Audiobook                            |
//!
//! Plugin-supplied formats are round-tripped losslessly through the
//! [`Custom`](FormatMetadataSchema::Custom) variant.
//!
//! # Serialised forms
//!
//! Each known variant serialises to its full JSON Schema document.  The schema
//! for a physical book, for example, is:
//!
//! ```json
//! {
//!   "type": "object",
//!   "properties": {
//!     "page_count": { "type": "integer", "minimum": 1 },
//!     "chapters": {
//!       "type": "array",
//!       "items": {
//!         "type": "object",
//!         "properties": {
//!           "name":       { "type": "string" },
//!           "page_start": { "type": "integer" },
//!           "page_end":   { "type": "integer" }
//!         },
//!         "required": ["name", "page_start", "page_end"]
//!       }
//!     }
//!   },
//!   "required": ["page_count"]
//! }
//! ```
//!
//! Deserialization recognises the variant by inspecting the top-level
//! `required` array: `"page_count"` → `PhysicalBook`,
//! `"virtual_page_count"` → `Ebook`, `"duration_seconds"` → `Audiobook`.
//! Anything else is preserved as [`Custom`](FormatMetadataSchema::Custom).
//!
//! FIXME: Integrate JSON schema validation logic against the variant's schema and a provided value
//! FIXME: Make use of schemars to represent schema documents.

use jsonschema::{Draft, Validator};
use serde::{
    Deserialize, Serialize,
    de::{self, Deserializer},
    ser::Serializer,
};
use specta::Type;

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

/// Describes how reading progress is tracked for a particular format.
///
/// Stored in (and deserialised from) the `metadata_schema` column of the
/// `formats` table as a JSON Schema document.
#[derive(Debug, Clone, PartialEq, Type)]
pub enum FormatMetadataSchema {
    /// Physical print edition (hardcover, trade paperback, mass market
    /// paperback).  Progress is a 1-based page number within `page_count`.
    PhysicalBook,

    /// Digital file edition (EPUB, PDF, MOBI, generic eBook).  Progress is a
    /// virtual page number within `virtual_page_count`, or a CFI string.
    Ebook,

    /// Spoken-word edition.  Progress is a seek position in seconds within
    /// `duration_seconds`.
    Audiobook,

    /// Plugin-defined or unrecognised format.  The raw JSON Schema value is
    /// preserved without modification so no information is lost on round-trip.
    #[specta(type = specta_typescript::Unknown<serde_json::Value>)]
    Custom(serde_json::Value),
}

impl FormatMetadataSchema {
    /// Validate a metadata value against this schema.
    ///
    /// Returns `Ok(())` if `value` conforms to the JSON Schema document
    /// associated with this variant, or a list of validation errors.
    pub fn validate(&self, value: &serde_json::Value) -> Result<(), FormatMetadataValidationError> {
        let schema = match self {
            Self::PhysicalBook => physical_book_schema(),
            Self::Ebook => ebook_schema(),
            Self::Audiobook => audiobook_schema(),
            Self::Custom(v) => v.clone(),
        };

        let compiled = Validator::options()
            .with_draft(Draft::Draft7)
            .build(&schema)
            .map_err(|e| FormatMetadataValidationError {
                errors: vec![e.to_string()],
            })?;

        let result: Vec<String> = compiled.iter_errors(value).map(|e| e.to_string()).collect();

        if result.is_empty() {
            Ok(())
        } else {
            Err(FormatMetadataValidationError { errors: result })
        }
    }
}

/// Errors returned when a metadata value fails validation against a format's
/// metadata schema.
#[derive(Debug, Clone)]
pub struct FormatMetadataValidationError {
    pub errors: Vec<String>,
}

impl std::fmt::Display for FormatMetadataValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "metadata validation failed: {}", self.errors.join("; "))
    }
}

impl std::error::Error for FormatMetadataValidationError {}

// ---------------------------------------------------------------------------
// Canonical schema definitions (via schemars)
// ---------------------------------------------------------------------------

use schemars::{
    JsonSchema,
    generate::{SchemaGenerator, SchemaSettings},
};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};

#[derive(JsonSchema, SerdeSerialize, SerdeDeserialize)]
struct PhysicalBookChapter {
    name: String,
    #[schemars(rename = "page_start")]
    page_start: i32,
    #[schemars(rename = "page_end")]
    page_end: i32,
}

#[derive(JsonSchema, SerdeSerialize, SerdeDeserialize)]
struct PhysicalBookDef {
    #[schemars(range(min = 1))]
    page_count: i32,
    #[serde(default)]
    #[schemars(default)]
    chapters: Vec<PhysicalBookChapter>,
}

#[derive(JsonSchema, SerdeSerialize, SerdeDeserialize)]
struct EbookChapter {
    name: String,
    #[schemars(rename = "virtual_page_start")]
    virtual_page_start: i32,
    #[schemars(rename = "virtual_page_end")]
    virtual_page_end: i32,
}

#[derive(JsonSchema, SerdeSerialize, SerdeDeserialize)]
struct EbookDef {
    #[schemars(range(min = 1))]
    virtual_page_count: i32,
    #[serde(default)]
    #[schemars(default)]
    chapters: Vec<EbookChapter>,
}

#[derive(JsonSchema, SerdeSerialize, SerdeDeserialize)]
struct AudiobookChapter {
    name: String,
    #[schemars(rename = "audio_start")]
    audio_start: i32,
    #[schemars(rename = "audio_end")]
    audio_end: i32,
}

#[derive(JsonSchema, SerdeSerialize, SerdeDeserialize)]
struct AudiobookDef {
    #[schemars(range(min = 1))]
    duration_seconds: i32,
    #[serde(default)]
    #[schemars(default)]
    chapters: Vec<AudiobookChapter>,
}

fn draft07_generator() -> SchemaGenerator {
    SchemaGenerator::new(SchemaSettings::draft07().with(|s| s.inline_subschemas = true))
}

fn strip_schema_meta(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
    }
    v
}

fn physical_book_schema() -> serde_json::Value {
    let schema = draft07_generator().into_root_schema_for::<PhysicalBookDef>();
    strip_schema_meta(serde_json::to_value(&schema).expect("physical book schema"))
}

fn ebook_schema() -> serde_json::Value {
    let schema = draft07_generator().into_root_schema_for::<EbookDef>();
    strip_schema_meta(serde_json::to_value(&schema).expect("ebook schema"))
}

fn audiobook_schema() -> serde_json::Value {
    let schema = draft07_generator().into_root_schema_for::<AudiobookDef>();
    strip_schema_meta(serde_json::to_value(&schema).expect("audiobook schema"))
}

// ---------------------------------------------------------------------------
// Serialise — emit the full JSON Schema document
// ---------------------------------------------------------------------------

impl Serialize for FormatMetadataSchema {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::PhysicalBook => physical_book_schema().serialize(s),
            Self::Ebook => ebook_schema().serialize(s),
            Self::Audiobook => audiobook_schema().serialize(s),
            // Pass the raw value straight through — no wrapping added.
            Self::Custom(v) => v.serialize(s),
        }
    }
}

// ---------------------------------------------------------------------------
// Deserialise — detect variant from the top-level `required` array
// ---------------------------------------------------------------------------

impl<'de> Deserialize<'de> for FormatMetadataSchema {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d).map_err(de::Error::custom)?;

        // The discriminant is whichever field name appears in the top-level
        // `required` array.  Each known schema has exactly one required field
        // at the root that is unique across all three variants.
        let required_fields: Vec<&str> = v
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|f| f.as_str()).collect())
            .unwrap_or_default();

        Ok(if required_fields.contains(&"page_count") {
            Self::PhysicalBook
        } else if required_fields.contains(&"virtual_page_count") {
            Self::Ebook
        } else if required_fields.contains(&"duration_seconds") {
            Self::Audiobook
        } else {
            Self::Custom(v)
        })
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

impl From<FormatMetadataSchema> for serde_json::Value {
    fn from(schema: FormatMetadataSchema) -> Self {
        serde_json::to_value(&schema).expect("FormatMetadataSchema is always serialisable")
    }
}

impl From<&FormatMetadataSchema> for serde_json::Value {
    fn from(schema: &FormatMetadataSchema) -> Self {
        serde_json::to_value(schema).expect("FormatMetadataSchema is always serialisable")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn roundtrip(schema: &FormatMetadataSchema) -> FormatMetadataSchema {
        let v: serde_json::Value = schema.into();
        serde_json::from_value(v).expect("roundtrip deserialise")
    }

    // --- Serialisation shape ------------------------------------------------

    #[test]
    fn physical_book_serialises_to_json_schema() {
        let v: serde_json::Value = (&FormatMetadataSchema::PhysicalBook).into();
        assert_eq!(v["type"], "object");
        assert_eq!(v["required"], json!(["page_count"]));
        assert!(v["properties"]["page_count"]["minimum"] == 1);
        assert!(v["properties"]["chapters"].is_object());
    }

    #[test]
    fn ebook_serialises_to_json_schema() {
        let v: serde_json::Value = (&FormatMetadataSchema::Ebook).into();
        assert_eq!(v["type"], "object");
        assert_eq!(v["required"], json!(["virtual_page_count"]));
        assert!(v["properties"]["virtual_page_count"]["minimum"] == 1);
        assert!(v["properties"]["chapters"].is_object());
    }

    #[test]
    fn audiobook_serialises_to_json_schema() {
        let v: serde_json::Value = (&FormatMetadataSchema::Audiobook).into();
        assert_eq!(v["type"], "object");
        assert_eq!(v["required"], json!(["duration_seconds"]));
        assert!(v["properties"]["duration_seconds"]["minimum"] == 1);
        assert!(v["properties"]["chapters"].is_object());
    }

    // --- Round-trips --------------------------------------------------------

    #[test]
    fn physical_book_roundtrips() {
        assert_eq!(
            roundtrip(&FormatMetadataSchema::PhysicalBook),
            FormatMetadataSchema::PhysicalBook
        );
    }

    #[test]
    fn ebook_roundtrips() {
        assert_eq!(
            roundtrip(&FormatMetadataSchema::Ebook),
            FormatMetadataSchema::Ebook
        );
    }

    #[test]
    fn audiobook_roundtrips() {
        assert_eq!(
            roundtrip(&FormatMetadataSchema::Audiobook),
            FormatMetadataSchema::Audiobook
        );
    }

    // --- Custom / fallback --------------------------------------------------

    #[test]
    fn plugin_schema_without_known_required_field_deserialises_as_custom() {
        let raw = json!({
            "type": "object",
            "properties": { "scroll_pct": { "type": "number" } },
            "required": ["scroll_pct"]
        });
        let schema: FormatMetadataSchema = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(schema, FormatMetadataSchema::Custom(raw));
    }

    #[test]
    fn json_without_required_array_deserialises_as_custom() {
        let raw = json!({ "type": "object", "properties": {} });
        let schema: FormatMetadataSchema = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(schema, FormatMetadataSchema::Custom(raw));
    }

    #[test]
    fn custom_serialises_as_raw_value_without_wrapper() {
        let inner = json!({
            "type": "object",
            "properties": { "scroll_pct": { "type": "number" } },
            "required": ["scroll_pct"]
        });
        let schema = FormatMetadataSchema::Custom(inner.clone());
        let v: serde_json::Value = schema.into();
        assert_eq!(v, inner);
    }

    #[test]
    fn custom_roundtrips_losslessly() {
        let inner = json!({
            "type": "object",
            "properties": { "scroll_pct": { "type": "number" } },
            "required": ["scroll_pct"]
        });
        let schema = FormatMetadataSchema::Custom(inner.clone());
        assert_eq!(roundtrip(&schema), FormatMetadataSchema::Custom(inner));
    }

    // --- Chapter structure --------------------------------------------------

    #[test]
    fn physical_book_chapter_items_have_page_start_and_page_end() {
        let v: serde_json::Value = (&FormatMetadataSchema::PhysicalBook).into();
        let item_required = &v["properties"]["chapters"]["items"]["required"];
        assert_eq!(*item_required, json!(["name", "page_start", "page_end"]));
    }

    #[test]
    fn ebook_chapter_items_have_virtual_page_start_and_end() {
        let v: serde_json::Value = (&FormatMetadataSchema::Ebook).into();
        let item_required = &v["properties"]["chapters"]["items"]["required"];
        assert_eq!(
            *item_required,
            json!(["name", "virtual_page_start", "virtual_page_end"])
        );
    }

    #[test]
    fn audiobook_chapter_items_have_audio_start_and_end() {
        let v: serde_json::Value = (&FormatMetadataSchema::Audiobook).into();
        let item_required = &v["properties"]["chapters"]["items"]["required"];
        assert_eq!(*item_required, json!(["name", "audio_start", "audio_end"]));
    }

    // --- Validation ---------------------------------------------------------

    #[test]
    fn physical_book_validates_correct_value() {
        let result = FormatMetadataSchema::PhysicalBook.validate(&json!({
            "page_count": 320,
            "chapters": [{"name": "Ch 1", "page_start": 1, "page_end": 30}]
        }));
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn physical_book_validates_minimal_value() {
        let result = FormatMetadataSchema::PhysicalBook.validate(&json!({
            "page_count": 100
        }));
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn physical_book_rejects_missing_required_field() {
        let result = FormatMetadataSchema::PhysicalBook.validate(&json!({}));
        assert!(result.is_err(), "expected Err for missing page_count");
        let err = result.unwrap_err();
        assert!(
            err.errors.iter().any(|e| e.contains("page_count")),
            "errors should mention page_count: {:?}",
            err.errors
        );
    }

    #[test]
    fn physical_book_rejects_zero_page_count() {
        let result = FormatMetadataSchema::PhysicalBook.validate(&json!({
            "page_count": 0
        }));
        assert!(result.is_err(), "expected Err for zero page_count");
    }

    #[test]
    fn physical_book_rejects_string_page_count() {
        let result = FormatMetadataSchema::PhysicalBook.validate(&json!({
            "page_count": "lots"
        }));
        assert!(result.is_err(), "expected Err for string page_count");
    }

    #[test]
    fn ebook_validates_correct_value() {
        let result = FormatMetadataSchema::Ebook.validate(&json!({
            "virtual_page_count": 250
        }));
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn ebook_rejects_missing_required_field() {
        let result = FormatMetadataSchema::Ebook.validate(&json!({
            "chapters": []
        }));
        assert!(
            result.is_err(),
            "expected Err for missing virtual_page_count"
        );
    }

    #[test]
    fn audiobook_validates_correct_value() {
        let result = FormatMetadataSchema::Audiobook.validate(&json!({
            "duration_seconds": 7200,
            "chapters": [{"name": "Intro", "audio_start": 0, "audio_end": 300}]
        }));
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn audiobook_rejects_missing_duration() {
        let result = FormatMetadataSchema::Audiobook.validate(&json!({}));
        assert!(result.is_err(), "expected Err for missing duration_seconds");
    }

    #[test]
    fn custom_validates_against_raw_schema() {
        let raw = json!({
            "type": "object",
            "properties": { "scroll_pct": { "type": "number", "minimum": 0.0, "maximum": 1.0 } },
            "required": ["scroll_pct"]
        });
        let schema = FormatMetadataSchema::Custom(raw);
        assert!(schema.validate(&json!({"scroll_pct": 0.5})).is_ok());
        assert!(schema.validate(&json!({"scroll_pct": 1.5})).is_err());
        assert!(schema.validate(&json!({})).is_err());
    }

    #[test]
    fn null_value_is_rejected() {
        let result = FormatMetadataSchema::PhysicalBook.validate(&serde_json::Value::Null);
        assert!(result.is_err(), "expected Err for null");
    }

    #[test]
    fn wrong_type_as_value_is_rejected() {
        let result = FormatMetadataSchema::PhysicalBook.validate(&json!("hello"));
        assert!(result.is_err(), "expected Err for string value");
    }

    // --- Validation error message shape -------------------------------------

    #[test]
    fn validation_error_is_displayable() {
        let result = FormatMetadataSchema::PhysicalBook.validate(&json!({}));
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("metadata validation failed:"));
        assert!(!err.errors.is_empty());
    }

    // --- schemars output stability ------------------------------------------

    #[test]
    fn schemars_output_does_not_contain_metadata_fields() {
        let v: serde_json::Value = (&FormatMetadataSchema::PhysicalBook).into();
        assert!(v.get("$schema").is_none(), "should not contain $schema");
        assert!(v.get("title").is_none(), "should not contain title");
    }
}
