//! Discovery primitive: replaces the manual pairing URI (`livtet://sync`) with
//! an `iroh` endpoint whose node id is the peer's address on the network.
//!
//! Binding an endpoint registers the relay/pkarr/DNS lookup that `iroh`'s `N0`
//! preset configures, plus an mDNS address lookup, so a peer that learns this
//! node id resolves a current address on the LAN or, failing that, through the
//! relay without any manual address entry.

use iroh::{Endpoint, SecretKey, endpoint::presets};
use iroh_mdns_address_lookup::MdnsAddressLookup;

use crate::identity::{NodeId, PeerIdentity};

/// The ALPN every sync stream runs under.
///
/// Set on the endpoint at bind time: iroh refuses incoming connections whose
/// ALPN the endpoint does not advertise, so without it this node can dial but
/// never be dialled — which is exactly the direction a pull-free sync needs.
pub const SYNC_ALPN: &[u8] = b"livtet/sync/1";

/// A live `iroh` endpoint plus the identity it advertises.
pub struct DiscoveryService {
    endpoint: Endpoint,
}

impl DiscoveryService {
    /// Bind an endpoint and begin publishing its address.
    ///
    /// Returns once the local socket is bound; relay registration and DNS
    /// publishing continue in the background, so a slow or absent network
    /// delays peer *resolution*, never startup.
    pub async fn bind() -> Result<Self, DiscoveryError> {
        Self::bind_with_secret(SecretKey::generate()).await
    }

    /// Bind with a caller-owned secret, so the node id survives restarts.
    pub async fn bind_with_secret(secret_key: SecretKey) -> Result<Self, DiscoveryError> {
        Self::bind_with_secret_and_port(secret_key, None).await
    }

    /// Bind with a caller-owned secret on a chosen UDP port.
    ///
    /// `None` leaves iroh's default sockets alone (an ephemeral port). `Some`
    /// binds the unspecified IPv4 address on that port, so the daemon's
    /// `--port 0` still asks the OS for a free port and a fixed one is
    /// reachable where the operator expects it.
    pub async fn bind_with_secret_and_port(
        secret_key: SecretKey,
        port: Option<u16>,
    ) -> Result<Self, DiscoveryError> {
        let mut builder = Endpoint::builder(presets::N0)
            .secret_key(secret_key)
            .alpns(vec![SYNC_ALPN.to_vec()]);
        if let Some(port) = port {
            builder = builder
                .bind_addr(std::net::SocketAddr::from(([0, 0, 0, 0], port)))
                .map_err(|error| DiscoveryError::Bind(error.to_string()))?;
        }
        let endpoint = builder
            .bind()
            .await
            .map_err(|error| DiscoveryError::Bind(error.to_string()))?;

        // LAN resolution: iroh publishes each endpoint's node id and direct
        // addresses over mDNS, so a node id learned out of band (QR, manual
        // entry, the `_livtet-sync._tcp` TXT record) resolves without a relay.
        // A failure here degrades discovery to the N0 relay/pkarr path only.
        match MdnsAddressLookup::builder().build(endpoint.id()) {
            Ok(mdns) => match endpoint.address_lookup() {
                Ok(services) => services.add(mdns),
                Err(error) => tracing::warn!(%error, "iroh address lookup unavailable"),
            },
            Err(error) => tracing::warn!(%error, "iroh mDNS lookup unavailable"),
        }

        Ok(Self { endpoint })
    }

    /// This device's node id, as peers must know it.
    pub fn node_id(&self) -> NodeId {
        NodeId(self.endpoint.id().to_string())
    }

    /// The identity advertised for pairing.
    pub fn identity(
        &self,
        name: impl Into<String>,
        device_type: impl Into<String>,
    ) -> PeerIdentity {
        PeerIdentity {
            node_id: self.node_id(),
            name: name.into(),
            device_type: device_type.into(),
        }
    }

    /// The bound endpoint, for code that wants to dial or accept connections.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Every socket the endpoint bound.
    pub fn bound_sockets(&self) -> Vec<std::net::SocketAddr> {
        self.endpoint.bound_sockets()
    }

    /// The UDP port the endpoint is listening on, preferring IPv4.
    ///
    /// `None` when iroh bound no socket, which lets callers fall back to the
    /// configured port.
    pub fn bound_port(&self) -> Option<u16> {
        let sockets = self.bound_sockets();
        sockets
            .iter()
            .find(|addr| addr.is_ipv4())
            .or_else(|| sockets.first())
            .map(std::net::SocketAddr::port)
    }

    /// Stop advertising and release the socket.
    pub async fn shutdown(&self) {
        self.endpoint.close().await;
    }
}

/// Why discovery could not start.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("iroh endpoint bind failed: {0}")]
    Bind(String),
}
