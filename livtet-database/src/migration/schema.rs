//! Project-specific schema helpers.
//!
//! Re-exports all helpers from `sea_orm_migration::schema` and adds
//! `DbId`-aware column builders.

// Re-export all upstream schema helpers so migrations only need one import.
use ::sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;
pub use sea_orm_migration::schema::*;

/// Create a table with SQLite's `STRICT` mode applied.
///
/// SeaORM's `TableCreateStatement` has no `.strict()` method, so we
/// render the statement to SQL via `sea_query::SqliteQueryBuilder`,
/// translate the type names to ones STRICT accepts, append `STRICT`,
/// and execute as a raw `CREATE TABLE`.
///
/// # Why we translate types
///
/// SQLite STRICT tables allow ONLY these column types:
/// `INT`, `INTEGER`, `REAL`, `TEXT`, `BLOB`, `ANY`.  Any other type
/// name (including `VARCHAR(255)`, `BLOB(16)`, `BIGINT`, `BOOLEAN`,
/// `TIMESTAMP_TEXT`) is rejected with `unknown datatype for ...`.
///
/// SeaORM's `SqliteQueryBuilder` emits:
///   - `string(...)` → `varchar(255)`
///   - `text(...)` → `text`
///   - `integer(...)` → `integer`
///   - `big_integer(...)` → `bigint`
///   - `float(...)` / `double(...)` → `real` / `double precision`
///   - `date(...)` / `timestamp(...)` → `date_text` / `timestamp_text`
///   - `boolean(...)` → `boolean`
///   - `json(...)` → `json_text`
///   - `binary_len(16)` (via `db_id()`) → `blob(16)`
///
/// `normalize_for_strict` maps each of those to a STRICT-legal
/// equivalent.  Length specifiers (`(16)`, `(255)`) are dropped
/// because STRICT doesn't understand them, and SQLite ignores them
/// at runtime anyway — length is enforced at the application layer.
///
/// In STRICT mode, columns refuse values that don't match their
/// declared affinity (e.g. an `INTEGER` column rejects `'Way too long'`).
/// This catches a whole class of bugs where application code accidentally
/// stores the wrong type in a column.
///
/// SQLite `STRICT` tables were added in SQLite 3.37.0.  The mise-pinned
/// SQLite used by this project is 3.51.3, well above that floor.
pub async fn create_strict_table(
    manager: &SchemaManager<'_>,
    table: &TableCreateStatement,
) -> Result<(), DbErr> {
    let sql = table.to_string(SqliteQueryBuilder);
    let sql = normalize_for_strict(&sql);
    let sql = sql.trim_end_matches(';');
    let strict_sql = format!("{sql} STRICT");
    manager
        .get_connection()
        .execute_unprepared(&strict_sql)
        .await
        .map(|_| ())
}

/// Translate SeaORM-emitted SQLite type names to the subset that
/// `STRICT` tables accept (`INT`, `INTEGER`, `REAL`, `TEXT`, `BLOB`,
/// `ANY`).  Length specifiers (`(NN)`) are dropped — `STRICT` doesn't
/// parse them, and SQLite ignores them on regular tables too.
///
/// Replacement order matters: longer / more specific substrings
/// must run first so a `BOOLEAN` pass doesn't accidentally corrupt
/// anything.  In practice each replacement here is unambiguous.
fn normalize_for_strict(sql: &str) -> String {
    // Rewrite `<TYPE>(NN)` to `<TYPE>` — drop the length specifier.
    // Walk the string linearly; only treat `(...)` that contain digits
    // as length-bearing.  This preserves CHECK constraints, default
    // expressions inside `(...)`, and anything else.
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '(' {
            // Look ahead: if from here to the matching `)` is only
            // ASCII digits (and optional whitespace), it's a length,
            // and we drop both `(` and the digits and `)`.
            if let Some(rel_end) = looks_like_length(bytes, i) {
                i += rel_end;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }

    let mut out = out
        // JSON  → TEXT (STRICT has no JSON type)
        .replace("json_text", "TEXT")
        .replace("JSON_TEXT", "TEXT")
        // TIMESTAMP_TEXT / DATE_TEXT / TIME_TEXT  → TEXT
        .replace("timestamp_text", "TEXT")
        .replace("TIMESTAMP_TEXT", "TEXT")
        .replace("date_text", "TEXT")
        .replace("DATE_TEXT", "TEXT")
        .replace("time_text", "TEXT")
        .replace("TIME_TEXT", "TEXT")
        // VARCHAR  → TEXT
        .replace("varchar", "TEXT")
        .replace("VARCHAR", "TEXT")
        // CHAR(  → TEXT(
        .replace("CHAR(", "TEXT(")
        // BIGINT, SMALLINT, TINYINT  → INTEGER
        .replace("bigint", "INTEGER")
        .replace("BIGINT", "INTEGER")
        .replace("smallint", "INTEGER")
        .replace("SMALLINT", "INTEGER")
        .replace("tinyint", "INTEGER")
        .replace("TINYINT", "INTEGER")
        // DOUBLE PRECISION, FLOAT, DOUBLE  → REAL
        .replace("double precision", "REAL")
        .replace("DOUBLE PRECISION", "REAL")
        .replace("float", "REAL")
        .replace("FLOAT", "REAL")
        .replace("double", "REAL")
        .replace("DOUBLE", "REAL")
        // BOOLEAN  → INTEGER (SQLite has no BOOLEAN; STRICT forbids it)
        .replace("boolean", "INTEGER")
        .replace("BOOLEAN", "INTEGER")
        // BINARY(  → BLOB( (drop length)
        .replace("binary(", "BLOB(")
        .replace("BINARY(", "BLOB(");

    // Final collapse: `BLOB(<digits>)` and `TEXT(<digits>)` (from CHAR(N))
    // still have lengths.  The length-stripping loop above already
    // handled them, but `CHAR(` was rewritten to `TEXT(` so the length
    // is still there to be stripped.  Run the length-strip loop once
    // more to clean those up.
    strip_length_specifiers(&mut out);
    out
}

/// If `start` points at `(` and the substring up to and including the
/// matching `)` consists solely of whitespace and ASCII digits, return
/// `Some(distance)`: the number of bytes from `start` to *just past*
/// the closing `)`.  Otherwise return `None`.
fn looks_like_length(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b')' {
            // We've consumed everything from start to i inclusive.
            return Some(i - start + 1);
        }
        if !c.is_ascii_digit() && !c.is_ascii_whitespace() {
            return None;
        }
        i += 1;
    }
    // Unmatched `(`; not a length.
    None
}

/// Re-scan a string and remove `(NN)` length specifiers that follow
/// known STRICT-allowed type names.  Length specifiers elsewhere
/// (default expressions, CHECK constraints) are left alone by virtue
/// of only stripping when preceded by a recognised type token.
fn strip_length_specifiers(sql: &mut String) {
    const STRICT_TYPES: &[&str] = &["INT", "INTEGER", "REAL", "TEXT", "BLOB", "ANY"];
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Try to match a STRICT type at position i.  On match,
        // append it and advance past it.
        let mut matched_type: Option<&'static str> = None;
        for t in STRICT_TYPES {
            if bytes[i..].starts_with(t.as_bytes())
                && (i + t.len() == bytes.len() || !bytes[i + t.len()].is_ascii_alphanumeric())
            {
                matched_type = Some(t);
                break;
            }
        }
        if let Some(t) = matched_type {
            out.push_str(t);
            i += t.len();
            // Skip whitespace.
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                out.push(bytes[i] as char);
                i += 1;
            }
            // If `(` follows and contains only digits, drop the `(NN)`.
            if i < bytes.len()
                && bytes[i] == b'('
                && let Some(rel_end) = looks_like_length(bytes, i)
            {
                i += rel_end;
                continue;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    *sql = out;
}

/// Shadow the upstream `timestamps()` to make `updated_at` nullable.
///
/// The upstream helper creates both `created_at` and `updated_at` as
/// `NOT NULL` with a `current_timestamp()` default.  A row that has
/// never been updated has no `updated_at`, so the column should be
/// nullable — matching the entity definitions that model the field as
/// `Option<time::PrimitiveDateTime>`.
pub fn timestamps(t: TableCreateStatement) -> TableCreateStatement {
    let mut t = t;
    t.col(timestamp(GeneralIds::CreatedAt).default(Expr::current_timestamp()))
        .col(timestamp_null(GeneralIds::UpdatedAt))
        .take()
}

#[derive(DeriveIden)]
enum GeneralIds {
    CreatedAt,
    UpdatedAt,
}

// ── DbId column helpers ──────────────────────────────────────────────

/// Create a non-nullable `BINARY(16)` column for `DbId` (ULID).
pub fn db_id<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).binary_len(16).not_null().take()
}

/// Create a nullable `BINARY(16)` column for `DbId`.
///
/// Useful for optional foreign-key references to `DbId` primary keys.
pub fn db_id_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).binary_len(16).null().take()
}

/// Create a `DbId` primary key column (non-nullable, no auto-increment).
pub fn pk_db_id<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col)
        .binary_len(16)
        .not_null()
        .primary_key()
        .take()
}

// ── Table and Column Identifiers ───────────────────────────────────────

// Core vocabulary tables

#[derive(DeriveIden)]
pub enum Authors {
    Table,
    Id,
    Name,
}

#[derive(DeriveIden)]
pub enum Tags {
    Table,
    Id,
    Name,
}

#[derive(DeriveIden)]
pub enum Genres {
    Table,
    Id,
    Name,
}

#[derive(DeriveIden)]
pub enum Subjects {
    Table,
    Id,
    Name,
}

#[derive(DeriveIden)]
pub enum Publishers {
    Table,
    Id,
    Name,
    Website,
    LogoUrl,
}

#[derive(DeriveIden)]
pub enum Series {
    Table,
    Id,
    Name,
    SortTitle,
    SeriesType,
}

#[derive(DeriveIden)]
pub enum Formats {
    Table,
    Id,
    Name,
    MetadataSchema,
    ProgressUnit,
}

#[derive(DeriveIden)]
pub enum Languages {
    Table,
    Id,
    Name,
    Code,
    FlagEmoji,
}

#[derive(DeriveIden)]
pub enum Identifiers {
    Table,
    Id,
    Value,
    Kind,
    Source,
    FetchedAt,
}

// Fixture tables

#[derive(DeriveIden)]
pub enum BookConditions {
    Table,
    Id,
    Name,
    Value,
}

// FRBR core tables

#[derive(DeriveIden)]
pub enum Works {
    Table,
    Id,
    Title,
    Description,
    SortTitle,
    SeriesType,
    LanguageId,
    PreferredEditionId,
}

#[derive(DeriveIden)]
pub enum Editions {
    Table,
    Id,
    WorkId,
    GroupId,
    Title,
    PublishedDate,
    FormatId,
    LanguageId,
    Notes,
    Description,
}

// Junction tables

#[derive(DeriveIden)]
pub enum WorkAuthors {
    Table,
    WorkId,
    AuthorId,
    Role,
}

#[derive(DeriveIden)]
pub enum WorkTags {
    Table,
    WorkId,
    TagId,
}

#[derive(DeriveIden)]
pub enum WorkGenres {
    Table,
    WorkId,
    GenreId,
}

#[derive(DeriveIden)]
pub enum WorkSubjects {
    Table,
    WorkId,
    SubjectId,
}

#[derive(DeriveIden)]
pub enum WorkPublishers {
    Table,
    WorkId,
    PublisherId,
}

#[derive(DeriveIden)]
pub enum WorkIdentifiers {
    Table,
    WorkId,
    IdentifierId,
}

#[derive(DeriveIden)]
pub enum EditionAuthors {
    Table,
    EditionId,
    AuthorId,
    Role,
}

#[derive(DeriveIden)]
pub enum EditionTags {
    Table,
    EditionId,
    TagId,
}

#[derive(DeriveIden)]
pub enum EditionGenres {
    Table,
    EditionId,
    GenreId,
}

#[derive(DeriveIden)]
pub enum EditionSubjects {
    Table,
    EditionId,
    SubjectId,
}

#[derive(DeriveIden)]
pub enum EditionPublishers {
    Table,
    EditionId,
    PublisherId,
}

#[derive(DeriveIden)]
pub enum EditionIdentifiers {
    Table,
    EditionId,
    IdentifierId,
}

#[derive(DeriveIden)]
pub enum SeriesEntries {
    Table,
    SeriesId,
    EditionId,
    Position,
}

#[derive(DeriveIden)]
pub enum EditionGroups {
    Table,
    Id,
    Label,
    Description,
}

#[derive(DeriveIden)]
pub enum EditionGroupIdentifiers {
    Table,
    EditionGroupId,
    IdentifierKind,
    IdentifierValue,
}

// Inventory and loans tables

#[derive(DeriveIden)]
pub enum OwnedEditions {
    Table,
    Id,
    EditionId,
    AcquiredAt,
    ConditionId,
    Notes,
}

#[derive(DeriveIden)]
pub enum LoanEntity {
    Table,
    Id,
    Name,
    Notes,
}

#[derive(DeriveIden)]
pub enum LoanEntityIdentifiers {
    Table,
    Id,
    LoanEntityId,
    Url,
    Label,
}

#[derive(DeriveIden)]
pub enum EditionsLoans {
    Table,
    Id,
    EditionId,
    LoanEntityId,
    OwnedEditionId,
    LoanedDate,
    DueDate,
    ReturnedDate,
}

#[derive(DeriveIden)]
pub enum DigitalInventory {
    Table,
    Id,
    EditionId,
    FilePath,
    CoverPath,
    Blurhash,
    DominantColor,
    FileHash,
    FileSizeBytes,
    Notes,
    AddedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum EditionSpecificCovers {
    Table,
    Id,
    EditionId,
    CoverPath,
    CreatedAt,
    UpdatedAt,
}

// Reading and annotations tables

#[derive(DeriveIden)]
pub enum Annotations {
    Table,
    Id,
    EditionId,
    UserId,
    Content,
    Location,
}

#[derive(DeriveIden)]
pub enum ReadingLists {
    Table,
    Id,
    Name,
    Description,
}

#[derive(DeriveIden)]
pub enum ReadingListBook {
    Table,
    ReadingListId,
    EditionId,
    Position,
    AddedAt,
}

#[derive(DeriveIden)]
pub enum ReadingProgress {
    Table,
    Id,
    EditionId,
    FormatId,
    Progress,
    ProgressUnit,
    LastLocation,
    TotalReadingTimeSecs,
    CreatedAt,
}

#[derive(DeriveIden)]
pub enum WorkStatus {
    Table,
    WorkId,
    Status,
    CreatedAt,
    UpdatedAt,
}

// Reading sources and sessions

#[derive(DeriveIden)]
pub enum ReadingSources {
    Table,
    Id,
    Urn,
    Name,
    Emoji,
    Color,
    Attributes,
    PluginId,
    DeletedAt,
}

#[derive(DeriveIden)]
pub enum ReadingSessions {
    Table,
    Id,
    EditionId,
    FormatId,
    SourceId,
    StartedAt,
    EndedAt,
    DurationSeconds,
    RawProgression,
    ProgressDelta,
    LastLocation,
    Notes,
}

// Plugins and devices tables

#[derive(DeriveIden)]
pub enum SearchHistory {
    Table,
    Id,
    Query,
    SearchedAt,
}

#[derive(DeriveIden)]
pub enum SavedSearches {
    Table,
    Id,
    Name,
    DefinitionJson,
    BindingsJson,
    OptionsJson,
}

#[derive(DeriveIden)]
pub enum EditionFiles {
    Table,
    Id,
    EditionId,
    FilePath,
    FileFormat,
    FileSizeBytes,
    FileLastModified,
    FileMode,
    SourcePlugin,
    SourceId,
    CreatedAt,
}
