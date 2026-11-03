//! Materialise remote sync changes idempotently.
//!
//! Pushed changes are applied inside the push transaction.  Table and
//! column names come from the compile-time entity enums, never from the
//! payload; every value is bound as a statement parameter.

use livtet_data::orm::entity::prelude::IdenStatic;
use livtet_data::orm::{
    ColumnTrait, ColumnType, ConnectionTrait, DatabaseBackend, DatabaseTransaction, EntityTrait,
    Iterable, PrimaryKeyToColumn, Statement, Value,
};
use livtet_types::DbId;
use serde_json::{Map, Value as Json};

use crate::types::{SyncChange, SyncError};

/// Apply one pushed change.  Thin wrapper over the macro-generated
/// [`crate::types::syncable_entity::dispatch_apply`].
pub(crate) async fn apply(txn: &DatabaseTransaction, change: &SyncChange) -> Result<(), SyncError> {
    crate::types::syncable_entity::dispatch_apply(txn, change).await
}

/// Apply a change to the table backing `E`.
pub(crate) async fn apply_one<E>(
    txn: &DatabaseTransaction,
    table: &'static str,
    change: &SyncChange,
) -> Result<(), SyncError>
where
    E: EntityTrait,
    E::Column: ColumnTrait + Iterable,
    E::PrimaryKey: PrimaryKeyToColumn + Iterable,
{
    let operation = change.operation.to_ascii_uppercase();
    let pk_names: Vec<String> = E::PrimaryKey::iter()
        .map(|pk| pk.into_column().as_str().to_string())
        .collect();

    match operation.as_str() {
        "DELETE" => {
            let resolved = resolve_primary_keys::<E>(change, None)?;
            let mut values = Vec::with_capacity(resolved.len());
            for (column, json) in E::PrimaryKey::iter()
                .map(|pk| pk.into_column())
                .zip(resolved.iter())
            {
                values.push(json_to_sql(
                    column.def().get_column_type(),
                    json,
                    &change.entity_type,
                )?);
            }
            let where_clause = pk_names
                .iter()
                .map(|name| format!("{name} = ?"))
                .collect::<Vec<_>>()
                .join(" AND ");
            let sql = format!("DELETE FROM {table} WHERE {where_clause}");
            let stmt = Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values);
            txn.execute_raw(stmt).await?;
            Ok(())
        }
        "INSERT" | "UPDATE" => {
            let payload = payload_object(change)?;
            let resolved = resolve_primary_keys::<E>(change, Some(&payload))?;
            let mut pk_values: Map<String, Json> = Map::new();
            for (name, value) in E::PrimaryKey::iter()
                .map(|pk| pk.into_column().as_str().to_string())
                .zip(resolved)
            {
                pk_values.insert(name, value);
            }

            let mut columns: Vec<String> = Vec::new();
            let mut values: Vec<Value> = Vec::new();
            for column in E::Column::iter() {
                let name = column.as_str().to_string();
                let json = match payload.get(&name) {
                    Some(value) => Some(value),
                    None => pk_values.get(&name),
                };
                let Some(json) = json else { continue };
                columns.push(name.clone());
                values.push(json_to_sql(
                    column.def().get_column_type(),
                    json,
                    &change.entity_type,
                )?);
            }

            if columns.is_empty() {
                return Ok(());
            }

            let placeholders = columns.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
            let updates = columns
                .iter()
                .filter(|name| !pk_names.contains(name))
                .map(|name| format!("{name} = excluded.{name}"))
                .collect::<Vec<_>>();
            let conflict = if updates.is_empty() {
                "DO NOTHING".to_string()
            } else {
                format!("DO UPDATE SET {}", updates.join(", "))
            };
            let sql = format!(
                "INSERT INTO {table} ({}) VALUES ({placeholders}) ON CONFLICT ({}) {conflict}",
                columns.join(", "),
                pk_names.join(", "),
            );
            let stmt = Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values);
            txn.execute_raw(stmt).await?;
            Ok(())
        }
        _ => Err(SyncError::InvalidOperation {
            entity_type: change.entity_type.clone(),
            operation: change.operation.clone(),
        }),
    }
}

/// Parse the payload of an INSERT/UPDATE into a JSON object.
fn payload_object(change: &SyncChange) -> Result<Map<String, Json>, SyncError> {
    let parsed: Json =
        serde_json::from_str(&change.payload).map_err(|err| SyncError::InvalidPayload {
            entity_type: change.entity_type.clone(),
            message: err.to_string(),
        })?;
    match parsed {
        Json::Object(map) => Ok(map),
        other => Err(SyncError::InvalidPayload {
            entity_type: change.entity_type.clone(),
            message: format!("expected a JSON object, got {other}"),
        }),
    }
}

/// Resolve one JSON value per primary-key column, in `E::PrimaryKey`
/// order.  Payload values win; otherwise the value comes from
/// `change.entity_id` (a bare id for single-PK tables, a JSON object for
/// composite keys).
fn resolve_primary_keys<E>(
    change: &SyncChange,
    payload: Option<&Map<String, Json>>,
) -> Result<Vec<Json>, SyncError>
where
    E: EntityTrait,
    E::Column: ColumnTrait,
    E::PrimaryKey: PrimaryKeyToColumn + Iterable,
{
    let pk_names: Vec<String> = E::PrimaryKey::iter()
        .map(|pk| pk.into_column().as_str().to_string())
        .collect();
    let single = pk_names.len() == 1;
    let entity_id: Option<Json> = serde_json::from_str(&change.entity_id).ok();

    let mut resolved = Vec::with_capacity(pk_names.len());
    for name in pk_names {
        if let Some(value) = payload.and_then(|p| p.get(&name)) {
            resolved.push(value.clone());
            continue;
        }
        let from_entity_id = match &entity_id {
            Some(Json::Object(map)) => map.get(&name).cloned(),
            Some(Json::String(id)) if single => Some(Json::String(id.clone())),
            _ if single => Some(Json::String(change.entity_id.clone())),
            _ => None,
        };
        match from_entity_id {
            Some(value) => resolved.push(value),
            None => {
                return Err(SyncError::InvalidEntityId {
                    entity_type: change.entity_type.clone(),
                    message: format!("missing key `{name}` in entity_id `{}`", change.entity_id),
                });
            }
        }
    }
    Ok(resolved)
}

/// Convert a payload value to a bound [`Value`] based on the column type.
fn json_to_sql(
    column_type: &ColumnType,
    value: &Json,
    entity_type: &str,
) -> Result<Value, SyncError> {
    if value.is_null() {
        return Ok(match column_type {
            ColumnType::Binary(_) | ColumnType::VarBinary(_) | ColumnType::Blob => {
                Value::Bytes(None)
            }
            ColumnType::TinyInteger
            | ColumnType::SmallInteger
            | ColumnType::Integer
            | ColumnType::BigInteger
            | ColumnType::TinyUnsigned
            | ColumnType::SmallUnsigned
            | ColumnType::Unsigned
            | ColumnType::BigUnsigned => Value::BigInt(None),
            ColumnType::Float | ColumnType::Double | ColumnType::Decimal(_) => Value::Double(None),
            ColumnType::Boolean => Value::Bool(None),
            _ => Value::String(None),
        });
    }

    let invalid = || SyncError::InvalidPayload {
        entity_type: entity_type.to_string(),
        message: format!("cannot bind {value} to {column_type:?}"),
    };

    match column_type {
        ColumnType::Binary(_) | ColumnType::VarBinary(_) | ColumnType::Blob => {
            let text = value.as_str().ok_or_else(invalid)?;
            let id = match DbId::from_hex(text) {
                Ok(id) => id,
                Err(_) => text.parse::<DbId>().map_err(|_| invalid())?,
            };
            Ok(Value::Bytes(Some(id.to_bytes().to_vec())))
        }
        ColumnType::TinyInteger
        | ColumnType::SmallInteger
        | ColumnType::Integer
        | ColumnType::BigInteger
        | ColumnType::TinyUnsigned
        | ColumnType::SmallUnsigned
        | ColumnType::Unsigned
        | ColumnType::BigUnsigned => {
            let n = value
                .as_i64()
                .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(invalid)?;
            Ok(Value::BigInt(Some(n)))
        }
        ColumnType::Float | ColumnType::Double | ColumnType::Decimal(_) => {
            let n = value
                .as_f64()
                .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(invalid)?;
            Ok(Value::Double(Some(n)))
        }
        ColumnType::Boolean => {
            let b = value
                .as_bool()
                .or_else(|| match value.as_i64() {
                    Some(0) => Some(false),
                    Some(1) => Some(true),
                    _ => None,
                })
                .ok_or_else(invalid)?;
            Ok(Value::Bool(Some(b)))
        }
        _ => Ok(Value::String(Some(match value {
            Json::String(text) => text.clone(),
            other => other.to_string(),
        }))),
    }
}
