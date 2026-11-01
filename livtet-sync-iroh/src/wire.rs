//! Framed sync requests over an iroh bidirectional stream.
//!
//! One request per stream. A frame is a `u32` big-endian length followed by
//! that many bytes of JSON; the response is a single frame on the same stream.
//! File and cover reads write their JSON header first and then the raw bytes,
//! so a large book never has to be buffered into one frame.
//!
//! The operations mirror the HTTP routes this transport replaces one for one.
//! They are carried here, not in `livtet-sync`, because the envelopes are this
//! transport's business: the domain types (`SyncChange`, `PullResponse`, …)
//! stay where they are and are reused unchanged.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use livtet_sync::types::{Conflict, FullDump, PullResponse, PushResponse, SyncChange, SyncStatus};

/// Largest JSON frame this transport will accept.
///
/// Every control message is small — a change batch is the biggest, and the
/// daemon's own pull limit bounds it — so anything approaching this is either a
/// corrupt length prefix or a peer trying to make the reader allocate. Files do
/// not go through frames, so this bound stays low.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// The identity a peer can confirm before pairing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// iroh endpoint id, which is also the pairing key.
    pub node_id: String,
    /// The local ULID the peer's sync rows are written against.
    pub device_id: String,
}

/// One operation on a sync stream.
///
/// `token` is the session token minted at approval; it is absent from the
/// operations that must work before a pairing exists (`Identity`, `Pair`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Who this peer is. Unauthenticated, like the HTTP identity route was.
    Identity,
    /// Aggregate status of this peer's sync server.
    Status { token: String },
    /// Changes after a version, addressed to the caller's device.
    Changes {
        token: String,
        since_version: i64,
        limit: Option<i64>,
    },
    /// Every syncable row, for a peer that starts from nothing.
    PullFull { token: String },
    /// A batch of local changes.
    Push {
        token: String,
        changes: Vec<SyncChange>,
    },
    /// Ask to be paired. The reply arrives when the reader decides, on this
    /// same stream.
    Pair {
        device_id: String,
        name: String,
        device_type: String,
        node_id: String,
    },
    /// Unresolved conflicts.
    Conflicts { token: String },
    /// Resolution for one conflict.
    ResolveConflict {
        token: String,
        id: i64,
        resolution: String,
        merged_payload: Option<String>,
    },
    /// Bytes of one inventory file, from `offset`, at most `length` of them.
    File {
        token: String,
        inventory_id: String,
        offset: u64,
        length: Option<u64>,
    },
    /// The extracted cover of one inventory item.
    Cover { token: String, inventory_id: String },
    /// What became of an offer this peer sent.
    OfferDecision {
        token: String,
        offer_id: String,
        accepted: bool,
    },
}

/// The answer to one [`Request`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Identity(Identity),
    Status(SyncStatus),
    Changes(PullResponse),
    Full(Box<FullDump>),
    Pushed(PushResponse),
    /// The pairing was approved; the peer may use this token from now on.
    Paired {
        device_id: String,
        session_token: String,
    },
    /// The pairing was refused, or could not be filed.
    Refused {
        reason: String,
    },
    Conflicts(Vec<Conflict>),
    Resolved(bool),
    /// A file header; the caller reads `length` raw bytes next.
    File(FileHeader),
    /// A cover header; the caller reads `length` raw bytes next.
    Cover(FileHeader),
    /// The sender recorded the decision.
    OfferDecided,
    /// Nothing was found for the id, or the range was empty.
    Missing {
        reason: String,
    },
    /// The request was understood but refused: bad token, unknown device.
    Denied {
        reason: String,
    },
}

/// Content type and size for the byte phase that follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHeader {
    pub content_type: String,
    /// Byte offset this phase starts at, which is the request's `offset`.
    pub offset: u64,
    pub length: u64,
}

/// Why a frame could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("frame io: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame body: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame of {size} bytes exceeds the {MAX_FRAME_BYTES}-byte limit")]
    TooLarge { size: u32 },
    #[error("stream: {0}")]
    Stream(String),
}

/// Write one length-prefixed JSON frame.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<(), WireError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(value)?;
    let size = u32::try_from(body.len()).unwrap_or(u32::MAX);
    if body.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge { size });
    }
    writer.write_all(&size.to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

/// Read one length-prefixed JSON frame.
///
/// A closed stream before the length prefix is an `UnexpectedEof`; a closed
/// stream mid-body too. Both mean the peer went away, which callers report as
/// an unreachable peer rather than a protocol error.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<T, WireError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut length = [0u8; 4];
    reader.read_exact(&mut length).await?;
    let size = u32::from_be_bytes(length);
    if size as usize > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge { size });
    }
    let mut body = vec![0u8; size as usize];
    reader.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[tokio::test]
    async fn a_request_survives_a_round_trip() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let request = Request::Changes {
            token: "tok".to_string(),
            since_version: 41,
            limit: Some(100),
        };

        write_frame(&mut client, &request).await.unwrap();
        let read: Request = read_frame(&mut server).await.unwrap();

        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            serde_json::to_value(&request).unwrap()
        );
    }

    #[tokio::test]
    async fn a_response_survives_a_round_trip() {
        let (mut server, mut client) = tokio::io::duplex(1024);
        let response = Response::Paired {
            device_id: "01M652BF35STC9CWKPF4WKDAZV".to_string(),
            session_token: "token".to_string(),
        };

        write_frame(&mut server, &response).await.unwrap();
        let read: Response = read_frame(&mut client).await.unwrap();

        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            serde_json::to_value(&response).unwrap()
        );
    }

    /// The tag is the contract both sides match on; a rename would silently
    /// break every peer that speaks the previous release.
    #[test]
    fn request_tags_are_stable() {
        let encoded = serde_json::to_value(Request::Identity).unwrap();

        assert_eq!(encoded["op"], "identity");
    }

    #[test]
    fn response_tags_are_stable() {
        let encoded = serde_json::to_value(Response::Denied {
            reason: "bad token".to_string(),
        })
        .unwrap();

        assert_eq!(encoded["result"], "denied");
    }

    /// A peer claiming an enormous frame must be refused before allocating it.
    #[tokio::test]
    async fn an_oversized_frame_is_refused_without_buffering_it() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        let limit = u32::try_from(MAX_FRAME_BYTES).unwrap() + 1;

        writer.write_all(&limit.to_be_bytes()).await.unwrap();
        let error = read_frame::<_, Request>(&mut reader).await.unwrap_err();

        assert!(matches!(
            error,
            WireError::TooLarge { size } if size == limit
        ));
    }

    /// A peer that hangs up mid-frame is unreachable, not malformed.
    #[tokio::test]
    async fn a_closed_stream_reads_as_io_error() {
        let (writer, mut reader) = tokio::io::duplex(64);
        drop(writer);

        let error = read_frame::<_, Request>(&mut reader).await.unwrap_err();

        assert!(matches!(error, WireError::Io(_)), "got {error:?}");
    }
}
