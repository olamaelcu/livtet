use livtet_core::DbId;
use livtet_data::sql::{self, AssertSqlSafe, Error, Row, SqlitePool};

use crate::progress_entry::ProgressEntry;

pub(crate) async fn resolve_urn_to_edition_id(
    pool: &SqlitePool,
    urn: &str,
) -> Result<Option<String>, Error> {
    let identifier_hex: Option<String> = sql::query_as(AssertSqlSafe(
        "SELECT lower(hex(id)) FROM identifiers WHERE value = ?",
    ))
    .bind(urn)
    .fetch_optional(pool)
    .await?
    .map(|(h,): (String,)| h);

    let Some(ref ident_hex) = identifier_hex else {
        return Ok(None);
    };

    let edition_hex: Option<String> = sql::query_as(AssertSqlSafe(
        "SELECT lower(hex(edition_id)) FROM edition_identifiers \
         WHERE lower(hex(identifier_id)) = ?",
    ))
    .bind(ident_hex)
    .fetch_optional(pool)
    .await?
    .map(|(h,): (String,)| h);

    if let Some(ed) = edition_hex {
        return Ok(Some(ed));
    }

    let work_hex: Option<String> = sql::query_as(AssertSqlSafe(
        "SELECT lower(hex(work_id)) FROM work_identifiers \
         WHERE lower(hex(identifier_id)) = ?",
    ))
    .bind(ident_hex)
    .fetch_optional(pool)
    .await?
    .map(|(h,): (String,)| h);

    let Some(ref work) = work_hex else {
        return Ok(None);
    };

    let edition_hex: Option<String> = sql::query_as(AssertSqlSafe(
        "SELECT lower(hex(id)) FROM editions WHERE lower(hex(work_id)) = ? LIMIT 1",
    ))
    .bind(work)
    .fetch_optional(pool)
    .await?
    .map(|(h,): (String,)| h);

    Ok(edition_hex)
}

pub(crate) async fn get_edition_info_query(
    pool: &SqlitePool,
    edition_id_hex: &str,
) -> Result<Option<serde_json::Value>, Error> {
    let row = sql::query(AssertSqlSafe(
        "SELECT lower(hex(id)), lower(hex(work_id)), lower(hex(group_id)), \
         title, published_date, lower(hex(format_id)), lower(hex(language_id)), \
         notes, description, created_at, updated_at \
         FROM editions WHERE lower(hex(id)) = ?",
    ))
    .bind(edition_id_hex.to_lowercase())
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => {
            use sql::Row;
            let json = serde_json::json!({
                "id": row.try_get::<String, _>(0).unwrap_or_default(),
                "work_id": row.try_get::<String, _>(1).unwrap_or_default(),
                "group_id": row.try_get::<Option<String>, _>(2).unwrap_or(None),
                "title": row.try_get::<Option<String>, _>(3).unwrap_or(None),
                "published_date": row.try_get::<Option<String>, _>(4).unwrap_or(None),
                "format_id": row.try_get::<Option<String>, _>(5).unwrap_or(None),
                "language_id": row.try_get::<Option<String>, _>(6).unwrap_or(None),
                "notes": row.try_get::<Option<String>, _>(7).unwrap_or(None),
                "description": row.try_get::<Option<String>, _>(8).unwrap_or(None),
                "created_at": row.try_get::<Option<String>, _>(9).unwrap_or(None),
                "updated_at": row.try_get::<Option<String>, _>(10).unwrap_or(None),
            });
            Ok(Some(json))
        }
        None => Ok(None),
    }
}

pub(crate) async fn get_edition_identifiers_query(
    pool: &SqlitePool,
    edition_id_hex: &str,
) -> Result<Vec<String>, Error> {
    let mut urns = Vec::new();

    {
        let rows = sql::query(AssertSqlSafe(
            "SELECT i.value FROM identifiers i \
             JOIN edition_identifiers ei ON i.id = ei.identifier_id \
             WHERE lower(hex(ei.edition_id)) = ?",
        ))
        .bind(edition_id_hex.to_lowercase())
        .fetch_all(pool)
        .await?;
        for row in &rows {
            if let Ok(v) = row.try_get::<String, _>(0) {
                urns.push(v);
            }
        }
    }

    {
        let rows = sql::query(AssertSqlSafe(
            "SELECT i.value FROM identifiers i \
             JOIN work_identifiers wi ON i.id = wi.identifier_id \
             JOIN editions e ON wi.work_id = e.work_id \
             WHERE lower(hex(e.id)) = ?",
        ))
        .bind(edition_id_hex.to_lowercase())
        .fetch_all(pool)
        .await?;
        for row in &rows {
            if let Ok(v) = row.try_get::<String, _>(0) {
                urns.push(v);
            }
        }
    }

    Ok(urns)
}

pub(crate) async fn find_default_format_for_edition(
    pool: &SqlitePool,
    edition_id: &str,
) -> Result<Option<String>, Error> {
    let row: Option<(Option<String>,)> = sql::query_as(AssertSqlSafe(
        "SELECT lower(hex(format_id)) FROM editions WHERE lower(hex(id)) = ?",
    ))
    .bind(edition_id.to_lowercase())
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(f,)| f))
}

pub(crate) async fn fetch_progress_query(
    pool: &SqlitePool,
    urn: &str,
) -> Result<Option<ProgressEntry>, Error> {
    let edition_id = match resolve_urn_to_edition_id(pool, urn).await? {
        Some(eid) => eid,
        None => return Ok(None),
    };

    let format_id = match find_default_format_for_edition(pool, &edition_id).await? {
        Some(fid) => fid,
        None => return Ok(None),
    };

    let row = sql::query(AssertSqlSafe(
        "SELECT lower(hex(id)), lower(hex(edition_id)), lower(hex(format_id)), \
         progress, last_location, total_reading_time_secs, created_at \
         FROM reading_progress \
         WHERE lower(hex(edition_id)) = ? AND lower(hex(format_id)) = ?",
    ))
    .bind(edition_id.to_lowercase())
    .bind(format_id.to_lowercase())
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => {
            use sql::Row;
            let id_hex: String = row.try_get(0).unwrap_or_default();
            let eid_hex: String = row.try_get(1).unwrap_or_default();
            let fid_hex: String = row.try_get(2).unwrap_or_default();
            let progress: f64 = row.try_get(3).unwrap_or(0.0);
            let last_location: Option<String> = row.try_get(4).ok().flatten();
            let total_secs: i64 = row.try_get(5).unwrap_or(0);
            let created_at: Option<String> = row.try_get(6).ok().flatten();

            let id = parse_db_id_hex(&id_hex);
            let eid = parse_db_id_hex(&eid_hex);
            let fid = parse_db_id_hex(&fid_hex);

            Ok(Some(ProgressEntry {
                id,
                edition_id: eid,
                format_id: fid,
                progress,
                last_location,
                total_reading_time_secs: total_secs,
                updated_at: created_at,
            }))
        }
        None => Ok(None),
    }
}

pub(crate) async fn upsert_progress_query(
    pool: &SqlitePool,
    urn: &str,
    progress: f64,
    last_location: Option<String>,
    total_reading_time_secs: i64,
) -> Result<(String, String), Error> {
    let edition_id = match resolve_urn_to_edition_id(pool, urn).await? {
        Some(eid) => eid,
        None => {
            return Err(Error::Protocol(format!("URN not found: {urn}")));
        }
    };

    let format_id = match find_default_format_for_edition(pool, &edition_id).await? {
        Some(fid) => fid,
        None => {
            return Err(Error::Protocol(format!(
                "no format for edition {edition_id}"
            )));
        }
    };

    let new_id = DbId::new();
    let edition_bytes = parse_db_id_hex(&edition_id).to_bytes().to_vec();
    let format_bytes = parse_db_id_hex(&format_id).to_bytes().to_vec();

    sql::query(AssertSqlSafe(
        "INSERT INTO reading_progress (id, edition_id, format_id, progress, \
         progress_unit, last_location, total_reading_time_secs, created_at) \
         VALUES (?, ?, ?, ?, 'percentage', ?, ?, datetime('now')) \
         ON CONFLICT(edition_id, format_id) DO UPDATE SET \
         progress = excluded.progress, \
         last_location = excluded.last_location, \
         total_reading_time_secs = excluded.total_reading_time_secs",
    ))
    .bind(new_id.to_bytes().to_vec())
    .bind(edition_bytes)
    .bind(format_bytes)
    .bind(progress)
    .bind(last_location)
    .bind(total_reading_time_secs)
    .execute(pool)
    .await?;

    Ok((edition_id, format_id))
}

pub(crate) async fn get_plugin_setting(
    pool: &SqlitePool,
    plugin_id: &str,
    key: &str,
) -> Result<Option<String>, Error> {
    let row: Option<(String,)> = sql::query_as(AssertSqlSafe(
        "SELECT value_json FROM plugin_settings WHERE plugin_id = ? AND setting_key = ?",
    ))
    .bind(plugin_id)
    .bind(key)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(v,)| v))
}

pub(crate) async fn set_plugin_setting(
    pool: &SqlitePool,
    plugin_id: &str,
    key: &str,
    value: &str,
) -> Result<(), Error> {
    let updated = sql::query(AssertSqlSafe(
        "UPDATE plugin_settings SET value_json = ?, updated_at = datetime('now') \
         WHERE plugin_id = ? AND setting_key = ?",
    ))
    .bind(value)
    .bind(plugin_id)
    .bind(key)
    .execute(pool)
    .await?;

    if updated.rows_affected() > 0 {
        return Ok(());
    }

    let id = DbId::new();
    sql::query(AssertSqlSafe(
        "INSERT INTO plugin_settings (id, plugin_id, setting_key, value_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))",
    ))
    .bind(id.to_bytes().to_vec())
    .bind(plugin_id)
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;

    Ok(())
}

fn parse_db_id_hex(hex_str: &str) -> DbId {
    hex::decode(hex_str)
        .ok()
        .and_then(|b| <[u8; 16]>::try_from(b).ok())
        .map(DbId::from_bytes)
        .unwrap_or_default()
}
