//! The iroh-facing side of sync: framed requests answered from the local engine.
//!
//! A token travels in the frame, and the caller's identity is the QUIC
//! connection's remote node id rather than a socket address. Authorization
//! therefore checks the token *against that node id*: a token issued to one
//! device is useless from another. Everything below the dispatch belongs to the
//! engine: `pull_changes`, `apply_changes`, the conflict list, and the file
//! paths.
//!
//! Every answered request is timed and recorded, so `requests.recent` can say
//! what a peer asked for and how long it took.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use camino::Utf8PathBuf;
use livtet_data::orm::{ConnectionTrait, DbBackend, EntityTrait, Statement};
use livtet_sync::{SyncChange, SyncStatus};
use livtet_sync_iroh::{
    ErrorCode, FileHeader, FileMetadata, Handler, Identity, Outcome, PROTOCOL_VERSION, Request,
    Response,
};
use livtet_types::DbId;

use crate::daemon::{DaemonState, discovery_node_id};
use crate::requests::{RequestRecord, record};

/// Build the handler the accept loop runs.
pub fn handler(state: Arc<DaemonState>) -> Handler {
    Arc::new(move |request, remote| {
        let state = Arc::clone(&state);
        Box::pin(async move { answer(&state, request, remote).await })
    })
}

/// A failure the caller should read, with the code that classifies it.
struct Failure {
    code: ErrorCode,
    message: String,
}

impl Failure {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Unknown, empty, and mismatched tokens all read the same way: the caller
    /// must not learn which of the three it sent.
    fn unauthorized() -> Self {
        Self::new(
            ErrorCode::Unauthorized,
            "the session token is not recognised",
        )
    }
}

/// One answer, plus the device it authenticated as for the request log.
struct Answered {
    outcome: Outcome,
    device_id: Option<String>,
}

impl Answered {
    fn reply(response: Response) -> Self {
        Self {
            outcome: Outcome::reply(response),
            device_id: None,
        }
    }

    fn failure(failure: Failure) -> Self {
        Self::reply(Response::Error {
            code: failure.code,
            message: failure.message,
        })
    }

    fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::failure(Failure::new(code, message))
    }

    fn file(header: FileHeader, path: Utf8PathBuf) -> Self {
        Self {
            outcome: Outcome::file(header, path),
            device_id: None,
        }
    }

    fn cover(header: FileHeader, path: Utf8PathBuf) -> Self {
        Self {
            outcome: Outcome::cover(header, path),
            device_id: None,
        }
    }

    /// Name the device this request authenticated as.
    fn with_device(mut self, device_id: impl Into<Option<String>>) -> Self {
        self.device_id = device_id.into();
        self
    }
}

impl From<Failure> for Answered {
    fn from(failure: Failure) -> Self {
        Self::failure(failure)
    }
}

/// Answer one frame and record it.
///
/// No path here panics: a bad request becomes an `error` frame, which the peer
/// surfaces as a classified failure rather than a dropped stream.
async fn answer(state: &Arc<DaemonState>, request: Request, remote: String) -> Outcome {
    let op = op_name(&request);
    let started = Instant::now();
    let answered = dispatch(state, request, remote.clone()).await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let outcome = outcome_label(&answered.outcome.response);
    let at = crate::daemon::now_string();

    state.requests_served.fetch_add(1, Ordering::Relaxed);
    *state.last_request_at.lock().await = Some(at.clone());
    record(
        &state.request_log,
        RequestRecord {
            op: op.to_string(),
            node_id: remote.clone(),
            device_id: answered.device_id.clone(),
            outcome: outcome.clone(),
            at: at.clone(),
            elapsed_ms,
        },
    );
    crate::requests::notify(
        &state.notifier,
        crate::rpc::method::NOTIFY_REQUEST_RECEIVED,
        serde_json::json!({
            "op": op,
            "node_id": remote,
            "device_id": answered.device_id,
            "outcome": outcome,
            "at": at,
            "elapsed_ms": elapsed_ms,
        }),
    );
    if let Some(completed) =
        sync_completed(&answered.outcome.response, answered.device_id.as_deref())
    {
        crate::requests::notify(
            &state.notifier,
            crate::rpc::method::NOTIFY_SYNC_COMPLETED,
            completed,
        );
    }

    answered.outcome
}

/// The operation tag a request is logged under.
fn op_name(request: &Request) -> &'static str {
    match request {
        Request::Identity => "identity",
        Request::Status { .. } => "status",
        Request::Changes { .. } => "changes",
        Request::PullFull { .. } => "pull_full",
        Request::Push { .. } => "push",
        Request::Pair { .. } => "pair",
        Request::PairStatus { .. } => "pair_status",
        Request::Conflicts { .. } => "conflicts",
        Request::ResolveConflict { .. } => "resolve_conflict",
        Request::File { .. } => "file",
        Request::FileInfo { .. } => "file_info",
        Request::Cover { .. } => "cover",
        Request::OfferDecision { .. } => "offer_decision",
    }
}

/// `ok` for a served request, `refused` for a declined pairing, and the
/// classified code for anything that failed.
fn outcome_label(response: &Response) -> String {
    match response {
        Response::Error { code, .. } => format!("error:{}", code.label()),
        Response::Refused { .. } => "refused".to_string(),
        _ => "ok".to_string(),
    }
}

/// A completed data operation, for the `sync.completed` notification.
///
/// Only an authenticated operation can complete a sync, so a peer without a
/// token never produces one.
fn sync_completed(response: &Response, device_id: Option<&str>) -> Option<serde_json::Value> {
    let device_id = device_id?;
    match response {
        Response::Changes(pull) => Some(serde_json::json!({
            "op": "changes",
            "device_id": device_id,
            "changes": pull.changes.len(),
            "has_more": pull.has_more,
            "latest_version": pull.latest_version,
        })),
        Response::Pushed(push) => Some(serde_json::json!({
            "op": "push",
            "device_id": device_id,
            "accepted": push.accepted,
            "conflicts": push.conflicts.len(),
            "latest_version": push.latest_version,
        })),
        Response::Full(dump) => Some(serde_json::json!({
            "op": "pull_full",
            "device_id": device_id,
            "version": dump.version,
        })),
        _ => None,
    }
}

/// Route one request to its handler.
async fn dispatch(state: &Arc<DaemonState>, request: Request, remote: String) -> Answered {
    // The connection's node id is the caller's identity. A frame claiming to be
    // someone else is refused before it reaches the engine.
    let self_node = discovery_node_id(state).await;
    if remote == self_node {
        return Answered::error(ErrorCode::BadRequest, "a device cannot sync with itself");
    }

    match request {
        Request::Identity => Answered::reply(Response::Identity(Identity {
            protocol: PROTOCOL_VERSION,
            node_id: self_node,
            device_id: state.device_id.clone(),
        })),

        Request::Status { token } => {
            let device = match authorize(state, &token, &remote).await {
                Ok(device) => device,
                Err(failure) => return failure.into(),
            };
            let response = match state.engine.get_latest_version().await {
                Ok(latest_version) => Response::Status(SyncStatus {
                    latest_version,
                    device_id: state.device_id.clone(),
                }),
                Err(error) => {
                    return Answered::error(ErrorCode::Internal, format!("status: {error}"));
                }
            };
            Answered::reply(response).with_device(device)
        }

        Request::Changes {
            token,
            since_version,
            limit,
        } => {
            let device_id = match authorize(state, &token, &remote).await {
                Ok(device_id) => device_id,
                Err(failure) => return failure.into(),
            };
            let limit = limit.unwrap_or(100);
            match state.engine.pull_changes(since_version, limit).await {
                Ok(mut response) => {
                    // Offers name their target; no other device may see them.
                    response
                        .changes
                        .retain(|change| change_is_for(change, &device_id));
                    Answered::reply(Response::Changes(response)).with_device(device_id)
                }
                Err(error) => Answered::error(ErrorCode::Internal, format!("changes: {error}")),
            }
        }

        Request::PullFull { token } => {
            let device = match authorize(state, &token, &remote).await {
                Ok(device) => device,
                Err(failure) => return failure.into(),
            };
            match state.engine.pull_full().await {
                Ok(dump) => Answered::reply(Response::Full(Box::new(dump))).with_device(device),
                Err(error) => Answered::error(ErrorCode::Internal, format!("pull-full: {error}")),
            }
        }

        Request::Push { token, changes } => {
            let device = match authorize(state, &token, &remote).await {
                Ok(device) => device,
                Err(failure) => return failure.into(),
            };
            match state.engine.push_changes(changes).await {
                Ok(response) => Answered::reply(Response::Pushed(response)).with_device(device),
                Err(error) => Answered::error(ErrorCode::Internal, format!("push: {error}")),
            }
        }

        Request::Pair {
            device_id,
            name,
            device_type,
        } => {
            if !crate::rate_limit::claim_limiter().allow(&remote) {
                return Answered::error(
                    ErrorCode::TooManyRequests,
                    "too many pairing attempts; try again in a minute",
                );
            }
            pair(state, &remote, device_id, name, device_type).await
        }

        Request::PairStatus { claim } => {
            // Not rate-limited: status polling is how a claimant survives a
            // dropped stream, and ten polls would exhaust the claim budget in
            // ten seconds. The claim id gates it instead.
            pair_status(state, &remote, claim).await
        }

        Request::Conflicts { token } => {
            let device = match authorize(state, &token, &remote).await {
                Ok(device) => device,
                Err(failure) => return failure.into(),
            };
            match state.engine.list_conflicts().await {
                Ok(conflicts) => {
                    Answered::reply(Response::Conflicts(conflicts)).with_device(device)
                }
                Err(error) => Answered::error(ErrorCode::Internal, format!("conflicts: {error}")),
            }
        }

        Request::ResolveConflict {
            token,
            id,
            resolution,
            merged_payload,
        } => {
            let device = match authorize(state, &token, &remote).await {
                Ok(device) => device,
                Err(failure) => return failure.into(),
            };
            if !matches!(resolution.as_str(), "local" | "remote" | "merged") {
                return Answered::error(
                    ErrorCode::BadRequest,
                    format!("unknown resolution: {resolution}"),
                );
            }
            match state
                .engine
                .resolve_conflict(id, &resolution, merged_payload.as_deref())
                .await
            {
                Ok(resolved) => Answered::reply(Response::Resolved(resolved)).with_device(device),
                Err(error) => Answered::error(ErrorCode::Internal, format!("resolve: {error}")),
            }
        }

        Request::File {
            token,
            inventory_id,
            offset,
            length,
            if_range,
        } => {
            file(
                state,
                token,
                &remote,
                inventory_id,
                offset,
                length,
                if_range,
            )
            .await
        }

        Request::FileInfo {
            token,
            inventory_id,
        } => file_info(state, token, &remote, inventory_id).await,

        Request::Cover {
            token,
            inventory_id,
        } => cover(state, token, &remote, inventory_id).await,

        Request::OfferDecision {
            token,
            offer_id,
            accepted,
        } => {
            let device_id = match authorize(state, &token, &remote).await {
                Ok(device_id) => device_id,
                Err(failure) => return failure.into(),
            };
            match offer_decision(state, &offer_id, &device_id, accepted).await {
                Ok(()) => Answered::reply(Response::OfferDecided).with_device(device_id),
                Err(failure) => Answered::failure(failure).with_device(device_id),
            }
        }
    }
}

/// Resolve a session token to the device it was issued to, and prove the
/// connection is that device.
///
/// `paired_devices.session_token` and `node_id` are created by client
/// migrations and are not on the SeaORM entity, so this is a targeted
/// statement. `node_id = ?` never matches a `NULL`, so a row that predates node
/// ids is unauthorized rather than trusted.
async fn authorize(state: &Arc<DaemonState>, token: &str, remote: &str) -> Result<String, Failure> {
    let token = token.trim();
    if token.is_empty() {
        return Err(Failure::unauthorized());
    }
    let statement = Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "SELECT device_id FROM paired_devices \
         WHERE session_token = ? AND node_id = ? LIMIT 1",
        [token.into(), remote.into()],
    );
    match state.engine.db().query_one_raw(statement).await {
        Ok(Some(row)) => {
            let bytes: Vec<u8> = row.try_get("", "device_id").map_err(|error| {
                Failure::new(ErrorCode::Internal, format!("device id: {error}"))
            })?;
            let arr: [u8; 16] = bytes
                .try_into()
                .map_err(|_| Failure::new(ErrorCode::Internal, "device id is not 16 bytes"))?;
            Ok(DbId::from_bytes(arr).to_string())
        }
        Ok(None) => Err(Failure::unauthorized()),
        Err(error) => Err(Failure::new(
            ErrorCode::Internal,
            format!("authorization: {error}"),
        )),
    }
}

/// Whether a change row is visible to the authenticated device.
fn change_is_for(change: &SyncChange, device_id: &str) -> bool {
    if change.entity_type != "file_offer" {
        return true;
    }
    serde_json::from_str::<serde_json::Value>(&change.payload)
        .ok()
        .and_then(|payload| {
            payload
                .get("to_device_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .is_none_or(|target| target == device_id)
}

/// Decode the 32-char hex inventory id the wire uses.
fn parse_inventory_id(raw: &str) -> Option<DbId> {
    let bytes = hex::decode(raw).ok()?;
    let arr: [u8; 16] = bytes.try_into().ok()?;
    Some(DbId::from_bytes(arr))
}

/// Read one inventory row's file or cover path.
async fn inventory_paths(
    state: &Arc<DaemonState>,
    inventory_id: &str,
) -> Result<(Option<String>, Option<String>), Failure> {
    use livtet_data::entities::digital_inventory;

    let id = parse_inventory_id(inventory_id)
        .ok_or_else(|| Failure::new(ErrorCode::BadRequest, "invalid inventory id"))?;
    let row = digital_inventory::Entity::find_by_id(id)
        .one(state.engine.db())
        .await
        .map_err(|error| Failure::new(ErrorCode::Internal, format!("inventory lookup: {error}")))?
        .ok_or_else(|| Failure::new(ErrorCode::NotFound, "no such inventory"))?;
    Ok((row.file_path, row.cover_path))
}

fn content_type_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "application/epub+zip"
    }
}

/// Size and etag of a file on disk.
///
/// The etag is the size and modification time in hex: it changes whenever the
/// bytes do, and a caller can send it back as `if_range` to continue a download
/// only while the file it started on is still there. Without a modification
/// time the size alone still detects an append or truncation.
async fn file_metadata(path: &str) -> Result<(u64, String), Failure> {
    let metadata = fs_err::tokio::metadata(path)
        .await
        .map_err(|_| Failure::new(ErrorCode::NotFound, "the file is not on disk"))?;
    let size = metadata.len();
    let etag = match metadata.modified() {
        Ok(modified) => {
            let nanos = modified
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            format!("{size:x}-{nanos:x}")
        }
        Err(_) => format!("{size:x}"),
    };
    Ok((size, etag))
}

/// Serve bytes of one inventory file, honouring a range and its precondition.
async fn file(
    state: &Arc<DaemonState>,
    token: String,
    remote: &str,
    inventory_id: String,
    offset: u64,
    length: Option<u64>,
    if_range: Option<String>,
) -> Answered {
    let device = match authorize(state, &token, remote).await {
        Ok(device) => device,
        Err(failure) => return failure.into(),
    };
    let (file_path, _) = match inventory_paths(state, &inventory_id).await {
        Ok(paths) => paths,
        Err(failure) => return failure.into(),
    };
    let Some(path) = file_path else {
        return Answered::error(ErrorCode::NotFound, "this inventory has no file");
    };
    let (total, etag) = match file_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(failure) => return failure.into(),
    };
    if offset > total {
        return Answered::error(
            ErrorCode::RangeNotSatisfiable { size: total },
            format!("offset {offset} is past the {total}-byte file"),
        )
        .with_device(Some(device));
    }

    // A matching `if_range` continues the partial file the caller started; a
    // stale (or absent) one means the range names a file the caller no longer
    // has, so the whole file is served — offset and length included.
    let (start, size) = if if_range.as_deref() == Some(etag.as_str()) {
        let available = total - offset;
        (offset, length.unwrap_or(available).min(available))
    } else {
        (0, total)
    };
    let header = FileHeader {
        content_type: content_type_for(&path).to_string(),
        offset: start,
        length: size,
        total,
        etag,
    };
    Answered::file(header, Utf8PathBuf::from(path)).with_device(Some(device))
}

/// Report one inventory file's size, content type and etag.
async fn file_info(
    state: &Arc<DaemonState>,
    token: String,
    remote: &str,
    inventory_id: String,
) -> Answered {
    let device = match authorize(state, &token, remote).await {
        Ok(device) => device,
        Err(failure) => return failure.into(),
    };
    let (file_path, _) = match inventory_paths(state, &inventory_id).await {
        Ok(paths) => paths,
        Err(failure) => return failure.into(),
    };
    let Some(path) = file_path else {
        return Answered::error(ErrorCode::NotFound, "this inventory has no file");
    };
    let (size, etag) = match file_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(failure) => return failure.into(),
    };
    Answered::reply(Response::FileInfo(FileMetadata {
        content_type: content_type_for(&path).to_string(),
        size,
        etag,
    }))
    .with_device(Some(device))
}

/// Serve the extracted cover for one inventory row.
async fn cover(
    state: &Arc<DaemonState>,
    token: String,
    remote: &str,
    inventory_id: String,
) -> Answered {
    let device = match authorize(state, &token, remote).await {
        Ok(device) => device,
        Err(failure) => return failure.into(),
    };
    let (_, cover_path) = match inventory_paths(state, &inventory_id).await {
        Ok(paths) => paths,
        Err(failure) => return failure.into(),
    };
    let Some(path) = cover_path else {
        return Answered::error(ErrorCode::NotFound, "this inventory has no cover");
    };
    let (total, etag) = match file_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(failure) => return failure.into(),
    };
    let header = FileHeader {
        content_type: content_type_for(&path).to_string(),
        offset: 0,
        length: total,
        total,
        etag,
    };
    Answered::cover(header, Utf8PathBuf::from(path)).with_device(Some(device))
}

/// Record a paired device's answer to an offer this desktop sent.
async fn offer_decision(
    state: &Arc<DaemonState>,
    offer_id: &str,
    device_id: &str,
    accepted: bool,
) -> Result<(), Failure> {
    let statement = Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "SELECT payload FROM change_log \
         WHERE entity_type = 'file_offer' AND json_extract(payload, '$.offer_id') = ? \
         LIMIT 1",
        [offer_id.into()],
    );
    let row = state
        .engine
        .db()
        .query_one_raw(statement)
        .await
        .map_err(|error| Failure::new(ErrorCode::Internal, format!("offer lookup: {error}")))?
        .ok_or_else(|| Failure::new(ErrorCode::NotFound, "no such offer"))?;
    let payload: String = row
        .try_get("", "payload")
        .map_err(|error| Failure::new(ErrorCode::Internal, format!("offer payload: {error}")))?;
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
    if payload
        .get("to_device_id")
        .and_then(serde_json::Value::as_str)
        != Some(device_id)
    {
        return Err(Failure::new(
            ErrorCode::Unauthorized,
            "the offer is addressed to another device",
        ));
    }

    (state.offer_sink)(crate::pairing::OfferDecision {
        offer_id: offer_id.to_string(),
        device_id: device_id.to_string(),
        accepted,
    });
    Ok(())
}

/// File a pairing claim and answer with its secret.
///
/// The claim is keyed by the connection's node id — never by a payload field —
/// so the row, the waiter and the eventual decision all belong to the identity
/// QUIC authenticated. A new claim replaces an earlier undecided one, and a row
/// that was decided can be claimed again: that just files a fresh request for a
/// fresh human decision.
async fn pair(
    state: &Arc<DaemonState>,
    remote: &str,
    device_id: String,
    name: String,
    device_type: String,
) -> Answered {
    use livtet_data::client_entities::pending_pairings;
    use livtet_data::orm::{ActiveModelTrait, EntityTrait, Set};
    use livtet_types::{DbId, DeviceType, PairingStatus};

    let peer_device_id = device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(&device_id).ok());
    let Some(peer_device_id) = peer_device_id else {
        return Answered::error(
            ErrorCode::BadRequest,
            format!("not a device id: {device_id}"),
        );
    };
    let Ok(desktop_id) = state.device_id.parse::<DbId>() else {
        return Answered::error(ErrorCode::Internal, "this device has no valid id");
    };

    let pending_id = DbId::from(PairingStatus::Pending);
    let now = time::OffsetDateTime::now_utc();
    let now_pdt = time::PrimitiveDateTime::new(now.date(), now.time());
    let expires = now + time::Duration::seconds(300);
    let expires_pdt = time::PrimitiveDateTime::new(expires.date(), expires.time());
    let device_type_id = Some(DbId::from(DeviceType::from_tag(&device_type)));

    let existing = match pending_pairings::Entity::find_by_id(remote.to_string())
        .one(state.engine.db())
        .await
    {
        Ok(row) => row,
        Err(error) => {
            return Answered::error(ErrorCode::Internal, format!("pairing table: {error}"));
        }
    };

    let write = match existing {
        // A decided ticket is final: replaying the claim must not reopen it.
        Some(row) if row.status_id != Some(pending_id) => {
            return Answered::reply(Response::Refused {
                reason: "this pairing was already decided".to_string(),
            });
        }
        Some(row) => {
            let mut model: pending_pairings::ActiveModel = row.into();
            model.desktop_id = Set(desktop_id);
            model.device_name = Set(Some(name.clone()));
            model.device_type_id = Set(device_type_id);
            model.device_id = Set(Some(peer_device_id));
            model.status_id = Set(Some(pending_id));
            model.listen_on = Set(None);
            model.origin_addr = Set(None);
            model.created_at = Set(now_pdt);
            model.expires_at = Set(expires_pdt);
            model.update(state.engine.db()).await.map(|_| ())
        }
        None => {
            let model = pending_pairings::ActiveModel {
                token: Set(remote.to_string()),
                desktop_id: Set(desktop_id),
                // No socket origin: the connection's node id is the identity
                // that matters, and it cannot be forged or spoofed.
                listen_on: Set(None),
                status_id: Set(Some(pending_id)),
                device_name: Set(Some(name.clone())),
                device_type_id: Set(device_type_id),
                device_id: Set(Some(peer_device_id)),
                origin_addr: Set(None),
                created_at: Set(now_pdt),
                expires_at: Set(expires_pdt),
            };
            model.insert(state.engine.db()).await.map(|_| ())
        }
    };
    if let Err(error) = write {
        return Answered::error(
            ErrorCode::Internal,
            format!("could not record the claim: {error}"),
        );
    }

    // The claim secret is what the claimant presents to resume; it is recorded
    // on the waiter so the decision can be keyed by it.
    let claim = ulid::Ulid::generate().to_string();
    let (sender, _receiver) = tokio::sync::broadcast::channel(1);
    {
        let mut waiters = state.pair_waiters.lock().await;
        waiters.insert(
            remote.to_string(),
            crate::pairing::PairingWaiter {
                claim: claim.clone(),
                sender,
            },
        );
    }
    crate::requests::notify(
        &state.notifier,
        crate::rpc::method::NOTIFY_PAIRING_REQUESTED,
        serde_json::json!({
            "node_id": remote,
            "device_id": &device_id,
            "name": &name,
        }),
    );
    tracing::info!(
        device = %device_id,
        name = %name,
        node_id = %remote,
        "pairing request over iroh",
    );

    Answered::reply(Response::PairingPending { claim })
}

/// Report what became of a claim this connection's node filed.
async fn pair_status(state: &Arc<DaemonState>, remote: &str, claim: String) -> Answered {
    let waiting = {
        let waiters = state.pair_waiters.lock().await;
        waiters
            .get(remote)
            .is_some_and(|waiter| waiter.claim == claim)
    };
    if waiting {
        return Answered::reply(Response::PairingPending { claim });
    }

    match crate::pairing::decided(&state.pair_decisions, &claim, remote).await {
        Some(decided) => Answered::reply(decided.response()),
        None => Answered::error(ErrorCode::NotFound, "no pairing claim with that id"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    use camino::Utf8PathBuf;
    use livtet_data::orm::SqlxSqliteConnector;
    use livtet_sync::SyncEngine;
    use livtet_sync_iroh::{Request, Response};
    use tokio::sync::Mutex;

    use super::*;
    use crate::config::ServerConfig;
    use crate::pairing::{apply_pairing_decision, new_pair_decisions, no_offer_decision_sink};
    use crate::requests::{new_request_log, recent};

    const LOCAL_DEVICE: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    const REMOTE_NODE: &str = "aa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899";
    const OTHER_NODE: &str = "bb11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899";
    /// A node id used only by the rate-limit test: the limiter is shared by the
    /// whole process, so a key another test touches would race it.
    const RATE_LIMITED_NODE: &str =
        "cc11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899";

    /// A daemon state over a throwaway database, with no endpoint bound.
    async fn test_state() -> (Arc<DaemonState>, Utf8PathBuf) {
        let root = temp_dir();
        let db_path = root.join("sync.db");
        let pool = livtet_data::connect_with_migrations_foreign_keys(
            db_path.as_ref(),
            &[livtet_data::Kind::Business, livtet_data::Kind::Client],
            false,
        )
        .await
        .expect("migrate");
        let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool.clone());
        let engine = SyncEngine::new(db, LOCAL_DEVICE.to_string());
        let state = Arc::new(DaemonState {
            config: ServerConfig {
                db_path,
                ..ServerConfig::default()
            },
            pool,
            engine,
            device_id: LOCAL_DEVICE.to_string(),
            device_name: Mutex::new(None),
            notifier: Arc::new(|_method: &str, _params: serde_json::Value| {}),
            started_at: time::OffsetDateTime::now_utc(),
            pair_waiters: crate::pairing::new_pair_waiters(),
            pair_decisions: new_pair_decisions(),
            offer_sink: no_offer_decision_sink(),
            write_gate: Mutex::new(()),
            lifecycle: Mutex::new(()),
            discovery: Mutex::new(None),
            advertiser: Mutex::new(None),
            request_log: new_request_log(),
            requests_served: Arc::new(AtomicU64::new(0)),
            last_request_at: Arc::new(Mutex::new(None)),
        });
        (state, root)
    }

    fn temp_dir() -> Utf8PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "livtet-sync-wire-{}-{nanos}-{seq}",
            std::process::id()
        ));
        fs_err::create_dir_all(&path).expect("temp dir");
        Utf8PathBuf::from_path_buf(path).expect("temp path is UTF-8")
    }

    async fn ask(state: &Arc<DaemonState>, request: Request, remote: &str) -> Response {
        answer(state, request, remote.to_string()).await.response
    }

    /// Seed a paired device holding `token`, bound to `node_id`.
    async fn seed_paired(
        state: &Arc<DaemonState>,
        device_id: DbId,
        node_id: Option<&str>,
        token: &str,
    ) {
        livtet_data::sql::query(
            "INSERT INTO paired_devices \
                 (device_id, name, listen_on, device_type_id, paired_at, last_sync_at, \
                  session_token, node_id, outbound) \
             VALUES (?, 'Peer', NULL, NULL, datetime('now'), NULL, ?, ?, 1)",
        )
        .bind(device_id.to_bytes().to_vec())
        .bind(token)
        .bind(node_id.map(str::to_string))
        .execute(&state.pool)
        .await
        .expect("seed paired device");
    }

    #[tokio::test]
    async fn a_token_only_authorizes_the_node_it_was_issued_to() {
        let (state, _root) = test_state().await;
        seed_paired(&state, DbId::new(), Some(REMOTE_NODE), "tok").await;

        let response = ask(
            &state,
            Request::Status {
                token: "tok".to_string(),
            },
            REMOTE_NODE,
        )
        .await;
        assert!(
            matches!(response, Response::Status(_)),
            "the issued node authenticates: {response:?}"
        );

        let response = ask(
            &state,
            Request::Status {
                token: "tok".to_string(),
            },
            OTHER_NODE,
        )
        .await;
        assert!(
            matches!(
                response,
                Response::Error {
                    code: ErrorCode::Unauthorized,
                    ..
                }
            ),
            "another node presenting the token is refused: {response:?}"
        );
    }

    #[tokio::test]
    async fn an_unknown_or_null_node_token_is_unauthorized() {
        let (state, _root) = test_state().await;
        // A row from before node ids were recorded has nothing to match.
        seed_paired(&state, DbId::new(), None, "legacy").await;

        for (token, remote) in [
            ("legacy", REMOTE_NODE),
            ("not-a-token", REMOTE_NODE),
            ("", REMOTE_NODE),
        ] {
            let response = ask(
                &state,
                Request::Status {
                    token: token.to_string(),
                },
                remote,
            )
            .await;
            match response {
                Response::Error { code, message } => {
                    assert_eq!(code, ErrorCode::Unauthorized, "token {token:?}");
                    assert_eq!(
                        message, "the session token is not recognised",
                        "every unauthorized answer reads the same"
                    );
                }
                other => panic!("expected an unauthorized error, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_claim_resumes_through_the_decision_map() {
        let (state, _root) = test_state().await;
        let peer_device = DbId::new();

        let response = ask(
            &state,
            Request::Pair {
                device_id: peer_device.to_string(),
                name: "Remote Phone".to_string(),
                device_type: "mobile".to_string(),
            },
            REMOTE_NODE,
        )
        .await;
        let claim = match response {
            Response::PairingPending { claim } => claim,
            other => panic!("a claim answers immediately: {other:?}"),
        };

        // The row is keyed by the connection identity, not a payload field.
        let (token, device): (String, Option<Vec<u8>>) =
            livtet_data::sql::query_as("SELECT token, device_id FROM pending_pairings")
                .fetch_one(&state.pool)
                .await
                .expect("pending row");
        assert_eq!(token, REMOTE_NODE);
        assert_eq!(device, Some(peer_device.to_bytes().to_vec()));

        let pending = ask(
            &state,
            Request::PairStatus {
                claim: claim.clone(),
            },
            REMOTE_NODE,
        )
        .await;
        assert!(
            matches!(&pending, Response::PairingPending { claim: pending } if pending == &claim),
            "a filed claim is pending: {pending:?}"
        );

        // Another node holding the secret learns nothing.
        let foreign = ask(
            &state,
            Request::PairStatus {
                claim: claim.clone(),
            },
            OTHER_NODE,
        )
        .await;
        assert!(
            matches!(
                foreign,
                Response::Error {
                    code: ErrorCode::NotFound,
                    ..
                }
            ),
            "a claim is bound to the node that filed it: {foreign:?}"
        );

        let told = apply_pairing_decision(
            &state.pair_waiters,
            &state.pair_decisions,
            REMOTE_NODE,
            "approved",
            "session-token",
            Some(&peer_device.to_string()),
        )
        .await;
        assert!(told, "the outstanding waiter is signalled");

        let decided = ask(
            &state,
            Request::PairStatus {
                claim: claim.clone(),
            },
            REMOTE_NODE,
        )
        .await;
        match decided {
            Response::Paired {
                device_id,
                session_token,
            } => {
                assert_eq!(device_id, peer_device.to_string());
                assert_eq!(session_token, "session-token");
            }
            other => panic!("the decision is resumable: {other:?}"),
        }
    }

    #[tokio::test]
    async fn pairing_calls_are_rate_limited() {
        let (state, _root) = test_state().await;
        let peer_device = DbId::new();

        // A node gets ten claims a minute; the eleventh is refused before the
        // table is touched again.
        for attempt in 0..10 {
            let response = ask(
                &state,
                Request::Pair {
                    device_id: peer_device.to_string(),
                    name: format!("Phone {attempt}"),
                    device_type: "mobile".to_string(),
                },
                RATE_LIMITED_NODE,
            )
            .await;
            assert!(
                matches!(response, Response::PairingPending { .. }),
                "claim {attempt} is within budget: {response:?}"
            );
        }

        let response = ask(
            &state,
            Request::Pair {
                device_id: peer_device.to_string(),
                name: "Phone 11".to_string(),
                device_type: "mobile".to_string(),
            },
            RATE_LIMITED_NODE,
        )
        .await;
        assert!(
            matches!(
                response,
                Response::Error {
                    code: ErrorCode::TooManyRequests,
                    ..
                }
            ),
            "the eleventh claim in the window is refused: {response:?}"
        );
    }

    /// Seed one inventory row whose file and cover are real files.
    async fn seed_inventory(
        state: &Arc<DaemonState>,
        directory: &camino::Utf8Path,
    ) -> (DbId, Vec<u8>) {
        let bytes = b"0123456789".to_vec();
        let file = directory.join("book.epub");
        fs_err::write(&file, &bytes).expect("write book");
        let cover = directory.join("cover.png");
        fs_err::write(&cover, b"cover-bytes").expect("write cover");

        let inventory = DbId::new();
        livtet_data::sql::query(
            "INSERT INTO digital_inventory \
                 (id, edition_id, file_path, cover_path, added_at) \
             VALUES (?, ?, ?, ?, datetime('now'))",
        )
        .bind(inventory.to_bytes().to_vec())
        .bind(DbId::new().to_bytes().to_vec())
        .bind(file.to_string())
        .bind(cover.to_string())
        .execute(&state.pool)
        .await
        .expect("seed inventory");
        (inventory, bytes)
    }

    #[tokio::test]
    async fn file_info_reports_the_size_and_an_etag() {
        let (state, root) = test_state().await;
        seed_paired(&state, DbId::new(), Some(REMOTE_NODE), "tok").await;
        let (inventory, bytes) = seed_inventory(&state, &root).await;

        let response = ask(
            &state,
            Request::FileInfo {
                token: "tok".to_string(),
                inventory_id: inventory.to_hex(),
            },
            REMOTE_NODE,
        )
        .await;
        let metadata = match response {
            Response::FileInfo(metadata) => metadata,
            other => panic!("file info: {other:?}"),
        };
        assert_eq!(metadata.size, bytes.len() as u64);
        assert_eq!(metadata.content_type, "application/epub+zip");
        assert!(
            metadata.etag.starts_with(&format!("{:x}-", bytes.len())),
            "the etag starts with the size: {}",
            metadata.etag
        );

        let missing = ask(
            &state,
            Request::FileInfo {
                token: "tok".to_string(),
                inventory_id: DbId::new().to_hex(),
            },
            REMOTE_NODE,
        )
        .await;
        assert!(matches!(
            missing,
            Response::Error {
                code: ErrorCode::NotFound,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn file_ranges_and_if_range_follow_the_etag() {
        let (state, root) = test_state().await;
        seed_paired(&state, DbId::new(), Some(REMOTE_NODE), "tok").await;
        let (inventory, bytes) = seed_inventory(&state, &root).await;

        let whole = ask(
            &state,
            Request::File {
                token: "tok".to_string(),
                inventory_id: inventory.to_hex(),
                offset: 0,
                length: None,
                if_range: None,
            },
            REMOTE_NODE,
        )
        .await;
        let (etag, total) = match &whole {
            Response::File(header) => {
                assert_eq!(header.offset, 0);
                assert_eq!(header.length, bytes.len() as u64);
                assert_eq!(header.total, bytes.len() as u64);
                (header.etag.clone(), header.total)
            }
            other => panic!("file: {other:?}"),
        };

        let ranged = ask(
            &state,
            Request::File {
                token: "tok".to_string(),
                inventory_id: inventory.to_hex(),
                offset: 2,
                length: Some(3),
                if_range: Some(etag.clone()),
            },
            REMOTE_NODE,
        )
        .await;
        match ranged {
            Response::File(header) => {
                assert_eq!(header.offset, 2);
                assert_eq!(header.length, 3);
                assert_eq!(header.etag, etag);
            }
            other => panic!("ranged file: {other:?}"),
        }

        // A stale etag means the file changed under the caller, so the whole
        // file is served rather than a range it no longer has.
        let stale = ask(
            &state,
            Request::File {
                token: "tok".to_string(),
                inventory_id: inventory.to_hex(),
                offset: 2,
                length: Some(3),
                if_range: Some("stale".to_string()),
            },
            REMOTE_NODE,
        )
        .await;
        match stale {
            Response::File(header) => {
                assert_eq!(header.offset, 0);
                assert_eq!(header.length, total);
            }
            other => panic!("stale file: {other:?}"),
        }

        let past_the_end = ask(
            &state,
            Request::File {
                token: "tok".to_string(),
                inventory_id: inventory.to_hex(),
                offset: total + 1,
                length: None,
                if_range: Some(etag),
            },
            REMOTE_NODE,
        )
        .await;
        assert!(
            matches!(
                past_the_end,
                Response::Error {
                    code: ErrorCode::RangeNotSatisfiable { size },
                    ..
                } if size == total
            ),
            "an offset past the end reports the size: {past_the_end:?}"
        );
    }

    #[tokio::test]
    async fn a_cover_header_carries_its_total_and_etag() {
        let (state, root) = test_state().await;
        seed_paired(&state, DbId::new(), Some(REMOTE_NODE), "tok").await;
        let (inventory, _bytes) = seed_inventory(&state, &root).await;

        let response = ask(
            &state,
            Request::Cover {
                token: "tok".to_string(),
                inventory_id: inventory.to_hex(),
            },
            REMOTE_NODE,
        )
        .await;
        match response {
            Response::Cover(header) => {
                assert_eq!(header.content_type, "image/png");
                assert_eq!(header.length, header.total);
                assert_eq!(header.total, b"cover-bytes".len() as u64);
                assert!(!header.etag.is_empty());
            }
            other => panic!("cover: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_served_request_is_logged_and_counted() {
        let (state, _root) = test_state().await;
        seed_paired(&state, DbId::new(), Some(REMOTE_NODE), "tok").await;

        let response = ask(
            &state,
            Request::Status {
                token: "tok".to_string(),
            },
            REMOTE_NODE,
        )
        .await;
        assert!(matches!(response, Response::Status(_)));
        assert_eq!(state.requests_served.load(Ordering::Relaxed), 1);
        assert!(state.last_request_at.lock().await.is_some());

        let entries = recent(&state.request_log, 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].op, "status");
        assert_eq!(entries[0].node_id, REMOTE_NODE);
        assert!(entries[0].device_id.is_some(), "the device is recorded");
        assert_eq!(entries[0].outcome, "ok");
        assert!(!entries[0].at.is_empty());
    }

    #[tokio::test]
    async fn a_failed_request_logs_its_error_code() {
        let (state, _root) = test_state().await;

        let response = ask(
            &state,
            Request::Status {
                token: "nope".to_string(),
            },
            REMOTE_NODE,
        )
        .await;
        assert!(matches!(response, Response::Error { .. }));

        let entries = recent(&state.request_log, 10);
        assert_eq!(entries[0].outcome, "error:unauthorized");
        assert_eq!(entries[0].device_id, None);
    }
}
