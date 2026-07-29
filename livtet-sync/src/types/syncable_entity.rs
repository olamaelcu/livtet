//! Trait + enum dispatcher for syncable entity types.
//!
//! Every database entity model that participates in the Livtet sync
//! protocol is registered in the [`define_syncable_entities!`] macro
//! invocation below.  The macro generates:
//!
//! * The [`SyncableEntityKind`] enum (with `#[serde(untagged)]`)
//! * `entity_type_name()` / `table_name()` / `to_payload()` /
//!   `from_payload()` methods on the enum
//! * The [`ALL_VARIANTS`] constant listing every entity type string
//!
//! [`entity_id()`] is manually implemented because each variant's PK
//! structure differs (single `DbId` vs compound keys).

use serde::{Deserialize, Serialize};

use super::{Result, SyncError};

// ─── ChangeLogEntity trait ──────────────────────────────────────────────────

/// Associates a sea-orm entity with its `change_log.entity_type` string.
///
/// Implemented automatically by [`define_syncable_entities!`] for every
/// registered entity.  Test helpers bound on this trait to look up the
/// correct `entity_type` filter at compile time, eliminating hard-coded
/// strings such as `"owned_edition"`.
///
/// # Example
/// ```rust,ignore
/// let rows = fetch_change_log_for_entity::<owned_edition::Entity>(&pool).await;
/// ```
pub trait ChangeLogEntity: livtet_data::orm::EntityTrait {
    /// The string stored in `change_log.entity_type` for mutations on this
    /// entity's table (e.g. `"owned_edition"` for the `owned_editions` table).
    const ENTITY_TYPE: &'static str;
}

// ─── Macro ──────────────────────────────────────────────────────────────

/// Define the `SyncableEntityKind` enum and its core dispatch methods.
///
/// Each entry is:
/// ```ignore
/// VariantName(ModelType) => ("entity_type", "table_name"),
/// ```
macro_rules! define_syncable_entities {
    (
        $(
            $(#[$variant_attr:meta])*
            $variant:ident($model:ty) => ($entity_type:expr, $table:expr),
        )+
    ) => {
        /// One variant per syncable entity model.
        ///
        /// `#[serde(untagged)]` means each variant serialises as its
        /// inner model directly — no extra tag.  This matches the
        /// existing `EntityDump` JSON format.
        #[cfg_attr(feature = "fake", derive(fake::Dummy))]
        #[derive(Clone, Debug, Serialize, Deserialize)]
        #[serde(untagged)]
        pub enum SyncableEntityKind {
            $(
                $(#[$variant_attr])*
                $variant($model),
            )+
        }

        impl SyncableEntityKind {
            /// The `change_log.entity_type` string for this variant.
            pub fn entity_type_name(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => $entity_type),+
                }
            }

            /// The database table name for this variant.
            pub fn table_name(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => $table),+
                }
            }

            /// Serialise to the canonical sync-payload JSON.
            pub fn to_payload(&self) -> Result<serde_json::Value> {
                match self {
                    $(Self::$variant(e) => serde_json::to_value(e)),+
                }
                .map_err(|e| SyncError::Serialization {
                    entity_type: self.entity_type_name().to_string(),
                    message: e.to_string(),
                })
            }

            /// Deserialise a sync-payload JSON value by entity type name.
            pub fn from_payload(
                entity_type: &str,
                payload: &serde_json::Value,
            ) -> Result<Self> {
                let deser_err = |e: serde_json::Error| SyncError::Deserialization {
                    entity_type: entity_type.to_string(),
                    message: e.to_string(),
                };
                match entity_type {
                    $(
                        $entity_type => serde_json::from_value(payload.clone())
                            .map(Self::$variant)
                            .map_err(deser_err),
                    )+
                    other => Err(SyncError::UnknownEntityType {
                        type_name: other.to_string(),
                    }),
                }
            }

            /// All registered entity type strings.
            pub const ALL_VARIANTS: &'static [&'static str] = &[
                $($entity_type),+
            ];
        }
    };
}

// ─── Entity list ───────────────────────────────────────────────────────
// Keep this list in sync with the `change_log` trigger definitions in
// `crate::change_log`.  Each entry includes:
//
//   * Variant name  – PascalCase, matches the inner model name
//   * Model type    – full path to the sea-ORM entity model
//   * Entity type   – the `change_log.entity_type` string
//   * Table name    – the SQLite table name

use livtet_data::entities::{
    annotations, digital_inventory, edition_authors, edition_genres, edition_groups,
    edition_publishers, edition_subjects, edition_tags, editions, editions_loans, owned_edition,
    reading_list_book, reading_lists, reading_progress, series_entries, work_authors, work_genres,
    work_publishers, work_subjects, work_tags, works,
};

define_syncable_entities! {
    Work(works::Model)             => ("work", "works"),
    Edition(editions::Model)       => ("edition", "editions"),
    SeriesEntry(series_entries::Model) => ("series_entry", "series_entries"),
    DigitalInventory(digital_inventory::Model) => ("digital_inventory", "digital_inventory"),
    OwnedEdition(owned_edition::Model)       => ("owned_edition", "owned_editions"),
    EditionLoan(editions_loans::Model)       => ("edition_loan", "editions_loans"),
    Annotation(annotations::Model)           => ("annotation", "annotations"),
    ReadingList(reading_lists::Model)        => ("reading_list", "reading_lists"),
    ReadingListBook(reading_list_book::Model) => ("reading_list_book", "reading_list_book"),
    ReadingProgress(reading_progress::Model)  => ("reading_progress", "reading_progress"),
    WorkAuthor(work_authors::Model)          => ("work_author", "work_authors"),
    WorkTag(work_tags::Model)                => ("work_tag", "work_tags"),
    WorkGenre(work_genres::Model)            => ("work_genre", "work_genres"),
    WorkSubject(work_subjects::Model)        => ("work_subject", "work_subjects"),
    WorkPublisher(work_publishers::Model)     => ("work_publisher", "work_publishers"),
    EditionAuthor(edition_authors::Model)     => ("edition_author", "edition_authors"),
    EditionTag(edition_tags::Model)           => ("edition_tag", "edition_tags"),
    EditionGenre(edition_genres::Model)       => ("edition_genre", "edition_genres"),
    EditionSubject(edition_subjects::Model)   => ("edition_subject", "edition_subjects"),
    EditionPublisher(edition_publishers::Model) => ("edition_publisher", "edition_publishers"),
    EditionGroup(edition_groups::Model)      => ("edition_group", "edition_groups"),
}

// ─── ChangeLogEntity impls ──────────────────────────────────────────────────
// One impl per entity registered in `define_syncable_entities!` above.
// These are written out explicitly (rather than generated by the macro)
// to avoid Rust coherence errors from associated-type projections.

impl ChangeLogEntity for works::Entity {
    const ENTITY_TYPE: &'static str = "work";
}
impl ChangeLogEntity for editions::Entity {
    const ENTITY_TYPE: &'static str = "edition";
}
impl ChangeLogEntity for series_entries::Entity {
    const ENTITY_TYPE: &'static str = "series_entry";
}
impl ChangeLogEntity for digital_inventory::Entity {
    const ENTITY_TYPE: &'static str = "digital_inventory";
}
impl ChangeLogEntity for owned_edition::Entity {
    const ENTITY_TYPE: &'static str = "owned_edition";
}
impl ChangeLogEntity for editions_loans::Entity {
    const ENTITY_TYPE: &'static str = "edition_loan";
}
impl ChangeLogEntity for annotations::Entity {
    const ENTITY_TYPE: &'static str = "annotation";
}
impl ChangeLogEntity for reading_lists::Entity {
    const ENTITY_TYPE: &'static str = "reading_list";
}
impl ChangeLogEntity for reading_list_book::Entity {
    const ENTITY_TYPE: &'static str = "reading_list_book";
}
impl ChangeLogEntity for reading_progress::Entity {
    const ENTITY_TYPE: &'static str = "reading_progress";
}
impl ChangeLogEntity for work_authors::Entity {
    const ENTITY_TYPE: &'static str = "work_author";
}
impl ChangeLogEntity for work_tags::Entity {
    const ENTITY_TYPE: &'static str = "work_tag";
}
impl ChangeLogEntity for work_genres::Entity {
    const ENTITY_TYPE: &'static str = "work_genre";
}
impl ChangeLogEntity for work_subjects::Entity {
    const ENTITY_TYPE: &'static str = "work_subject";
}
impl ChangeLogEntity for work_publishers::Entity {
    const ENTITY_TYPE: &'static str = "work_publisher";
}
impl ChangeLogEntity for edition_authors::Entity {
    const ENTITY_TYPE: &'static str = "edition_author";
}
impl ChangeLogEntity for edition_tags::Entity {
    const ENTITY_TYPE: &'static str = "edition_tag";
}
impl ChangeLogEntity for edition_genres::Entity {
    const ENTITY_TYPE: &'static str = "edition_genre";
}
impl ChangeLogEntity for edition_subjects::Entity {
    const ENTITY_TYPE: &'static str = "edition_subject";
}
impl ChangeLogEntity for edition_publishers::Entity {
    const ENTITY_TYPE: &'static str = "edition_publisher";
}
impl ChangeLogEntity for edition_groups::Entity {
    const ENTITY_TYPE: &'static str = "edition_group";
}

// ─── entity_id() — PK-structure-dependent ──────────────────────────────

impl SyncableEntityKind {
    /// Entity identifier for change-log tracking.
    ///
    /// For entities with a single `id: DbId` primary key this returns
    /// the ULID string.  For compound-key entities it returns a JSON
    /// object matching the `change_log` trigger format.
    pub fn entity_id(&self) -> String {
        match self {
            Self::Work(e) => e.id.to_string(),
            Self::Edition(e) => e.id.to_string(),
            Self::DigitalInventory(e) => e.id.to_string(),
            Self::OwnedEdition(e) => e.id.to_string(),
            Self::EditionLoan(e) => e.id.to_string(),
            Self::Annotation(e) => e.id.to_string(),
            Self::ReadingList(e) => e.id.to_string(),
            Self::ReadingProgress(e) => e.id.to_string(),
            Self::EditionGroup(e) => e.id.to_string(),

            // Composite PK: use the first PK field as the entity identifier.
            Self::SeriesEntry(e) => e.series_id.to_string(),

            // Composite PK: JSON object matching the trigger format.
            Self::ReadingListBook(e) => serde_json::json!({
                "reading_list_id": e.reading_list_id.to_string(),
                "edition_id": e.edition_id.to_string(),
            })
            .to_string(),

            Self::WorkAuthor(e) => serde_json::json!({
                "work_id": e.work_id.to_string(),
                "author_id": e.author_id.to_string(),
                "role": e.role,
            })
            .to_string(),
            Self::WorkTag(e) => serde_json::json!({
                "work_id": e.work_id.to_string(),
                "tag_id": e.tag_id.to_string(),
            })
            .to_string(),
            Self::WorkGenre(e) => serde_json::json!({
                "work_id": e.work_id.to_string(),
                "genre_id": e.genre_id.to_string(),
            })
            .to_string(),
            Self::WorkSubject(e) => serde_json::json!({
                "work_id": e.work_id.to_string(),
                "subject_id": e.subject_id.to_string(),
            })
            .to_string(),
            Self::WorkPublisher(e) => serde_json::json!({
                "work_id": e.work_id.to_string(),
                "publisher_id": e.publisher_id.to_string(),
            })
            .to_string(),
            Self::EditionAuthor(e) => serde_json::json!({
                "edition_id": e.edition_id.to_string(),
                "author_id": e.author_id.to_string(),
                "role": e.role,
            })
            .to_string(),
            Self::EditionTag(e) => serde_json::json!({
                "edition_id": e.edition_id.to_string(),
                "tag_id": e.tag_id.to_string(),
            })
            .to_string(),
            Self::EditionGenre(e) => serde_json::json!({
                "edition_id": e.edition_id.to_string(),
                "genre_id": e.genre_id.to_string(),
            })
            .to_string(),
            Self::EditionSubject(e) => serde_json::json!({
                "edition_id": e.edition_id.to_string(),
                "subject_id": e.subject_id.to_string(),
            })
            .to_string(),
            Self::EditionPublisher(e) => serde_json::json!({
                "edition_id": e.edition_id.to_string(),
                "publisher_id": e.publisher_id.to_string(),
            })
            .to_string(),
        }
    }
}

// ─── Full-dump fetcher ──────────────────────────────────────────────────

/// Generic fetch — all rows from an entity table, serialized to JSON.
///
/// Uses sea-ORM typed queries and serde model serialisation instead of
/// raw SQL, so `DbId` columns are correctly round-tripped (ULID strings
/// in JSON, not silently dropped `Vec<u8>`).
async fn fetch_all_json<E>(
    db: &livtet_data::orm::DatabaseConnection,
    entity_type: &str,
) -> Result<Vec<serde_json::Value>>
where
    E: livtet_data::orm::EntityTrait,
    E::Model: serde::Serialize,
{
    let rows = E::find().all(db).await.map_err(SyncError::from)?;
    rows.into_iter()
        .map(|r| {
            serde_json::to_value(r).map_err(|e| SyncError::Serialization {
                entity_type: entity_type.to_string(),
                message: e.to_string(),
            })
        })
        .collect()
}

impl SyncableEntityKind {
    /// Fetch all rows for every `EntityDump` entity type and build the dump
    /// using sea-ORM typed queries (replaces the old `query_table` helper).
    pub async fn dump_all(db: &livtet_data::orm::DatabaseConnection) -> Result<super::EntityDump> {
        Ok(super::EntityDump {
            works: fetch_all_json::<livtet_data::entities::works::Entity>(db, "work").await?,
            editions: fetch_all_json::<livtet_data::entities::editions::Entity>(db, "edition")
                .await?,
            edition_groups: fetch_all_json::<livtet_data::entities::edition_groups::Entity>(
                db,
                "edition_group",
            )
            .await?,
            series_entries: fetch_all_json::<livtet_data::entities::series_entries::Entity>(
                db,
                "series_entry",
            )
            .await?,
            digital_inventory: fetch_all_json::<livtet_data::entities::digital_inventory::Entity>(
                db,
                "digital_inventory",
            )
            .await?,
            owned_editions: fetch_all_json::<livtet_data::entities::owned_edition::Entity>(
                db,
                "owned_edition",
            )
            .await?,
            editions_loans: fetch_all_json::<livtet_data::entities::editions_loans::Entity>(
                db,
                "edition_loan",
            )
            .await?,
            annotations: fetch_all_json::<livtet_data::entities::annotations::Entity>(
                db,
                "annotation",
            )
            .await?,
            reading_lists: fetch_all_json::<livtet_data::entities::reading_lists::Entity>(
                db,
                "reading_list",
            )
            .await?,
            reading_list_book: fetch_all_json::<livtet_data::entities::reading_list_book::Entity>(
                db,
                "reading_list_book",
            )
            .await?,
            reading_progress: fetch_all_json::<livtet_data::entities::reading_progress::Entity>(
                db,
                "reading_progress",
            )
            .await?,
        })
    }
}

// ─── Static tables ─────────────────────────────────────────────────────

/// Entity type strings that participate in the full dump (`EntityDump`).
pub const ENTITY_DUMP_TYPES: &[&str] = &[
    "work",
    "edition",
    "edition_group",
    "series_entry",
    "digital_inventory",
    "owned_edition",
    "edition_loan",
    "annotation",
    "reading_list",
    "reading_list_book",
    "reading_progress",
];

/// Map entity type string → table name.
///
/// This replaces the `entity_type_to_table` function in the sync engine.
/// The source of truth is the macro's string constants — this function
/// must be kept in sync.
pub fn entity_type_to_table(entity_type: &str) -> Option<&'static str> {
    match entity_type {
        "annotation" => Some("annotations"),
        "digital_inventory" => Some("digital_inventory"),
        "edition" => Some("editions"),
        "edition_author" => Some("edition_authors"),
        "edition_genre" => Some("edition_genres"),
        "edition_group" => Some("edition_groups"),
        "edition_loan" => Some("editions_loans"),
        "edition_publisher" => Some("edition_publishers"),
        "edition_subject" => Some("edition_subjects"),
        "edition_tag" => Some("edition_tags"),
        "owned_edition" => Some("owned_editions"),
        "reading_list" => Some("reading_lists"),
        "reading_list_book" => Some("reading_list_book"),
        "reading_progress" => Some("reading_progress"),
        "series_entry" => Some("series_entries"),
        "work" => Some("works"),
        "work_author" => Some("work_authors"),
        "work_genre" => Some("work_genres"),
        "work_publisher" => Some("work_publishers"),
        "work_subject" => Some("work_subjects"),
        "work_tag" => Some("work_tags"),
        _ => None,
    }
}

// ─── SyncedEntity wrapper ───────────────────────────────────────────────

/// A sync-tracked entity pairing the model with its change-log metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncedEntity {
    pub kind: SyncableEntityKind,
    pub version: i64,
    pub entity_type: String,
    pub entity_id: String,
    pub payload: serde_json::Value,
}

impl SyncedEntity {
    /// Wrap a `SyncableEntityKind` with its change-log metadata.
    pub fn new(kind: SyncableEntityKind, version: i64) -> Self {
        Self {
            entity_type: kind.entity_type_name().to_string(),
            entity_id: kind.entity_id(),
            payload: kind.to_payload().unwrap_or_default(),
            kind,
            version,
        }
    }

    /// Build from a `SyncChange` row, deserialising the payload.
    pub fn from_change(change: &super::SyncChange) -> Result<Self> {
        let payload: serde_json::Value =
            serde_json::from_str(&change.payload).map_err(|e| SyncError::Deserialization {
                entity_type: change.entity_type.clone(),
                message: e.to_string(),
            })?;
        let kind = SyncableEntityKind::from_payload(&change.entity_type, &payload)?;
        Ok(Self {
            entity_type: change.entity_type.clone(),
            entity_id: change.entity_id.to_string(),
            payload,
            kind,
            version: change.version,
        })
    }
}

// ─── Trait ──────────────────────────────────────────────────────────────

/// Implemented by entity models that participate in the sync protocol.
///
/// Every sea-ORM entity type that has `change_log` triggers should
/// implement this trait.  The trait provides standardised conversion
/// to/from the canonical sync-payload JSON format.
pub trait SyncableEntity: Clone + Serialize + for<'de> Deserialize<'de> + Send + 'static {
    /// The sea-ORM entity type for this model (the one with `Entity::find()`).
    type Entity: livtet_data::orm::EntityTrait<Model = Self>;

    /// Short name used in `change_log.entity_type` (e.g. `"work"`, `"edition"`).
    fn entity_type_name() -> &'static str;

    /// Database table name (e.g. `"works"`, `"owned_editions"`).
    fn table_name() -> &'static str;

    /// Entity identifier for change-log tracking.
    fn entity_id(&self) -> String;

    /// Serialise to the canonical sync-payload JSON.
    fn to_payload(&self) -> Result<serde_json::Value> {
        serde_json::to_value(self).map_err(|e| SyncError::Serialization {
            entity_type: Self::entity_type_name().to_string(),
            message: e.to_string(),
        })
    }

    /// Deserialise from a sync-payload JSON value.
    fn from_payload(v: &serde_json::Value) -> Result<Self> {
        serde_json::from_value(v.clone()).map_err(|e| SyncError::Deserialization {
            entity_type: Self::entity_type_name().to_string(),
            message: e.to_string(),
        })
    }
}
