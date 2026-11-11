//! Client for the platform's pairing rendezvous.
//!
//! It is the code a reader types, not the transport, that this crate owns. Two
//! devices that share no network can still pair if one publishes its iroh node
//! id under a short code and the other resolves that code to the id it must
//! dial; the platform serves as the directory in between, and iroh still carries
//! the claim itself. So the whole HTTP surface is two calls, and everything here
//! exists to keep them agreeing with the platform's implementation.
//!
//! The one place they can silently disagree is the code itself. The platform
//! uppercases what a reader types and drops the separators they naturally
//! insert, so a code copied off a screen with a space in the middle still
//! resolves. A second implementation of that rule which differs by a character
//! turns into "the code is wrong" for a code that works everywhere else, which
//! is why [`normalize_code`] follows the platform's alphabet exactly rather than
//! being generous.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Where the platform's rendezvous lives unless a caller says otherwise.
pub const DEFAULT_SERVER: &str = "https://livtet.olamaelcu.net";

/// Length of a pairing code, before and after normalization.
pub const CODE_LENGTH: usize = 6;

/// The characters a code may carry: Crockford base32.
///
/// `I`, `L`, `O` and `U` are absent because a reader confuses them with `1`,
/// `1`, `0` and `V` — which is also why normalization upper-cases before
/// checking: a typed `o` becomes the `O` that is *not* in the set, so it is
/// rejected rather than silently read as a zero and pointed at another device.
const CODE_ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Path of the rendezvous endpoints, relative to the configured base.
const CODE_PATH: &str = "/api/iroh/rendezvous";

/// Why a code could not be published or resolved.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RendezvousError {
    /// What was typed is not a code, so no request was made.
    #[error("not a pairing code: {0}")]
    InvalidCode(String),
    /// The request never reached the server — DNS, connect, TLS, or timeout.
    /// Kept apart from [`Self::Server`] because nothing was answered, so there
    /// is no status to report and the caller may sensibly retry.
    #[error("could not reach the rendezvous server: {0}")]
    Unreachable(String),
    /// The server has no such code: it expired, or it was never issued.
    #[error("that code has expired")]
    Expired,
    /// The server answered, and refused. The message is the body's own when it
    /// carries one, so the UI can show the platform's wording.
    #[error("the rendezvous server answered {status}: {message}")]
    Server { status: u16, message: String },
    /// A successful answer this client cannot read.
    ///
    /// A dedicated variant rather than a fabricated `Server` status: the
    /// response was a 2xx, so quoting a status here would misdirect whoever
    /// reads the error, while the payload really is the server's fault.
    #[error("the rendezvous server sent an answer this client cannot read: {0}")]
    Decode(String),
    /// The HTTP client itself could not be built, which is local (TLS setup)
    /// and fatal for every request this client would have made.
    #[error("could not build the rendezvous client: {0}")]
    Config(String),
}

/// A code this device published, and when the platform forgets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedCode {
    /// The code to show the reader.
    pub code: String,
    /// Expiry as epoch milliseconds, exactly as the platform reports it.
    pub expires_at: i64,
}

/// The device a code names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendezvousPeer {
    /// iroh node id to dial.
    pub node_id: String,
    /// Name the publishing device advertised, when it had one.
    pub name: Option<String>,
    /// Flavour the publisher reported, e.g. `"mobile"`.
    pub device_type: String,
}

/// A short code and a device directory, over HTTPS.
#[derive(Debug, Clone)]
pub struct RendezvousClient {
    /// Base URL without a trailing slash.
    base: String,
    http: reqwest::Client,
}

impl RendezvousClient {
    /// Build a client pointed at `base` (a bare origin, e.g.
    /// [`DEFAULT_SERVER`]). A trailing `/` is dropped so paths joined onto it
    /// never double up.
    pub fn new(base: &str) -> Result<Self, RendezvousError> {
        let http = reqwest::Client::builder()
            .use_preconfigured_tls(tls_config()?)
            .build()
            .map_err(|error| RendezvousError::Config(error.to_string()))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            http,
        })
    }

    /// Publish `node_id` under a fresh code and return the code.
    ///
    /// A `name` of `None` is omitted from the body rather than sent as null,
    /// because the platform already has a fallback for a nameless device and a
    /// present-but-null field would bypass it.
    pub async fn publish(
        &self,
        node_id: &str,
        name: Option<&str>,
        device_type: &str,
    ) -> Result<PublishedCode, RendezvousError> {
        let response = self
            .http
            .post(format!("{}{CODE_PATH}", self.base))
            .json(&PublishRequest {
                node_id,
                name,
                device_type,
            })
            .send()
            .await
            .map_err(unreachable)?;

        let status = response.status();
        if !status.is_success() {
            return Err(self.server_error(response).await);
        }

        let body: PublishResponse = response
            .json()
            .await
            .map_err(|error| RendezvousError::Decode(error.to_string()))?;
        Ok(PublishedCode {
            code: body.code,
            expires_at: body.expires_at,
        })
    }

    /// Look up the device a code names.
    ///
    /// The code is normalized first, so callers may pass what the reader typed.
    /// An expired code and an unknown one are one answer — 404 — and both mean
    /// the same thing to the person holding the code: ask for a fresh one.
    pub async fn resolve(&self, code: &str) -> Result<RendezvousPeer, RendezvousError> {
        let code = normalize_code(code)?;
        let response = self
            .http
            .get(format!("{}{CODE_PATH}/{code}", self.base))
            .send()
            .await
            .map_err(unreachable)?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            tracing::debug!("rendezvous code not found; it expired or was never issued");
            return Err(RendezvousError::Expired);
        }
        if !status.is_success() {
            return Err(self.server_error(response).await);
        }

        // The answer's `expiresAt` is deliberately not surfaced: a code the
        // server still knows is by definition unexpired, so a caller given it
        // would only have a second, staler source of truth.
        let body: ResolveResponse = response
            .json()
            .await
            .map_err(|error| RendezvousError::Decode(error.to_string()))?;
        Ok(RendezvousPeer {
            node_id: body.node_id,
            name: body.name,
            device_type: body.device_type,
        })
    }

    /// Turn a refusal into an error, preferring the platform's own wording.
    async fn server_error(&self, response: reqwest::Response) -> RendezvousError {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ErrorBody>(&body)
            .ok()
            .map(|body| body.message)
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| {
                if body.is_empty() {
                    format!("HTTP {status}")
                } else {
                    body
                }
            });
        tracing::debug!(status, %message, "rendezvous server refused the request");
        RendezvousError::Server { status, message }
    }
}

/// Normalize a typed code into the form the platform keys on.
///
/// Uppercase, with spaces and `-` removed; anything else must already be six
/// characters of [`CODE_ALPHABET`]. Codes are short enough that a reader copies
/// them by eye, so the separators they add are a feature of the input, not an
/// error to report.
pub fn normalize_code(raw: &str) -> Result<String, RendezvousError> {
    let code: String = raw
        .chars()
        .filter(|character| !matches!(character, ' ' | '-'))
        .map(|character| character.to_ascii_uppercase())
        .collect();
    let valid = code.chars().count() == CODE_LENGTH
        && code
            .chars()
            .all(|character| CODE_ALPHABET.contains(character));
    if valid {
        Ok(code)
    } else {
        Err(RendezvousError::InvalidCode(raw.to_string()))
    }
}

/// TLS for every request this crate makes.
///
/// The provider is `ring` on purpose: iroh links rustls with `tls-ring`
/// (`livtet-sync-iroh` pins that feature), and rustls refuses to install a
/// second default provider — process-wide, at the first handshake. Building the
/// client config by hand is what keeps this crate on the same provider instead
/// of picking rustls's own default.
fn tls_config() -> Result<rustls::ClientConfig, RendezvousError> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|error| RendezvousError::Config(format!("TLS versions: {error}")))
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth())
}

/// A request that never reached the server.
fn unreachable(error: reqwest::Error) -> RendezvousError {
    RendezvousError::Unreachable(error.to_string())
}

/// Body of [`RendezvousClient::publish`].
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishRequest<'a> {
    node_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    device_type: &'a str,
}

/// Answer to [`RendezvousClient::publish`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishResponse {
    code: String,
    expires_at: i64,
}

/// Answer to [`RendezvousClient::resolve`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveResponse {
    node_id: String,
    #[serde(default)]
    name: Option<String>,
    device_type: String,
}

/// The error body the platform sends, when it sends one.
#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: String,
}

#[cfg(test)]
mod tests {
    use httpmock::prelude::*;

    use super::*;

    #[tokio::test]
    async fn publish_sends_the_node_and_reads_the_code() {
        let server = MockServer::start();
        let publish = server.mock(|when, then| {
            when.method(POST)
                .path("/api/iroh/rendezvous")
                .json_body(serde_json::json!({
                    "nodeId": "node-one",
                    "name": "Reading Desk",
                    "deviceType": "desktop",
                }));
            then.status(201).json_body(serde_json::json!({
                "code": "AB2C34",
                "expiresAt": 1_760_000_000_000_i64,
            }));
        });

        let client = RendezvousClient::new(&server.base_url()).expect("client");
        let published = client
            .publish("node-one", Some("Reading Desk"), "desktop")
            .await
            .expect("publish");

        publish.assert();
        assert_eq!(published.code, "AB2C34");
        assert_eq!(published.expires_at, 1_760_000_000_000);
    }

    #[test]
    fn a_nameless_publish_omits_the_name_field() {
        // `null` would be a name to the platform, which then skips its own
        // fallback for a nameless device; absent is what it means.
        let body = serde_json::to_value(PublishRequest {
            node_id: "node-one",
            name: None,
            device_type: "desktop",
        })
        .expect("serialize");
        assert!(body.get("name").is_none(), "{body}");
    }

    #[tokio::test]
    async fn resolve_reads_the_device_a_code_names() {
        let server = MockServer::start();
        let resolve = server.mock(|when, then| {
            when.method(GET).path("/api/iroh/rendezvous/AB2C34");
            then.status(200).json_body(serde_json::json!({
                "nodeId": "node-two",
                "name": "Phone",
                "deviceType": "mobile",
                "expiresAt": 1_760_000_000_000_i64,
            }));
        });

        // A trailing slash on the base must not become a doubled path.
        let client = RendezvousClient::new(&format!("{}/", server.base_url()))
            .expect("client with trailing slash");
        let peer = client.resolve("ab-2c 34").await.expect("resolve");

        resolve.assert();
        assert_eq!(peer.node_id, "node-two");
        assert_eq!(peer.name.as_deref(), Some("Phone"));
        assert_eq!(peer.device_type, "mobile");
    }

    #[tokio::test]
    async fn a_missing_code_is_expired() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/iroh/rendezvous/AB2C34");
            then.status(404)
                .json_body(serde_json::json!({ "message": "no such code" }));
        });

        let client = RendezvousClient::new(&server.base_url()).expect("client");
        assert_eq!(
            client.resolve("AB2C34").await,
            Err(RendezvousError::Expired)
        );
    }

    #[tokio::test]
    async fn a_refusal_carries_the_servers_wording() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/api/iroh/rendezvous");
            then.status(429)
                .json_body(serde_json::json!({ "message": "too many codes" }));
        });

        let client = RendezvousClient::new(&server.base_url()).expect("client");
        let error = client
            .publish("node-one", None, "desktop")
            .await
            .expect_err("refused");
        assert_eq!(
            error,
            RendezvousError::Server {
                status: 429,
                message: "too many codes".to_string(),
            }
        );
    }

    #[test]
    fn normalization_matches_the_platform() {
        assert_eq!(normalize_code("ab-c 123").expect("normalized"), "ABC123");
        assert_eq!(normalize_code("ab2c34").expect("normalized"), "AB2C34");

        // `O` is not in the alphabet, and a wrong length is not a code at all.
        assert!(normalize_code("O12345").is_err());
        for wrong in ["AB123", "AB12345"] {
            assert!(
                normalize_code(wrong).is_err(),
                "{wrong} is not {CODE_LENGTH} characters"
            );
        }
    }
}
