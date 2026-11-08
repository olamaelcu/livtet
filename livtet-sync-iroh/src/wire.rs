//! Framed sync requests over an iroh bidirectional stream.
//!
//! One request per stream. A message is framed in chunks: a `u32` big-endian
//! total length, then a series of `u32` big-endian chunk lengths each followed
//! by that many bytes. The chunks carry one JSON document between them; the
//! total is what bounds the reader, so a peer can stream a large push without
//! either side holding a single frame-sized prefix. File and cover reads write
//! their JSON header first and then the raw bytes, so a large book never has to
//! be buffered into one message.
//!
//! The operations mirror the HTTP routes this transport replaces one for one.
//! They are carried here, not in `livtet-sync`, because the envelopes are this
//! transport's business: the domain types (`SyncChange`, `PullResponse`, …)
//! stay where they are and are reused unchanged.
//!
//! One operation is not a sync operation: [`Request::Opds`] forwards an OPDS
//! request to a paired desktop and [`Response::Opds`] carries the route's HTTP
//! answer back. Its body is bytes, encoded inside the frame by serde's byte
//! array, so a PDF or an EPUB needs no UTF-8 conversion; because it rides in
//! the frame it is bounded by [`MAX_MESSAGE_BYTES`].

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use livtet_sync::types::{Conflict, FullDump, PullResponse, PushResponse, SyncChange, SyncStatus};

/// The protocol revision this codec speaks.
///
/// A peer that does not match is refused by the ALPN, and this value restates
/// the revision in the handshake so a mismatch is visible in a log rather than
/// only as a failed connection.
pub const PROTOCOL_VERSION: u32 = 2;

/// Largest message this transport will accept, across all chunks.
///
/// A push of a whole library is the biggest legitimate message; 256 MiB leaves
/// room for one while staying far below anything a corrupt length prefix could
/// make a reader allocate. Files do not go through messages, so this bound is
/// only about control traffic.
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024 * 1024;

/// Largest single chunk. A writer emits at most this much per length prefix so
/// a reader can copy chunk by chunk instead of buffering the whole message.
pub const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// The identity a peer can confirm before pairing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// The protocol revision the peer speaks, restating the ALPN.
    pub protocol: u32,
    /// iroh endpoint id, which is also the pairing key.
    pub node_id: String,
    /// The local ULID the peer's sync rows are written against.
    pub device_id: String,
    /// The name this device advertises, when it has one.
    ///
    /// A peer that pairs by node id never saw an mDNS advertisement, so this is
    /// the only way its paired-devices row can be labelled with the other
    /// device's name rather than with its own. Absent from older peers, which is
    /// why it is optional and not sent when empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One operation on a sync stream.
///
/// `token` is the session token minted at approval; it is absent from the
/// operations that must work before a pairing exists (`Identity`, `Pair`,
/// `PairStatus`). The caller's node id is never a payload field: it is the
/// authenticated QUIC connection identity.
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
    /// File a pairing claim. The claim is keyed by the connection's node id,
    /// and the answer arrives on a later `PairStatus` on any connection.
    Pair {
        device_id: String,
        name: String,
        device_type: String,
    },
    /// What became of a claim this device filed, named by the claim secret.
    PairStatus { claim: String },
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
    ///
    /// `if_range` is the etag the caller already holds: when it still matches,
    /// the requested range is served; when it does not, the whole file is.
    File {
        token: String,
        inventory_id: String,
        offset: u64,
        length: Option<u64>,
        if_range: Option<String>,
    },
    /// Size, content type and etag of one inventory file, without its bytes.
    FileInfo { token: String, inventory_id: String },
    /// The extracted cover of one inventory item.
    Cover { token: String, inventory_id: String },
    /// An OPDS request for a paired desktop to serve in-process.
    ///
    /// `path` carries the query string with the path (`/search?query=dune`),
    /// so a page, a search, or a feed is one field. `headers` carries the
    /// negotiation headers that change what the route returns. `body` is raw
    /// bytes for a method that has one and empty for a `GET`; it is encoded as
    /// a byte array, never as text, and counts against [`MAX_MESSAGE_BYTES`]
    /// like every other frame payload. The answer is [`Response::Opds`].
    Opds {
        /// The HTTP method, upper-case (`GET`, `POST`, …).
        method: String,
        /// Path and query, starting with `/`.
        path: String,
        /// The headers the route consults to pick a representation.
        headers: OpdsHeaders,
        /// Request body bytes; empty when the method carries none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        body: Vec<u8>,
    },
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
    /// The claim was filed; the reader decides out of band.
    PairingPending {
        claim: String,
    },
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
    /// Size, content type and etag of one inventory file.
    FileInfo(FileMetadata),
    /// A cover header; the caller reads `length` raw bytes next.
    Cover(FileHeader),
    /// An OPDS answer: the route's HTTP status, its media type, and its body.
    ///
    /// A non-2xx status is an ordinary answer, not [`Response::Error`]: a 404
    /// for an unknown publication or a 304 for an unchanged feed is exactly
    /// what the e-reader must see. The body is raw bytes inside the frame, so
    /// it is charged against [`MAX_MESSAGE_BYTES`]; callers serving a larger
    /// publication must chunk it across several `Opds` requests.
    Opds(OpdsResponse),
    /// The sender recorded the decision.
    OfferDecided,
    /// The request could not be served. The code says why without the caller
    /// having to parse the message.
    Error {
        code: ErrorCode,
        message: String,
    },
}

/// Why a request could not be served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Malformed request, a bad id, or a resolution this side does not know.
    BadRequest,
    /// Unknown session token, or one issued to another node.
    Unauthorized,
    /// No such row, claim, or file.
    NotFound,
    /// The request conflicts with the current state.
    Conflict,
    /// The caller is over its budget for this operation.
    TooManyRequests,
    /// The requested offset is past the end of the file.
    RangeNotSatisfiable {
        /// The current file size, so the caller can restart from a known end.
        size: u64,
    },
    /// A conditional request whose precondition no longer holds.
    PreconditionFailed,
    /// The server failed while serving the request.
    Internal,
}

impl ErrorCode {
    /// A stable lower-case name for logs and request records.
    pub fn label(&self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::TooManyRequests => "too_many_requests",
            Self::RangeNotSatisfiable { .. } => "range_not_satisfiable",
            Self::PreconditionFailed => "precondition_failed",
            Self::Internal => "internal",
        }
    }
}

/// Content type and range for the byte phase that follows. `total` and `etag`
/// describe the whole file, so a ranged read still knows what it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHeader {
    pub content_type: String,
    /// Byte offset this phase starts at.
    pub offset: u64,
    /// Bytes in the phase that follows.
    pub length: u64,
    /// Size of the whole file.
    pub total: u64,
    /// Opaque version tag, sent back as `if_range` to continue a download.
    pub etag: String,
}

/// A file's metadata, as `FileInfo` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMetadata {
    pub content_type: String,
    pub size: u64,
    pub etag: String,
}

/// The negotiation headers of an OPDS request.
///
/// Only headers that change the representation travel: `Host` is rewritten by
/// the desktop itself, `Authorization` would duplicate the authenticated QUIC
/// connection, and hop-by-hop headers mean nothing across a frame. An absent
/// header is `None` rather than an empty string, so a route can tell "not
/// sent" from "sent empty".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpdsHeaders {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept_encoding: Option<String>,
}

/// An OPDS route's answer, as one HTTP-shaped value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpdsResponse {
    /// The HTTP status the route produced.
    pub status: u16,
    /// Media type of `body`, e.g. `application/atom+xml;profile=opds-catalog`.
    pub content_type: String,
    /// Raw response bytes: a feed document or a publication, never text-only.
    pub body: Vec<u8>,
}

impl Response {
    /// Whether this answer fits one framed message.
    ///
    /// The body rides inside the frame, so the whole encoded response — not
    /// just `body` — is charged against [`MAX_MESSAGE_BYTES`], which lives in
    /// this module and is 256 MiB (`MAX_MESSAGE_BYTES = 256 * 1024 * 1024`).
    /// `false` is the signal to split the body across several `Opds` requests
    /// rather than hand `write_frame` a message it must refuse.
    pub fn fits_one_frame(&self) -> bool {
        serde_json::to_vec(self).is_ok_and(|encoded| encoded.len() <= MAX_MESSAGE_BYTES)
    }
}

/// Why a frame could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("frame io: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame body: {0}")]
    Json(#[from] serde_json::Error),
    #[error("message of {size} bytes exceeds the {MAX_MESSAGE_BYTES}-byte limit")]
    TooLarge { size: u64 },
    #[error("chunk of {chunk} bytes overflows the {total}-byte message after {read} bytes")]
    ChunkOverflow { total: u64, chunk: u64, read: u64 },
    #[error("empty chunk in a {total}-byte message")]
    EmptyChunk { total: u64 },
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("stream: {0}")]
    Stream(String),
}

/// Write one chunked JSON message.
///
/// The total length goes first so the reader knows how much to expect and
/// can refuse an implausible size before reading a byte of body.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<(), WireError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_MESSAGE_BYTES {
        return Err(WireError::TooLarge {
            size: body.len() as u64,
        });
    }
    writer.write_all(&(body.len() as u32).to_be_bytes()).await?;
    for chunk in body.chunks(MAX_CHUNK_BYTES) {
        writer
            .write_all(&(chunk.len() as u32).to_be_bytes())
            .await?;
        writer.write_all(chunk).await?;
    }
    writer.flush().await?;
    Ok(())
}

/// Read one chunked JSON message.
///
/// A closed stream before the length prefix is an `UnexpectedEof`; a closed
/// stream mid-body too. Both mean the peer went away, which callers report as
/// an unreachable peer rather than a protocol error.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<T, WireError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut total = [0u8; 4];
    reader.read_exact(&mut total).await?;
    let total = u64::from(u32::from_be_bytes(total));
    if total > MAX_MESSAGE_BYTES as u64 {
        return Err(WireError::TooLarge { size: total });
    }

    // Grown one chunk at a time: a peer that claims a large total and hangs up
    // costs only what it actually sent, not the size it claimed.
    let mut body: Vec<u8> = Vec::new();
    while (body.len() as u64) < total {
        let mut chunk = [0u8; 4];
        reader.read_exact(&mut chunk).await?;
        let chunk = u64::from(u32::from_be_bytes(chunk));
        if chunk > MAX_CHUNK_BYTES as u64 {
            return Err(WireError::TooLarge { size: chunk });
        }
        if chunk == 0 {
            return Err(WireError::EmptyChunk { total });
        }
        let read = body.len() as u64;
        if read + chunk > total {
            return Err(WireError::ChunkOverflow { total, chunk, read });
        }
        body.resize((read + chunk) as usize, 0);
        reader.read_exact(&mut body[read as usize..]).await?;
    }
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

    /// A message larger than one chunk has to be split, sent, and reassembled
    /// exactly: this is the property that replaces the old single-frame size
    /// limit for a whole-library push.
    #[tokio::test]
    async fn a_message_bigger_than_one_chunk_round_trips() {
        let big = "a".repeat(MAX_CHUNK_BYTES + 1024);
        let expected = big.len();
        let (mut writer, mut reader) = tokio::io::duplex(64 * 1024);

        let sending = tokio::spawn(async move {
            write_frame(&mut writer, &big).await.unwrap();
        });
        let read: String = read_frame(&mut reader).await.unwrap();
        sending.await.unwrap();

        assert_eq!(read.len(), expected);
        assert!(read.bytes().all(|byte| byte == b'a'));
    }

    /// A peer claiming an enormous total must be refused before the reader
    /// allocates anything for it.
    #[tokio::test]
    async fn an_oversized_total_is_refused_without_buffering_it() {
        let (mut writer, mut reader) = tokio::io::duplex(16);
        let limit = (MAX_MESSAGE_BYTES as u64) + 1;

        writer
            .write_all(&u32::try_from(limit).unwrap().to_be_bytes())
            .await
            .unwrap();
        let error = read_frame::<_, Request>(&mut reader).await.unwrap_err();

        assert!(matches!(
            error,
            WireError::TooLarge { size } if size == limit
        ));
    }

    /// A chunk longer than the cap is refused rather than grown into.
    #[tokio::test]
    async fn an_oversized_chunk_is_refused() {
        let (mut writer, mut reader) = tokio::io::duplex(16);
        let total = 8u32;
        let chunk = u32::try_from(MAX_CHUNK_BYTES + 1).unwrap();

        writer.write_all(&total.to_be_bytes()).await.unwrap();
        writer.write_all(&chunk.to_be_bytes()).await.unwrap();
        let error = read_frame::<_, Request>(&mut reader).await.unwrap_err();

        assert!(
            matches!(error, WireError::TooLarge { size } if size == u64::from(chunk)),
            "got {error:?}"
        );
    }

    /// A chunk that runs past the declared total is a corrupt frame, not a
    /// reason to allocate past it.
    #[tokio::test]
    async fn a_chunk_past_the_total_is_refused() {
        let (mut writer, mut reader) = tokio::io::duplex(16);

        writer.write_all(&4u32.to_be_bytes()).await.unwrap();
        writer.write_all(&8u32.to_be_bytes()).await.unwrap();
        let error = read_frame::<_, Request>(&mut reader).await.unwrap_err();

        assert!(
            matches!(
                error,
                WireError::ChunkOverflow {
                    total: 4,
                    chunk: 8,
                    read: 0
                }
            ),
            "got {error:?}"
        );
    }

    /// A peer that hangs up mid-frame is unreachable, not malformed.
    #[tokio::test]
    async fn a_closed_stream_reads_as_io_error() {
        let (writer, mut reader) = tokio::io::duplex(64);
        drop(writer);

        let error = read_frame::<_, Request>(&mut reader).await.unwrap_err();

        assert!(matches!(error, WireError::Io(_)), "got {error:?}");
    }

    /// The tag is the contract both sides match on; a rename would silently
    /// break every peer that speaks the previous release.
    #[test]
    fn request_tags_are_stable() {
        let encoded = serde_json::to_value(Request::Identity).unwrap();
        assert_eq!(encoded["op"], "identity");

        let pair_status = serde_json::to_value(Request::PairStatus {
            claim: "claim".to_string(),
        })
        .unwrap();
        assert_eq!(pair_status["op"], "pair_status");

        let file_info = serde_json::to_value(Request::FileInfo {
            token: "token".to_string(),
            inventory_id: "id".to_string(),
        })
        .unwrap();
        assert_eq!(file_info["op"], "file_info");

        let opds = serde_json::to_value(Request::Opds {
            method: "GET".to_string(),
            path: "/search?query=dune".to_string(),
            headers: OpdsHeaders::default(),
            body: Vec::new(),
        })
        .unwrap();
        assert_eq!(opds["op"], "opds");
        assert_eq!(opds["path"], "/search?query=dune");
    }

    /// OPDS bodies are arbitrary bytes: a publication download must survive
    /// the frame without a UTF-8 conversion turning it into mojibake.
    #[tokio::test]
    async fn an_opds_request_round_trips_a_query_and_binary_body() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let body = vec![0x00, 0xff, 0x80, b'%', 0xc3, 0x28, 0x00, 0x9f, 0x92, 0x96];
        let request = Request::Opds {
            method: "POST".to_string(),
            path: "/search?query=dune&page=2".to_string(),
            headers: OpdsHeaders {
                accept: Some("application/atom+xml;profile=opds-catalog".to_string()),
                accept_language: Some("de".to_string()),
                accept_encoding: None,
            },
            body: body.clone(),
        };

        write_frame(&mut client, &request).await.unwrap();
        let read: Request = read_frame(&mut server).await.unwrap();

        match read {
            Request::Opds {
                method,
                path,
                headers,
                body: read_body,
            } => {
                assert_eq!(method, "POST");
                assert_eq!(path, "/search?query=dune&page=2");
                assert_eq!(
                    headers.accept.as_deref(),
                    Some("application/atom+xml;profile=opds-catalog")
                );
                assert_eq!(headers.accept_language.as_deref(), Some("de"));
                assert_eq!(headers.accept_encoding, None);
                assert_eq!(read_body, body);
            }
            other => panic!("expected an opds request, got {other:?}"),
        }
    }

    /// A non-2xx OPDS status is a served answer, and its body is bytes too.
    #[tokio::test]
    async fn an_opds_response_round_trips_a_non_200_status() {
        let (mut server, mut client) = tokio::io::duplex(64 * 1024);
        let body = vec![0x00, 0x9f, 0x92, 0x96, 0xff];
        let response = Response::Opds(OpdsResponse {
            status: 404,
            content_type: "application/atom+xml;profile=opds-catalog".to_string(),
            body: body.clone(),
        });

        write_frame(&mut server, &response).await.unwrap();
        let read: Response = read_frame(&mut client).await.unwrap();

        match read {
            Response::Opds(opds) => {
                assert_eq!(opds.status, 404);
                assert_eq!(
                    opds.content_type,
                    "application/atom+xml;profile=opds-catalog"
                );
                assert_eq!(opds.body, body);
                assert!(Response::Opds(opds).fits_one_frame());
            }
            other => panic!("expected an opds response, got {other:?}"),
        }
    }

    /// A publication body larger than one chunk is carried as several chunks of
    /// one framed message, and arrives byte for byte: the 8 MiB chunk cap
    /// bounds a write syscall, not the answer.
    #[tokio::test]
    async fn an_opds_body_larger_than_one_chunk_round_trips() {
        let body: Vec<u8> = (0..MAX_CHUNK_BYTES + 1024)
            .map(|i| (i % 251) as u8)
            .collect();
        let expected = body.clone();
        let response = Response::Opds(OpdsResponse {
            status: 200,
            content_type: "application/epub+zip".to_string(),
            body,
        });
        assert!(
            response.fits_one_frame(),
            "a body past one chunk still fits one framed message"
        );

        let (mut writer, mut reader) = tokio::io::duplex(64 * 1024);
        let sending = tokio::spawn(async move {
            write_frame(&mut writer, &response).await.unwrap();
        });
        let read: Response = read_frame(&mut reader).await.unwrap();
        sending.await.unwrap();

        match read {
            Response::Opds(opds) => {
                assert_eq!(opds.body.len(), expected.len());
                assert_eq!(opds.body, expected);
            }
            other => panic!("expected the publication, got {other:?}"),
        }
    }

    #[test]
    fn response_tags_are_stable() {
        let encoded = serde_json::to_value(Response::Error {
            code: ErrorCode::Unauthorized,
            message: "bad token".to_string(),
        })
        .unwrap();
        assert_eq!(encoded["result"], "error");
        assert_eq!(encoded["code"], "unauthorized");

        let pending = serde_json::to_value(Response::PairingPending {
            claim: "claim".to_string(),
        })
        .unwrap();
        assert_eq!(pending["result"], "pairing_pending");

        let opds = serde_json::to_value(Response::Opds(OpdsResponse {
            status: 404,
            content_type: "application/atom+xml".to_string(),
            body: Vec::new(),
        }))
        .unwrap();
        assert_eq!(opds["result"], "opds");
        assert_eq!(opds["status"], 404);
    }

    /// A range failure carries the size the caller needs to restart from.
    #[test]
    fn a_range_failure_carries_the_size() {
        let encoded = serde_json::to_value(Response::Error {
            code: ErrorCode::RangeNotSatisfiable { size: 4096 },
            message: "past the end".to_string(),
        })
        .unwrap();

        assert_eq!(encoded["code"]["range_not_satisfiable"]["size"], 4096);
        assert_eq!(
            ErrorCode::RangeNotSatisfiable { size: 4096 }.label(),
            "range_not_satisfiable"
        );
    }

    /// The identity states the revision, so a connected peer can log a
    /// mismatch even though the ALPN already refused the incompatible ones.
    #[tokio::test]
    async fn the_identity_carries_the_protocol_version() {
        let identity = Identity {
            protocol: PROTOCOL_VERSION,
            node_id: "node".to_string(),
            device_id: "device".to_string(),
            name: Some("Desktop".to_string()),
        };
        let (mut writer, mut reader) = tokio::io::duplex(1024);

        write_frame(&mut writer, &Response::Identity(identity.clone()))
            .await
            .unwrap();
        let read: Response = read_frame(&mut reader).await.unwrap();

        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            serde_json::to_value(Response::Identity(identity)).unwrap()
        );
    }
}
