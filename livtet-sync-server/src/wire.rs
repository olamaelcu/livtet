//! The iroh-facing side of sync: framed requests answered from the local engine.
//!
//! These are the same operations the HTTP routes served, with the transport
//! swapped: a token that arrived in a header now arrives in the frame, and the
//! caller's identity is the QUIC connection's remote node id rather than a
//! socket address. Everything below the dispatch is unchanged — the engine owns
//! `pull_changes`, `apply_changes`, the conflict list, and the file paths.

use std::sync::Arc;

use camino::Utf8PathBuf;
use livtet_data::orm::{ConnectionTrait, DbBackend, EntityTrait, Statement};
use livtet_sync::{SyncChange, SyncStatus};
use livtet_sync_iroh::{FileHeader, Handler, Identity, Outcome, Request, Response};
use livtet_types::DbId;

use crate::daemon::{DaemonState, discovery_node_id};

/// How long a pairing claim may wait for its reader to decide.
///
/// The approval is human-paced and the ticket's own window is five minutes;
/// the stream stays open for that long so the claimant learns the answer
/// without polling.
const PAIR_DECISION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Build the handler the accept loop runs.
pub fn handler(state: Arc<DaemonState>) -> Handler {
    Arc::new(move |request, remote| {
        let state = Arc::clone(&state);
        Box::pin(async move { answer(&state, request, remote).await })
    })
}

/// Answer one frame. No path here panics: a bad request becomes a `Denied`
/// frame, which the peer surfaces as a failure rather than a dropped stream.
async fn answer(state: &Arc<DaemonState>, request: Request, remote: String) -> Outcome {
    // The connection's node id is the caller's identity. A frame claiming to be
    // someone else is refused before it reaches the engine.
    let self_node = discovery_node_id(state).await;
    if remote == self_node {
        return Outcome::reply(Response::Denied {
            reason: "a device cannot sync with itself".to_string(),
        });
    }

    match request {
        Request::Identity => Outcome::reply(Response::Identity(Identity {
            node_id: self_node,
            device_id: state.device_id.clone(),
        })),

        Request::Status { token } => match authorize(state, &token).await {
            Ok(_) => match state.engine.get_latest_version().await {
                Ok(latest_version) => Outcome::reply(Response::Status(SyncStatus {
                    latest_version,
                    device_id: state.device_id.clone(),
                })),
                Err(error) => denied(format!("status: {error}")),
            },
            Err(reason) => Outcome::reply(Response::Denied { reason }),
        },

        Request::Changes {
            token,
            since_version,
            limit,
        } => {
            let device_id = match authorize(state, &token).await {
                Ok(device_id) => device_id,
                Err(reason) => return Outcome::reply(Response::Denied { reason }),
            };
            let limit = limit.unwrap_or(100);
            match state.engine.pull_changes(since_version, limit).await {
                Ok(mut response) => {
                    // Offers name their target; no other device may see them.
                    response
                        .changes
                        .retain(|change| change_is_for(change, &device_id));
                    Outcome::reply(Response::Changes(response))
                }
                Err(error) => denied(format!("changes: {error}")),
            }
        }

        Request::PullFull { token } => match authorize(state, &token).await {
            Ok(_) => match state.engine.pull_full().await {
                Ok(dump) => Outcome::reply(Response::Full(Box::new(dump))),
                Err(error) => denied(format!("pull-full: {error}")),
            },
            Err(reason) => Outcome::reply(Response::Denied { reason }),
        },

        Request::Push { token, changes } => match authorize(state, &token).await {
            Ok(_) => match state.engine.push_changes(changes).await {
                Ok(response) => Outcome::reply(Response::Pushed(response)),
                Err(error) => denied(format!("push: {error}")),
            },
            Err(reason) => Outcome::reply(Response::Denied { reason }),
        },

        Request::Pair {
            device_id,
            name,
            device_type,
            node_id,
        } => pair(state, device_id, name, device_type, node_id).await,

        Request::Conflicts { token } => match authorize(state, &token).await {
            Ok(_) => match state.engine.list_conflicts().await {
                Ok(conflicts) => Outcome::reply(Response::Conflicts(conflicts)),
                Err(error) => denied(format!("conflicts: {error}")),
            },
            Err(reason) => Outcome::reply(Response::Denied { reason }),
        },

        Request::ResolveConflict {
            token,
            id,
            resolution,
            merged_payload,
        } => {
            if !matches!(resolution.as_str(), "local" | "remote" | "merged") {
                return denied(format!("unknown resolution: {resolution}"));
            }
            match authorize(state, &token).await {
                Ok(_) => {
                    match state
                        .engine
                        .resolve_conflict(id, &resolution, merged_payload.as_deref())
                        .await
                    {
                        Ok(resolved) => Outcome::reply(Response::Resolved(resolved)),
                        Err(error) => denied(format!("resolve: {error}")),
                    }
                }
                Err(reason) => Outcome::reply(Response::Denied { reason }),
            }
        }

        Request::File {
            token,
            inventory_id,
            offset,
            length,
        } => file(state, token, inventory_id, offset, length).await,

        Request::Cover {
            token,
            inventory_id,
        } => cover(state, token, inventory_id).await,

        Request::OfferDecision {
            token,
            offer_id,
            accepted,
        } => match authorize(state, &token).await {
            Ok(device_id) => match offer_decision(state, &offer_id, &device_id, accepted).await {
                Ok(()) => Outcome::reply(Response::OfferDecided),
                Err(reason) => denied(reason),
            },
            Err(reason) => Outcome::reply(Response::Denied { reason }),
        },
    }
}

/// A failure the caller should read, not a dropped stream.
fn denied(reason: String) -> Outcome {
    Outcome::reply(Response::Denied { reason })
}

/// Resolve a session token to the device it was issued to.
///
/// `paired_devices.session_token` is created by a client migration and is not
/// on the SeaORM entity, so this is a targeted statement. Unknown and absent
/// tokens answer the same way: the caller must not learn which of the two it
/// sent.
async fn authorize(state: &Arc<DaemonState>, token: &str) -> Result<String, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("no session token".to_string());
    }
    let statement = Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "SELECT device_id FROM paired_devices WHERE session_token = ? LIMIT 1",
        [token.into()],
    );
    match state.engine.db().query_one_raw(statement).await {
        Ok(Some(row)) => {
            let bytes: Vec<u8> = row
                .try_get("", "device_id")
                .map_err(|error| format!("device id: {error}"))?;
            let arr: [u8; 16] = bytes
                .try_into()
                .map_err(|_| "device id is not 16 bytes".to_string())?;
            Ok(DbId::from_bytes(arr).to_string())
        }
        Ok(None) => Err("the session token is not recognised".to_string()),
        Err(error) => Err(format!("authorization: {error}")),
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
) -> Result<(Option<String>, Option<String>), String> {
    use livtet_data::entities::digital_inventory;

    let id = parse_inventory_id(inventory_id).ok_or_else(|| "invalid inventory id".to_string())?;
    let row = digital_inventory::Entity::find_by_id(id)
        .one(state.engine.db())
        .await
        .map_err(|error| format!("inventory lookup: {error}"))?
        .ok_or_else(|| "no such inventory".to_string())?;
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

/// Serve bytes of one inventory file, from `offset`, at most `length`.
async fn file(
    state: &Arc<DaemonState>,
    token: String,
    inventory_id: String,
    offset: u64,
    length: Option<u64>,
) -> Outcome {
    if let Err(reason) = authorize(state, &token).await {
        return Outcome::reply(Response::Denied { reason });
    }
    let (file_path, _) = match inventory_paths(state, &inventory_id).await {
        Ok(paths) => paths,
        Err(reason) => return Outcome::reply(Response::Missing { reason }),
    };
    let Some(path) = file_path else {
        return Outcome::reply(Response::Missing {
            reason: "this inventory has no file".to_string(),
        });
    };
    let metadata = match fs_err::tokio::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(_) => {
            return Outcome::reply(Response::Missing {
                reason: "the file is not on disk".to_string(),
            });
        }
    };
    let total = metadata.len();
    let available = total.saturating_sub(offset);
    let size = length.unwrap_or(available).min(available);
    let header = FileHeader {
        content_type: content_type_for(&path).to_string(),
        offset,
        length: size,
    };
    Outcome::file(header, Utf8PathBuf::from(path))
}

/// Serve the extracted cover for one inventory row.
async fn cover(state: &Arc<DaemonState>, token: String, inventory_id: String) -> Outcome {
    if let Err(reason) = authorize(state, &token).await {
        return Outcome::reply(Response::Denied { reason });
    }
    let (_, cover_path) = match inventory_paths(state, &inventory_id).await {
        Ok(paths) => paths,
        Err(reason) => return Outcome::reply(Response::Missing { reason }),
    };
    let Some(path) = cover_path else {
        return Outcome::reply(Response::Missing {
            reason: "this inventory has no cover".to_string(),
        });
    };
    let metadata = match fs_err::tokio::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(_) => {
            return Outcome::reply(Response::Missing {
                reason: "the cover is not on disk".to_string(),
            });
        }
    };
    let header = FileHeader {
        content_type: content_type_for(&path).to_string(),
        offset: 0,
        length: metadata.len(),
    };
    Outcome::cover(header, Utf8PathBuf::from(path))
}

/// Record a paired device's answer to an offer this desktop sent.
async fn offer_decision(
    state: &Arc<DaemonState>,
    offer_id: &str,
    device_id: &str,
    accepted: bool,
) -> Result<(), String> {
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
        .map_err(|error| format!("offer lookup: {error}"))?
        .ok_or_else(|| "no such offer".to_string())?;
    let payload: String = row
        .try_get("", "payload")
        .map_err(|error| format!("offer payload: {error}"))?;
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
    if payload
        .get("to_device_id")
        .and_then(serde_json::Value::as_str)
        != Some(device_id)
    {
        return Err("the offer is addressed to another device".to_string());
    }

    (state.offer_sink)(livtet_sync_http::OfferDecision {
        offer_id: offer_id.to_string(),
        device_id: device_id.to_string(),
        accepted,
    });
    Ok(())
}

/// File a pairing claim and hold the stream until the reader decides.
async fn pair(
    state: &Arc<DaemonState>,
    device_id: String,
    name: String,
    device_type: String,
    node_id: String,
) -> Outcome {
    use livtet_data::client_entities::pending_pairings;
    use livtet_data::orm::{ActiveModelTrait, EntityTrait, Set};
    use livtet_iroh::NodeId;
    use livtet_types::{DbId, DeviceType, PairingStatus};

    let node: NodeId = match node_id.parse() {
        Ok(node) => node,
        Err(_) => {
            return Outcome::reply(Response::Refused {
                reason: "not a node id".to_string(),
            });
        }
    };

    let pending_id = DbId::from(PairingStatus::Pending);
    let now = time::OffsetDateTime::now_utc();
    let now_pdt = time::PrimitiveDateTime::new(now.date(), now.time());
    let expires = now + time::Duration::seconds(300);
    let expires_pdt = time::PrimitiveDateTime::new(expires.date(), expires.time());
    let device_type_id = Some(DbId::from(DeviceType::from_tag(&device_type)));
    let peer_device_id = device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(&device_id).ok());
    let desktop_id = match state.device_id.parse::<DbId>() {
        Ok(id) => id,
        Err(_) => {
            return Outcome::reply(Response::Refused {
                reason: "this device has no valid id".to_string(),
            });
        }
    };

    let existing = match pending_pairings::Entity::find_by_id(node.0.clone())
        .one(state.engine.db())
        .await
    {
        Ok(row) => row,
        Err(error) => {
            return Outcome::reply(Response::Refused {
                reason: format!("pairing table: {error}"),
            });
        }
    };

    let write = match existing {
        // A decided ticket must not be reopened by replaying the claim.
        Some(row) if row.status_id != Some(pending_id) => {
            return Outcome::reply(Response::Refused {
                reason: "this pairing was already decided".to_string(),
            });
        }
        Some(row) => {
            let mut model: pending_pairings::ActiveModel = row.into();
            model.desktop_id = Set(desktop_id);
            model.device_name = Set(Some(name.clone()));
            model.device_type_id = Set(device_type_id);
            model.device_id = Set(peer_device_id);
            model.expires_at = Set(expires_pdt);
            model.update(state.engine.db()).await.map(|_| ())
        }
        None => {
            let model = pending_pairings::ActiveModel {
                token: Set(node.0.clone()),
                desktop_id: Set(desktop_id),
                // No socket origin: the connection's node id is the identity
                // that matters, and it cannot be forged or spoofed.
                listen_on: Set(None),
                status_id: Set(Some(pending_id)),
                device_name: Set(Some(name.clone())),
                device_type_id: Set(device_type_id),
                device_id: Set(peer_device_id),
                origin_addr: Set(None),
                created_at: Set(now_pdt),
                expires_at: Set(expires_pdt),
            };
            model.insert(state.engine.db()).await.map(|_| ())
        }
    };
    if let Err(error) = write {
        return Outcome::reply(Response::Refused {
            reason: format!("could not record the claim: {error}"),
        });
    }

    // The claim secret proves the waiter is the device that filed this claim;
    // the approval fans the decision out on the matching broadcast.
    let claim = ulid::Ulid::generate().to_string();
    let (sender, mut receiver) = tokio::sync::broadcast::channel(1);
    {
        let mut waiters = state.pair_waiters.lock().await;
        waiters.insert(
            node.0.clone(),
            livtet_sync_http::pairing::PairingWaiter {
                claim: claim.clone(),
                sender,
            },
        );
    }
    tracing::info!(
        device = %device_id,
        name = %name,
        node_id = node.0,
        "pairing request over iroh",
    );

    match tokio::time::timeout(PAIR_DECISION_TIMEOUT, receiver.recv()).await {
        Ok(Ok(decision)) if decision.event == "approved" => Outcome::reply(Response::Paired {
            device_id: peer_device_id.map(|id| id.to_string()).unwrap_or_default(),
            session_token: decision.session_token,
        }),
        Ok(Ok(decision)) => Outcome::reply(Response::Refused {
            reason: format!("pairing {}", decision.event),
        }),
        Ok(Err(_)) => Outcome::reply(Response::Refused {
            reason: "the pairing was dropped before a decision".to_string(),
        }),
        Err(_) => Outcome::reply(Response::Refused {
            reason: "the pairing was not decided in time".to_string(),
        }),
    }
}
