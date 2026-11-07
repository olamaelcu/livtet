//! Client-side pull of a paired peer's library, and receipt of offered books.
//!
//! The wire protocol is pull-based: a sender only records a *book offer* in its
//! own change log, and the bytes move when the receiver (this device) asks for
//! them. So receiving a book is three steps, mirrored from the desktop daemon
//! (`livtet-sync-server/src/daemon.rs`):
//!
//! 1. [`sync_pull`] fetches changes after a cursor. Rows describing an offered
//!    book are held back and returned as an [`SyncOffer`]; every other row is a
//!    library change and is applied immediately.
//! 2. The reader decides per book. [`sync_offer_accept`] tells the peer, streams
//!    the file and cover into the caller's directories, then applies the held
//!    rows *with* those paths so the edition points at a local file.
//!    [`sync_offer_reject`] just answers.
//! 3. The caller persists the cursor, which is what makes the next pull
//!    incremental.
//!
//! The session token is a parameter, not read from DataStore here: the FFI has
//! no storage of its own on this side of the boundary, and the caller already
//! owns the per-peer token.

use std::collections::{HashMap, HashSet};
use std::io::Write;

use camino::Utf8Path;
use livtet_core::data::SharedState;
use livtet_core::data::migrator::Kind;
use livtet_core::types::DbId;
use livtet_sync::{ApplyOptions, InventoryPaths, SyncChange, SyncEngine};

use crate::error::LivtetError;
use crate::sync::connect;

/// The change-log entity type that marks a book offer.
const OFFER_ENTITY_TYPE: &str = "file_offer";

/// Read size for a book download: large enough to keep the stream busy, small
/// enough that progress lands often while a big file is moving.
const CHUNK_BYTES: usize = 256 * 1024;

/// Keys an offer payload uses, as written by the offering daemon.
const OFFER_ID: &str = "offer_id";
const INVENTORY_ID: &str = "inventory_id";
const OFFER_TITLE: &str = "title";
const OFFER_AUTHORS: &str = "authors";
const EDITION_ID: &str = "edition_id";
const WORK_ID: &str = "work_id";
const FILE_NAME: &str = "file_name";
const HAS_COVER: &str = "has_cover";

/// A book a paired device is offering, waiting on this device's reader.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncOffer {
    pub offer_id: String,
    pub inventory_id: String,
    pub edition_id: String,
    pub work_id: String,
    /// The sender's file name, used to name the local copy.
    pub file_name: String,
    pub has_cover: bool,
    /// The rows that describe this book, held back until it is accepted.
    /// Opaque to the caller: echo it back to [`sync_offer_accept`].
    pub deferred_changes: String,
}

/// What one pull did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncPullReport {
    /// Library rows written locally (everything that was not an offer).
    pub applied: u32,
    pub conflicts: u32,
    /// Pass this back as `since_version` for the next pull.
    pub cursor: i64,
    /// The peer had more rows than the page size; pull again immediately.
    pub has_more: bool,
    pub offers: Vec<SyncOffer>,
}

/// Byte-level progress for one file moving from a peer.
///
/// The download streams to disk in chunks, so the caller can show how far the
/// current book has got instead of only which book is in flight.
#[uniffi::export(callback_interface)]
pub trait TransferProgressListener: Send + Sync {
    /// `done` of `total` bytes written for the file being received.
    fn on_progress(&self, done: u64, total: u64);
}

/// A book that landed on disk.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncReceivedBook {
    pub file_path: String,
    pub cover_path: Option<String>,
    /// Library rows written when the held rows were applied.
    pub applied: u32,
}

/// Pull changes from `peer_node_id`, applying everything except book offers.
///
/// `session_token` is the token that peer issued this device when it approved
/// the pairing; without it the peer refuses the request.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_pull(
    secret_key_path: String,
    session_token: String,
    db_path: String,
    device_id: String,
    peer_node_id: String,
    since_version: i64,
    limit: i64,
) -> Result<SyncPullReport, LivtetError> {
    // The cursor is keyed by the peer, so the node id outlives the connection.
    let peer = peer_node_id.clone();
    let client = connect(secret_key_path, peer_node_id)
        .await?
        .with_token(session_token);
    // A device starting from nothing cannot replay a change log: the rows it needs
    // were written before the log's first retained entry, so applying them fails
    // on foreign keys that were never created here. Bootstrap from the full dump
    // instead, which is every syncable row in dependency order.
    if since_version <= 0 {
        return bootstrap(&client, db_path, limit, peer).await;
    }
    let pull = client
        .changes(since_version, Some(limit))
        .await
        .map_err(|error| LivtetError::Sync(format!("could not pull changes: {error}")))?;

    let state = SharedState::connect(&db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    let engine = SyncEngine::new(
        livtet_core::data::orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(state.pool.clone()),
        device_id.clone(),
    );
    let (offers, metadata) = split_offers(pull.changes);
    remember_offers(&state.pool, &offers).await?;
    remember_cursor(&state.pool, &peer, pull.latest_version).await?;

    let mut applied = 0;
    let mut conflicts = 0;
    if !metadata.is_empty() {
        let outcome = engine
            .apply_changes(
                metadata,
                ApplyOptions {
                    inventory_paths: HashMap::new(),
                },
            )
            .await
            .map_err(|error| LivtetError::Sync(format!("could not apply pulled rows: {error}")))?;
        applied = outcome.applied as u32;
        conflicts = outcome.conflicts.len() as u32;
    }

    Ok(SyncPullReport {
        applied,
        conflicts,
        cursor: pull.latest_version,
        has_more: pull.has_more,
        offers: offers.into_iter().map(|pulled| pulled.offer).collect(),
    })
}

/// Accept an offer: download the file and cover, tell the peer, register the book.
///
/// `deferred_changes` comes from the [`SyncOffer`]: those rows are applied after
/// the bytes land, carrying the local paths, which is what gives the edition a
/// file to open.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_offer_accept(
    secret_key_path: String,
    session_token: String,
    db_path: String,
    device_id: String,
    peer_node_id: String,
    offer: SyncOffer,
    books_dir: String,
    covers_dir: String,
    progress: Option<Box<dyn TransferProgressListener>>,
) -> Result<SyncReceivedBook, LivtetError> {
    let client = connect(secret_key_path, peer_node_id)
        .await?
        .with_token(session_token);

    let books_dir = Utf8Path::new(&books_dir);
    let covers_dir = Utf8Path::new(&covers_dir);
    std::fs::create_dir_all(books_dir)
        .map_err(|error| LivtetError::Sync(format!("could not create {books_dir}: {error}")))?;
    std::fs::create_dir_all(covers_dir)
        .map_err(|error| LivtetError::Sync(format!("could not create {covers_dir}: {error}")))?;

    // Name the local copy after the inventory id so two books with the same file
    // name cannot collide, keeping the sender's extension for readers.
    let base = Utf8Path::new(offer.file_name.as_str())
        .file_name()
        .filter(|name| !name.is_empty())
        .unwrap_or("book");
    let file_path = books_dir.join(format!("{}-{base}", offer.inventory_id));

    let (header, mut body) = client
        .file(&offer.inventory_id, 0, None, None)
        .await
        .map_err(|error| LivtetError::Sync(format!("could not download the book: {error}")))?;
    // Streamed rather than read whole: a book can be hundreds of megabytes, and
    // writing each chunk is also what gives the caller byte-level progress.
    let total = header.total;
    let mut file = std::fs::File::create(file_path.as_std_path())
        .map_err(|error| LivtetError::Sync(format!("could not create {file_path}: {error}")))?;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut written = 0u64;
    loop {
        let read = body
            .read_chunk(&mut buffer)
            .await
            .map_err(|error| LivtetError::Sync(format!("could not read the book: {error}")))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| LivtetError::Sync(format!("could not write {file_path}: {error}")))?;
        written += read as u64;
        if let Some(listener) = progress.as_deref() {
            listener.on_progress(written, total);
        }
    }
    file.flush()
        .map_err(|error| LivtetError::Sync(format!("could not flush {file_path}: {error}")))?;

    let mut cover_path = None;
    if offer.has_cover
        && let Some((header, cover)) = client
            .cover(&offer.inventory_id)
            .await
            .map_err(|error| LivtetError::Sync(format!("could not download the cover: {error}")))?
    {
        let extension = cover_extension(&header.content_type);
        let path = covers_dir.join(format!("{}.{extension}", offer.inventory_id));
        std::fs::write(path.as_std_path(), &cover)
            .map_err(|error| LivtetError::Sync(format!("could not write {path}: {error}")))?;
        cover_path = Some(path.to_string());
    }

    // Tell the peer before touching the database: a decided offer it never hears
    // about is a book the reader asked for and did not get.
    client
        .offer_decision(&offer.offer_id, true)
        .await
        .map_err(|error| LivtetError::Sync(format!("could not record the decision: {error}")))?;

    let deferred: Vec<SyncChange> = serde_json::from_str(&offer.deferred_changes)
        .map_err(|error| LivtetError::Sync(format!("offer rows are unreadable: {error}")))?;
    let mut applied = 0;
    if !deferred.is_empty() {
        let engine = engine(db_path.clone(), device_id).await?;
        let outcome = engine
            .apply_changes(
                deferred,
                ApplyOptions {
                    inventory_paths: HashMap::from([(
                        offer.inventory_id.clone(),
                        InventoryPaths {
                            file_path: Some(file_path.to_string()),
                            cover_path: cover_path.clone(),
                        },
                    )]),
                },
            )
            .await
            .map_err(|error| LivtetError::Sync(format!("could not register the book: {error}")))?;
        applied = outcome.applied as u32;
    }

    // The decision closes the offer out of the inbox; the book is in the library
    // now, so the row is history the screen no longer shows.
    mark_offer(&db_path, &offer.offer_id, "accepted").await?;

    Ok(SyncReceivedBook {
        file_path: file_path.to_string(),
        cover_path,
        applied,
    })
}

/// Decline an offer. The book stays on the peer; nothing is written locally.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_offer_reject(
    secret_key_path: String,
    session_token: String,
    db_path: String,
    peer_node_id: String,
    offer_id: String,
) -> Result<(), LivtetError> {
    let client = connect(secret_key_path, peer_node_id)
        .await?
        .with_token(session_token);
    client
        .offer_decision(&offer_id, false)
        .await
        .map_err(|error| LivtetError::Sync(format!("could not decline the offer: {error}")))?;
    mark_offer(&db_path, &offer_id, "rejected").await
}

/// How many change-log pages to scan for offers after a bootstrap.
const MAX_BOOTSTRAP_PAGES: usize = 20;

/// Mirror a peer's whole library into an empty database.
///
/// The dump is applied table by table in dependency order (works before editions
/// before inventory) so no row references a parent that has not been written. A
/// dump carries state, not the change log, so offers are then read from the log in
/// a pass that records them without re-applying rows the dump already held.
async fn bootstrap(
    client: &livtet_sync_iroh::Client,
    db_path: String,
    limit: i64,
    peer_node_id: String,
) -> Result<SyncPullReport, LivtetError> {
    let dump = client
        .pull_full()
        .await
        .map_err(|error| LivtetError::Sync(format!("could not fetch the library: {error}")))?;
    let applied = apply_dump(&db_path, &dump).await?;

    let mut offers: Vec<PulledOffer> = Vec::new();
    let mut cursor = dump.version;
    let mut probe = 0i64;
    for _ in 0..MAX_BOOTSTRAP_PAGES {
        let page = client
            .changes(probe, Some(limit))
            .await
            .map_err(|error| LivtetError::Sync(format!("could not read offers: {error}")))?;
        let (page_offers, _rows) = split_offers(page.changes);
        for pulled in page_offers {
            if !offers.iter().any(|seen| seen.offer.offer_id == pulled.offer.offer_id) {
                offers.push(pulled);
            }
        }
        cursor = cursor.max(page.latest_version);
        if !page.has_more {
            break;
        }
        probe = page.latest_version;
    }

    // A bootstrap is also the first look at whatever is waiting, so it records
    // both the offers it found and where the next pull should resume.
    let state = SharedState::connect(&db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    remember_offers(&state.pool, &offers).await?;
    remember_cursor(&state.pool, &peer_node_id, cursor).await?;

    Ok(SyncPullReport {
        applied,
        conflicts: 0,
        cursor,
        has_more: false,
        offers: offers.into_iter().map(|pulled| pulled.offer).collect(),
    })
}

/// Insert every row of a dump, parents first.
///
/// Columns come from each row's own keys and values from `json_extract`, so one
/// bind carries a whole row and SQLite applies its JSON typing — no per-column
/// marshalling table to keep in sync with the schema.
async fn apply_dump(db_path: &str, dump: &livtet_sync::FullDump) -> Result<u32, LivtetError> {
    let state = SharedState::connect(db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    let tables: [(&str, &[serde_json::Value]); 11] = [
        ("works", &dump.entities.works),
        ("editions", &dump.entities.editions),
        ("edition_groups", &dump.entities.edition_groups),
        ("series_entries", &dump.entities.series_entries),
        ("digital_inventory", &dump.entities.digital_inventory),
        ("owned_editions", &dump.entities.owned_editions),
        ("editions_loans", &dump.entities.editions_loans),
        ("annotations", &dump.entities.annotations),
        ("reading_lists", &dump.entities.reading_lists),
        ("reading_list_book", &dump.entities.reading_list_book),
        ("reading_progress", &dump.entities.reading_progress),
    ];

    let mut written = 0u32;
    for (table, rows) in tables {
        for row in rows {
            let Some(object) = row.as_object() else {
                continue;
            };
            if object.is_empty() {
                continue;
            }
            let mut insert =
                livtet_core::data::sql::QueryBuilder::<livtet_core::data::sql::Sqlite>::new(
                    "INSERT OR REPLACE INTO ",
                );
            insert.push(table);
            insert.push(" (");
            let columns: Vec<String> = object.keys().map(|key| quote_column(key)).collect();
            insert.push(columns.join(", "));
            insert.push(") VALUES (");
            for (index, (column, value)) in object.iter().enumerate() {
                if index > 0 {
                    insert.push(", ");
                }
                bind_json(&mut insert, column, value);
            }
            insert.push(")");
            insert.build().execute(&state.pool).await.map_err(|error| {
                LivtetError::Sync(format!("could not mirror a {table} row: {error}"))
            })?;
            written += 1;
        }
    }
    Ok(written)
}

/// Quote a column name for the generated statement.
fn quote_column(name: &str) -> String {
    let mut quoted = String::with_capacity(name.len() + 2);
    quoted.push('"');
    for ch in name.chars() {
        if ch == '"' {
            quoted.push('"');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

/// Bind one column's value with the storage class its column expects.
///
/// Ids cross the wire as text — ULID when a serialized model is dumped, hex when
/// a change payload names an entity — while the tables declare them `BLOB` (a row
/// id is SeaORM's 16-byte value), so an id is decoded to bytes. Other values keep
/// their JSON type: numbers and booleans bind as integers, null as `NULL`, and
/// everything else as text. Only id columns are decoded — an id-looking string in
/// a text column is data, not an identifier.
fn bind_json(
    insert: &mut livtet_core::data::sql::QueryBuilder<livtet_core::data::sql::Sqlite>,
    column: &str,
    value: &serde_json::Value,
) {
    match value {
        serde_json::Value::Null => {
            insert.push("NULL");
        }
        serde_json::Value::Bool(flag) => {
            insert.push_bind(i64::from(*flag));
        }
        serde_json::Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                insert.push_bind(int);
            } else if let Some(float) = number.as_f64() {
                insert.push_bind(float);
            } else {
                insert.push("NULL");
            }
        }
        serde_json::Value::String(text) => {
            match decode_id_column(column, text) {
                Some(bytes) => insert.push_bind(bytes),
                None => insert.push_bind(text.clone()),
            };
        }
        other => {
            insert.push_bind(other.to_string());
        }
    }
}

/// The bytes behind an id column's hex text, or `None` when this is not one.
fn decode_id_column(column: &str, text: &str) -> Option<Vec<u8>> {
    let is_id = column == "id" || column.ends_with("_id");
    if !is_id {
        return None;
    }
    Some(DbId::from_wire(text)?.to_bytes().to_vec())
}

/// Open the library database and a sync engine over it.
async fn engine(db_path: String, device_id: String) -> Result<SyncEngine, LivtetError> {
    let state = SharedState::connect(&db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    // `SyncEngine` works over SeaORM's connection, not the raw sqlx pool.
    let db = livtet_core::data::orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(state.pool.clone());
    Ok(SyncEngine::new(db, device_id))
}

/// One offer as the pull found it, with the fields only its payload carries.
///
/// The wire record ([`SyncOffer`]) is the accept payload; the inbox needs to show
/// a book before any catalog row exists, so the display fields are read out of
/// the payload here rather than left for the UI to parse.
struct PulledOffer {
    offer: SyncOffer,
    from_device_id: String,
    title: String,
    /// Author names as a JSON array, which is how the row stores them.
    authors: String,
    offered_at: String,
}

/// One book a peer is offering, as the inbox shows it.
///
/// Read from `inbound_offers`, so an offer survives the app closing: it is a
/// decision the reader has not made yet, not a message in a buffer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncOfferSummary {
    pub offer_id: String,
    /// The device that offered it, as its `DbId` string.
    pub from_device_id: String,
    pub title: String,
    pub authors: Vec<String>,
    /// The sender's file name, which is where the format shows.
    pub file_name: Option<String>,
    pub has_cover: bool,
    pub offered_at: String,
    pub inventory_id: String,
    pub edition_id: String,
    pub work_id: String,
    /// Opaque rows applied on acceptance; echo back to [`sync_offer_accept`].
    pub deferred_changes: String,
}

/// Keep the offers a pull found, without resurrecting ones already decided.
async fn remember_offers(
    pool: &livtet_core::data::sql::SqlitePool,
    offers: &[PulledOffer],
) -> Result<(), LivtetError> {
    for pulled in offers {
        let offer = &pulled.offer;
        livtet_core::data::sql::query(
            "INSERT INTO inbound_offers \
             (offer_id, from_device_id, inventory_id, edition_id, work_id, title, authors, \
              file_name, has_cover, offered_at, received_at, status, decided_at, deferred_changes) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), 'pending', NULL, ?) \
             ON CONFLICT(offer_id) DO NOTHING",
        )
        .bind(offer.offer_id.clone())
        .bind(pulled.from_device_id.clone())
        .bind(offer.inventory_id.clone())
        .bind(offer.edition_id.clone())
        .bind(offer.work_id.clone())
        .bind(pulled.title.clone())
        .bind(pulled.authors.clone())
        .bind(Some(offer.file_name.clone()).filter(|name| !name.is_empty()))
        .bind(offer.has_cover)
        .bind(pulled.offered_at.clone())
        .bind(offer.deferred_changes.clone())
        .execute(pool)
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    }
    Ok(())
}

/// Store where the next pull from `device_id` should resume.
async fn remember_cursor(
    pool: &livtet_core::data::sql::SqlitePool,
    peer_node_id: &str,
    cursor: i64,
) -> Result<(), LivtetError> {
    livtet_core::data::sql::query(
        "INSERT INTO client_settings (key, value, updated_at) VALUES (?, ?, datetime('now')) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(cursor_key(peer_node_id))
    .bind(cursor.to_string())
    .execute(pool)
    .await
    .map_err(|error| LivtetError::Database(error.to_string()))?;
    Ok(())
}

/// The settings key the daemon uses for the same cursor.
///
/// Keyed by node id rather than device id: the node id is what a dial starts
/// from and the one this device always has, while a peer's ULID is only known
/// once it identifies itself — a cursor must not depend on that answer.
fn cursor_key(peer_node_id: &str) -> String {
    format!("sync.cursor.{peer_node_id}")
}

/// Where the next pull from `peer_node_id` should resume.
///
/// Zero means nothing has been pulled from it yet, which makes the next pull a
/// bootstrap.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_cursor(db_path: String, peer_node_id: String) -> Result<i64, LivtetError> {
    use livtet_core::data::sql::Row;

    let state = SharedState::connect(&db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    let row = livtet_core::data::sql::query(
        "SELECT value FROM client_settings WHERE key = ?",
    )
    .bind(cursor_key(&peer_node_id))
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| LivtetError::Database(error.to_string()))?;

    Ok(row
        .and_then(|row| row.try_get::<String, _>("value").ok())
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0))
}

/// The offers nobody has decided yet, newest first.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_offers(db_path: String) -> Result<Vec<SyncOfferSummary>, LivtetError> {
    use livtet_core::data::sql::Row;

    let state = SharedState::connect(&db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    let rows = livtet_core::data::sql::query(
        "SELECT offer_id, from_device_id, inventory_id, edition_id, work_id, title, authors, \
         file_name, has_cover, offered_at, deferred_changes \
         FROM inbound_offers WHERE status = 'pending' ORDER BY received_at DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|error| LivtetError::Database(error.to_string()))?;

    let mut offers = Vec::with_capacity(rows.len());
    for row in rows {
        let text = |column: &str| row.try_get::<String, _>(column).unwrap_or_default();
        offers.push(SyncOfferSummary {
            offer_id: text("offer_id"),
            from_device_id: text("from_device_id"),
            title: text("title"),
            authors: serde_json::from_str::<Vec<String>>(&text("authors")).unwrap_or_default(),
            file_name: row.try_get::<Option<String>, _>("file_name").ok().flatten(),
            has_cover: row.try_get::<bool, _>("has_cover").unwrap_or(false),
            offered_at: text("offered_at"),
            inventory_id: text("inventory_id"),
            edition_id: text("edition_id"),
            work_id: text("work_id"),
            deferred_changes: text("deferred_changes"),
        });
    }
    Ok(offers)
}

/// Record a decision so the inbox stops showing it.
async fn mark_offer(db_path: &str, offer_id: &str, status: &str) -> Result<(), LivtetError> {
    let state = SharedState::connect(db_path, &[Kind::Business, Kind::Client])
        .await
        .map_err(|error| LivtetError::Database(error.to_string()))?;
    livtet_core::data::sql::query(
        "UPDATE inbound_offers SET status = ?, decided_at = datetime('now') WHERE offer_id = ?",
    )
    .bind(status.to_string())
    .bind(offer_id.to_string())
    .execute(&state.pool)
    .await
    .map_err(|error| LivtetError::Database(error.to_string()))?;
    Ok(())
}

/// Split a pulled page into offers (with the rows describing them) and the rows
/// that can be applied straight away.
fn split_offers(changes: Vec<SyncChange>) -> (Vec<PulledOffer>, Vec<SyncChange>) {
    let parsed: Vec<(SyncChange, PulledOffer)> = changes
        .iter()
        .filter(|change| change.entity_type == OFFER_ENTITY_TYPE)
        .filter_map(|change| {
            let payload: serde_json::Value = serde_json::from_str(&change.payload).ok()?;
            let text = |key: &str| {
                payload
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let offer_id = text(OFFER_ID);
            if offer_id.is_empty() {
                return None;
            }
            Some((
                change.clone(),
                PulledOffer {
                    from_device_id: change.device_id.clone(),
                    offered_at: change.changed_at.clone(),
                    title: text(OFFER_TITLE),
                    authors: serde_json::to_string(
                        &payload
                            .get(OFFER_AUTHORS)
                            .and_then(serde_json::Value::as_array)
                            .map(|names| {
                                names
                                    .iter()
                                    .filter_map(serde_json::Value::as_str)
                                    .map(str::to_string)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default(),
                    )
                    .unwrap_or_else(|_| "[]".to_string()),
                    offer: SyncOffer {
                        offer_id,
                        inventory_id: text(INVENTORY_ID),
                        edition_id: text(EDITION_ID),
                        work_id: text(WORK_ID),
                        file_name: text(FILE_NAME),
                        has_cover: payload
                            .get(HAS_COVER)
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                        deferred_changes: String::new(),
                    },
                },
            ))
        })
        .collect();

    // A pull holds back the rows that describe an offered book — inventory,
    // edition, work — because applying them before the reader decides would make
    // the book appear with no file behind it.
    let held_ids: HashSet<String> = parsed
        .iter()
        .flat_map(|(_, pulled)| {
            [
                &pulled.offer.inventory_id,
                &pulled.offer.edition_id,
                &pulled.offer.work_id,
            ]
                .into_iter()
                .filter(|id| !id.is_empty())
                .cloned()
        })
        .collect();

    let (deferred, metadata): (Vec<SyncChange>, Vec<SyncChange>) = changes
        .into_iter()
        .filter(|change| change.entity_type != OFFER_ENTITY_TYPE)
        .partition(|change| held_ids.contains(&change.entity_id));

    let offers = parsed
        .into_iter()
        .map(|(_, mut pulled)| {
            let offer = &mut pulled.offer;
            let held: Vec<SyncChange> = deferred
                .iter()
                .filter(|change| {
                    [&offer.inventory_id, &offer.edition_id, &offer.work_id]
                        .into_iter()
                        .any(|id| !id.is_empty() && id == &change.entity_id)
                })
                .cloned()
                .collect();
            offer.deferred_changes = serde_json::to_string(&held).unwrap_or_else(|_| "[]".into());
            pulled
        })
        .collect();

    (offers, metadata)
}

/// Pick a cover extension from the content type the peer reported.
fn cover_extension(content_type: &str) -> &'static str {
    if content_type.contains("jpeg") || content_type.contains("jpg") {
        "jpg"
    } else if content_type.contains("webp") {
        "webp"
    } else {
        "png"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livtet_iroh::DiscoveryService;
    use livtet_sync::PullResponse;
    use livtet_sync_iroh::{FileHeader, FileMetadata, Handler, Outcome, Request, Response};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A dump with no rows: the bootstrap path then writes nothing but must still
    /// advance the cursor and surface offers from the log.
    fn empty_dump() -> livtet_sync::EntityDump {
        livtet_sync::EntityDump {
            works: vec![],
            editions: vec![],
            edition_groups: vec![],
            series_entries: vec![],
            digital_inventory: vec![],
            owned_editions: vec![],
            editions_loans: vec![],
            annotations: vec![],
            reading_lists: vec![],
            reading_list_book: vec![],
            reading_progress: vec![],
        }
    }

    fn header(content_type: &str, length: usize) -> FileHeader {
        FileHeader {
            content_type: content_type.to_string(),
            offset: 0,
            length: length as u64,
            total: length as u64,
            etag: "etag-1".to_string(),
        }
    }

    /// A peer offers one book; pulling lists it, accepting writes the exact
    /// bytes into the caller's directory and tells the peer.
    #[tokio::test]
    async fn an_offered_book_is_pulled_and_written_to_disk() {
        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let book = b"PK\x03\x04 pretend epub payload".to_vec();
        let cover = b"\x89PNG pretend cover".to_vec();
        let book_src = dir.path().join("source.epub");
        let cover_src = dir.path().join("source.png");
        std::fs::write(book_src.as_std_path(), &book).expect("write book");
        std::fs::write(cover_src.as_std_path(), &cover).expect("write cover");

        let offer_payload = serde_json::json!({
            "offer_id": "offer-1",
            "inventory_id": "inv-1",
            "edition_id": "edi-1",
            "work_id": "wor-1",
            "file_name": "Designing Data-Intensive Applications.epub",
            "has_cover": true,
            "title": "The Reverse Direction",
            "authors": ["Jacky Alcine"],
        })
        .to_string();
        let changes = vec![SyncChange {
            id: 1,
            entity_type: OFFER_ENTITY_TYPE.to_string(),
            entity_id: "offer-1".to_string(),
            operation: "upsert".to_string(),
            version: 7,
            payload: offer_payload,
            changed_at: "2026-11-03T22:00:00Z".to_string(),
            device_id: "desk-device".to_string(),
        }];

        let decided = Arc::new(AtomicBool::new(false));
        let peer = DiscoveryService::bind().await.expect("peer endpoint");
        let peer_node_id = peer.node_id().as_str().to_string();
        let handler: Handler = Arc::new({
            let changes = changes.clone();
            let decided = Arc::clone(&decided);
            let book_len = book.len();
            let cover_len = cover.len();
            move |request, _remote| {
                let changes = changes.clone();
                let decided = Arc::clone(&decided);
                let book_src = book_src.clone();
                let cover_src = cover_src.clone();
                Box::pin(async move {
                    match request {
                        Request::Changes { .. } => {
                            Outcome::reply(Response::Changes(PullResponse {
                                changes,
                                has_more: false,
                                latest_version: 7,
                            }))
                        }
                        Request::PullFull { .. } => {
                            Outcome::reply(Response::Full(Box::new(livtet_sync::FullDump {
                                version: 9,
                                device_id: "desk-device".to_string(),
                                entities: empty_dump(),
                            })))
                        }
                        Request::FileInfo { .. } => {
                            Outcome::reply(Response::FileInfo(FileMetadata {
                                content_type: "application/epub+zip".to_string(),
                                size: book_len as u64,
                                etag: "etag-1".to_string(),
                            }))
                        }
                        Request::File { .. } => {
                            Outcome::file(header("application/epub+zip", book_len), book_src)
                        }
                        Request::Cover { .. } => {
                            Outcome::cover(header("image/png", cover_len), cover_src)
                        }
                        Request::OfferDecision { accepted, .. } => {
                            decided.store(accepted, Ordering::SeqCst);
                            Outcome::reply(Response::OfferDecided)
                        }
                        other => Outcome::reply(Response::Refused {
                            reason: format!("unexpected request: {other:?}"),
                        }),
                    }
                })
            }
        });
        let _serving = livtet_sync_iroh::spawn_server(peer.endpoint().clone(), handler);

        let key_path = dir.path().join("sync.key").to_string();
        let db_path = dir.path().join("library.db").to_string();

        let report = sync_pull(
            key_path.clone(),
            "session-token".to_string(),
            db_path.clone(),
            "phone-device".to_string(),
            peer_node_id.clone(),
            1,
            50,
        )
        .await
        .expect("pull");

        assert_eq!(
            report.cursor, 7,
            "the cursor must advance to the peer's version"
        );
        assert!(!report.has_more);
        assert_eq!(
            report.offers.len(),
            1,
            "one offer should be listed: {report:?}"
        );
        let offer = report.offers[0].clone();
        assert_eq!(offer.offer_id, "offer-1");
        assert_eq!(offer.inventory_id, "inv-1");
        assert_eq!(offer.edition_id, "edi-1");
        assert!(offer.has_cover);

        // The offer is now the reader's pending decision, so it is in the inbox
        // with the fields its payload carried — not just in this return value.
        let inbox = sync_offers(db_path.clone()).await.expect("list offers");
        assert_eq!(inbox.len(), 1, "one book is waiting: {inbox:?}");
        assert_eq!(inbox[0].title, "The Reverse Direction");
        assert_eq!(inbox[0].authors, vec!["Jacky Alcine".to_string()]);
        assert_eq!(inbox[0].from_device_id, "desk-device");

        // And the pull recorded where to resume, so the next one is incremental.
        assert_eq!(
            sync_cursor(db_path.clone(), peer_node_id.clone())
                .await
                .expect("read the cursor"),
            7,
            "the cursor must be stored per peer"
        );

        let books_dir = dir.path().join("books").to_string();
        let covers_dir = dir.path().join("covers").to_string();
        let received = sync_offer_accept(
            key_path,
            "session-token".to_string(),
            db_path.clone(),
            "phone-device".to_string(),
            peer_node_id,
            offer,
            books_dir.clone(),
            covers_dir,
            None,
        )
        .await
        .expect("accept");

        assert_eq!(
            std::fs::read(&received.file_path).expect("read received book"),
            book,
            "the received bytes must match the offered file"
        );
        assert!(
            received.file_path.starts_with(&books_dir),
            "the book must land in the requested directory: {}",
            received.file_path
        );
        let cover_path = received.cover_path.expect("a cover was offered");
        assert_eq!(std::fs::read(&cover_path).expect("read cover"), cover);
        assert!(
            decided.load(Ordering::SeqCst),
            "the peer must be told the offer was accepted"
        );

        // A decided offer leaves the inbox: the book is in the library now, and a
        // list that kept asking would be a list the reader stops reading.
        assert!(
            sync_offers(db_path).await.expect("list offers").is_empty(),
            "an accepted offer is no longer pending"
        );
    }

    /// A fresh device bootstraps from the full dump, not the change log.
    ///
    /// The log cannot stand in for a library on a device that has none: its rows
    /// reference parents written before the log's first retained entry. So the
    /// first pull asks for the dump, takes its version as the cursor, and still
    /// reports offers that are sitting in the log.
    #[tokio::test]
    async fn a_fresh_device_bootstraps_from_the_full_dump() {
        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let offer_payload = serde_json::json!({
            "offer_id": "offer-9",
            "inventory_id": "inv-9",
            "edition_id": "edi-9",
            "work_id": "wor-9",
            "file_name": "Book.epub",
            "has_cover": false,
        })
        .to_string();
        let changes = vec![SyncChange {
            id: 3,
            entity_type: OFFER_ENTITY_TYPE.to_string(),
            entity_id: "offer-9".to_string(),
            operation: "upsert".to_string(),
            version: 7,
            payload: offer_payload,
            changed_at: "2026-11-03T22:00:00Z".to_string(),
            device_id: "desk-device".to_string(),
        }];

        let bootstrapped = Arc::new(AtomicBool::new(false));
        let peer = DiscoveryService::bind().await.expect("peer endpoint");
        let peer_node_id = peer.node_id().as_str().to_string();
        let handler: Handler = Arc::new({
            let changes = changes.clone();
            let bootstrapped = Arc::clone(&bootstrapped);
            move |request, _remote| {
                let changes = changes.clone();
                let bootstrapped = Arc::clone(&bootstrapped);
                Box::pin(async move {
                    match request {
                        Request::PullFull { .. } => {
                            bootstrapped.store(true, Ordering::SeqCst);
                            Outcome::reply(Response::Full(Box::new(livtet_sync::FullDump {
                                version: 9,
                                device_id: "desk-device".to_string(),
                                entities: empty_dump(),
                            })))
                        }
                        Request::Changes { .. } => {
                            Outcome::reply(Response::Changes(PullResponse {
                                changes,
                                has_more: false,
                                latest_version: 7,
                            }))
                        }
                        other => Outcome::reply(Response::Refused {
                            reason: format!("unexpected request: {other:?}"),
                        }),
                    }
                })
            }
        });
        let _serving = livtet_sync_iroh::spawn_server(peer.endpoint().clone(), handler);

        let report = sync_pull(
            dir.path().join("sync.key").to_string(),
            "session-token".to_string(),
            dir.path().join("library.db").to_string(),
            "phone-device".to_string(),
            peer_node_id,
            0,
            50,
        )
        .await
        .expect("bootstrap");

        assert!(
            bootstrapped.load(Ordering::SeqCst),
            "a zero cursor must ask for the full dump"
        );
        assert_eq!(report.cursor, 9, "the cursor follows the dump's version");
        assert_eq!(
            report.offers.len(),
            1,
            "an offer in the log must still surface"
        );
        assert_eq!(report.offers[0].offer_id, "offer-9");
    }

    /// A dump carries model fields, so an id arrives as ULID text
    /// ([`DbId`]'s `Display`) while the table declares `BLOB`.
    ///
    /// Binding that text aborted a real bootstrap with "cannot store TEXT
    /// value in BLOB column works.id" before any row landed, so `works` is
    /// first in dependency order and nothing else was written either.
    #[tokio::test]
    async fn a_dumped_row_stores_its_ulid_id_as_bytes() {
        use livtet_core::data::orm::EntityTrait;

        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let db_path = dir.path().join("library.db").to_string();
        let work_id = DbId::new();

        let mut entities = empty_dump();
        entities.works = vec![serde_json::json!({
            "id": work_id.to_string(),
            "title": "Designing Data-Intensive Applications",
            "created_at": "2026-11-04 11:00:00",
        })];
        let dump = livtet_sync::FullDump {
            version: 1,
            device_id: "desk-device".to_string(),
            entities,
        };

        let written = apply_dump(&db_path, &dump).await.expect("dump applies");
        assert_eq!(written, 1, "the dumped row must be written");

        let state = SharedState::connect(&db_path, &[Kind::Business, Kind::Client])
            .await
            .expect("connect");
        let db =
            livtet_core::data::orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(state.pool.clone());
        let row = livtet_core::data::entities::works::Entity::find_by_id(work_id)
            .one(&db)
            .await
            .expect("query the mirrored work")
            .expect("the mirrored work must read back under its own id");
        assert_eq!(row.title, "Designing Data-Intensive Applications");
    }
}
