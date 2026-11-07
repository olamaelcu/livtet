//! Pairing decisions and book offers, independent of who is deciding.
//!
//! The daemon answers these over JSON-RPC from its own UI. An embedded endpoint
//! — the mobile FFI — answers the same decisions from the phone's screen. Both
//! end up writing the same rows, so the logic lives here once and each host
//! only translates its own transport into these calls.
//!
//! Everything below takes a [`WireState`]: the pending pairing rows, the paired
//! devices table, and the change log are all it needs.

use livtet_data::client_entities::{paired_devices, pending_pairings};
use livtet_data::entities::{authors, digital_inventory, editions, work_authors, works};
use livtet_data::orm::sea_query::OnConflict;
use livtet_data::orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use livtet_types::{DbId, DeviceType, PairingStatus};
use thiserror::Error;

use crate::pairing::apply_pairing_decision;
use crate::wire_state::WireState;

/// Why a decision or an offer could not be made.
#[derive(Debug, Error)]
pub enum ClaimError {
    /// No pending claim names this node.
    #[error("unknown pairing node id: {0}")]
    UnknownClaim(String),
    /// The device id in the request does not parse.
    #[error("invalid device id: {0}")]
    InvalidDevice(String),
    /// The edition id in the request does not parse.
    #[error("invalid edition id: {0}")]
    InvalidEdition(String),
    /// The device has not been paired, so it may not be offered a book.
    #[error("device is not paired: {0}")]
    NotPaired(String),
    /// The edition does not exist, or has no file to send.
    #[error("{0}")]
    NoFile(String),
    /// The database refused a read or write.
    #[error("{0}")]
    Db(String),
}

impl From<livtet_data::orm::DbErr> for ClaimError {
    fn from(error: livtet_data::orm::DbErr) -> Self {
        Self::Db(error.to_string())
    }
}

/// The device and the token an approval produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimApproval {
    /// The claimant's device id, as recorded on its row.
    pub device_id: String,
    /// The token this side minted. Inbound requests must present it.
    pub session_token: String,
}

/// One claim waiting on a human decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingClaim {
    /// The claimant's iroh node id: the key a decision is addressed to.
    pub node_id: String,
    /// The claimant's device id, when it sent one.
    pub device_id: Option<String>,
    /// The name it advertises.
    pub name: Option<String>,
}

/// A book offered to a paired device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedOffer {
    /// The offer's id, which the peer echoes in its decision.
    pub offer_id: String,
    /// The inventory row whose bytes the peer will fetch.
    pub inventory_id: String,
}

/// Approve the claim `node_id` filed, minting the token it will present.
///
/// The row is written before the claimant is answered, so a poll that arrives
/// while the broadcast is still in flight cannot observe a claim that is
/// approved but not paired.
pub async fn approve_claim(
    state: &WireState,
    node_id: &str,
) -> Result<ClaimApproval, ClaimError> {
    let db = state.engine.db();

    let pending = pending_pairings::Entity::find_by_id(node_id.to_string())
        .one(db)
        .await?
        .ok_or_else(|| ClaimError::UnknownClaim(node_id.to_string()))?;

    let session_token = ulid::Ulid::generate().to_string();
    let device_id = pending.device_id.unwrap_or_else(DbId::new);
    let decided_device_id = device_id.to_string();

    let model = paired_devices::ActiveModel {
        device_id: Set(device_id),
        name: Set(pending.device_name.clone()),
        // The peer's node id is its address; there is no HTTP endpoint to
        // store. Legacy rows keep whatever they recorded.
        listen_on: Set(None),
        device_type_id: Set(pending.device_type_id),
        paired_at: Set(primitive_now()),
        last_sync_at: Set(None),
        // Approving lets the claimant dial us, so the row is dial-addressable in
        // principle; the token recorded below is the one *we* issue and check
        // their requests against. Pulling from them needs their token instead,
        // which only a claim can supply.
        outbound: Set(true),
    };
    paired_devices::Entity::insert(model)
        .on_conflict(
            OnConflict::column(paired_devices::Column::DeviceId)
                .update_columns([
                    paired_devices::Column::Name,
                    paired_devices::Column::ListenOn,
                    paired_devices::Column::DeviceTypeId,
                    paired_devices::Column::PairedAt,
                    paired_devices::Column::Outbound,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;

    // `session_token` and `node_id` are added by later client migrations and are
    // not on the entity, so they are written with a targeted statement.
    livtet_data::sql::query(
        "UPDATE paired_devices SET session_token = ?, node_id = ? WHERE device_id = ?",
    )
    .bind(session_token.clone())
    .bind(node_id.to_string())
    .bind(device_id.to_bytes().to_vec())
    .execute(&state.pool)
    .await
    .map_err(|error| ClaimError::Db(error.to_string()))?;

    let mut pending_model: pending_pairings::ActiveModel = pending.into();
    pending_model.status_id = Set(Some(DbId::from(PairingStatus::Approved)));
    pending_model.update(db).await?;

    let _ = apply_pairing_decision(
        &state.pair_waiters,
        &state.pair_decisions,
        node_id,
        "approved",
        &session_token,
        Some(decided_device_id.as_str()),
    )
    .await;

    Ok(ClaimApproval {
        device_id: decided_device_id,
        session_token,
    })
}

/// Reject the claim `node_id` filed. The claimant's next poll learns why.
pub async fn reject_claim(state: &WireState, node_id: &str) -> Result<(), ClaimError> {
    let db = state.engine.db();

    let pending = pending_pairings::Entity::find_by_id(node_id.to_string())
        .one(db)
        .await?
        .ok_or_else(|| ClaimError::UnknownClaim(node_id.to_string()))?;

    let decided_device_id = pending.device_id.map(|id| id.to_string());

    let mut model: pending_pairings::ActiveModel = pending.into();
    model.status_id = Set(Some(DbId::from(PairingStatus::Rejected)));
    model.update(db).await?;

    let _ = apply_pairing_decision(
        &state.pair_waiters,
        &state.pair_decisions,
        node_id,
        "rejected",
        "",
        decided_device_id.as_deref(),
    )
    .await;

    Ok(())
}

/// The claims nobody has decided yet, newest first.
pub async fn pending_claims(state: &WireState) -> Result<Vec<PendingClaim>, ClaimError> {
    let pending_id = DbId::from(PairingStatus::Pending);
    let rows = pending_pairings::Entity::find()
        .all(state.engine.db())
        .await?;

    Ok(rows
        .into_iter()
        .filter(|row| row.status_id == Some(pending_id))
        .map(|row| PendingClaim {
            node_id: row.token,
            device_id: row.device_id.map(|id| id.to_string()),
            name: row.device_name,
        })
        .collect())
}

/// Offer `edition_id`'s file to a paired device.
///
/// The offer is a `change_log` row, so it rides the version cursor the receiver
/// already pulls; the catalog rows are touched so the same pull carries the
/// metadata the receiver needs to show the book.
pub async fn offer_book(
    state: &WireState,
    edition_id: &str,
    device_id: &str,
) -> Result<CreatedOffer, ClaimError> {
    let device_id = parse_id(device_id).ok_or_else(|| ClaimError::InvalidDevice(device_id.into()))?;
    let edition_id =
        parse_id(edition_id).ok_or_else(|| ClaimError::InvalidEdition(edition_id.into()))?;

    let db = state.engine.db();
    if paired_devices::Entity::find_by_id(device_id)
        .one(db)
        .await?
        .is_none()
    {
        return Err(ClaimError::NotPaired(device_id.to_string()));
    }

    let edition = editions::Entity::find_by_id(edition_id)
        .one(db)
        .await?
        .ok_or_else(|| ClaimError::InvalidEdition(edition_id.to_string()))?;
    let inventory = digital_inventory::Entity::find()
        .filter(digital_inventory::Column::EditionId.eq(edition_id))
        .one(db)
        .await?
        .ok_or_else(|| ClaimError::NoFile("this edition has no file to send".to_string()))?;
    let Some(file_path) = inventory.file_path.as_deref() else {
        return Err(ClaimError::NoFile(
            "this edition has no file to send".to_string(),
        ));
    };
    if !camino::Utf8Path::new(file_path).is_file() {
        return Err(ClaimError::NoFile(
            "the book file is missing from disk".to_string(),
        ));
    }

    let offer_id = ulid::Ulid::generate().to_string();
    let inventory_id = inventory.id.to_hex();

    // The receiver is asked about this book before any catalog row lands, so
    // the title and authors cannot come from its own tables: the offer carries
    // them.
    let work = works::Entity::find_by_id(edition.work_id)
        .one(db)
        .await?
        .ok_or_else(|| ClaimError::InvalidEdition(edition_id.to_string()))?;
    let title = edition
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| work.title.clone());
    let mut authors: Vec<String> = authors::Entity::find()
        .filter(
            authors::Column::Id.is_in(
                work_authors::Entity::find()
                    .filter(work_authors::Column::WorkId.eq(edition.work_id))
                    .all(db)
                    .await?
                    .into_iter()
                    .map(|link| link.author_id)
                    .collect::<Vec<_>>(),
            ),
        )
        .all(db)
        .await?
        .into_iter()
        .map(|author| author.name)
        .collect();
    authors.sort();

    let payload = serde_json::json!({
        "offer_id": offer_id,
        "inventory_id": inventory_id,
        "edition_id": edition_id.to_hex(),
        "work_id": edition.work_id.to_hex(),
        "to_device_id": device_id.to_string(),
        "title": title,
        "authors": authors,
        "file_name": camino::Utf8Path::new(file_path)
            .file_name()
            .unwrap_or("book"),
        "file_size_bytes": inventory.file_size_bytes,
        "file_hash": inventory.file_hash,
        "has_cover": inventory
            .cover_path
            .as_deref()
            .is_some_and(|path| camino::Utf8Path::new(path).is_file()),
    })
    .to_string();

    // An offer is a change-log row so it rides the same version cursor the
    // receiver already pulls. `OFFER` is deliberately not an apply operation:
    // the receiver handles the row itself and must not replay it as a table
    // mutation.
    livtet_data::sql::query(
        "INSERT INTO change_log (entity_type, entity_id, operation, version, payload, changed_at, device_id) \
         VALUES ('file_offer', ?, 'OFFER', (SELECT COALESCE(MAX(version), 0) + 1 FROM change_log), ?, datetime('now'), ?)",
    )
    .bind(inventory_id.clone())
    .bind(payload)
    .bind(state.device_id.clone())
    .execute(&state.pool)
    .await
    .map_err(|error| ClaimError::Db(format!("could not record the file offer: {error}")))?;

    // Touching `updated_at` makes the existing audit triggers emit full
    // UPDATE payloads for the work, edition, and file, so a normal pull also
    // carries the catalog record and cover path.
    livtet_data::sql::query(
        "UPDATE digital_inventory SET updated_at = datetime('now') WHERE id = ?",
    )
    .bind(inventory.id.to_bytes().to_vec())
    .execute(&state.pool)
    .await
    .map_err(stage_error)?;
    livtet_data::sql::query("UPDATE editions SET updated_at = datetime('now') WHERE id = ?")
        .bind(edition_id.to_bytes().to_vec())
        .execute(&state.pool)
        .await
        .map_err(stage_error)?;
    livtet_data::sql::query(
        "UPDATE works SET updated_at = datetime('now') WHERE id = (SELECT work_id FROM editions WHERE id = ?)",
    )
    .bind(edition_id.to_bytes().to_vec())
    .execute(&state.pool)
    .await
    .map_err(stage_error)?;

    Ok(CreatedOffer {
        offer_id,
        inventory_id,
    })
}

/// Record the device type a claim carried, so a paired-devices list can group it.
pub async fn set_device_type(
    state: &WireState,
    device_id: DbId,
    device_type: &str,
) -> Result<(), ClaimError> {
    let type_id = DbId::from(DeviceType::from_tag(device_type));
    livtet_data::sql::query("UPDATE paired_devices SET device_type_id = ? WHERE device_id = ?")
        .bind(type_id.to_bytes().to_vec())
        .bind(device_id.to_bytes().to_vec())
        .execute(&state.pool)
        .await
        .map_err(|error| ClaimError::Db(error.to_string()))?;
    Ok(())
}

/// Parse a device or catalog id from either of its textual forms.
fn parse_id(raw: &str) -> Option<DbId> {
    raw.parse::<DbId>().ok().or_else(|| DbId::from_hex(raw).ok())
}

fn stage_error(error: impl std::fmt::Display) -> ClaimError {
    ClaimError::Db(format!("could not stage the book metadata: {error}"))
}

/// Now, in the form the entity timestamps use.
fn primitive_now() -> time::PrimitiveDateTime {
    let now = time::OffsetDateTime::now_utc();
    time::PrimitiveDateTime::new(now.date(), now.time())
}
