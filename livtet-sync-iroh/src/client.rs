//! Dial side of the framed protocol.
//!
//! A peer is addressed by its node id. Nothing here knows a host or port:
//! iroh resolves the id through its address lookup (mDNS on the LAN, pkarr and
//! the relay off it) and punches through NAT where it can, so the caller never
//! needs the other side to accept an inbound port.
//!
//! [`Client`] is the typed layer over one peer: it dials by node id, sends one
//! request per stream, and turns an `error` response into
//! [`ClientError::Remote`] instead of making every caller match on it.

use iroh::endpoint::Connection;
use iroh::endpoint::RecvStream;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use livtet_sync::types::{Conflict, FullDump, PullResponse, PushResponse, SyncChange, SyncStatus};
use tokio::io::{AsyncReadExt, AsyncWrite};

use crate::wire::{CoverImage, 
    ErrorCode, FileHeader, FileMetadata, Identity, Request, Response, WireError, read_frame,
    write_frame,
};

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
/// The reply to `Pair` is only `PairingPending`; the decision arrives on a
/// later `PairStatus`, so this never blocks on a human.
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

/// Why a typed call failed.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The peer could not be reached at all.
    #[error(transparent)]
    Dial(#[from] DialError),
    /// The exchange broke before a usable answer arrived.
    #[error(transparent)]
    Wire(#[from] WireError),
    /// The peer answered with an [`ErrorCode`].
    #[error("{code:?}: {message}")]
    Remote { code: ErrorCode, message: String },
}

/// What a pairing claim has become.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingOutcome {
    /// The claim was filed and is waiting on the reader.
    Pending { claim: String },
    /// The reader approved; the token authenticates every later request.
    Paired {
        device_id: String,
        session_token: String,
    },
    /// The reader declined, or the claim could not be filed.
    Refused { reason: String },
}

/// The raw bytes that follow a file or cover header.
///
/// The stream is consumed exactly `header.length` bytes at a time so a caller
/// streaming a book to disk and a caller that wants one `Vec<u8>` both see the
/// same body, with no over-read into the next stream.
pub struct ByteBody {
    stream: RecvStream,
    remaining: u64,
}

impl ByteBody {
    /// Bytes not yet read.
    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Read the whole body into one buffer.
    pub async fn read_all(mut self) -> Result<Vec<u8>, ClientError> {
        let mut bytes = vec![0u8; self.remaining as usize];
        self.stream.read_exact(&mut bytes).await.map_err(|error| {
            WireError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                error,
            ))
        })?;
        self.remaining = 0;
        Ok(bytes)
    }

    /// Copy the body to `writer`, returning how many bytes crossed.
    pub async fn copy_to<W>(mut self, writer: &mut W) -> Result<u64, ClientError>
    where
        W: AsyncWrite + Unpin,
    {
        let mut window = (&mut self.stream).take(self.remaining);
        let copied = tokio::io::copy(&mut window, writer)
            .await
            .map_err(WireError::Io)?;
        self.remaining -= copied;
        Ok(copied)
    }

    /// Read up to `out.len()` more bytes, returning how many were read.
    ///
    /// Zero means the body is spent. A caller writing to disk in a loop can use
    /// this to report how far a transfer has got; [`read_all`](Self::read_all)
    /// only offers the finished buffer.
    pub async fn read_chunk(&mut self, out: &mut [u8]) -> Result<usize, ClientError> {
        if self.remaining == 0 || out.is_empty() {
            return Ok(0);
        }
        let take = (out.len() as u64).min(self.remaining) as usize;
        self.stream
            .read_exact(&mut out[..take])
            .await
            .map_err(|error| {
                WireError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    error,
                ))
            })?;
        self.remaining -= take as u64;
        Ok(take)
    }
}

/// One peer's typed sync client.
///
/// Cheap to build and worth reusing: the endpoint is shared, and the token is
/// attached once with [`Client::with_token`].
pub struct Client {
    endpoint: Endpoint,
    node_id: String,
    token: Option<String>,
}

impl Client {
    /// A client for `node_id` that carries no session token.
    pub fn new(endpoint: Endpoint, node_id: impl Into<String>) -> Self {
        Self {
            endpoint,
            node_id: node_id.into(),
            token: None,
        }
    }

    /// Attach the session token the peer issued this device.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Open a connection to the peer.
    pub async fn connect(&self) -> Result<Connection, ClientError> {
        Ok(dial(&self.endpoint, &self.node_id).await?)
    }

    /// The token this client authenticates with, or a local failure.
    fn require_token(&self) -> Result<String, ClientError> {
        self.token.clone().ok_or_else(|| {
            ClientError::Wire(WireError::Protocol(
                "this client holds no session token".to_string(),
            ))
        })
    }

    /// Send one request and read its answer, mapping `Error` to `Remote`.
    async fn send(&self, request: Request) -> Result<Exchange, ClientError> {
        let connection = self.connect().await?;
        let exchange = call(&connection, &request).await?;
        match exchange.response {
            Response::Error { code, message } => Err(ClientError::Remote { code, message }),
            _ => Ok(exchange),
        }
    }

    /// Send one request that has no byte phase and expect a specific shape.
    async fn expect(
        &self,
        request: Request,
        operation: &'static str,
    ) -> Result<Response, ClientError> {
        let exchange = self.send(request).await?;
        if exchange.bytes.is_some() {
            return Err(unexpected(operation, &exchange.response));
        }
        Ok(exchange.response)
    }

    /// Who this peer is, and what protocol it speaks.
    pub async fn identity(&self) -> Result<Identity, ClientError> {
        match self.expect(Request::Identity, "identity").await? {
            Response::Identity(identity) => Ok(identity),
            other => Err(unexpected("identity", &other)),
        }
    }

    /// Aggregate status of the peer's sync server.
    pub async fn status(&self) -> Result<SyncStatus, ClientError> {
        match self
            .expect(
                Request::Status {
                    token: self.require_token()?,
                },
                "status",
            )
            .await?
        {
            Response::Status(status) => Ok(status),
            other => Err(unexpected("status", &other)),
        }
    }

    /// Changes after `since_version`, addressed to this device.
    pub async fn changes(
        &self,
        since_version: i64,
        limit: Option<i64>,
    ) -> Result<PullResponse, ClientError> {
        match self
            .expect(
                Request::Changes {
                    token: self.require_token()?,
                    since_version,
                    limit,
                },
                "changes",
            )
            .await?
        {
            Response::Changes(pull) => Ok(pull),
            other => Err(unexpected("changes", &other)),
        }
    }

    /// Every syncable row, for a device that starts from nothing.
    pub async fn pull_full(&self) -> Result<FullDump, ClientError> {
        match self
            .expect(
                Request::PullFull {
                    token: self.require_token()?,
                },
                "pull_full",
            )
            .await?
        {
            Response::Full(dump) => Ok(*dump),
            other => Err(unexpected("pull_full", &other)),
        }
    }

    /// Send a batch of local changes.
    pub async fn push(&self, changes: Vec<SyncChange>) -> Result<PushResponse, ClientError> {
        match self
            .expect(
                Request::Push {
                    token: self.require_token()?,
                    changes,
                },
                "push",
            )
            .await?
        {
            Response::Pushed(push) => Ok(push),
            other => Err(unexpected("push", &other)),
        }
    }

    /// File a pairing claim. The connection's node id is the identity.
    pub async fn pair(
        &self,
        device_id: &str,
        name: &str,
        device_type: &str,
    ) -> Result<PairingOutcome, ClientError> {
        let response = self
            .expect(
                Request::Pair {
                    device_id: device_id.to_string(),
                    name: name.to_string(),
                    device_type: device_type.to_string(),
                },
                "pair",
            )
            .await?;
        pairing_outcome("pair", &response)
    }

    /// What became of a claim this device filed.
    pub async fn pair_status(&self, claim: &str) -> Result<PairingOutcome, ClientError> {
        let response = self
            .expect(
                Request::PairStatus {
                    claim: claim.to_string(),
                },
                "pair_status",
            )
            .await?;
        pairing_outcome("pair_status", &response)
    }

    /// Unresolved conflicts on the peer.
    pub async fn conflicts(&self) -> Result<Vec<Conflict>, ClientError> {
        match self
            .expect(
                Request::Conflicts {
                    token: self.require_token()?,
                },
                "conflicts",
            )
            .await?
        {
            Response::Conflicts(conflicts) => Ok(conflicts),
            other => Err(unexpected("conflicts", &other)),
        }
    }

    /// Resolve one conflict; `true` when a row changed.
    pub async fn resolve_conflict(
        &self,
        id: i64,
        resolution: &str,
        merged_payload: Option<&str>,
    ) -> Result<bool, ClientError> {
        match self
            .expect(
                Request::ResolveConflict {
                    token: self.require_token()?,
                    id,
                    resolution: resolution.to_string(),
                    merged_payload: merged_payload.map(str::to_string),
                },
                "resolve_conflict",
            )
            .await?
        {
            Response::Resolved(resolved) => Ok(resolved),
            other => Err(unexpected("resolve_conflict", &other)),
        }
    }

    /// Open a file's byte phase. `offset`, `length` and `if_range` follow the
    /// range rules: an offset past the end is `RangeNotSatisfiable`, and a
    /// stale (or absent) `if_range` serves the whole file.
    pub async fn file(
        &self,
        inventory_id: &str,
        offset: u64,
        length: Option<u64>,
        if_range: Option<&str>,
    ) -> Result<(FileHeader, ByteBody), ClientError> {
        let exchange = self
            .send(Request::File {
                token: self.require_token()?,
                inventory_id: inventory_id.to_string(),
                offset,
                length,
                if_range: if_range.map(str::to_string),
            })
            .await?;
        let header = match &exchange.response {
            Response::File(header) => header.clone(),
            other => return Err(unexpected("file", other)),
        };
        let remaining = header.length;
        let stream = exchange
            .bytes
            .ok_or_else(|| unexpected("file", &exchange.response))?;
        Ok((header, ByteBody { stream, remaining }))
    }

    /// Size, content type and etag of one inventory file.
    pub async fn file_info(&self, inventory_id: &str) -> Result<FileMetadata, ClientError> {
        match self
            .expect(
                Request::FileInfo {
                    token: self.require_token()?,
                    inventory_id: inventory_id.to_string(),
                },
                "file_info",
            )
            .await?
        {
            Response::FileInfo(metadata) => Ok(metadata),
            other => Err(unexpected("file_info", &other)),
        }
    }

    /// Covers for editions, as the peer has them.
    ///
    /// Absent editions (no cover on that device) are omitted rather than reported: a
    /// cover is an ornament, and a library should not fail to sync over one.
    pub async fn covers(
        &self,
        edition_ids: &[String],
    ) -> Result<Vec<CoverImage>, ClientError> {
        let exchange = self
            .send(Request::Covers {
                token: self.require_token()?,
                edition_ids: edition_ids.to_vec(),
            })
            .await?;
        match exchange.response {
            Response::CoverBatch(covers) => Ok(covers),
            Response::Error {
                code: ErrorCode::NotFound,
                ..
            } => Ok(Vec::new()),
            other => Err(unexpected("covers", &other)),
        }
    }

    /// One inventory item's extracted cover, or `None` when it has none.
    pub async fn cover(
        &self,
        inventory_id: &str,
    ) -> Result<Option<(FileHeader, Vec<u8>)>, ClientError> {
        let exchange = match self
            .send(Request::Cover {
                token: self.require_token()?,
                inventory_id: inventory_id.to_string(),
            })
            .await
        {
            Ok(exchange) => exchange,
            Err(ClientError::Remote {
                code: ErrorCode::NotFound,
                ..
            }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let header = match &exchange.response {
            Response::Cover(header) => header.clone(),
            other => return Err(unexpected("cover", other)),
        };
        let stream = exchange
            .bytes
            .ok_or_else(|| unexpected("cover", &exchange.response))?;
        let body = ByteBody {
            stream,
            remaining: header.length,
        };
        Ok(Some((header, body.read_all().await?)))
    }

    /// Tell the sender what became of an offer it made.
    pub async fn offer_decision(&self, offer_id: &str, accepted: bool) -> Result<(), ClientError> {
        match self
            .expect(
                Request::OfferDecision {
                    token: self.require_token()?,
                    offer_id: offer_id.to_string(),
                    accepted,
                },
                "offer_decision",
            )
            .await?
        {
            Response::OfferDecided => Ok(()),
            other => Err(unexpected("offer_decision", &other)),
        }
    }
}

/// A reply this operation cannot use.
fn unexpected(operation: &'static str, response: &Response) -> ClientError {
    ClientError::Wire(WireError::Protocol(format!(
        "{operation}: unexpected reply {response:?}"
    )))
}

/// Fold a pairing reply into [`PairingOutcome`].
fn pairing_outcome(
    operation: &'static str,
    response: &Response,
) -> Result<PairingOutcome, ClientError> {
    match response {
        Response::PairingPending { claim } => Ok(PairingOutcome::Pending {
            claim: claim.clone(),
        }),
        Response::Paired {
            device_id,
            session_token,
        } => Ok(PairingOutcome::Paired {
            device_id: device_id.clone(),
            session_token: session_token.clone(),
        }),
        Response::Refused { reason } => Ok(PairingOutcome::Refused {
            reason: reason.clone(),
        }),
        other => Err(unexpected(operation, other)),
    }
}
