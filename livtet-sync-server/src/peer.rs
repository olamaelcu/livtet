//! How this daemon talks to one paired peer.
//!
//! A peer is dialled by its iroh node id: sync data moves over framed streams
//! under [`livtet_sync_iroh::SYNC_ALPN`], so neither side needs a reachable
//! inbound port and no address is stored on the pairing row. iroh resolves the
//! node id to a current path on every dial.

use std::sync::Arc;

use camino::Utf8Path;
use livtet_sync::PullResponse;
use livtet_types::DbId;

use crate::daemon::DaemonState;
use crate::error::ServerError;
use livtet_sync_iroh::{Request, Response};

/// One peer's framed stream.
pub struct Peer {
    connection: livtet_sync_iroh::Connection,
    token: String,
}

impl Peer {
    /// Dial a paired device by node id.
    pub async fn connect(
        state: &Arc<DaemonState>,
        node_id: &str,
        token: &str,
    ) -> Result<Peer, ServerError> {
        let node_id = node_id.trim();
        if node_id.is_empty() {
            return Err(ServerError::Server(
                "the peer has no node id to dial".to_string(),
            ));
        }
        let endpoint = state
            .discovery
            .lock()
            .await
            .as_ref()
            .map(|service| service.endpoint().clone())
            .ok_or_else(|| {
                ServerError::Server(
                    "peer discovery is not bound, so the peer cannot be dialled".to_string(),
                )
            })?;
        let connection = livtet_sync_iroh::dial(&endpoint, node_id)
            .await
            .map_err(|error| ServerError::Server(format!("reach the peer: {error}")))?;
        Ok(Peer {
            connection,
            token: token.to_string(),
        })
    }

    /// Changes after `since_version`, addressed to this device.
    pub async fn changes(
        &self,
        since_version: i64,
        limit: i64,
    ) -> Result<PullResponse, ServerError> {
        let exchange = livtet_sync_iroh::call(
            &self.connection,
            &Request::Changes {
                token: self.token.clone(),
                since_version,
                limit: Some(limit),
            },
        )
        .await
        .map_err(|error| ServerError::Server(format!("pull changes: {error}")))?;
        match exchange.response {
            Response::Changes(pull) => Ok(pull),
            Response::Denied { reason } => Err(ServerError::Server(format!(
                "pull changes refused: {reason}"
            ))),
            other => Err(unexpected(&other, "changes")),
        }
    }

    /// Write one inventory file to `dest`.
    pub async fn download_file(
        &self,
        inventory_id: &str,
        dest: &Utf8Path,
    ) -> Result<(), ServerError> {
        let exchange = livtet_sync_iroh::call(
            &self.connection,
            &Request::File {
                token: self.token.clone(),
                inventory_id: inventory_id.to_string(),
                offset: 0,
                length: None,
            },
        )
        .await
        .map_err(|error| ServerError::Server(format!("download file: {error}")))?;
        match &exchange.response {
            Response::File(_) => {}
            Response::Denied { reason } => {
                return Err(ServerError::Server(format!(
                    "download file refused: {reason}"
                )));
            }
            other => return Err(unexpected(other, "file")),
        }
        let mut stream = exchange
            .bytes
            .ok_or_else(|| ServerError::Server("file reply had no bytes".to_string()))?;
        let mut file = fs_err::tokio::File::create(dest.as_std_path())
            .await
            .map_err(|error| ServerError::Server(format!("write received file: {error}")))?;
        tokio::io::copy(&mut stream, &mut file)
            .await
            .map_err(|error| ServerError::Server(format!("receive file: {error}")))?;
        Ok(())
    }

    /// The extracted cover for `inventory_id`, as `(content type, bytes)`.
    pub async fn cover(
        &self,
        inventory_id: &str,
    ) -> Result<Option<(String, Vec<u8>)>, ServerError> {
        let exchange = livtet_sync_iroh::call(
            &self.connection,
            &Request::Cover {
                token: self.token.clone(),
                inventory_id: inventory_id.to_string(),
            },
        )
        .await
        .map_err(|error| ServerError::Server(format!("download cover: {error}")))?;
        let header = match &exchange.response {
            Response::Cover(header) => header.clone(),
            Response::Missing { .. } | Response::Denied { .. } => return Ok(None),
            other => return Err(unexpected(other, "cover")),
        };
        let mut stream = exchange
            .bytes
            .ok_or_else(|| ServerError::Server("cover reply had no bytes".to_string()))?;
        let mut bytes = Vec::with_capacity(header.length as usize);
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut bytes)
            .await
            .map_err(|error| ServerError::Server(format!("receive cover: {error}")))?;
        Ok(Some((header.content_type, bytes)))
    }

    /// Tell the sender what became of an offer it made.
    pub async fn offer_decision(&self, offer_id: &str, accepted: bool) -> Result<(), ServerError> {
        let exchange = livtet_sync_iroh::call(
            &self.connection,
            &Request::OfferDecision {
                token: self.token.clone(),
                offer_id: offer_id.to_string(),
                accepted,
            },
        )
        .await
        .map_err(|error| ServerError::Server(format!("offer decision: {error}")))?;
        match exchange.response {
            Response::OfferDecided => Ok(()),
            Response::Denied { reason } => Err(ServerError::Server(format!(
                "offer decision refused: {reason}"
            ))),
            other => Err(unexpected(&other, "offer decision")),
        }
    }
}

/// A reply this operation cannot use.
fn unexpected(response: &Response, operation: &str) -> ServerError {
    ServerError::Server(format!(
        "{operation}: unexpected reply {}",
        match response {
            Response::Identity(_) => "identity",
            Response::Status(_) => "status",
            Response::Changes(_) => "changes",
            Response::Full(_) => "pull-full",
            Response::Pushed(_) => "push",
            Response::Paired { .. } => "paired",
            Response::Refused { .. } => "refused",
            Response::Conflicts(_) => "conflicts",
            Response::Resolved(_) => "resolved",
            Response::File(_) => "file",
            Response::Cover(_) => "cover",
            Response::OfferDecided => "offer decided",
            Response::Missing { .. } => "missing",
            Response::Denied { .. } => "denied",
        }
    ))
}

/// The transport for a device referenced by the ULID string of a pairing row.
pub async fn peer_by_id(state: &Arc<DaemonState>, device_id: &str) -> Result<Peer, ServerError> {
    let id = device_id
        .parse::<DbId>()
        .ok()
        .or_else(|| DbId::from_hex(device_id).ok())
        .ok_or_else(|| ServerError::Server(format!("invalid device id: {device_id}")))?;
    peer_for(state, id).await
}

/// Load the transport details for a paired device.
///
/// Only an outbound row can be dialled: its session token was issued by the
/// peer, so it authenticates there. The row's node id is the address.
pub async fn peer_for(state: &Arc<DaemonState>, device_id: DbId) -> Result<Peer, ServerError> {
    use livtet_data::client_entities::paired_devices;
    use livtet_data::orm::EntityTrait;

    let row = paired_devices::Entity::find_by_id(device_id)
        .one(state.engine.db())
        .await
        .map_err(|error| ServerError::Server(format!("paired device lookup: {error}")))?
        .ok_or_else(|| ServerError::Server("device is not paired".to_string()))?;
    if !row.outbound {
        return Err(ServerError::Server(
            "device was not claimed by this installation".to_string(),
        ));
    }
    let token: String = livtet_data::sql::query_scalar(
        "SELECT session_token FROM paired_devices WHERE device_id = ?",
    )
    .bind(device_id.to_bytes().to_vec())
    .fetch_one(&state.pool)
    .await
    .map_err(|error| ServerError::Server(format!("session token lookup: {error}")))?;
    let node_id: Option<String> =
        livtet_data::sql::query_scalar("SELECT node_id FROM paired_devices WHERE device_id = ?")
            .bind(device_id.to_bytes().to_vec())
            .fetch_one(&state.pool)
            .await
            .unwrap_or(None);

    Peer::connect(state, node_id.as_deref().unwrap_or_default(), &token).await
}
