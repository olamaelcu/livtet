//! Dial side of the framed protocol.
//!
//! A peer is addressed by its node id. Nothing here knows a host or port:
//! iroh resolves the id through its address lookup (mDNS on the LAN, pkarr and
//! the relay off it) and punches through NAT where it can, so the caller never
//! needs the other side to accept an inbound port.

use iroh::endpoint::{Connection, RecvStream};
use iroh::{Endpoint, EndpointAddr, EndpointId};

use crate::wire::{Request, Response, WireError, read_frame, write_frame};

/// Why a peer could not be dialled.
#[derive(Debug, thiserror::Error)]
pub enum DialError {
    #[error("not a node id: {0}")]
    NodeId(String),
    #[error("could not reach the peer: {0}")]
    Connect(String),
}

/// Open a sync connection to `node_id`.
pub async fn dial(endpoint: &Endpoint, node_id: &str) -> Result<Connection, DialError> {
    let id: EndpointId = node_id
        .trim()
        .parse()
        .map_err(|_| DialError::NodeId(node_id.to_string()))?;
    endpoint
        .connect(EndpointAddr::from(id), crate::SYNC_ALPN)
        .await
        .map_err(|error| DialError::Connect(error.to_string()))
}

/// A response, and the stream its byte phase arrives on.
pub struct Exchange {
    pub response: Response,
    /// Present for [`Response::File`] and [`Response::Cover`]: the caller reads
    /// the header's `length` bytes from it next.
    pub bytes: Option<RecvStream>,
}

/// Send one request and read its answer.
///
/// The reply to `Pair` arrives only when the other reader decides, so this can
/// wait as long as they take; callers that need a bound wrap it themselves.
pub async fn call(connection: &Connection, request: &Request) -> Result<Exchange, WireError> {
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| WireError::Stream(error.to_string()))?;
    write_frame(&mut send, request).await?;

    let response: Response = read_frame(&mut recv).await?;
    let wants_bytes = matches!(response, Response::File(_) | Response::Cover(_));
    Ok(Exchange {
        response,
        bytes: wants_bytes.then_some(recv),
    })
}
