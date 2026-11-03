//! How this daemon talks to one paired peer.
//!
//! A peer is dialled by its iroh node id: sync data moves over framed streams
//! under [`livtet_sync_iroh::SYNC_ALPN`], so neither side needs a reachable
//! inbound port and no address is stored on the pairing row. iroh resolves the
//! node id to a current path on every dial.
//!
//! The typed [`livtet_sync_iroh::Client`] does the framing; this module only
//! resolves a pairing row to the node id and token it holds, and keeps the
//! daemon's longer-standing return types.

use std::sync::Arc;

use camino::Utf8Path;
use livtet_sync::PullResponse;
use livtet_sync_iroh::{Client, ClientError};
use livtet_types::DbId;

use crate::daemon::DaemonState;
use crate::error::ServerError;

/// One peer's framed stream.
pub struct Peer {
    client: Client,
}

impl Peer {
    /// Build a peer transport around `client`.
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Dial a paired device by node id, authenticating with `token`.
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
        Ok(Peer::new(
            Client::new(endpoint, node_id).with_token(token.to_string()),
        ))
    }

    /// Changes after `since_version`, addressed to this device.
    pub async fn changes(
        &self,
        since_version: i64,
        limit: i64,
    ) -> Result<PullResponse, ServerError> {
        self.client
            .changes(since_version, Some(limit))
            .await
            .map_err(|error| remote("pull changes", error))
    }

    /// Write one inventory file to `dest`.
    pub async fn download_file(
        &self,
        inventory_id: &str,
        dest: &Utf8Path,
    ) -> Result<(), ServerError> {
        let (_header, body) = self
            .client
            .file(inventory_id, 0, None, None)
            .await
            .map_err(|error| remote("download file", error))?;
        let mut file = fs_err::tokio::File::create(dest.as_std_path())
            .await
            .map_err(|error| ServerError::Server(format!("write received file: {error}")))?;
        body.copy_to(&mut file)
            .await
            .map_err(|error| remote("receive file", error))?;
        Ok(())
    }

    /// The extracted cover for `inventory_id`, as `(content type, bytes)`.
    ///
    /// `None` when the peer has no cover for it: that is a normal answer, not a
    /// failure, because plenty of books have none.
    pub async fn cover(
        &self,
        inventory_id: &str,
    ) -> Result<Option<(String, Vec<u8>)>, ServerError> {
        match self.client.cover(inventory_id).await {
            Ok(Some((header, bytes))) => Ok(Some((header.content_type, bytes))),
            Ok(None) => Ok(None),
            Err(error) => Err(remote("download cover", error)),
        }
    }

    /// Tell the sender what became of an offer it made.
    pub async fn offer_decision(&self, offer_id: &str, accepted: bool) -> Result<(), ServerError> {
        self.client
            .offer_decision(offer_id, accepted)
            .await
            .map_err(|error| remote("offer decision", error))
    }
}

/// A typed client failure, as the daemon reports it.
fn remote(operation: &str, error: ClientError) -> ServerError {
    ServerError::Server(format!("{operation}: {error}"))
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
