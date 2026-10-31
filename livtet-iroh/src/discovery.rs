//! Discovery primitive: replaces the manual pairing URI (`livtet://sync`) with
//! an `iroh` endpoint whose node id is the peer's address on the network.
//!
//! Binding an endpoint registers the relay/pkarr/DNS lookup that `iroh`'s `N0`
//! preset configures, plus an mDNS address lookup, so a peer that learns this
//! node id resolves a current address on the LAN or, failing that, through the
//! relay without any manual address entry.

use iroh::{Endpoint, endpoint::presets};
use iroh_mdns_address_lookup::MdnsAddressLookup;

use crate::identity::{NodeId, PeerIdentity};

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
        let endpoint = Endpoint::bind(presets::N0)
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
