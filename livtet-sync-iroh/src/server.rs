//! Accept side of the framed protocol.
//!
//! One task per connection and one per stream: a pairing claim holds its stream
//! open for as long as the reader takes to approve, which must not stop the
//! next peer from asking for changes.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use iroh::Endpoint;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::wire::{FileHeader, Request, Response, WireError, read_frame, write_frame};

/// A future a [`Handler`] returns.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The bytes that follow a [`Response::File`] or [`Response::Cover`] header.
///
/// A path rather than a buffer: a book is opened and streamed, never held in
/// memory whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBody {
    pub path: PathBuf,
    pub offset: u64,
    pub length: u64,
}

/// What a handler answered, plus the bytes to send after the header.
pub struct Outcome {
    pub response: Response,
    pub body: Option<FileBody>,
}

impl Outcome {
    /// An answer with no byte phase.
    pub fn reply(response: Response) -> Self {
        Self {
            response,
            body: None,
        }
    }

    /// An answer whose header is followed by `length` bytes of `path`.
    pub fn file(header: FileHeader, path: PathBuf) -> Self {
        let body = FileBody {
            path,
            offset: header.offset,
            length: header.length,
        };
        Self {
            response: Response::File(header),
            body: Some(body),
        }
    }

    /// An answer whose cover header is followed by `length` bytes of `path`.
    pub fn cover(header: FileHeader, path: PathBuf) -> Self {
        let body = FileBody {
            path,
            offset: header.offset,
            length: header.length,
        };
        Self {
            response: Response::Cover(header),
            body: Some(body),
        }
    }
}

/// Answers one request from the peer whose node id is the second argument.
pub type Handler = Arc<dyn Fn(Request, String) -> BoxFuture<Outcome> + Send + Sync>;

/// Accept connections on `endpoint` until it is closed.
///
/// The returned task ends when the endpoint does, so the caller can await it
/// during shutdown to let in-flight streams finish.
pub fn spawn(endpoint: Endpoint, handler: Handler) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                let Ok(accepting) = incoming.accept() else {
                    return;
                };
                let Ok(connection) = accepting.await else {
                    return;
                };
                serve_connection(connection, handler).await;
            });
        }
    })
}

/// Answer every stream this peer opens, until the connection closes.
async fn serve_connection(connection: Connection, handler: Handler) {
    let remote = connection.remote_id().to_string();
    loop {
        let Ok((send, recv)) = connection.accept_bi().await else {
            break;
        };
        let handler = Arc::clone(&handler);
        let remote = remote.clone();
        tokio::spawn(async move {
            if let Err(error) = exchange(send, recv, handler, remote).await {
                tracing::debug!(%error, "sync stream ended early");
            }
        });
    }
}

/// Read one request, run the handler, write the answer and any byte phase.
async fn exchange(
    mut send: SendStream,
    mut recv: RecvStream,
    handler: Handler,
    remote: String,
) -> Result<(), WireError> {
    let request: Request = read_frame(&mut recv).await?;
    let Outcome { response, body } = handler(request, remote).await;

    write_frame(&mut send, &response).await?;
    if let Some(body) = body {
        let mut file = tokio::fs::File::open(&body.path).await?;
        file.seek(std::io::SeekFrom::Start(body.offset)).await?;
        let mut window = file.take(body.length);
        tokio::io::copy(&mut window, &mut send).await?;
    }
    let _ = send.finish();
    Ok(())
}
