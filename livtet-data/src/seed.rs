//! Database seeding for development/testing/debug builds.
//!
//! Generates realistic library data to simulate a user's library
//! after weeks of usage. Only compiled when the `fake` feature is enabled.

use std::collections::HashMap;

// ── SeaORM entity aliases ───────────────────────────────────────────────────
use crate::entities;
use entities::{
    annotations::{ActiveModel as AnnotationActiveModel, Entity as AnnotationsEntity},
    authors::{ActiveModel as AuthorActiveModel, Entity as AuthorsEntity},
    current_work_status::{
        ActiveModel as CurrentWorkStatusActiveModel, Entity as CurrentWorkStatusEntity,
    },
    digital_inventory::{
        ActiveModel as DigitalInventoryActiveModel, Entity as DigitalInventoryEntity,
    },
    edition_authors::{ActiveModel as EditionAuthorActiveModel, Entity as EditionAuthorsEntity},
    edition_groups::{ActiveModel as EditionGroupActiveModel, Entity as EditionGroupsEntity},
    edition_identifiers::{
        ActiveModel as EditionIdentifierActiveModel, Entity as EditionIdentifiersEntity,
    },
    edition_publishers::{
        ActiveModel as EditionPublisherActiveModel, Entity as EditionPublishersEntity,
    },
    edition_tags::{ActiveModel as EditionTagActiveModel, Entity as EditionTagsEntity},
    editions::{ActiveModel as EditionActiveModel, Entity as EditionsEntity},
    editions_loans::{ActiveModel as EditionsLoansActiveModel, Entity as EditionsLoansEntity},
    formats::{ActiveModel as FormatActiveModel, Entity as FormatsEntity},
    genres::{ActiveModel as GenreActiveModel, Entity as GenresEntity},
    identifiers::{ActiveModel as IdentifierActiveModel, Entity as IdentifiersEntity},
    languages::{ActiveModel as LanguageActiveModel, Entity as LanguagesEntity},
    loan_entity::{ActiveModel as LoanEntityActiveModel, Entity as LoanEntitiesEntity},
    loan_entity_identifier::{
        ActiveModel as LoanEntityIdentifierActiveModel, Entity as LoanEntityIdentifiersEntity,
    },
    owned_edition::{ActiveModel as OwnedEditionActiveModel, Entity as OwnedEditionsEntity},
    publishers::{ActiveModel as PublisherActiveModel, Entity as PublishersEntity},
    reading_list_book::{
        ActiveModel as ReadingListBookActiveModel, Entity as ReadingListBookEntity,
    },
    reading_lists::{ActiveModel as ReadingListActiveModel, Entity as ReadingListsEntity},
    reading_progress::{
        ActiveModel as ReadingProgressActiveModel, Entity as ReadingProgressEntity,
    },
    reading_sessions::{ActiveModel as ReadingSessionActiveModel, Entity as ReadingSessionsEntity},
    reading_sources::{ActiveModel as ReadingSourceActiveModel, Entity as ReadingSourcesEntity},
    saved_search::{ActiveModel as SavedSearchActiveModel, Entity as SavedSearchesEntity},
    series::{ActiveModel as SeriesActiveModel, Entity as SeriesEntity},
    series_entries::{ActiveModel as SeriesEntryActiveModel, Entity as SeriesEntriesEntity},
    subjects::{ActiveModel as SubjectActiveModel, Entity as SubjectsEntity},
    tags::{ActiveModel as TagActiveModel, Entity as TagsEntity},
    work_authors::{ActiveModel as WorkAuthorActiveModel, Entity as WorkAuthorsEntity},
    work_genres::{ActiveModel as WorkGenreActiveModel, Entity as WorkGenresEntity},
    work_identifiers::{ActiveModel as WorkIdentifierActiveModel, Entity as WorkIdentifiersEntity},
    work_publishers::{ActiveModel as WorkPublisherActiveModel, Entity as WorkPublishersEntity},
    work_subjects::{ActiveModel as WorkSubjectActiveModel, Entity as WorkSubjectsEntity},
    work_tags::{ActiveModel as WorkTagActiveModel, Entity as WorkTagsEntity},
    works::{ActiveModel as WorkActiveModel, Entity as WorksEntity},
};
use fake::Fake;
use livtet_types::{
    CommonLanguages, DbId, KnownFormats, KnownGenres, KnownReadingSources, KnownSubjects,
    WorkStatus,
};
use sea_orm::{
    ActiveModelTrait, DatabaseConnection, DatabaseTransaction, EntityTrait, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime, PrimitiveDateTime};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedResult {
    pub works_created: u32,
    pub editions_created: u32,
    pub authors_created: u32,
    pub publishers_created: u32,
    pub reading_status_count: u32,
    pub annotations_created: u32,
    pub digital_inventory_created: u32,
    pub loans_created: u32,
    pub reading_sessions_created: u32,
    pub saved_searches_created: u32,
    pub reading_lists_created: u32,
}

#[derive(Debug, Clone)]
pub struct SeedConfig {
    pub num_works: u32,
    pub editions_per_work: (u8, u8),
    pub actively_reading_pct: f64,
    pub did_not_finish_pct: f64,
    pub edition_group_pct: f64,
    pub annotation_pct: f64,
    pub loan_pct: f64,
    pub digital_inventory_pct: f64,
}

impl Default for SeedConfig {
    fn default() -> Self {
        Self::realistic()
    }
}

impl SeedConfig {
    pub fn realistic() -> Self {
        Self {
            num_works: 30,
            editions_per_work: (1, 3),
            actively_reading_pct: 0.10,
            did_not_finish_pct: 0.20,
            edition_group_pct: 0.10,
            annotation_pct: 0.50,
            loan_pct: 0.05,
            digital_inventory_pct: 0.30,
        }
    }
}

const SEED_TIMESTAMP_BASE_MS: i64 = 1735689600300;

fn primitive_now() -> PrimitiveDateTime {
    let now = OffsetDateTime::from_unix_timestamp(SEED_TIMESTAMP_BASE_MS / 1000).unwrap();
    PrimitiveDateTime::new(now.date(), now.time())
}

fn dbid_from_ulid(u: ulid::Ulid) -> DbId {
    DbId(u)
}

pub async fn seed_database(
    pool: &DatabaseConnection,
    config: &SeedConfig,
) -> Result<SeedResult, sea_orm::DbErr> {
    let txn = pool.begin().await?;
    let result = seed_database_inner(&txn, config).await;
    txn.commit().await?;

    // Reclaim any pages freed by the bulk inserts under auto_vacuum=INCREMENTAL.
    // N=100 is a safe batch size that won't block for long.
    use sea_orm::ConnectionTrait;
    let _: Result<sea_orm::ExecResult, sea_orm::DbErr> = pool
        .execute_unprepared("PRAGMA incremental_vacuum(100)")
        .await;

    result
}

async fn seed_database_inner(
    txn: &DatabaseTransaction,
    config: &SeedConfig,
) -> Result<SeedResult, sea_orm::DbErr> {
    let timestamp = primitive_now();

    let mut result = SeedResult {
        works_created: 0,
        editions_created: 0,
        authors_created: 0,
        publishers_created: 0,
        reading_status_count: 0,
        annotations_created: 0,
        digital_inventory_created: 0,
        loans_created: 0,
        reading_sessions_created: 0,
        saved_searches_created: 0,
        reading_lists_created: 0,
    };

    seed_known_formats(txn).await?;
    seed_known_languages(txn, timestamp).await?;
    let _genre_ids = seed_known_genres(txn, timestamp).await?;
    let _subject_ids = seed_known_subjects(txn, timestamp).await?;
    let _tag_ids = seed_tags(txn, timestamp).await?;

    let (works, editions) =
        generate_works_and_editions(txn, config, timestamp, &mut result).await?;

    let work_ids: Vec<DbId> = works.iter().map(|w| w.id).collect();
    let edition_ids: Vec<DbId> = editions.iter().map(|e| e.id).collect();
    let work_edition_map: HashMap<DbId, Vec<DbId>> =
        editions.iter().fold(HashMap::new(), |mut acc, e| {
            acc.entry(e.work_id).or_default().push(e.id);
            acc
        });

    let author_ids = seed_authors(txn, config.num_works, &mut result).await?;
    let publisher_ids = seed_publishers(txn, config.num_works, timestamp, &mut result).await?;

    link_work_authors(txn, &work_ids, &author_ids).await?;
    link_edition_authors(txn, &edition_ids, &author_ids).await?;
    link_work_genres(txn, &work_ids).await?;
    link_work_subjects(txn, &work_ids).await?;
    link_work_tags(txn, &work_ids).await?;
    link_edition_tags(txn, &edition_ids).await?;
    link_work_publishers(txn, &work_ids, &publisher_ids).await?;
    link_edition_publishers(txn, &edition_ids, &publisher_ids).await?;
    add_identifiers(txn, &edition_ids, &work_ids).await?;
    add_saved_searches(txn, timestamp, &mut result).await?;

    let (work_with_status, status_summary) =
        assign_reading_statuses(txn, &work_ids, &work_edition_map, config, timestamp).await?;
    result.reading_status_count = status_summary;

    add_digital_inventory(txn, &work_with_status, config, timestamp, &mut result).await?;
    add_annotations(txn, &work_with_status, config, timestamp, &mut result).await?;
    add_edition_groups(txn, &work_edition_map, config, timestamp).await?;
    let (_loan_entity_ids, _loan_summary) =
        add_loans(txn, &edition_ids, config, timestamp, &mut result).await?;
    add_owned_editions(txn, &work_with_status, config, timestamp).await?;
    add_reading_sessions(txn, &work_with_status, config, timestamp, &mut result).await?;
    add_reading_lists(txn, &work_edition_map, timestamp, &mut result).await?;
    add_series(txn, &work_edition_map, timestamp).await?;

    Ok(result)
}

async fn seed_known_formats(pool: &DatabaseTransaction) -> Result<(), sea_orm::DbErr> {
    for format in KnownFormats::all() {
        let format_id = dbid_from_ulid(format.ulid());
        let existing = FormatsEntity::find_by_id(format_id).one(pool).await?;
        if existing.is_none() {
            let model = FormatActiveModel {
                id: Set(format_id),
                name: Set(format.name().to_string()),
                metadata_schema: Set(serde_json::Value::Null),
                progress_unit: Set(Some(format.default_progress_unit().to_string())),
            };
            FormatsEntity::insert(model).exec(pool).await?;
        }
    }
    Ok(())
}

async fn seed_known_languages(
    pool: &DatabaseTransaction,
    timestamp: PrimitiveDateTime,
) -> Result<(), sea_orm::DbErr> {
    for lang in CommonLanguages::all() {
        let lang_id = dbid_from_ulid(lang.ulid());
        let existing = LanguagesEntity::find_by_id(lang_id).one(pool).await?;
        if existing.is_none() {
            let model = LanguageActiveModel {
                id: Set(lang_id),
                name: Set(lang.name().to_string()),
                code: Set(lang.code().to_string()),
                flag_emoji: Set(Some(lang.flag_emoji().to_string())),
                created_at: Set(timestamp),
                updated_at: Set(None),
            };
            LanguagesEntity::insert(model).exec(pool).await?;
        }
    }
    Ok(())
}

async fn seed_known_genres(
    pool: &DatabaseTransaction,
    timestamp: PrimitiveDateTime,
) -> Result<Vec<DbId>, sea_orm::DbErr> {
    let mut ids = Vec::new();
    for genre in KnownGenres::all() {
        let genre_id = dbid_from_ulid(genre.ulid());
        ids.push(genre_id);
        let existing = GenresEntity::find_by_id(genre_id).one(pool).await?;
        if existing.is_none() {
            let model = GenreActiveModel {
                id: Set(genre_id),
                name: Set(genre.name().to_string()),
                created_at: Set(timestamp),
            };
            GenresEntity::insert(model).exec(pool).await?;
        }
    }
    Ok(ids)
}

async fn seed_known_subjects(
    pool: &DatabaseTransaction,
    timestamp: PrimitiveDateTime,
) -> Result<Vec<DbId>, sea_orm::DbErr> {
    let mut ids = Vec::new();
    for subject in KnownSubjects::all() {
        let subject_id = dbid_from_ulid(subject.ulid());
        ids.push(subject_id);
        let existing = SubjectsEntity::find_by_id(subject_id).one(pool).await?;
        if existing.is_none() {
            let model = SubjectActiveModel {
                id: Set(subject_id),
                name: Set(subject.name().to_string()),
                created_at: Set(timestamp),
                updated_at: Set(None),
            };
            SubjectsEntity::insert(model).exec(pool).await?;
        }
    }
    Ok(ids)
}

async fn seed_tags(
    pool: &DatabaseTransaction,
    timestamp: PrimitiveDateTime,
) -> Result<Vec<DbId>, sea_orm::DbErr> {
    let tag_names = [
        "bestseller",
        "award-winner",
        "award-nominated",
        "series",
        "standalone",
        "translated",
        "based-on-true-events",
        "favourite",
        "to-re-read",
        "bookclub",
    ];
    let mut ids = Vec::new();
    for name in tag_names.iter() {
        let tag_id = DbId::new();
        let model = TagActiveModel {
            id: Set(tag_id),
            name: Set((*name).to_string()),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        TagsEntity::insert(model).exec(pool).await?;
        ids.push(tag_id);
    }
    Ok(ids)
}

async fn generate_works_and_editions(
    pool: &DatabaseTransaction,
    config: &SeedConfig,
    timestamp: PrimitiveDateTime,
    result: &mut SeedResult,
) -> Result<(Vec<entities::works::Model>, Vec<entities::editions::Model>), sea_orm::DbErr> {
    let mut works = Vec::new();
    let mut editions = Vec::new();

    let formats = KnownFormats::all();
    let num_formats = formats.len();
    let english_id = dbid_from_ulid(CommonLanguages::English.ulid());

    for _ in 0..config.num_works {
        let work_id = DbId::new();
        let title: String = fake::faker::lorem::en::Words(3..5)
            .fake::<Vec<String>>()
            .join(" ");
        let description: Option<String> = Some(fake::faker::lorem::en::Paragraph(1..3).fake());

        let work = WorkActiveModel {
            id: Set(work_id),
            title: Set(title.clone()),
            description: Set(description),
            sort_title: Set(None),
            series_type: Set(None),
            language_id: Set(Some(english_id)),
            preferred_edition_id: Set(None),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        WorksEntity::insert(work).exec(pool).await?;
        result.works_created += 1;
        works.push(entities::works::Model {
            id: work_id,
            title,
            description: None,
            sort_title: None,
            series_type: None,
            language_id: Some(english_id),
            preferred_edition_id: None,
            created_at: timestamp,
            updated_at: None,
        });

        let num_editions_for_work =
            rand::random_range(config.editions_per_work.0..=config.editions_per_work.1) as u32;

        for edition_idx in 0..num_editions_for_work {
            let edition_id = DbId::new();
            let edition_title: Option<String> = {
                let format_name = formats[(edition_idx as usize) % num_formats].name();
                Some(format!("{format_name} Edition"))
            };
            let published_date =
                (OffsetDateTime::from_unix_timestamp(SEED_TIMESTAMP_BASE_MS / 1000).unwrap()
                    - Duration::days(rand::random_range(30..1825)))
                .date();
            let format_id = dbid_from_ulid(formats[(edition_idx as usize) % num_formats].ulid());

            let edition = EditionActiveModel {
                id: Set(edition_id),
                work_id: Set(work_id),
                group_id: Set(None),
                title: Set(edition_title.clone()),
                published_date: Set(Some(published_date)),
                format_id: Set(Some(format_id)),
                language_id: Set(Some(english_id)),
                notes: Set(None),
                description: Set(None),
                created_at: Set(timestamp),
                updated_at: Set(None),
            };
            EditionsEntity::insert(edition).exec(pool).await?;
            result.editions_created += 1;

            editions.push(entities::editions::Model {
                id: edition_id,
                work_id,
                group_id: None,
                title: edition_title.clone(),
                published_date: Some(published_date),
                format_id: Some(format_id),
                language_id: Some(english_id),
                notes: None,
                description: None,
                created_at: timestamp,
                updated_at: None,
            });
        }
    }

    Ok((works, editions))
}

async fn seed_authors(
    pool: &DatabaseTransaction,
    num_works: u32,
    result: &mut SeedResult,
) -> Result<Vec<DbId>, sea_orm::DbErr> {
    let mut ids = Vec::new();
    let target_authors = (num_works as f64 * 1.5).ceil() as u32;

    for _ in 0..target_authors {
        let author_id = DbId::new();
        let name: String = fake::faker::name::en::Name().fake();
        let model = AuthorActiveModel {
            id: Set(author_id),
            name: Set(name),
        };
        AuthorsEntity::insert(model).exec(pool).await?;
        result.authors_created += 1;
        ids.push(author_id);
    }
    Ok(ids)
}

async fn seed_publishers(
    pool: &DatabaseTransaction,
    num_works: u32,
    timestamp: PrimitiveDateTime,
    result: &mut SeedResult,
) -> Result<Vec<DbId>, sea_orm::DbErr> {
    let mut ids = Vec::new();
    let target_publishers = (num_works as f64 * 0.4).ceil() as u32;

    for _ in 0..target_publishers.max(1) {
        let publisher_id = DbId::new();
        let name: String = fake::faker::company::en::CompanyName().fake();
        let model = PublisherActiveModel {
            id: Set(publisher_id),
            name: Set(name),
            website: Set(None),
            logo_url: Set(None),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        PublishersEntity::insert(model).exec(pool).await?;
        result.publishers_created += 1;
        ids.push(publisher_id);
    }
    Ok(ids)
}

async fn link_work_authors(
    pool: &DatabaseTransaction,
    work_ids: &[DbId],
    author_ids: &[DbId],
) -> Result<(), sea_orm::DbErr> {
    for work_id in work_ids {
        let count = rand::random_range(1u8..=3u8);
        let mut used_authors: Vec<DbId> = Vec::new();
        for _ in 0..count {
            if author_ids.is_empty() {
                break;
            }
            let author_id = author_ids[rand::random_range(0..author_ids.len())];
            if used_authors.contains(&author_id) {
                continue;
            }
            used_authors.push(author_id);
            let model = WorkAuthorActiveModel {
                work_id: Set(*work_id),
                author_id: Set(author_id),
                role: Set("author".to_string()),
            };
            WorkAuthorsEntity::insert(model).exec(pool).await?;
        }
    }
    Ok(())
}

async fn link_edition_authors(
    pool: &DatabaseTransaction,
    edition_ids: &[DbId],
    author_ids: &[DbId],
) -> Result<(), sea_orm::DbErr> {
    if author_ids.is_empty() {
        return Ok(());
    }
    for edition_id in edition_ids {
        let author_id = author_ids[rand::random_range(0..author_ids.len())];
        let model = EditionAuthorActiveModel {
            edition_id: Set(*edition_id),
            author_id: Set(author_id),
            role: Set("author".to_string()),
        };
        EditionAuthorsEntity::insert(model).exec(pool).await?;
    }
    Ok(())
}

async fn link_work_genres(
    pool: &DatabaseTransaction,
    work_ids: &[DbId],
) -> Result<(), sea_orm::DbErr> {
    let genres = KnownGenres::all();
    for work_id in work_ids {
        let count = rand::random_range(1usize..=3usize);
        let mut used = Vec::new();
        for _ in 0..count {
            let idx = rand::random_range(0..genres.len());
            if used.contains(&idx) {
                continue;
            }
            used.push(idx);
            let genre_id = dbid_from_ulid(genres[idx].ulid());
            let model = WorkGenreActiveModel {
                work_id: Set(*work_id),
                genre_id: Set(genre_id),
            };
            WorkGenresEntity::insert(model).exec(pool).await?;
        }
    }
    Ok(())
}

async fn link_work_subjects(
    pool: &DatabaseTransaction,
    work_ids: &[DbId],
) -> Result<(), sea_orm::DbErr> {
    let subjects = KnownSubjects::all();
    for work_id in work_ids {
        let count = rand::random_range(0usize..=3usize);
        let mut used = Vec::new();
        for _ in 0..count {
            let idx = rand::random_range(0..subjects.len());
            if used.contains(&idx) {
                continue;
            }
            used.push(idx);
            let subject_id = dbid_from_ulid(subjects[idx].ulid());
            let model = WorkSubjectActiveModel {
                work_id: Set(*work_id),
                subject_id: Set(subject_id),
            };
            WorkSubjectsEntity::insert(model).exec(pool).await?;
        }
    }
    Ok(())
}

async fn link_work_tags(
    pool: &DatabaseTransaction,
    work_ids: &[DbId],
) -> Result<(), sea_orm::DbErr> {
    let all_tags = TagsEntity::find().all(pool).await?;
    if all_tags.is_empty() {
        return Ok(());
    }
    for work_id in work_ids {
        let count = rand::random_range(0usize..=4usize);
        let mut used = Vec::new();
        for _ in 0..count {
            let idx = rand::random_range(0..all_tags.len());
            if used.contains(&idx) {
                continue;
            }
            used.push(idx);
            let tag_id = all_tags[idx].id;
            let model = WorkTagActiveModel {
                work_id: Set(*work_id),
                tag_id: Set(tag_id),
            };
            WorkTagsEntity::insert(model).exec(pool).await?;
        }
    }
    Ok(())
}

async fn link_edition_tags(
    pool: &DatabaseTransaction,
    edition_ids: &[DbId],
) -> Result<(), sea_orm::DbErr> {
    let all_tags = TagsEntity::find().all(pool).await?;
    if all_tags.is_empty() {
        return Ok(());
    }
    for edition_id in edition_ids {
        if rand::random_range(0u8..=10u8) > 3 {
            continue;
        }
        let tag_id = all_tags[rand::random_range(0..all_tags.len())].id;
        let model = EditionTagActiveModel {
            edition_id: Set(*edition_id),
            tag_id: Set(tag_id),
        };
        EditionTagsEntity::insert(model).exec(pool).await?;
    }
    Ok(())
}

async fn link_work_publishers(
    pool: &DatabaseTransaction,
    work_ids: &[DbId],
    publisher_ids: &[DbId],
) -> Result<(), sea_orm::DbErr> {
    if publisher_ids.is_empty() {
        return Ok(());
    }
    for work_id in work_ids {
        let publisher_id = publisher_ids[rand::random_range(0..publisher_ids.len())];
        let model = WorkPublisherActiveModel {
            work_id: Set(*work_id),
            publisher_id: Set(publisher_id),
        };
        WorkPublishersEntity::insert(model).exec(pool).await?;
    }
    Ok(())
}

async fn link_edition_publishers(
    pool: &DatabaseTransaction,
    edition_ids: &[DbId],
    publisher_ids: &[DbId],
) -> Result<(), sea_orm::DbErr> {
    if publisher_ids.is_empty() {
        return Ok(());
    }
    for edition_id in edition_ids {
        let publisher_id = publisher_ids[rand::random_range(0..publisher_ids.len())];
        let model = EditionPublisherActiveModel {
            edition_id: Set(*edition_id),
            publisher_id: Set(publisher_id),
        };
        EditionPublishersEntity::insert(model).exec(pool).await?;
    }
    Ok(())
}

async fn add_identifiers(
    pool: &DatabaseTransaction,
    edition_ids: &[DbId],
    work_ids: &[DbId],
) -> Result<(), sea_orm::DbErr> {
    for edition_id in edition_ids {
        let isbn_value = format!("urn:isbn:{}", rand::random_range(10u64..999_999_999u64));
        let identifier_id = DbId::new();
        let identifier = IdentifierActiveModel {
            id: Set(identifier_id),
            value: Set(isbn_value),
            kind: Set("isbn".to_string()),
        };
        IdentifiersEntity::insert(identifier).exec(pool).await?;

        let link = EditionIdentifierActiveModel {
            edition_id: Set(*edition_id),
            identifier_id: Set(identifier_id),
        };
        EditionIdentifiersEntity::insert(link).exec(pool).await?;
    }

    for work_id in work_ids {
        let wikidata_value = format!("urn:wikidata:Q{}", rand::random_range(1u64..100_000_000u64));
        let identifier_id = DbId::new();
        let identifier = IdentifierActiveModel {
            id: Set(identifier_id),
            value: Set(wikidata_value),
            kind: Set("wikidata".to_string()),
        };
        IdentifiersEntity::insert(identifier).exec(pool).await?;

        let link = WorkIdentifierActiveModel {
            work_id: Set(*work_id),
            identifier_id: Set(identifier_id),
        };
        WorkIdentifiersEntity::insert(link).exec(pool).await?;
    }
    Ok(())
}

async fn add_saved_searches(
    pool: &DatabaseTransaction,
    timestamp: PrimitiveDateTime,
    result: &mut SeedResult,
) -> Result<(), sea_orm::DbErr> {
    let searches = [
        ("Sci-Fi", r#"{"genre": "science fiction"}"#),
        ("Award Winners", r#"{"tag": "award-winner"}"#),
        ("Favourite Authors", r#"{"author": "favourite"}"#),
        ("Short Stories", r#"{"genre": "short stories"}"#),
        (
            "African American Lit",
            r#"{"genre": "african american literature"}"#,
        ),
    ];
    for (name, definition) in searches.iter() {
        let model = SavedSearchActiveModel {
            id: Set(DbId::new()),
            name: Set((*name).to_string()),
            definition_json: Set((*definition).to_string()),
            bindings_json: Set(None),
            options_json: Set(None),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        SavedSearchesEntity::insert(model).exec(pool).await?;
        result.saved_searches_created += 1;
    }
    Ok(())
}

#[derive(Clone)]
#[allow(dead_code)]
struct WorkWithStatus {
    work_id: DbId,
    primary_edition_id: DbId,
    format_id: DbId,
    status: WorkStatus,
    progress: f64,
}

async fn assign_reading_statuses(
    pool: &DatabaseTransaction,
    work_ids: &[DbId],
    work_edition_map: &HashMap<DbId, Vec<DbId>>,
    config: &SeedConfig,
    timestamp: PrimitiveDateTime,
) -> Result<(Vec<WorkWithStatus>, u32), sea_orm::DbErr> {
    let mut result = Vec::new();
    let formats = KnownFormats::all();

    let mut work_ids_shuffled = work_ids.to_vec();
    use rand::seq::SliceRandom;
    work_ids_shuffled.shuffle(&mut rand::rng());

    let actively_reading_target =
        ((work_ids.len() as f64) * config.actively_reading_pct).ceil() as usize + 4;
    let actively_reading_count = actively_reading_target.min(work_ids.len());

    let remaining_after_active: Vec<DbId> = work_ids_shuffled
        .iter()
        .skip(actively_reading_count)
        .copied()
        .collect();
    let dnf_target =
        ((remaining_after_active.len() as f64) * config.did_not_finish_pct).ceil() as usize;
    let dnf_count = dnf_target.min(remaining_after_active.len());

    for work_id in work_ids_shuffled.iter().take(actively_reading_count) {
        let editions = match work_edition_map.get(work_id) {
            Some(es) if !es.is_empty() => es,
            _ => continue,
        };
        let primary_edition_id = editions[0];
        let format_id = dbid_from_ulid(formats[rand::random_range(0..formats.len())].ulid());

        let progress = rand::random_range(0.30..0.85);

        let status_model = CurrentWorkStatusActiveModel {
            work_id: Set(*work_id),
            status: Set(WorkStatus::Reading),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        CurrentWorkStatusEntity::insert(status_model)
            .exec(pool)
            .await?;

        let progress_id = DbId::new();
        let progress_model = ReadingProgressActiveModel {
            id: Set(progress_id),
            edition_id: Set(primary_edition_id),
            format_id: Set(format_id),
            progress: Set(progress),
            progress_unit: Set(Some("ratio".to_string())),
            last_location: Set(Some(format!("page_{}", rand::random_range(50u64..500u64)))),
            total_reading_time_secs: Set(rand::random_range(600i64..7200i64)),
            created_at: Set(timestamp),
        };
        ReadingProgressEntity::insert(progress_model)
            .exec(pool)
            .await?;

        result.push(WorkWithStatus {
            work_id: *work_id,
            primary_edition_id,
            format_id,
            status: WorkStatus::Reading,
            progress,
        });
    }

    for work_id in remaining_after_active.iter().take(dnf_count) {
        let editions = match work_edition_map.get(work_id) {
            Some(es) if !es.is_empty() => es,
            _ => continue,
        };
        let primary_edition_id = editions[0];
        let format_id = dbid_from_ulid(formats[rand::random_range(0..formats.len())].ulid());

        let progress = rand::random_range(0.05..0.80);

        let status_model = CurrentWorkStatusActiveModel {
            work_id: Set(*work_id),
            status: Set(WorkStatus::Abandoned),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        CurrentWorkStatusEntity::insert(status_model)
            .exec(pool)
            .await?;

        let progress_id = DbId::new();
        let progress_model = ReadingProgressActiveModel {
            id: Set(progress_id),
            edition_id: Set(primary_edition_id),
            format_id: Set(format_id),
            progress: Set(progress),
            progress_unit: Set(Some("ratio".to_string())),
            last_location: Set(Some(format!("page_{}", rand::random_range(10u64..300u64)))),
            total_reading_time_secs: Set(rand::random_range(0i64..3600i64)),
            created_at: Set(timestamp),
        };
        ReadingProgressEntity::insert(progress_model)
            .exec(pool)
            .await?;

        result.push(WorkWithStatus {
            work_id: *work_id,
            primary_edition_id,
            format_id,
            status: WorkStatus::Abandoned,
            progress,
        });
    }

    let remaining_for_finished: Vec<DbId> = remaining_after_active
        .iter()
        .skip(dnf_count)
        .copied()
        .collect();
    for work_id in remaining_for_finished {
        let editions = match work_edition_map.get(&work_id) {
            Some(es) if !es.is_empty() => es,
            _ => continue,
        };
        let primary_edition_id = editions[0];
        let format_id = dbid_from_ulid(formats[rand::random_range(0..formats.len())].ulid());

        let status_model = CurrentWorkStatusActiveModel {
            work_id: Set(work_id),
            status: Set(WorkStatus::Finished),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        CurrentWorkStatusEntity::insert(status_model)
            .exec(pool)
            .await?;

        let progress_id = DbId::new();
        let progress_model = ReadingProgressActiveModel {
            id: Set(progress_id),
            edition_id: Set(primary_edition_id),
            format_id: Set(format_id),
            progress: Set(1.0),
            progress_unit: Set(Some("ratio".to_string())),
            last_location: Set(Some("end".to_string())),
            total_reading_time_secs: Set(rand::random_range(3600i64..14400i64)),
            created_at: Set(timestamp),
        };
        ReadingProgressEntity::insert(progress_model)
            .exec(pool)
            .await?;

        result.push(WorkWithStatus {
            work_id,
            primary_edition_id,
            format_id,
            status: WorkStatus::Finished,
            progress: 1.0,
        });
    }

    Ok((result.clone(), result.len() as u32))
}

async fn add_digital_inventory(
    pool: &DatabaseTransaction,
    work_with_status: &[WorkWithStatus],
    config: &SeedConfig,
    timestamp: PrimitiveDateTime,
    result: &mut SeedResult,
) -> Result<(), sea_orm::DbErr> {
    for ws in work_with_status {
        if rand::random_range(0.0f64..1.0f64) > config.digital_inventory_pct {
            continue;
        }
        let inventory_id = DbId::new();
        let file_path = format!("/fake/library/book_{}.epub", inventory_id);
        let cover_path = format!("/fake/library/cover_{}.jpg", inventory_id);
        let file_hash = format!("blake3:{}", rand::random_range(0u64..u64::MAX));
        let model = DigitalInventoryActiveModel {
            id: Set(inventory_id),
            edition_id: Set(ws.primary_edition_id),
            file_path: Set(Some(file_path)),
            cover_path: Set(Some(cover_path)),
            blurhash: Set(None),
            dominant_color: Set(None),
            file_hash: Set(Some(file_hash)),
            file_size_bytes: Set(Some(rand::random_range(100_000i64..10_000_000i64))),
            file_format: Set(Some("EPUB".into())),
            notes: Set(None),
            added_at: Set(timestamp),
            updated_at: Set(None),
        };
        DigitalInventoryEntity::insert(model).exec(pool).await?;
        result.digital_inventory_created += 1;
    }
    Ok(())
}

async fn add_annotations(
    pool: &DatabaseTransaction,
    work_with_status: &[WorkWithStatus],
    config: &SeedConfig,
    timestamp: PrimitiveDateTime,
    result: &mut SeedResult,
) -> Result<(), sea_orm::DbErr> {
    for ws in work_with_status {
        if rand::random_range(0.0f64..1.0f64) > config.annotation_pct {
            continue;
        }
        let annotation_id = DbId::new();
        let content: String = fake::faker::lorem::en::Sentence(8..15).fake();
        let model = AnnotationActiveModel {
            id: Set(annotation_id),
            edition_id: Set(ws.primary_edition_id),
            user_id: Set(DbId::new()),
            content: Set(content),
            location: Set(Some("0002".to_string())),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        AnnotationsEntity::insert(model).exec(pool).await?;
        result.annotations_created += 1;
    }
    Ok(())
}

async fn add_edition_groups(
    pool: &DatabaseTransaction,
    work_edition_map: &HashMap<DbId, Vec<DbId>>,
    config: &SeedConfig,
    timestamp: PrimitiveDateTime,
) -> Result<(), sea_orm::DbErr> {
    let total_editions: usize = work_edition_map.values().map(|v| v.len()).sum();
    let target_count = ((total_editions as f64) * config.edition_group_pct).ceil() as usize;

    if target_count == 0 {
        return Ok(());
    }

    let mut all_editions: Vec<DbId> = work_edition_map
        .values()
        .flat_map(|v| v.iter().copied())
        .collect();
    use rand::seq::SliceRandom;
    all_editions.shuffle(&mut rand::rng());
    let selected: Vec<DbId> = all_editions.into_iter().take(target_count).collect();

    let group_size = 3usize;
    for chunk in selected.chunks(group_size) {
        let group_id = DbId::new();
        let label: String = fake::faker::lorem::en::Words(2..4)
            .fake::<Vec<String>>()
            .join(" ");
        let group_model = EditionGroupActiveModel {
            id: Set(group_id),
            label: Set(label),
            description: Set(None),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        EditionGroupsEntity::insert(group_model).exec(pool).await?;

        for edition_id in chunk {
            use entities::editions::ActiveModel as EditionUpdateActiveModel;
            let mut edition_update = <EditionUpdateActiveModel as std::default::Default>::default();
            edition_update.id = Set(*edition_id);
            edition_update.group_id = Set(Some(group_id));
            let _ = edition_update.update(pool).await;
        }
    }
    Ok(())
}

async fn add_loans(
    pool: &DatabaseTransaction,
    edition_ids: &[DbId],
    config: &SeedConfig,
    timestamp: PrimitiveDateTime,
    result: &mut SeedResult,
) -> Result<(Vec<DbId>, u32), sea_orm::DbErr> {
    let mut loan_entity_ids = Vec::new();
    let target_count = ((edition_ids.len() as f64) * config.loan_pct).ceil() as usize;
    if target_count == 0 {
        return Ok((loan_entity_ids, 0));
    }

    let mut loan_entities: Vec<DbId> = Vec::new();
    for _ in 0..target_count.max(1) {
        let entity_id = DbId::new();
        let name: String = fake::faker::name::en::Name().fake();
        let model = LoanEntityActiveModel {
            id: Set(entity_id),
            name: Set(name),
            notes: Set(None),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        LoanEntitiesEntity::insert(model).exec(pool).await?;
        loan_entities.push(entity_id);
        loan_entity_ids.push(entity_id);

        let link_id = DbId::new();
        let url: String = format!("urn:loan:{}", rand::random_range(1000u64..9999u64));
        let link_model = LoanEntityIdentifierActiveModel {
            id: Set(link_id),
            loan_entity_id: Set(entity_id),
            url: Set(url),
            label: Set(None),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        LoanEntityIdentifiersEntity::insert(link_model)
            .exec(pool)
            .await?;
    }

    let mut loans_created = 0u32;
    for (idx, edition_id) in edition_ids.iter().enumerate() {
        if idx >= target_count {
            break;
        }
        let loan_entity_id = loan_entities[idx % loan_entities.len()];
        let loan_id = DbId::new();
        let loaned_date = (OffsetDateTime::from_unix_timestamp(SEED_TIMESTAMP_BASE_MS / 1000)
            .unwrap()
            - Duration::days(7))
        .date();
        let due_date = Some(
            (OffsetDateTime::from_unix_timestamp(SEED_TIMESTAMP_BASE_MS / 1000).unwrap()
                + Duration::days(14))
            .date(),
        );
        let model = EditionsLoansActiveModel {
            id: Set(loan_id),
            edition_id: Set(*edition_id),
            loan_entity_id: Set(loan_entity_id),
            owned_edition_id: Set(None),
            loaned_date: Set(loaned_date),
            due_date: Set(due_date),
            returned_date: Set(None),
        };
        EditionsLoansEntity::insert(model).exec(pool).await?;
        loans_created += 1;
        result.loans_created += 1;
    }

    Ok((loan_entity_ids, loans_created))
}

async fn add_owned_editions(
    pool: &DatabaseTransaction,
    work_with_status: &[WorkWithStatus],
    config: &SeedConfig,
    timestamp: PrimitiveDateTime,
) -> Result<(), sea_orm::DbErr> {
    for ws in work_with_status {
        if rand::random_range(0.0f64..1.0f64) > config.digital_inventory_pct {
            continue;
        }
        let model = OwnedEditionActiveModel {
            id: Set(DbId::new()),
            edition_id: Set(ws.primary_edition_id),
            acquired_at: Set(Some(
                (OffsetDateTime::from_unix_timestamp(SEED_TIMESTAMP_BASE_MS / 1000).unwrap()
                    - Duration::days(60))
                .date(),
            )),
            condition_id: Set(None),
            notes: Set(None),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        OwnedEditionsEntity::insert(model).exec(pool).await?;
    }
    Ok(())
}

async fn add_reading_sessions(
    pool: &DatabaseTransaction,
    work_with_status: &[WorkWithStatus],
    config: &SeedConfig,
    timestamp: PrimitiveDateTime,
    result: &mut SeedResult,
) -> Result<(), sea_orm::DbErr> {
    let _ = config;
    let sources = KnownReadingSources::all();
    let source_ids: Vec<DbId> = sources.iter().map(|s| dbid_from_ulid(s.ulid())).collect();

    for source in sources.iter() {
        let model = ReadingSourceActiveModel {
            id: Set(dbid_from_ulid(source.ulid())),
            urn: Set(source.urn()),
            name: Set(source.name().to_string()),
            emoji: Set(Some(source.emoji().to_string())),
            color: Set(Some(source.color().to_string())),
            attributes: Set(Some(serde_json::Value::Null)),
            plugin_id: Set(None),
            deleted_at: Set(None),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        ReadingSourcesEntity::insert(model).exec(pool).await?;
    }

    for ws in work_with_status {
        if ws.status != WorkStatus::Reading {
            continue;
        }
        let session_count = rand::random_range(1u8..=5u8);

        let source_id = source_ids[rand::random_range(0..source_ids.len())];

        for session_idx in 0..session_count {
            let session_id = DbId::new();
            let started_at = primitive_now()
                .assume_utc()
                .saturating_sub(Duration::days((session_idx as i64 + 1) * 2));
            let duration_secs = rand::random_range(600i64..3600i64);
            let ended_at = Some(started_at + Duration::seconds(duration_secs));
            let progress_delta = ws.progress / session_count as f64;

            let model = ReadingSessionActiveModel {
                id: Set(session_id),
                edition_id: Set(ws.primary_edition_id),
                format_id: Set(ws.format_id),
                source_id: Set(Some(source_id)),
                started_at: Set(PrimitiveDateTime::new(started_at.date(), started_at.time())),
                ended_at: Set(ended_at.map(|t| PrimitiveDateTime::new(t.date(), t.time()))),
                duration_seconds: Set(Some(duration_secs)),
                raw_progression: Set(None),
                progress_delta: Set(progress_delta),
                last_location: Set(Some(format!("page_{}", session_idx * 50 + 50))),
                notes: Set(None),
                created_at: Set(timestamp),
                updated_at: Set(None),
            };
            ReadingSessionsEntity::insert(model).exec(pool).await?;
            result.reading_sessions_created += 1;
        }
    }
    Ok(())
}

async fn add_reading_lists(
    pool: &DatabaseTransaction,
    work_edition_map: &HashMap<DbId, Vec<DbId>>,
    timestamp: PrimitiveDateTime,
    result: &mut SeedResult,
) -> Result<(), sea_orm::DbErr> {
    let lists = [
        ("Currently Reading", "Books I'm reading right now"),
        ("To Read", "My queue"),
        ("Favourites", "Books I loved"),
    ];
    let all_editions: Vec<DbId> = work_edition_map
        .values()
        .flat_map(|v| v.iter().copied())
        .collect();

    for (idx, (name, desc)) in lists.iter().enumerate() {
        let list_id = DbId::new();
        let model = ReadingListActiveModel {
            id: Set(list_id),
            name: Set((*name).to_string()),
            description: Set(Some((*desc).to_string())),
            created_at: Set(timestamp),
            updated_at: Set(None),
        };
        ReadingListsEntity::insert(model).exec(pool).await?;
        result.reading_lists_created += 1;

        if idx == 0 && !all_editions.is_empty() {
            let take = all_editions.len().min(3);
            for (pos, edition_id) in all_editions.iter().take(take).enumerate() {
                let link = ReadingListBookActiveModel {
                    reading_list_id: Set(list_id),
                    edition_id: Set(*edition_id),
                    position: Set(pos as i32),
                    added_at: Set(timestamp),
                };
                ReadingListBookEntity::insert(link).exec(pool).await?;
            }
        }
    }
    Ok(())
}

async fn add_series(
    pool: &DatabaseTransaction,
    work_edition_map: &HashMap<DbId, Vec<DbId>>,
    timestamp: PrimitiveDateTime,
) -> Result<(), sea_orm::DbErr> {
    let works_with_multi_editions: Vec<(DbId, Vec<DbId>)> = work_edition_map
        .iter()
        .filter(|(_, editions)| editions.len() > 1)
        .map(|(k, v)| (*k, v.clone()))
        .collect();

    if works_with_multi_editions.len() < 2 {
        return Ok(());
    }

    let series_id = DbId::new();
    let series_name: String = fake::faker::lorem::en::Words(2..4)
        .fake::<Vec<String>>()
        .join(" ");
    let series = SeriesActiveModel {
        id: Set(series_id),
        name: Set(series_name),
        sort_title: Set(None),
        series_type: Set(Some("sequential".to_string())),
        created_at: Set(timestamp),
        updated_at: Set(None),
    };
    SeriesEntity::insert(series).exec(pool).await?;

    for (pos, (_work_id, editions)) in works_with_multi_editions.iter().take(3).enumerate() {
        if let Some(edition_id) = editions.first() {
            let entry = SeriesEntryActiveModel {
                series_id: Set(series_id),
                edition_id: Set(*edition_id),
                position: Set(pos as i32 + 1),
                created_at: Set(timestamp),
            };
            SeriesEntriesEntity::insert(entry).exec(pool).await?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
trait PrimitiveDateTimeExt {
    fn assume_utc(self) -> OffsetDateTime;
    fn saturating_sub(self, duration: Duration) -> OffsetDateTime;
}

impl PrimitiveDateTimeExt for PrimitiveDateTime {
    fn assume_utc(self) -> OffsetDateTime {
        OffsetDateTime::new_utc(self.date(), self.time())
    }
    fn saturating_sub(self, duration: Duration) -> OffsetDateTime {
        self.assume_utc() - duration
    }
}
