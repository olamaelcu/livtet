//! Pairing over iroh, exposed to the mobile clients.
//!
//! Pairing never touches HTTP and never needs a host or port: the peer is
//! addressed by its iroh node id, which the `_livtet-sync._tcp` TXT `id` record
//! carries. Dialling resolves the addresses of that id (mDNS on the LAN, the
//! relay off it), so the phone can pair with a desktop it can only reach
//! outbound.
//!
//! The endpoint's secret key is persisted at the path the caller names, because
//! the node id derived from it *is* this device's identity: the desktop records
//! the connection's node id when the claim is filed, and every later
//! `pair_status` poll must present the same one.

use std::time::{Duration, Instant};

use camino::Utf8Path;
use livtet_iroh::{DiscoveryService, SecretKey};
use livtet_sync_iroh::{Client, PairingOutcome};

use crate::error::LivtetError;

/// How long to wait between `pair_status` polls while the reader decides.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// What became of a pairing attempt.
///
/// `Pending` is not a failure: the claim is filed and the desktop has not
/// answered yet, so the caller can wait for the reader or poll again with
/// [`sync_pairing_status`] using the same `claim`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SyncPairingResult {
    /// The desktop approved; `session_token` authenticates later sync requests.
    Paired {
        device_id: String,
        session_token: String,
    },
    /// The desktop declined, or the claim could not be filed.
    Refused { reason: String },
    /// Filed and undecided; `claim` is what [`sync_pairing_status`] continues.
    Pending { claim: String },
}

/// File a pairing claim with `peer_node_id` and wait for the reader's decision.
///
/// Binds a sync endpoint using (or creating) the secret key at
/// `secret_key_path`, files the claim, then polls until the decision lands or
/// `wait_secs` elapses. A `Pending` result leaves the claim valid on the peer.
#[uniffi::export(async_runtime = "tokio")]
pub async fn pair_sync_device(
    secret_key_path: String,
    peer_node_id: String,
    device_id: String,
    name: String,
    device_type: String,
    wait_secs: u32,
) -> Result<SyncPairingResult, LivtetError> {
    let client = connect(secret_key_path, peer_node_id).await?;
    let outcome = client
        .pair(&device_id, &name, &device_type)
        .await
        .map_err(|error| LivtetError::Sync(format!("could not file the pairing claim: {error}")))?;

    match outcome {
        PairingOutcome::Pending { claim } => await_decision(&client, claim, wait_secs).await,
        PairingOutcome::Paired {
            device_id,
            session_token,
        } => Ok(SyncPairingResult::Paired {
            device_id,
            session_token,
        }),
        PairingOutcome::Refused { reason } => Ok(SyncPairingResult::Refused { reason }),
    }
}

/// Continue waiting on a claim already filed by [`pair_sync_device`].
///
/// Rebinding derives the same node id from the same persisted key, which is
/// what the peer keyed the claim on.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_pairing_status(
    secret_key_path: String,
    peer_node_id: String,
    claim: String,
    wait_secs: u32,
) -> Result<SyncPairingResult, LivtetError> {
    let client = connect(secret_key_path, peer_node_id).await?;
    await_decision(&client, claim, wait_secs).await
}

/// Bind a sync endpoint from the persisted key and aim a client at `node_id`.
async fn connect(secret_key_path: String, peer_node_id: String) -> Result<Client, LivtetError> {
    let secret = load_or_create_secret(Utf8Path::new(&secret_key_path))?;
    let discovery = DiscoveryService::bind_with_secret(secret)
        .await
        .map_err(|error| {
            LivtetError::Sync(format!("could not start the sync endpoint: {error}"))
        })?;
    Ok(Client::new(discovery.endpoint().clone(), peer_node_id))
}

/// Poll `pair_status` until the peer decides or `wait_secs` runs out.
async fn await_decision(
    client: &Client,
    first_claim: String,
    wait_secs: u32,
) -> Result<SyncPairingResult, LivtetError> {
    let deadline = Instant::now() + Duration::from_secs(u64::from(wait_secs));
    let mut claim = first_claim;

    loop {
        if Instant::now() >= deadline {
            return Ok(SyncPairingResult::Pending { claim });
        }
        tokio::time::sleep(POLL_INTERVAL).await;

        match client.pair_status(&claim).await.map_err(|error| {
            LivtetError::Sync(format!("could not read the pairing status: {error}"))
        })? {
            PairingOutcome::Pending { claim: next } => claim = next,
            PairingOutcome::Paired {
                device_id,
                session_token,
            } => {
                return Ok(SyncPairingResult::Paired {
                    device_id,
                    session_token,
                });
            }
            PairingOutcome::Refused { reason } => return Ok(SyncPairingResult::Refused { reason }),
        }
    }
}

/// Read the 32-byte iroh secret key at `path`, creating it on first use.
///
/// Stored as hex, the same form the desktop daemon keeps in the OS keyring. The
/// file holds the device's node identity, so it is never rewritten once valid;
/// a malformed file is an error rather than a silent regeneration, which would
/// strand the desktop's record of this device under an unreachable node id.
fn load_or_create_secret(path: &Utf8Path) -> Result<SecretKey, LivtetError> {
    if path.exists() {
        let encoded = std::fs::read_to_string(path)
            .map_err(|error| LivtetError::Sync(format!("could not read {path}: {error}")))?;
        let bytes: [u8; 32] = hex::decode(encoded.trim())
            .ok()
            .and_then(|decoded| decoded.try_into().ok())
            .ok_or_else(|| {
                LivtetError::Sync(format!("{path} does not hold a 32-byte hex secret key"))
            })?;
        return Ok(SecretKey::from_bytes(&bytes));
    }

    let secret = SecretKey::generate();
    if let Some(parent) = path.parent().filter(|parent| !parent.as_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| LivtetError::Sync(format!("could not create {parent}: {error}")))?;
    }
    std::fs::write(path, hex::encode(secret.to_bytes()))
        .map_err(|error| LivtetError::Sync(format!("could not write {path}: {error}")))?;
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use livtet_sync_iroh::{Handler, Outcome, Request, Response};
    use std::sync::Arc;

    /// File a claim against a peer that approves it, over a real iroh connection.
    ///
    /// Both endpoints live in this process: the peer answers `Pair` with a pending
    /// claim and `PairStatus` with an approval, so the test covers the whole path
    /// the phone takes — bind an endpoint from the persisted key, dial by node id,
    /// file the claim, poll, and surface the token — without a database or a second
    /// machine.
    #[tokio::test]
    async fn a_claim_is_filed_and_approved_over_iroh() {
        let peer = DiscoveryService::bind().await.expect("peer endpoint");
        let peer_node_id = peer.node_id().as_str().to_string();

        let handler: Handler = Arc::new(|request, _remote| {
            Box::pin(async move {
                match request {
                    Request::Pair { .. } => Outcome::reply(Response::PairingPending {
                        claim: "claim-1".to_string(),
                    }),
                    Request::PairStatus { claim } => {
                        assert_eq!(claim, "claim-1", "the filed claim is what gets polled");
                        Outcome::reply(Response::Paired {
                            device_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
                            session_token: "token-1".to_string(),
                        })
                    }
                    other => Outcome::reply(Response::Refused {
                        reason: format!("unexpected request: {other:?}"),
                    }),
                }
            })
        });
        let _serving = livtet_sync_iroh::spawn_server(peer.endpoint().clone(), handler);

        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let result = pair_sync_device(
            dir.path().join("sync.key").to_string(),
            peer_node_id,
            "01ARZ3NDEKTSV4RRFFQ69G5FAW".to_string(),
            "Reader's phone".to_string(),
            "android".to_string(),
            10,
        )
        .await
        .expect("pair call");

        assert_eq!(
            result,
            SyncPairingResult::Paired {
                device_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
                session_token: "token-1".to_string(),
            }
        );
    }

    #[test]
    fn a_key_is_created_once_and_reused() {
        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let path = dir.path().join("sync.key");

        let first = load_or_create_secret(&path).expect("create");
        let second = load_or_create_secret(&path).expect("reuse");

        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            hex::encode(first.to_bytes())
        );
    }

    #[test]
    fn a_malformed_key_is_an_error_rather_than_a_new_identity() {
        let dir = camino_tempfile::Utf8TempDir::new().expect("temp dir");
        let path = dir.path().join("sync.key");
        std::fs::write(&path, "not-a-key").expect("write");

        let error = load_or_create_secret(&path).expect_err("must reject");

        assert!(
            matches!(&error, LivtetError::Sync(message) if message.contains("32-byte hex")),
            "unexpected error: {error}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "not-a-key",
            "the bad file must be left alone"
        );
    }
}
