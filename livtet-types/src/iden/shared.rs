use strum::{AsRefStr, IntoStaticStr};

// Core vocabulary tables

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Authors {
    #[strum(serialize = "authors")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Tags {
    #[strum(serialize = "tags")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Genres {
    #[strum(serialize = "genres")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Subjects {
    #[strum(serialize = "subjects")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Publishers {
    #[strum(serialize = "publishers")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
    #[strum(serialize = "website")]
    Website,
    #[strum(serialize = "logo_url")]
    LogoUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Series {
    #[strum(serialize = "series")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
    #[strum(serialize = "sort_title")]
    SortTitle,
    #[strum(serialize = "series_type")]
    SeriesType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Formats {
    #[strum(serialize = "formats")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
    #[strum(serialize = "metadata_schema")]
    MetadataSchema,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Languages {
    #[strum(serialize = "languages")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
    #[strum(serialize = "code")]
    Code,
    #[strum(serialize = "flag_emoji")]
    FlagEmoji,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Identifiers {
    #[strum(serialize = "identifiers")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "value")]
    Value,
    #[strum(serialize = "kind")]
    Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum BookConditions {
    #[strum(serialize = "book_conditions")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
    #[strum(serialize = "value")]
    Value,
}

// ── FRBR core tables ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Works {
    #[strum(serialize = "works")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "title")]
    Title,
    #[strum(serialize = "description")]
    Description,
    #[strum(serialize = "sort_title")]
    SortTitle,
    #[strum(serialize = "series_type")]
    SeriesType,
    #[strum(serialize = "language_id")]
    LanguageId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Editions {
    #[strum(serialize = "editions")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "work_id")]
    WorkId,
    #[strum(serialize = "group_id")]
    GroupId,
    #[strum(serialize = "title")]
    Title,
    #[strum(serialize = "published_date")]
    PublishedDate,
    #[strum(serialize = "format_id")]
    FormatId,
    #[strum(serialize = "language_id")]
    LanguageId,
    #[strum(serialize = "notes")]
    Notes,
    #[strum(serialize = "description")]
    Description,
}

// ── Junction tables ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum WorkAuthors {
    #[strum(serialize = "work_authors")]
    Table,
    #[strum(serialize = "work_id")]
    WorkId,
    #[strum(serialize = "author_id")]
    AuthorId,
    #[strum(serialize = "role")]
    Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum WorkTags {
    #[strum(serialize = "work_tags")]
    Table,
    #[strum(serialize = "work_id")]
    WorkId,
    #[strum(serialize = "tag_id")]
    TagId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum WorkGenres {
    #[strum(serialize = "work_genres")]
    Table,
    #[strum(serialize = "work_id")]
    WorkId,
    #[strum(serialize = "genre_id")]
    GenreId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum WorkSubjects {
    #[strum(serialize = "work_subjects")]
    Table,
    #[strum(serialize = "work_id")]
    WorkId,
    #[strum(serialize = "subject_id")]
    SubjectId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum WorkPublishers {
    #[strum(serialize = "work_publishers")]
    Table,
    #[strum(serialize = "work_id")]
    WorkId,
    #[strum(serialize = "publisher_id")]
    PublisherId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum WorkIdentifiers {
    #[strum(serialize = "work_identifiers")]
    Table,
    #[strum(serialize = "work_id")]
    WorkId,
    #[strum(serialize = "identifier_id")]
    IdentifierId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionAuthors {
    #[strum(serialize = "edition_authors")]
    Table,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "author_id")]
    AuthorId,
    #[strum(serialize = "role")]
    Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionTags {
    #[strum(serialize = "edition_tags")]
    Table,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "tag_id")]
    TagId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionGenres {
    #[strum(serialize = "edition_genres")]
    Table,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "genre_id")]
    GenreId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionSubjects {
    #[strum(serialize = "edition_subjects")]
    Table,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "subject_id")]
    SubjectId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionPublishers {
    #[strum(serialize = "edition_publishers")]
    Table,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "publisher_id")]
    PublisherId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionIdentifiers {
    #[strum(serialize = "edition_identifiers")]
    Table,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "identifier_id")]
    IdentifierId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum SeriesEntries {
    #[strum(serialize = "series_entries")]
    Table,
    #[strum(serialize = "series_id")]
    SeriesId,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "position")]
    Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionGroups {
    #[strum(serialize = "edition_groups")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "label")]
    Label,
    #[strum(serialize = "description")]
    Description,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionGroupIdentifiers {
    #[strum(serialize = "edition_group_identifiers")]
    Table,
    #[strum(serialize = "edition_group_id")]
    EditionGroupId,
    #[strum(serialize = "identifier_kind")]
    IdentifierKind,
    #[strum(serialize = "identifier_value")]
    IdentifierValue,
}

// ── Inventory and loans tables ────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum OwnedEditions {
    #[strum(serialize = "owned_editions")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "acquired_at")]
    AcquiredAt,
    #[strum(serialize = "condition_id")]
    ConditionId,
    #[strum(serialize = "notes")]
    Notes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum LoanEntity {
    #[strum(serialize = "loan_entity")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
    #[strum(serialize = "notes")]
    Notes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum LoanEntityIdentifiers {
    #[strum(serialize = "loan_entity_identifiers")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "loan_entity_id")]
    LoanEntityId,
    #[strum(serialize = "url")]
    Url,
    #[strum(serialize = "label")]
    Label,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionsLoans {
    #[strum(serialize = "editions_loans")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "loan_entity_id")]
    LoanEntityId,
    #[strum(serialize = "owned_edition_id")]
    OwnedEditionId,
    #[strum(serialize = "loaned_date")]
    LoanedDate,
    #[strum(serialize = "due_date")]
    DueDate,
    #[strum(serialize = "returned_date")]
    ReturnedDate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum DigitalInventory {
    #[strum(serialize = "digital_inventory")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "file_path")]
    FilePath,
    #[strum(serialize = "cover_path")]
    CoverPath,
    #[strum(serialize = "blurhash")]
    Blurhash,
    #[strum(serialize = "dominant_color")]
    DominantColor,
    #[strum(serialize = "file_hash")]
    FileHash,
    #[strum(serialize = "file_size_bytes")]
    FileSizeBytes,
    #[strum(serialize = "notes")]
    Notes,
    #[strum(serialize = "added_at")]
    AddedAt,
    #[strum(serialize = "updated_at")]
    UpdatedAt,
}

// ── Reading and annotations tables ────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum Annotations {
    #[strum(serialize = "annotations")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "user_id")]
    UserId,
    #[strum(serialize = "content")]
    Content,
    #[strum(serialize = "location")]
    Location,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum ReadingLists {
    #[strum(serialize = "reading_lists")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "name")]
    Name,
    #[strum(serialize = "description")]
    Description,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum ReadingListBook {
    #[strum(serialize = "reading_list_books")]
    Table,
    #[strum(serialize = "reading_list_id")]
    ReadingListId,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "position")]
    Position,
    #[strum(serialize = "added_at")]
    AddedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum ReadingProgress {
    #[strum(serialize = "reading_progress")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "format_id")]
    FormatId,
    #[strum(serialize = "progress")]
    Progress,
    #[strum(serialize = "progress_unit")]
    ProgressUnit,
    #[strum(serialize = "last_location")]
    LastLocation,
    #[strum(serialize = "total_reading_time_secs")]
    TotalReadingTimeSecs,
    #[strum(serialize = "created_at")]
    CreatedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum WorkStatus {
    #[strum(serialize = "work_status")]
    Table,
    #[strum(serialize = "work_id")]
    WorkId,
    #[strum(serialize = "status")]
    Status,
    #[strum(serialize = "created_at")]
    CreatedAt,
    #[strum(serialize = "updated_at")]
    UpdatedAt,
}

// ── Plugins tables ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum InstalledPlugins {
    #[strum(serialize = "installed_plugins")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "plugin_id")]
    PluginId,
    #[strum(serialize = "name")]
    Name,
    #[strum(serialize = "version")]
    Version,
    #[strum(serialize = "description")]
    Description,
    #[strum(serialize = "enabled")]
    Enabled,
    #[strum(serialize = "manifest_json")]
    ManifestJson,
    #[strum(serialize = "source_path")]
    SourcePath,
    #[strum(serialize = "installed_at")]
    InstalledAt,
}

// ── Edition plugin metadata ───────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum EditionPluginMetadata {
    #[strum(serialize = "edition_plugin_metadata")]
    Table,
    #[strum(serialize = "id")]
    Id,
    #[strum(serialize = "edition_id")]
    EditionId,
    #[strum(serialize = "plugin_id")]
    PluginId,
    #[strum(serialize = "key")]
    Key,
    #[strum(serialize = "value")]
    Value,
    #[strum(serialize = "created_at")]
    CreatedAt,
    #[strum(serialize = "updated_at")]
    UpdatedAt,
}
