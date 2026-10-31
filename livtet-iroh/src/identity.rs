//! Peer identity primitive: replaces pairing tokens with an iroh node identity.
//!
//! Previous pairing (`pending_pairings` in `livtet-data`) used a single-use
//! `token` (`daemon.rs:545`) and `listen_on` address. This replaces both with a
//! persistent `NodeId` derived from the peer's public key.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Persistent peer identity (node public key → node id).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PeerIdentity {
    /// Iroh node id (hex representation).
    pub node_id: NodeId,
    /// Human-readable device name.
    pub name: String,
    /// Device type tag (`DeviceType` from `livtet-types`).
    pub device_type: String,
}

/// Node identifier derived from the public key.
///
/// The wire form is the hex string iroh prints for `EndpointId`; this type
/// keeps that string distinct from arbitrary text so pairing code cannot
/// accidentally treat a display name as an identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NodeId(pub String);

impl NodeId {
    /// Borrow the wire form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for NodeId {
    type Err = InvalidNodeId;

    /// Accept the forms a node id can take on the wire: the hex form iroh
    /// prints for an `EndpointId`, and the Crockford-base32 ULID form older
    /// clients send as their device id. Rejecting whitespace and punctuation
    /// keeps `/sync/pair` from storing junk as a pairing key.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidNodeId::Empty);
        }
        if s.len() > 256 {
            return Err(InvalidNodeId::TooLong);
        }
        if !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(InvalidNodeId::InvalidChars);
        }
        Ok(Self(s.to_string()))
    }
}

/// Why a string could not be read as a [`NodeId`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidNodeId {
    #[error("node id is empty")]
    Empty,
    #[error("node id is longer than 256 bytes")]
    TooLong,
    #[error("node id contains characters outside [0-9A-Za-z-]")]
    InvalidChars,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_hex_and_ulid_node_ids() {
        let hex = "aa11bb22cc33dd44ee55ff6677889900";
        assert_eq!(hex.parse::<NodeId>().expect("hex").as_str(), hex);

        let ulid = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        assert_eq!(ulid.parse::<NodeId>().expect("ulid").as_str(), ulid);
    }

    #[test]
    fn rejects_values_that_cannot_name_a_peer() {
        assert_eq!("".parse::<NodeId>(), Err(InvalidNodeId::Empty));
        assert_eq!(
            "has space".parse::<NodeId>(),
            Err(InvalidNodeId::InvalidChars)
        );
        assert_eq!(
            "path/like".parse::<NodeId>(),
            Err(InvalidNodeId::InvalidChars)
        );
        assert_eq!(
            "a".repeat(257).parse::<NodeId>(),
            Err(InvalidNodeId::TooLong)
        );
    }
}
