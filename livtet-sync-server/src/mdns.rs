//! mDNS advertisement and browse of the sync endpoint.
//!
//! The iOS and Android clients browse `_livtet-sync._tcp` (`NWBrowser` and
//! `NsdManager` respectively) and read the peer's node id from the TXT record.
//! The SRV record carries the HTTP port, so a discovered peer is immediately
//! reachable for `POST /sync/pair`. The desktop does the same browse on request
//! (`discovery.nearby`) to populate its nearby-device list.
//!
//! This is deliberately separate from iroh's own mDNS address lookup
//! (`livtet-iroh`): iroh publishes node-id → address records for QUIC
//! connections and resolves a known id, while this record tells the *apps*
//! which peers exist and where the HTTP pairing surface is.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;

use crate::error::ServerError;

/// Service type the mobile clients browse.
pub const SERVICE_TYPE: &str = "_livtet-sync._tcp.local.";

/// TXT keys the clients read. `id` is the pairing key; the rest are display.
pub const TXT_NODE_ID: &str = "id";
pub const TXT_DEVICE_ID: &str = "device_id";
/// Human-readable name the peer chose (or a stable generated one).
pub const TXT_DEVICE_NAME: &str = "name";
pub const TXT_VERSION: &str = "version";
pub const TXT_HOSTNAME: &str = "hostname";
pub const TXT_DEVICE_FLAVOR: &str = "deviceFlavor";

/// Words a generated device name is made of. Short, concrete, and easy to
/// say aloud: the name is read across a room when confirming a pairing.
const NAME_ADJECTIVES: [&str; 16] = [
    "Amber", "Brisk", "Calm", "Dusty", "Eager", "Fuzzy", "Gentle", "Happy", "Ivory", "Jolly",
    "Kind", "Lucky", "Mellow", "Nimble", "Quiet", "Sunny",
];
const NAME_NOUNS: [&str; 16] = [
    "Otter", "Falcon", "Heron", "Lynx", "Marmot", "Osprey", "Panda", "Quail", "Raven", "Seal",
    "Sparrow", "Tiger", "Wren", "Yak", "Zebra", "Finch",
];

/// A stable, friendly name generated from a device id.
///
/// Stable so reconnecting devices keep the name the reader saw, and derived
/// rather than stored so no migration or setting is needed. The daemon
/// advertises it so peers never have to see a ULID.
pub fn friendly_name(device_id: &str) -> String {
    // FNV-1a: tiny, deterministic, and not used for anything security-shaped.
    let hash = device_id
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    let adjective = NAME_ADJECTIVES[(hash % NAME_ADJECTIVES.len() as u64) as usize];
    let noun = NAME_NOUNS[((hash >> 8) % NAME_NOUNS.len() as u64) as usize];
    format!("{adjective} {noun}")
}

/// A registered advertisement, unregistered on drop via [`Advertiser::shutdown`].
pub struct Advertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertiser {
    /// Announce `node_id` on `ip:port`.
    ///
    /// `device_name` is the display name peers read from the TXT record; a
    /// `None` or blank value falls back to [`friendly_name`].
    ///
    /// `ip` is the address the HTTP server actually bound. An unspecified
    /// address (`0.0.0.0`) switches the record to "all interface addresses";
    /// a loopback address is published as-is, which is only reachable from the
    /// same machine (or an iOS simulator sharing its network stack).
    pub fn start(
        node_id: &str,
        device_id: &str,
        device_name: Option<&str>,
        ip: IpAddr,
        port: u16,
    ) -> Result<Self, ServerError> {
        let info = service_info(node_id, device_id, device_name, ip, port)?;
        let daemon = ServiceDaemon::new()
            .map_err(|error| ServerError::Server(format!("mDNS daemon: {error}")))?;
        if let Err(error) = daemon.register(info.clone()) {
            let _ = daemon.shutdown();
            return Err(ServerError::Server(format!("mDNS register: {error}")));
        }
        tracing::info!(
            fullname = info.get_fullname(),
            port,
            "advertising sync service over mDNS"
        );
        Ok(Self {
            daemon,
            fullname: info.get_fullname().to_string(),
        })
    }

    /// Withdraw the record and stop the responder.
    pub fn shutdown(self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// One peer heard advertising `_livtet-sync._tcp` on the local network.
///
/// Everything here comes from the advertisement: the node id is the pairing
/// key, and the addresses + port are what `POST /sync/pair` would be sent to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NearbyPeer {
    /// Node id from the TXT record; the value a pairing key is made of.
    pub node_id: String,
    /// Stable device id from the TXT record.
    pub device_id: String,
    /// Display name (`hostname` TXT field, else the SRV host).
    pub name: String,
    /// `desktop`, `ios`, `android`, … from the `deviceFlavor` TXT field.
    pub device_type: String,
    /// Advertiser's crate version, when present.
    pub version: String,
    /// Fully-qualified SRV host, without the trailing dot.
    pub hostname: String,
    /// Reachable addresses in stable order (IPv4 and IPv6, as advertised).
    pub addresses: Vec<String>,
    /// Port the peer's HTTP pairing surface listens on.
    pub port: u16,
}

/// How long a browse listens before returning what it heard.
///
/// mDNS queries are answered promptly once they land, but the first query can
/// take a moment to leave every interface; a one-second window was observed to
/// miss the advertisement on a busy network, so two seconds gives the query a
/// retransmit's worth of room while keeping the settings poll responsive.
pub const NEARBY_WINDOW: Duration = Duration::from_millis(2_000);

/// Listen on `_livtet-sync._tcp` for [`NEARBY_WINDOW`] and return the peers.
///
/// A fresh `mdns-sd` daemon is created per call: the settings page polls, and
/// a long-lived receiver per poll would accumulate listeners and hold stale
/// records. The daemon is stopped before returning, so a caller only ever sees
/// peers that answered this window.
pub fn browse_nearby() -> Result<Vec<NearbyPeer>, ServerError> {
    browse_for(NEARBY_WINDOW)
}

/// Browse for an explicit window. Exposed so callers that can afford a longer
/// listen (and tests) can trade latency for completeness.
pub fn browse_for(window: Duration) -> Result<Vec<NearbyPeer>, ServerError> {
    let daemon = ServiceDaemon::new()
        .map_err(|error| ServerError::Server(format!("mDNS daemon: {error}")))?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|error| ServerError::Server(format!("mDNS browse: {error}")))?;

    let deadline = Instant::now() + window;
    // Node id → peer: an advertisement is repeated, and the same peer can
    // resolve once per interface.
    let mut heard: HashMap<String, NearbyPeer> = HashMap::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(event) = receiver.recv_timeout(remaining) else {
            break;
        };
        if let ServiceEvent::ServiceResolved(info) = event
            && let Some(peer) = peer_from_resolved(&info)
        {
            heard.insert(peer.node_id.clone(), peer);
        }
    }

    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();

    let mut peers: Vec<NearbyPeer> = heard.into_values().collect();
    peers.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    Ok(peers)
}

/// Decode one resolved advertisement, or `None` when it is not a Livtet peer.
fn peer_from_resolved(info: &ResolvedService) -> Option<NearbyPeer> {
    let node_id = info.get_property_val_str(TXT_NODE_ID)?.to_string();
    if node_id.is_empty() {
        return None;
    }

    let hostname = info.get_hostname().trim_end_matches('.').to_string();
    let name = info
        .get_property_val_str(TXT_DEVICE_NAME)
        .filter(|value| !value.is_empty())
        .or_else(|| info.get_property_val_str(TXT_HOSTNAME))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| hostname.clone());

    let mut addresses: Vec<String> = info
        .get_addresses()
        .iter()
        .map(|address| address.to_ip_addr().to_string())
        .collect();
    addresses.sort();

    Some(NearbyPeer {
        node_id,
        device_id: info
            .get_property_val_str(TXT_DEVICE_ID)
            .unwrap_or_default()
            .to_string(),
        name,
        device_type: info
            .get_property_val_str(TXT_DEVICE_FLAVOR)
            .unwrap_or("unknown")
            .to_string(),
        version: info
            .get_property_val_str(TXT_VERSION)
            .unwrap_or_default()
            .to_string(),
        hostname,
        addresses,
        port: info.get_port(),
    })
}

/// Build the mDNS record. Split out from [`Advertiser::start`] so the TXT
/// contract can be asserted without multicast.
///
/// The TXT `name` is `device_name` when it has any non-blank content, else a
/// name generated from `device_id`, so peers always have something to show.
pub fn service_info(
    node_id: &str,
    device_id: &str,
    device_name: Option<&str>,
    ip: IpAddr,
    port: u16,
) -> Result<ServiceInfo, ServerError> {
    let short = node_id.chars().take(12).collect::<String>();
    let instance = format!("livtet-{short}");
    // `mdns-sd` rejects a host name without the local suffix; the TXT field
    // carries the bare form for display.
    let host = format!("livtet-{short}.local.");
    let display_host = format!("livtet-{short}");
    let name = device_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| friendly_name(device_id));

    let mut props: HashMap<String, String> = HashMap::new();
    props.insert(TXT_NODE_ID.to_string(), node_id.to_string());
    props.insert(TXT_DEVICE_ID.to_string(), device_id.to_string());
    props.insert(TXT_DEVICE_NAME.to_string(), name);
    props.insert(TXT_DEVICE_FLAVOR.to_string(), "desktop".to_string());
    props.insert(
        TXT_VERSION.to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    );
    props.insert(TXT_HOSTNAME.to_string(), display_host);

    let info = if ip.is_unspecified() {
        ServiceInfo::new(SERVICE_TYPE, &instance, &host, (), port, props)
            .map(ServiceInfo::enable_addr_auto)
    } else {
        ServiceInfo::new(SERVICE_TYPE, &instance, &host, ip, port, props)
    }
    .map_err(|error| ServerError::Server(format!("mDNS record: {error}")))?;

    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn txt(info: &ServiceInfo, key: &str) -> Option<String> {
        info.get_properties()
            .get_property_val_str(key)
            .map(str::to_string)
    }

    #[test]
    fn record_carries_the_node_identity_and_http_port() {
        let info = service_info(
            "aa11bb22cc33dd44ee55ff6677889900",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            None,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
            41234,
        )
        .expect("record");

        assert_eq!(info.get_port(), 41234);
        assert_eq!(
            txt(&info, TXT_NODE_ID).as_deref(),
            Some("aa11bb22cc33dd44ee55ff6677889900")
        );
        assert_eq!(
            txt(&info, TXT_DEVICE_ID).as_deref(),
            Some("01ARZ3NDEKTSV4RRFFQ69G5FAV")
        );
        assert_eq!(txt(&info, TXT_DEVICE_FLAVOR).as_deref(), Some("desktop"));
        assert_eq!(
            txt(&info, TXT_DEVICE_NAME).as_deref(),
            Some(friendly_name("01ARZ3NDEKTSV4RRFFQ69G5FAV").as_str())
        );
        assert!(txt(&info, TXT_VERSION).is_some());
        assert_eq!(
            txt(&info, TXT_HOSTNAME).as_deref(),
            Some("livtet-aa11bb22cc33")
        );
        assert!(info.get_fullname().starts_with("livtet-aa11bb22cc33."));
        // `mdns-sd` refuses to register a host name without the local suffix.
        assert!(
            info.get_hostname().ends_with(".local."),
            "host name must be fully qualified: {}",
            info.get_hostname()
        );
    }

    #[test]
    fn loopback_binds_one_address_and_unspecified_uses_auto() {
        let loopback = service_info(
            "ab",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            None,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            3120,
        )
        .expect("record");
        assert_eq!(
            loopback.get_addresses().len(),
            1,
            "a bound loopback address is published as-is"
        );

        let any = service_info(
            "ab",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            None,
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            3120,
        )
        .expect("record");
        assert_eq!(
            any.get_addresses().len(),
            0,
            "0.0.0.0 defers to the daemon's per-interface addresses"
        );
    }

    #[test]
    fn a_resolved_advertisement_becomes_a_nearby_peer() {
        let info = service_info(
            "aa11bb22cc33dd44ee55ff6677889900",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            None,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
            41234,
        )
        .expect("record")
        .as_resolved_service();

        let peer = peer_from_resolved(&info).expect("peer");

        assert_eq!(peer.node_id, "aa11bb22cc33dd44ee55ff6677889900");
        assert_eq!(peer.device_id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(peer.name, friendly_name("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert!(!peer.name.starts_with("livtet-"));
        assert_eq!(peer.device_type, "desktop");
        assert_eq!(peer.port, 41234);
        assert_eq!(peer.addresses, vec!["192.168.1.20".to_string()]);
        // The trailing dot is an mDNS fully-qualified form, not display text.
        assert_eq!(peer.hostname, "livtet-aa11bb22cc33.local");
    }

    #[test]
    fn a_chosen_device_name_is_advertised_and_decoded() {
        let info = service_info(
            "aa11bb22cc33dd44ee55ff6677889900",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            Some("Josh's MacBook"),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
            41234,
        )
        .expect("record");

        assert_eq!(
            txt(&info, TXT_DEVICE_NAME).as_deref(),
            Some("Josh's MacBook")
        );

        let peer = peer_from_resolved(&info.as_resolved_service()).expect("peer");
        assert_eq!(peer.name, "Josh's MacBook");
    }

    #[test]
    fn an_absent_or_blank_device_name_falls_back_to_the_generated_name() {
        let generated = friendly_name("01ARZ3NDEKTSV4RRFFQ69G5FAV");

        let blank = service_info(
            "ab",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            Some("   "),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            3120,
        )
        .expect("record");
        assert_eq!(txt(&blank, TXT_DEVICE_NAME), Some(generated.clone()));

        let absent = service_info(
            "ab",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            None,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            3120,
        )
        .expect("record");
        assert_eq!(txt(&absent, TXT_DEVICE_NAME), Some(generated));
    }

    #[test]
    fn a_resolved_service_without_a_node_id_is_not_a_livtet_peer() {
        let mut props: HashMap<String, String> = HashMap::new();
        props.insert(TXT_DEVICE_ID.to_string(), "device".to_string());
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            "other",
            "other.local.",
            "192.168.1.30",
            8080,
            props,
        )
        .expect("record")
        .as_resolved_service();

        assert!(peer_from_resolved(&info).is_none());
    }

    #[test]
    fn the_generated_name_is_stable_and_human_readable() {
        let name = friendly_name("01ARZ3NDEKTSV4RRFFQ69G5FAV");

        assert_eq!(name, friendly_name("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert_ne!(name, friendly_name("01ARZ3NDEKTSV4RRFFQ69G5FAW"));
        let mut words = name.split(' ');
        assert!(words.next().is_some_and(|word| word.len() > 2));
        assert!(words.next().is_some_and(|word| word.len() > 2));
        assert_eq!(words.next(), None);
    }
}
