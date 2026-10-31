//! Discovery primitive: replaces the manual pairing URI (`livtet://sync`) with
//! an `iroh` endpoint whose node id is the peer's address on the network.
//!
//! Binding an endpoint registers the relay/pkarr/DNS lookup that `iroh`'s `N0`
//! preset configures, so a peer that learns this node id can resolve a current
//! address and connect regardless of NAT or address churn. LAN mDNS is a
//! separate address-lookup crate and is not enabled yet.

use iroh::{Endpoint, endpoint::presets};

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
