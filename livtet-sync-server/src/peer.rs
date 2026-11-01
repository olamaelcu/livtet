//! How this daemon talks to one paired peer.
//!
//! A peer is dialled, never awaited: sync data moves over iroh streams under
//! [`livtet_sync_iroh::SYNC_ALPN`], which need no reachable port on either side.
//! Rows paired before the transport change have no node id, so they keep using
//! the HTTP surface until they are paired again; that branch disappears with
//! `livtet-sync-http`.

use std::sync::Arc;

use camino::Utf8Path;
use livtet_sync::PullResponse;
use livtet_types::DbId;

use crate::daemon::DaemonState;
use crate::error::ServerError;
use livtet_sync_iroh::{Request, Response};

/// One peer's live transport.
pub enum Peer {
    /// A framed iroh stream to the peer's node id.
    Iroh {
        connection: livtet_sync_iroh::Connection,
        token: String,
        node_id: String,
    },
    /// The retiring HTTP surface, for a row that predates node ids.
    Http {
        client: reqwest::Client,
        base: String,
        token: String,
    },
}

impl Peer {
    /// Open a transport for a paired device.
    ///
    /// Prefers iroh whenever the row carries a node id and the endpoint is
    /// bound: it is the only transport that works when the peer cannot accept
    /// an inbound connection.
    pub async fn connect(
        state: &Arc<DaemonState>,
        node_id: Option<&str>,
        listen_on: Option<&str>,
        token: &str,
    ) -> Result<Peer, ServerError> {
        let mut dial_error = None;
        if let Some(node_id) = node_id.map(str::trim).filter(|id| !id.is_empty()) {
            let endpoint = state
                .discovery
                .lock()
                .await
                .as_ref()
                .map(|service| service.endpoint().clone());
            if let Some(endpoint) = endpoint {
                match livtet_sync_iroh::dial(&endpoint, node_id).await {
                    Ok(connection) => {
                        return Ok(Peer::Iroh {
                            connection,
                            token: token.to_string(),
                            node_id: node_id.to_string(),
                        });
                    }
                    // A peer that cannot be dialled by id may still be
                    // reachable on its stored address (an older build, or a
                    // relay that is down while the LAN is up), so fall through
                    // instead of failing the pull outright.
                    Err(error) => {
                        tracing::debug!(%node_id, %error, "could not dial by node id");
                        dial_error = Some(error);
                    }
                }
            }
        }

        let listen_on = listen_on
            .map(str::trim)
            .filter(|address| !address.is_empty())
            .ok_or_else(|| match dial_error {
                Some(error) => ServerError::Server(format!("reach the peer: {error}")),
                None => ServerError::Server(
                    "the peer has neither a node id nor an address to reach it on".to_string(),
                ),
            })?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|error| ServerError::Server(format!("peer client: {error}")))?;
        Ok(Peer::Http {
            client,
            base: format!("http://{}", listen_on.trim_end_matches('/')),
            token: token.to_string(),
        })
    }

    /// Where this peer is reached, for logs and errors.
    pub fn describe(&self) -> String {
        match self {
            Peer::Iroh { node_id, .. } => format!("iroh:{node_id}"),
            Peer::Http { base, .. } => base.clone(),
        }
    }

    /// Changes after `since_version`, addressed to this device.
    pub async fn changes(
        &self,
        since_version: i64,
        limit: i64,
    ) -> Result<PullResponse, ServerError> {
        match self {
            Peer::Iroh {
                connection, token, ..
            } => {
                let exchange = livtet_sync_iroh::call(
                    connection,
                    &Request::Changes {
                        token: token.clone(),
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
            Peer::Http {
                client,
                base,
                token,
            } => {
                let response = client
                    .get(format!(
                        "{base}/sync/changes?since_version={since_version}&limit={limit}"
                    ))
                    .header("x-livtet-session", token)
                    .send()
                    .await
                    .map_err(|error| ServerError::Server(format!("pull changes: {error}")))?;
                if !response.status().is_success() {
                    return Err(ServerError::Server(format!(
                        "pull changes returned HTTP {}",
                        response.status()
                    )));
                }
                response
                    .json()
                    .await
                    .map_err(|error| ServerError::Server(format!("pull changes body: {error}")))
            }
        }
    }

    /// Write one inventory file to `dest`.
    pub async fn download_file(
        &self,
        inventory_id: &str,
        dest: &Utf8Path,
    ) -> Result<(), ServerError> {
        match self {
            Peer::Iroh {
                connection, token, ..
            } => {
                let exchange = livtet_sync_iroh::call(
                    connection,
                    &Request::File {
                        token: token.clone(),
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
                    .map_err(|error| {
                        ServerError::Server(format!("write received file: {error}"))
                    })?;
                tokio::io::copy(&mut stream, &mut file)
                    .await
                    .map_err(|error| ServerError::Server(format!("receive file: {error}")))?;
                Ok(())
            }
            Peer::Http {
                client,
                base,
                token,
            } => {
                let bytes = client
                    .get(format!("{base}/sync/files/{inventory_id}"))
                    .header("x-livtet-session", token)
                    .send()
                    .await
                    .map_err(|error| ServerError::Server(format!("download file: {error}")))?
                    .bytes()
                    .await
                    .map_err(|error| ServerError::Server(format!("download file body: {error}")))?;
                fs_err::tokio::write(dest.as_std_path(), &bytes)
                    .await
                    .map_err(|error| {
                        ServerError::Server(format!("write received file: {error}"))
                    })?;
                Ok(())
            }
        }
    }

    /// The extracted cover for `inventory_id`, as `(content type, bytes)`.
    pub async fn cover(
        &self,
        inventory_id: &str,
    ) -> Result<Option<(String, Vec<u8>)>, ServerError> {
        match self {
            Peer::Iroh {
                connection, token, ..
            } => {
                let exchange = livtet_sync_iroh::call(
                    connection,
                    &Request::Cover {
                        token: token.clone(),
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
            Peer::Http {
                client,
                base,
                token,
            } => {
                let response = client
                    .get(format!("{base}/sync/covers/{inventory_id}"))
                    .header("x-livtet-session", token)
                    .send()
                    .await
                    .map_err(|error| ServerError::Server(format!("download cover: {error}")))?;
                if !response.status().is_success() {
                    return Ok(None);
                }
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("image/png")
                    .to_string();
                let bytes = response.bytes().await.map_err(|error| {
                    ServerError::Server(format!("download cover body: {error}"))
                })?;
                Ok(Some((content_type, bytes.to_vec())))
            }
        }
    }

    /// Tell the sender what became of an offer it made.
    pub async fn offer_decision(&self, offer_id: &str, accepted: bool) -> Result<(), ServerError> {
        match self {
            Peer::Iroh {
                connection, token, ..
            } => {
                let exchange = livtet_sync_iroh::call(
                    connection,
                    &Request::OfferDecision {
                        token: token.clone(),
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
            Peer::Http {
                client,
                base,
                token,
            } => {
                let response = client
                    .post(format!("{base}/sync/offers/{offer_id}/decision"))
                    .header("x-livtet-session", token)
                    .json(&serde_json::json!({ "accepted": accepted }))
                    .send()
                    .await
                    .map_err(|error| ServerError::Server(format!("offer decision: {error}")))?;
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(ServerError::Server(format!(
                        "offer decision returned HTTP {}",
                        response.status()
                    )))
                }
            }
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
/// peer, so it authenticates there. An approval row holds a token this device
/// issued, which the peer's server does not accept.
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
    let listen_on = row.listen_on.as_ref().map(ToString::to_string);

    Peer::connect(state, node_id.as_deref(), listen_on.as_deref(), &token).await
}
