//! Index-mutating paths: full [`SearchIndex::reindex`] and single-doc
//! upserts / deletes.

use std::collections::HashMap;

/// Progress hook fired by [`SearchIndex::reindex_with_progress`] and
/// [`SearchIndex::migrate_to_with_progress`]. Consumers (typically the
/// CLI's `reindex` command) use this to drive an `indicatif` bar or
/// spinner so the user has feedback during what can be a long rebuild.
#[derive(Debug, Clone, Copy)]
pub enum ReindexEvent {
    /// Index dir is loaded and the DB is being queried.
    Loading,
    /// A batch of documents has been indexed.
    ///
    /// `done` is the count of edition + author documents committed so
    /// far; `total` is the final count the loop will reach.
    Indexing { done: u64, total: u64 },
}

impl ReindexEvent {
    /// No-op unless the event carries concrete counts.
    pub fn is_terminal(self) -> bool {
        !matches!(self, ReindexEvent::Loading)
    }
}

use livtet_data::orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use rayon::prelude::*;
use tantivy::{Term, doc, schema::*};

use crate::{
    doc::{AuthorDoc, EditionDoc},
    index::{SearchError, SearchIndex},
    schema::{fields, kinds},
    search::hash_work_id,
};

impl SearchIndex {
    /// Rebuild the index from scratch by streaming every edition
    /// (and every author) from the DB through the schema. This is
    /// the only safe way to apply a schema change, so the design
    /// plan keeps it as the migration path.
    ///
    /// The pre-existing edition documents are deleted at the start
    /// via `delete_all_documents()`. Author docs are added on top.
    #[tracing::instrument(level = "info", name = "search.reindex", skip_all)]
    pub async fn reindex(&self, db: &DatabaseConnection) -> Result<(), SearchError> {
        self.reindex_with_progress(db, &mut |_| {}).await
    }

    /// Like [`SearchIndex::reindex`] but fires [`ReindexEvent`]s so
    /// callers can drive a progress bar.
    pub async fn reindex_with_progress<F>(
        &self,
        db: &DatabaseConnection,
        on_event: &mut F,
    ) -> Result<(), SearchError>
    where
        F: FnMut(ReindexEvent) + Send + Sync,
    {
        let start = std::time::Instant::now();
        use livtet_data::entities::{
            authors::Entity as Authors, digital_inventory::Entity as DigitalInventory,
            edition_authors::Entity as EditionAuthors,
            edition_genres::Entity as EditionGenres, edition_identifiers::Entity as EditionIds,
            edition_publishers::Entity as EditionPublishers,
            edition_subjects::Entity as EditionSubjects, edition_tags::Entity as EditionTags,
            editions::Entity as Editions, formats::Entity as Formats, genres::Entity as Genres,
            identifiers::Entity as Identifiers, languages::Entity as Languages,
            publishers::Entity as Publishers, series_entries::Entity as SeriesEntries,
            subjects::Entity as Subjects, tags::Entity as Tags,
            work_authors::Entity as WorkAuthors, works::Entity as Works,
        };

        // ---- Phase 1: clear existing documents and load everything
        //              we need from the DB into HashMaps so we can
        //              resolve joins in O(1).
        {
            let mut writer = self.writer.write().await;
            writer.delete_all_documents()?;
            writer.commit()?;
        }

        let all_editions = Editions::find().all(db).await?;
        let all_works = Works::find().all(db).await?;
        let all_authors = Authors::find().all(db).await?;
        let all_tags = Tags::find().all(db).await?;
        let all_genres = Genres::find().all(db).await?;
        let all_subjects = Subjects::find().all(db).await?;
        let all_publishers = Publishers::find().all(db).await?;
        let all_formats = Formats::find().all(db).await?;
        let all_languages = Languages::find().all(db).await?;

        let edition_ids: Vec<livtet_types::DbId> = all_editions.iter().map(|e| e.id).collect();

        // edition_authors: edition_id -> [author_id]
        let edition_author_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionAuthors::find()
                .filter(
                    livtet_data::entities::edition_authors::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let mut edition_to_authors: HashMap<livtet_types::DbId, Vec<(livtet_types::DbId, String)>> =
            HashMap::new();
        // author_id -> name
        let authors_by_id: HashMap<livtet_types::DbId, &str> = all_authors
            .iter()
            .map(|a| (a.id, a.name.as_str()))
            .collect();
        for ea in &edition_author_rows {
            if let Some(name) = authors_by_id.get(&ea.author_id) {
                edition_to_authors
                    .entry(ea.edition_id)
                    .or_default()
                    .push((ea.author_id, (*name).to_string()));
            }
        }
        // work_authors: work_id -> [author_id, author_id, ...] for
        // the work-level fallback when an edition has no
        // edition_authors row.
        let work_ids: Vec<livtet_types::DbId> = all_editions.iter().map(|e| e.work_id).collect();
        let work_author_rows = if work_ids.is_empty() {
            Vec::new()
        } else {
            WorkAuthors::find()
                .filter(livtet_data::entities::work_authors::Column::WorkId.is_in(work_ids.clone()))
                .all(db)
                .await?
        };
        let mut work_to_authors: HashMap<livtet_types::DbId, Vec<(livtet_types::DbId, String)>> =
            HashMap::new();
        for wa in &work_author_rows {
            if let Some(name) = authors_by_id.get(&wa.author_id) {
                work_to_authors
                    .entry(wa.work_id)
                    .or_default()
                    .push((wa.author_id, (*name).to_string()));
            }
        }

        // Tags / genres / subjects / publishers junction tables.
        let edition_tag_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionTags::find()
                .filter(
                    livtet_data::entities::edition_tags::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let tags_by_id: HashMap<livtet_types::DbId, &str> =
            all_tags.iter().map(|t| (t.id, t.name.as_str())).collect();
        let mut edition_to_tag_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &edition_tag_rows {
            edition_to_tag_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.tag_id);
        }

        let edition_genre_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionGenres::find()
                .filter(
                    livtet_data::entities::edition_genres::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let genres_by_id: HashMap<livtet_types::DbId, &str> =
            all_genres.iter().map(|g| (g.id, g.name.as_str())).collect();
        let mut edition_to_genre_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &edition_genre_rows {
            edition_to_genre_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.genre_id);
        }

        let edition_subject_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionSubjects::find()
                .filter(
                    livtet_data::entities::edition_subjects::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let subjects_by_id: HashMap<livtet_types::DbId, &str> = all_subjects
            .iter()
            .map(|s| (s.id, s.name.as_str()))
            .collect();
        let mut edition_to_subject_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &edition_subject_rows {
            edition_to_subject_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.subject_id);
        }

        let edition_publisher_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionPublishers::find()
                .filter(
                    livtet_data::entities::edition_publishers::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let publishers_by_id: HashMap<livtet_types::DbId, &str> = all_publishers
            .iter()
            .map(|p| (p.id, p.name.as_str()))
            .collect();
        let mut edition_to_publisher_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &edition_publisher_rows {
            edition_to_publisher_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.publisher_id);
        }

        // Series entries (series_id per edition).
        let series_entry_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            SeriesEntries::find()
                .filter(
                    livtet_data::entities::series_entries::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let mut edition_to_series_ids: HashMap<livtet_types::DbId, Vec<livtet_types::DbId>> =
            HashMap::new();
        for r in &series_entry_rows {
            edition_to_series_ids
                .entry(r.edition_id)
                .or_default()
                .push(r.series_id);
        }

        // Digital inventory: edition_id → has_file
        let all_inventory = DigitalInventory::find().all(db).await?;
        let inventory_editions: std::collections::HashSet<livtet_types::DbId> =
            all_inventory.iter().map(|r| r.edition_id).collect();

        // Identifiers via edition_identifier → identifiers. Filter to
        // both `isbn` and the non-isbn kinds so we can canonicalise
        // ISBNs via `livtet_types::Isbn::parse`.
        let edition_id_rows = if edition_ids.is_empty() {
            Vec::new()
        } else {
            EditionIds::find()
                .filter(
                    livtet_data::entities::edition_identifiers::Column::EditionId
                        .is_in(edition_ids.clone()),
                )
                .all(db)
                .await?
        };
        let ident_pk_ids: Vec<livtet_types::DbId> =
            edition_id_rows.iter().map(|r| r.identifier_id).collect();
        let identifiers: Vec<livtet_data::entities::identifiers::Model> = if ident_pk_ids.is_empty()
        {
            Vec::new()
        } else {
            Identifiers::find()
                .filter(livtet_data::entities::identifiers::Column::Id.is_in(ident_pk_ids))
                .all(db)
                .await?
        };
        let ident_by_id: HashMap<livtet_types::DbId, &livtet_data::entities::identifiers::Model> =
            identifiers.iter().map(|i| (i.id, i)).collect();

        let mut edition_isbns: HashMap<livtet_types::DbId, Vec<String>> = HashMap::new();
        let mut edition_ident_kinds: HashMap<livtet_types::DbId, Vec<String>> = HashMap::new();
        let mut edition_ident_values: HashMap<livtet_types::DbId, Vec<String>> = HashMap::new();
        for row in &edition_id_rows {
            let Some(ident) = ident_by_id.get(&row.identifier_id) else {
                continue;
            };
            // Store every kind as-is, but canonicalise ISBNs to
            // ISBN-13 via Isbn::parse. Unparseable ISBN rows still
            // surface as a `kind = isbn, value = raw` pair so they
            // remain searchable; we just don't claim canonical form.
            if ident.kind == "isbn" {
                match livtet_types::Isbn::parse(&ident.value) {
                    Ok(canonical) => {
                        edition_isbns
                            .entry(row.edition_id)
                            .or_default()
                            .push(canonical.to_string());
                        edition_ident_kinds
                            .entry(row.edition_id)
                            .or_default()
                            .push(ident.kind.clone());
                        edition_ident_values
                            .entry(row.edition_id)
                            .or_default()
                            .push(canonical.to_string());
                    }
                    Err(_) => {
                        edition_ident_kinds
                            .entry(row.edition_id)
                            .or_default()
                            .push(ident.kind.clone());
                        edition_ident_values
                            .entry(row.edition_id)
                            .or_default()
                            .push(ident.value.clone());
                    }
                }
            } else {
                edition_ident_kinds
                    .entry(row.edition_id)
                    .or_default()
                    .push(ident.kind.clone());
                edition_ident_values
                    .entry(row.edition_id)
                    .or_default()
                    .push(ident.value.clone());
            }
        }

        // Source provenance: the identifier entity no longer carries
        // `source` / `fetched_at` columns, so every edition defaults
        // to the `"catalog"` provenance for indexing purposes.
        let edition_to_source: HashMap<livtet_types::DbId, String> = HashMap::new();

        let formats_by_id: HashMap<livtet_types::DbId, &str> = all_formats
            .iter()
            .map(|f| (f.id, f.name.as_str()))
            .collect();
        let languages_by_id: HashMap<livtet_types::DbId, &str> = all_languages
            .iter()
            .map(|l| (l.id, l.name.as_str()))
            .collect();
        let works_by_id: HashMap<livtet_types::DbId, &livtet_data::entities::works::Model> =
            all_works.iter().map(|w| (w.id, w)).collect();

        // ---- Phase 2: write documents.
        let mut writer = self.writer.write().await;
        let edition_id_field = self
            .schema
            .get_field(fields::EDITION_ID)
            .expect("edition_id");
        let work_id_field = self.schema.get_field(fields::WORK_ID).expect("work_id");
        let work_id_hash_field = self
            .schema
            .get_field(fields::WORK_ID_HASH)
            .expect("work_id_hash");
        let author_id_field = self.schema.get_field(fields::AUTHOR_ID).expect("author_id");
        let kind_field = self.schema.get_field(fields::KIND).expect("kind");
        let title_field = self.schema.get_field(fields::TITLE).expect("title");
        let edition_title_field = self
            .schema
            .get_field(fields::EDITION_TITLE)
            .expect("edition_title");
        let work_description_field = self
            .schema
            .get_field(fields::WORK_DESCRIPTION)
            .expect("work_description");
        let edition_description_field = self
            .schema
            .get_field(fields::EDITION_DESCRIPTION)
            .expect("edition_description");
        let authors_field = self.schema.get_field(fields::AUTHORS).expect("authors");
        let tags_field = self.schema.get_field(fields::TAGS).expect("tags");
        let genres_field = self.schema.get_field(fields::GENRES).expect("genres");
        let subjects_field = self.schema.get_field(fields::SUBJECTS).expect("subjects");
        let publishers_field = self
            .schema
            .get_field(fields::PUBLISHERS)
            .expect("publishers");
        let identifier_kinds_field = self
            .schema
            .get_field(fields::IDENTIFIER_KINDS)
            .expect("identifier_kinds");
        let identifier_values_field = self
            .schema
            .get_field(fields::IDENTIFIER_VALUES)
            .expect("identifier_values");
        let notes_field = self.schema.get_field(fields::NOTES).expect("notes");
        let format_field = self.schema.get_field(fields::FORMAT).expect("format");
        let language_field = self.schema.get_field(fields::LANGUAGE).expect("language");
        let language_facet_field = self
            .schema
            .get_field(fields::LANGUAGE_FACET)
            .expect("language_facet");
        let publisher_facet_field = self
            .schema
            .get_field(fields::PUBLISHER_FACET)
            .expect("publisher_facet");
        let subject_facet_field = self
            .schema
            .get_field(fields::SUBJECT_FACET)
            .expect("subject_facet");
        let genre_facet_field = self
            .schema
            .get_field(fields::GENRE_FACET)
            .expect("genre_facet");
        let pub_date_field = self.schema.get_field(fields::PUB_DATE).expect("pub_date");
        let published_year_field = self
            .schema
            .get_field(fields::PUBLISHED_YEAR)
            .expect("published_year");
        let title_sort_field = self
            .schema
            .get_field(fields::TITLE_SORT)
            .expect("title_sort");
        let primary_author_sort_field = self
            .schema
            .get_field(fields::PRIMARY_AUTHOR_SORT)
            .expect("primary_author_sort");
        let created_at_field = self
            .schema
            .get_field(fields::CREATED_AT)
            .expect("created_at");
        let updated_at_field = self
            .schema
            .get_field(fields::UPDATED_AT)
            .expect("updated_at");
        let popularity_field = self
            .schema
            .get_field(fields::POPULARITY)
            .expect("popularity");
        let source_field = self.schema.get_field(fields::SOURCE).expect("source");
        let has_file_field = self.schema.get_field(fields::HAS_FILE).expect("has_file");
        let tag_id_field = self.schema.get_field(fields::TAG_ID).expect("tag_id");
        let genre_id_field = self.schema.get_field(fields::GENRE_ID).expect("genre_id");
        let subject_id_field = self
            .schema
            .get_field(fields::SUBJECT_ID)
            .expect("subject_id");
        let series_id_field = self.schema.get_field(fields::SERIES_ID).expect("series_id");
        let publisher_id_field = self
            .schema
            .get_field(fields::PUBLISHER_ID)
            .expect("publisher_id");

        // Build all edition documents in parallel. Document
        // construction only reads the lookup maps and copies data
        // into a fresh `TantivyDocument` per edition; the index
        // writer is touched only in the sequential commit loop
        // below. This is the pre-build-Vec pattern: rayon handles
        // the CPU-bound work, and we keep the writer single-threaded
        // so we don't rely on `IndexWriter`'s Send-ness (tantivy's
        // public API doesn't guarantee it across all configurations).
        let edition_docs: Vec<tantivy::TantivyDocument> = all_editions
            .par_iter()
            .map(|edition| {
                let work = works_by_id.get(&edition.work_id);
                let edition_title = edition.title.clone().unwrap_or_default();
                let work_title = work.map(|w| w.title.clone()).unwrap_or_default();
                let resolved_title = if edition_title.is_empty() {
                    work_title.clone()
                } else {
                    edition_title.clone()
                };
                // Author resolution: edition_authors → work_authors →
                // empty.
                let authors = edition_to_authors
                    .get(&edition.id)
                    .cloned()
                    .or_else(|| work_to_authors.get(&edition.work_id).cloned())
                    .unwrap_or_default();
                let author_names: Vec<String> = authors.iter().map(|(_, n)| n.clone()).collect();
                // Categorical IDs are stored as text strings — tantivy's
                // text-fast fields accept any value, and the text form
                // lets us query `author_id:01HX...` and `author_id:01HY...`
                // against the same multi-valued column.
                let author_id_strs: Vec<String> =
                    authors.iter().map(|(id, _)| id.to_string()).collect();

                // Identifier kind/value vectors are aligned.
                let ident_kinds = edition_ident_kinds
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default();
                let ident_values = edition_ident_values
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default();

                // Tags / genres / subjects / publishers by name (for
                // text) and by id (for fast fields).
                let tag_names: Vec<String> = edition_to_tag_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|tid| tags_by_id.get(&tid).map(|n| (*n).to_string()))
                    .collect();
                let tag_id_strs: Vec<String> = edition_to_tag_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let genre_names: Vec<String> = edition_to_genre_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|gid| genres_by_id.get(&gid).map(|n| (*n).to_string()))
                    .collect();
                let genre_id_strs: Vec<String> = edition_to_genre_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let subject_names: Vec<String> = edition_to_subject_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|sid| subjects_by_id.get(&sid).map(|n| (*n).to_string()))
                    .collect();
                let subject_id_strs: Vec<String> = edition_to_subject_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let publisher_names: Vec<String> = edition_to_publisher_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|pid| publishers_by_id.get(&pid).map(|n| (*n).to_string()))
                    .collect();
                let publisher_id_strs: Vec<String> = edition_to_publisher_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let series_id_strs: Vec<String> = edition_to_series_ids
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();

                let format_name = edition
                    .format_id
                    .and_then(|fid| formats_by_id.get(&fid).map(|n| (*n).to_string()));
                let language_name = edition
                    .language_id
                    .and_then(|lid| languages_by_id.get(&lid).map(|n| (*n).to_string()));
                let work_language_name = work
                    .and_then(|w| w.language_id)
                    .and_then(|lid| languages_by_id.get(&lid).map(|n| (*n).to_string()));
                let language_name = language_name.or(work_language_name);

                let work_id_hash = hash_work_id(&edition.work_id.to_string());

                let pub_date_value = edition.published_date.map(|d| {
                    let nanos = d.midnight().assume_utc().unix_timestamp_nanos();
                    tantivy::DateTime::from_timestamp_millis((nanos / 1_000_000) as i64)
                });
                let published_year = edition.published_date.map(|d| d.year() as u64).unwrap_or(0);
                let created_at_value = Some({
                    let nanos = edition.created_at.assume_utc().unix_timestamp_nanos();
                    tantivy::DateTime::from_timestamp_millis((nanos / 1_000_000) as i64)
                });

                let mut d = doc!();
                // IDs and kind discriminator
                d.add_text(edition_id_field, edition.id.to_string());
                d.add_text(work_id_field, edition.work_id.to_string());
                d.add_u64(work_id_hash_field, work_id_hash);
                for aid in &author_id_strs {
                    d.add_text(author_id_field, aid);
                }
                d.add_text(kind_field, kinds::EDITION);

                // Full-text + multi-valued
                d.add_text(title_field, &resolved_title);
                if !edition_title.is_empty() {
                    d.add_text(edition_title_field, &edition_title);
                }
                if let Some(w) = work
                    && let Some(desc) = &w.description
                {
                    d.add_text(work_description_field, desc);
                }
                if let Some(desc) = &edition.description {
                    d.add_text(edition_description_field, desc);
                }
                for n in &author_names {
                    d.add_text(authors_field, n);
                }
                for n in &tag_names {
                    d.add_text(tags_field, n);
                }
                for n in &genre_names {
                    d.add_text(genres_field, n);
                }
                for n in &subject_names {
                    d.add_text(subjects_field, n);
                }
                for n in &publisher_names {
                    d.add_text(publishers_field, n);
                }
                for k in &ident_kinds {
                    d.add_text(identifier_kinds_field, k);
                }
                for v in &ident_values {
                    d.add_text(identifier_values_field, v);
                }
                d.add_text(notes_field, edition.notes.clone().unwrap_or_default());

                // Filters / sort / facet
                if let Some(f) = &format_name {
                    d.add_text(format_field, f);
                }
                if let Some(l) = &language_name {
                    d.add_text(language_field, l);
                    d.add_facet(
                        language_facet_field,
                        Facet::from_text(&format!("/{}", l))
                            .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
                    );
                }
                for p in &publisher_names {
                    d.add_facet(
                        publisher_facet_field,
                        Facet::from_text(&format!("/{}", p))
                            .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
                    );
                }
                for s in &subject_names {
                    d.add_facet(
                        subject_facet_field,
                        Facet::from_text(&format!("/{}", s))
                            .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
                    );
                }
                for g in &genre_names {
                    d.add_facet(
                        genre_facet_field,
                        Facet::from_text(&format!("/{}", g))
                            .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
                    );
                }
                if let Some(pd) = pub_date_value {
                    d.add_date(pub_date_field, pd);
                }
                if published_year > 0 {
                    d.add_u64(published_year_field, published_year);
                }
                d.add_text(title_sort_field, resolved_title.to_lowercase());
                if let Some((_, primary_author)) = authors.first() {
                    d.add_text(primary_author_sort_field, primary_author.to_lowercase());
                }
                if let Some(ts) = created_at_value {
                    d.add_date(created_at_field, ts);
                }
                // updated_at — same treatment as created_at, using the
                // edition's updated_at when available.
                if let Some(ua) = edition.updated_at {
                    let nanos = ua.assume_utc().unix_timestamp_nanos();
                    d.add_date(
                        updated_at_field,
                        tantivy::DateTime::from_timestamp_millis((nanos / 1_000_000) as i64),
                    );
                }
                // popularity is currently unset; we still add the field
                // so the fast column exists.
                d.add_u64(popularity_field, 0u64);

                // Source provenance
                let edition_source = edition_to_source
                    .get(&edition.id)
                    .cloned()
                    .unwrap_or_else(|| "catalog".to_string());
                d.add_text(source_field, &edition_source);

                // has_file
                let has_file = inventory_editions.contains(&edition.id);
                d.add_bool(has_file_field, has_file);

                // Categorical IDs.
                for t in &tag_id_strs {
                    d.add_text(tag_id_field, t);
                }
                for g in &genre_id_strs {
                    d.add_text(genre_id_field, g);
                }
                for s in &subject_id_strs {
                    d.add_text(subject_id_field, s);
                }
                for s in &series_id_strs {
                    d.add_text(series_id_field, s);
                }
                for p in &publisher_id_strs {
                    d.add_text(publisher_id_field, p);
                }

                d
            })
            .collect();

        // Commit the pre-built documents to the index in a single
        // thread. Tantivy's `IndexWriter` Send-ness is not part of
        // its public API contract, so we keep writer access strictly
        // serial — the rayon work above is purely CPU-bound document
        // construction, with no shared mutable state.
        //
        // `add_document` takes the `Document` by value, so we move
        // each one out of the `Vec`. `edition_docs` is no longer
        // needed after this loop and is dropped at the end of the
        // scope.
        let total = edition_docs.len() as u64 + all_authors.len() as u64;
        on_event(ReindexEvent::Indexing { done: 0, total });
        let mut done = 0u64;
        for d in edition_docs {
            writer.add_document(d)?;
            done += 1;
            on_event(ReindexEvent::Indexing { done, total });
        }

        // Authors get their own documents, indexed with `kind = "author"`.
        for author in &all_authors {
            let mut d = doc!();
            d.add_text(author_id_field, author.id.to_string());
            d.add_text(kind_field, kinds::AUTHOR);
            d.add_text(title_field, &author.name);
            d.add_text(authors_field, &author.name);
            d.add_text(title_sort_field, author.name.to_lowercase());
            d.add_text(primary_author_sort_field, author.name.to_lowercase());
            writer.add_document(d)?;
            done += 1;
            on_event(ReindexEvent::Indexing { done, total });
        }

        writer.commit()?;
        self.reader.reload()?;
        tracing::debug!(
            target: "livtet.search.perf",
            elapsed_ms = start.elapsed().as_millis(),
            "search reindex complete"
        );
        Ok(())
    }

    /// Add or update one edition in the index.
    ///
    /// For now this delegates to a full reindex because Tantivy
    /// can't rewrite a single document cheaply. The seam exists so
    /// callers (the Tauri command) can swap in a true upsert path
    /// later without changing call sites.
    pub async fn add_edition(
        &self,
        db: &DatabaseConnection,
        edition_id: livtet_types::DbId,
    ) -> Result<(), SearchError> {
        let _ = (db, edition_id);
        self.reindex(db).await
    }

    /// Single-doc upsert from a denormalized [`EditionDoc`].
    ///
    /// Unlike [`add_edition`], this never touches the database; the
    /// caller (typically the NAPI binding) assembles the document
    /// shape and writes it directly. Used to keep the NAPI hot path
    /// off the SQL reindex seam while still surfacing edits in
    /// search results.
    ///
    /// Semantics: any existing document whose `edition_id` matches
    /// `doc.edition_id` is deleted before the new one is added, so a
    /// second call with the same id overwrites rather than
    /// duplicating.
    pub async fn upsert_edition(&self, doc: EditionDoc) -> Result<(), SearchError> {
        let edition_id_field = self
            .schema
            .get_field(fields::EDITION_ID)
            .expect("edition_id");
        let work_id_field = self.schema.get_field(fields::WORK_ID).expect("work_id");
        let work_id_hash_field = self
            .schema
            .get_field(fields::WORK_ID_HASH)
            .expect("work_id_hash");
        let author_id_field = self.schema.get_field(fields::AUTHOR_ID).expect("author_id");
        let kind_field = self.schema.get_field(fields::KIND).expect("kind");
        let title_field = self.schema.get_field(fields::TITLE).expect("title");
        let edition_title_field = self
            .schema
            .get_field(fields::EDITION_TITLE)
            .expect("edition_title");
        let work_description_field = self
            .schema
            .get_field(fields::WORK_DESCRIPTION)
            .expect("work_description");
        let edition_description_field = self
            .schema
            .get_field(fields::EDITION_DESCRIPTION)
            .expect("edition_description");
        let authors_field = self.schema.get_field(fields::AUTHORS).expect("authors");
        let tags_field = self.schema.get_field(fields::TAGS).expect("tags");
        let genres_field = self.schema.get_field(fields::GENRES).expect("genres");
        let subjects_field = self.schema.get_field(fields::SUBJECTS).expect("subjects");
        let publishers_field = self
            .schema
            .get_field(fields::PUBLISHERS)
            .expect("publishers");
        let identifier_kinds_field = self
            .schema
            .get_field(fields::IDENTIFIER_KINDS)
            .expect("identifier_kinds");
        let identifier_values_field = self
            .schema
            .get_field(fields::IDENTIFIER_VALUES)
            .expect("identifier_values");
        let notes_field = self.schema.get_field(fields::NOTES).expect("notes");
        let format_field = self.schema.get_field(fields::FORMAT).expect("format");
        let language_field = self.schema.get_field(fields::LANGUAGE).expect("language");
        let language_facet_field = self
            .schema
            .get_field(fields::LANGUAGE_FACET)
            .expect("language_facet");
        let publisher_facet_field = self
            .schema
            .get_field(fields::PUBLISHER_FACET)
            .expect("publisher_facet");
        let subject_facet_field = self
            .schema
            .get_field(fields::SUBJECT_FACET)
            .expect("subject_facet");
        let genre_facet_field = self
            .schema
            .get_field(fields::GENRE_FACET)
            .expect("genre_facet");
        let pub_date_field = self.schema.get_field(fields::PUB_DATE).expect("pub_date");
        let published_year_field = self
            .schema
            .get_field(fields::PUBLISHED_YEAR)
            .expect("published_year");
        let title_sort_field = self
            .schema
            .get_field(fields::TITLE_SORT)
            .expect("title_sort");
        let primary_author_sort_field = self
            .schema
            .get_field(fields::PRIMARY_AUTHOR_SORT)
            .expect("primary_author_sort");
        let created_at_field = self
            .schema
            .get_field(fields::CREATED_AT)
            .expect("created_at");
        let updated_at_field = self
            .schema
            .get_field(fields::UPDATED_AT)
            .expect("updated_at");
        let popularity_field = self
            .schema
            .get_field(fields::POPULARITY)
            .expect("popularity");
        let source_field = self.schema.get_field(fields::SOURCE).expect("source");

        let work_id_hash = hash_work_id(&doc.work_id);
        let pub_date_value = doc
            .pub_date
            .map(|secs| tantivy::DateTime::from_timestamp_millis(secs.saturating_mul(1_000)));
        let published_year = doc.published_year.map(|y| y.max(0) as u64).unwrap_or(0);
        let created_at_value =
            tantivy::DateTime::from_timestamp_millis(doc.created_at.saturating_mul(1_000));

        let mut d = doc!();
        d.add_text(edition_id_field, &doc.edition_id);
        d.add_text(work_id_field, &doc.work_id);
        d.add_u64(work_id_hash_field, work_id_hash);
        for aid in &doc.authors_ids {
            d.add_text(author_id_field, aid);
        }
        d.add_text(kind_field, kinds::EDITION);
        d.add_text(title_field, &doc.title);
        if let Some(et) = &doc.edition_title {
            d.add_text(edition_title_field, et);
        }
        if let Some(desc) = &doc.work_description {
            d.add_text(work_description_field, desc);
        }
        if let Some(desc) = &doc.edition_description {
            d.add_text(edition_description_field, desc);
        }
        for n in &doc.authors {
            d.add_text(authors_field, n);
        }
        for n in &doc.tags {
            d.add_text(tags_field, n);
        }
        for n in &doc.genres {
            d.add_text(genres_field, n);
        }
        for n in &doc.subjects {
            d.add_text(subjects_field, n);
        }
        for n in &doc.publishers {
            d.add_text(publishers_field, n);
        }
        for k in &doc.identifier_kinds {
            d.add_text(identifier_kinds_field, k);
        }
        for v in &doc.identifier_values {
            d.add_text(identifier_values_field, v);
        }
        d.add_text(notes_field, doc.notes.clone().unwrap_or_default());
        if let Some(f) = &doc.format {
            d.add_text(format_field, f);
        }
        if let Some(l) = &doc.language {
            d.add_text(language_field, l);
            d.add_facet(
                language_facet_field,
                Facet::from_text(&format!("/{}", l))
                    .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
            );
        }
        for p in &doc.publishers {
            d.add_facet(
                publisher_facet_field,
                Facet::from_text(&format!("/{}", p))
                    .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
            );
        }
        for s in &doc.subjects {
            d.add_facet(
                subject_facet_field,
                Facet::from_text(&format!("/{}", s))
                    .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
            );
        }
        for g in &doc.genres {
            d.add_facet(
                genre_facet_field,
                Facet::from_text(&format!("/{}", g))
                    .unwrap_or_else(|_| Facet::from_text("/misc").unwrap()),
            );
        }
        if let Some(pd) = pub_date_value {
            d.add_date(pub_date_field, pd);
        }
        if published_year > 0 {
            d.add_u64(published_year_field, published_year);
        }
        d.add_text(title_sort_field, doc.title_sort.to_lowercase());
        if let Some(primary) = &doc.primary_author_sort {
            d.add_text(primary_author_sort_field, primary.to_lowercase());
        }
        d.add_date(created_at_field, created_at_value);
        if let Some(secs) = doc.updated_at {
            d.add_date(
                updated_at_field,
                tantivy::DateTime::from_timestamp_millis(secs.saturating_mul(1_000)),
            );
        }
        d.add_u64(popularity_field, doc.popularity.max(0) as u64);
        // NAPI-sourced docs don't carry a source provenance string;
        // default to the same "catalog" bucket reindex uses.
        d.add_text(source_field, "catalog");

        let mut writer = self.writer.write().await;
        let term = Term::from_field_text(edition_id_field, &doc.edition_id);
        writer.delete_term(term);
        writer.add_document(d)?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Single-doc upsert from a denormalized [`AuthorDoc`].
    ///
    /// Mirrors [`upsert_edition`]: any existing document whose
    /// `author_id` matches `doc.author_id` is deleted before the
    /// new one is added. Author documents carry the `kind = "author"`
    /// discriminator so [`HitKind::Person`] queries resolve them.
    pub async fn upsert_author(&self, doc: AuthorDoc) -> Result<(), SearchError> {
        let author_id_field = self.schema.get_field(fields::AUTHOR_ID).expect("author_id");
        let kind_field = self.schema.get_field(fields::KIND).expect("kind");
        let title_field = self.schema.get_field(fields::TITLE).expect("title");
        let authors_field = self.schema.get_field(fields::AUTHORS).expect("authors");
        let title_sort_field = self
            .schema
            .get_field(fields::TITLE_SORT)
            .expect("title_sort");
        let primary_author_sort_field = self
            .schema
            .get_field(fields::PRIMARY_AUTHOR_SORT)
            .expect("primary_author_sort");
        let source_field = self.schema.get_field(fields::SOURCE).expect("source");

        let mut d = doc!();
        d.add_text(author_id_field, &doc.author_id);
        d.add_text(kind_field, kinds::AUTHOR);
        d.add_text(title_field, &doc.name);
        d.add_text(authors_field, &doc.name);
        d.add_text(title_sort_field, doc.sort_name.to_lowercase());
        d.add_text(primary_author_sort_field, doc.sort_name.to_lowercase());
        d.add_text(source_field, &doc.source);

        let mut writer = self.writer.write().await;
        let term = Term::from_field_text(author_id_field, &doc.author_id);
        writer.delete_term(term);
        writer.add_document(d)?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Delete one edition from the index by its ULID.
    ///
    /// Tantivy can't surgically drop one document, so we delete by
    /// term on the indexed `edition_id` field. If `edition_id` was
    /// never indexed (e.g. the editor never reindexed) this is a
    /// harmless no-op.
    pub async fn delete_edition(&self, edition_id: livtet_types::DbId) -> Result<(), SearchError> {
        let edition_id_field = self
            .schema
            .get_field(fields::EDITION_ID)
            .expect("edition_id");
        let term = Term::from_field_text(edition_id_field, &edition_id.to_string());
        let mut writer = self.writer.write().await;
        writer.delete_term(term);
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }
}
