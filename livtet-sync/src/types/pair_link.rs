//! The pairing link a QR code carries.
//!
//! A reader pairs a second device by scanning a code that names one iroh node
//! id. The string in that code is the contract between the device showing it
//! and the device reading it, so it lives here rather than being formatted by
//! hand in each client: the desktop renders it, the phone parses it, and a
//! printed or browser-delivered code has to be accepted too.
//!
//! The canonical form is
//!
//! ```text
//! livtet://sync?v=2&nodeId=<node id>[&name=<advertised name>]
//! ```
//!
//! Two older shapes still parse, because codes already printed or bookmarked
//! cannot be reissued: `livtet://pair?node=<id>` and the web form
//! `https://livtet.olamaelcu.net/pair?node=<id>`. They all describe the same
//! thing, and [`PairLink::to_uri`] always writes the canonical one.

use std::fmt;

use thiserror::Error;

/// Scheme the app registers for its own links.
pub const SCHEME: &str = "livtet";

/// Host segment of the canonical link.
pub const HOST: &str = "sync";

/// Host segment the pre-`sync` links used.
pub const LEGACY_HOST: &str = "pair";

/// The site that serves browser-delivered pairing codes.
pub const WEB_HOST: &str = "livtet.olamaelcu.net";

/// Protocol version this crate speaks, as written in `v=`.
pub const PROTOCOL_VERSION: u16 = 2;

/// Query key carrying the node id in the canonical and web forms.
const KEY_NODE_ID: &str = "nodeId";

/// Query key carrying the node id in the legacy forms.
const KEY_NODE: &str = "node";

/// Query key carrying the protocol version.
const KEY_VERSION: &str = "v";

/// Query key carrying the advertised device name.
const KEY_NAME: &str = "name";

/// Why a string was not a pairing link.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PairLinkError {
    /// The string does not start with a scheme and `://`.
    #[error("not a pairing link: {0}")]
    NotALink(String),
    /// The scheme is neither `livtet` nor `https`.
    #[error("unexpected scheme: {0}")]
    UnsupportedScheme(String),
    /// The host segment is not one this format defines.
    #[error("unexpected host: {0}")]
    UnsupportedHost(String),
    /// The link carries no node id.
    #[error("the link names no node id")]
    MissingNodeId,
    /// The node id is empty or holds characters a node id cannot.
    #[error("invalid node id: {0}")]
    InvalidNodeId(String),
    /// The link was written by a protocol version this build does not speak.
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u16),
    /// `v=` was present but not a number.
    #[error("malformed protocol version: {0}")]
    MalformedVersion(String),
}

/// A parsed pairing link: which node to dial, what to call it, and under which
/// protocol version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairLink {
    /// Protocol version written in the link. Always [`PROTOCOL_VERSION`] when
    /// built by [`PairLink::new`].
    pub version: u16,
    /// The peer's iroh node id. Opaque here: the transport validates it as an
    /// endpoint id when it dials.
    pub node_id: String,
    /// The name the peer advertises, when the code carried one.
    pub name: Option<String>,
}

impl PairLink {
    /// A canonical link for `node_id`, naming the device when `name` is set.
    pub fn new(node_id: impl Into<String>, name: Option<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            node_id: node_id.into(),
            name: name.filter(|name| !name.is_empty()),
        }
    }

    /// The canonical link, the one every client should render.
    pub fn to_uri(&self) -> String {
        let mut uri = format!("{SCHEME}://{HOST}?{KEY_VERSION}={}", self.version);
        uri.push('&');
        uri.push_str(KEY_NODE_ID);
        uri.push('=');
        uri.push_str(&encode(&self.node_id));
        if let Some(name) = &self.name {
            uri.push('&');
            uri.push_str(KEY_NAME);
            uri.push('=');
            uri.push_str(&encode(name));
        }
        uri
    }

    /// The web form of the same link, for codes rendered in a browser.
    pub fn to_web_uri(&self) -> String {
        format!(
            "https://{WEB_HOST}/{HOST}?{KEY_VERSION}={}&{KEY_NODE_ID}={}",
            self.version,
            encode(&self.node_id)
        )
    }

    /// Parse any pairing link this crate has emitted, current or historical.
    ///
    /// Accepts `livtet://sync?v=2&nodeId=…`, `livtet://pair?node=…`, and
    /// `https://livtet.olamaelcu.net/{sync,pair}?…`.
    pub fn parse(raw: &str) -> Result<Self, PairLinkError> {
        let raw = raw.trim();
        let (scheme, rest) = raw
            .split_once("://")
            .ok_or_else(|| PairLinkError::NotALink(raw.to_string()))?;

        let scheme = scheme.to_ascii_lowercase();
        let is_web = match scheme.as_str() {
            SCHEME => false,
            "https" | "http" => true,
            other => return Err(PairLinkError::UnsupportedScheme(other.to_string())),
        };

        let (authority, query) = match rest.split_once('?') {
            Some((authority, query)) => (authority, query),
            None => (rest, ""),
        };

        // Strip an empty path or a single trailing slash so `livtet://sync/`
        // and the absolute URL forms parse the same way.
        let authority = authority.trim_end_matches('/');
        let (host, path) = match is_web {
            true => match authority.split_once('/') {
                Some((host, path)) => (host, path.trim_end_matches('/')),
                None => (authority, ""),
            },
            false => (authority, ""),
        };
        let host = host.to_ascii_lowercase();

        if is_web && !host.eq_ignore_ascii_case(WEB_HOST) {
            return Err(PairLinkError::UnsupportedHost(host));
        }
        if !is_web && !path.is_empty() {
            return Err(PairLinkError::UnsupportedHost(format!("{host}/{path}")));
        }

        // `livtet://sync` puts the host name in the authority; the web form
        // puts it in the first path segment.
        let kind = match (is_web, path.is_empty()) {
            (false, _) => host.clone(),
            (true, false) => path.to_ascii_lowercase(),
            (true, true) => return Err(PairLinkError::UnsupportedHost(host)),
        };
        if kind != HOST && kind != LEGACY_HOST {
            return Err(PairLinkError::UnsupportedHost(kind));
        }

        let mut node_id = None;
        let mut version = None;
        let mut name = None;
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = match pair.split_once('=') {
                Some((key, value)) => (key, value),
                None => (pair, ""),
            };
            let value = decode(value);
            match key {
                k if k.eq_ignore_ascii_case(KEY_NODE_ID) || k.eq_ignore_ascii_case(KEY_NODE) => {
                    node_id.get_or_insert(value);
                }
                k if k.eq_ignore_ascii_case(KEY_VERSION) => {
                    let parsed = value
                        .parse::<u16>()
                        .map_err(|_| PairLinkError::MalformedVersion(value.clone()))?;
                    version = Some(parsed);
                }
                k if k.eq_ignore_ascii_case(KEY_NAME) => {
                    if !value.is_empty() {
                        name = Some(value);
                    }
                }
                // Unknown parameters are ignored so a newer producer can add
                // one without breaking an older reader.
                _ => {}
            }
        }

        let node_id = node_id.ok_or(PairLinkError::MissingNodeId)?;
        if node_id.is_empty() || node_id.len() > 128 || !node_id.chars().all(is_node_id_char) {
            return Err(PairLinkError::InvalidNodeId(node_id));
        }

        // A legacy `pair?node=` link carries no version and predates `v=`, so a
        // missing version means the version this crate speaks.
        let version = version.unwrap_or(PROTOCOL_VERSION);
        if version != PROTOCOL_VERSION {
            return Err(PairLinkError::UnsupportedVersion(version));
        }

        Ok(Self {
            version,
            node_id,
            name,
        })
    }
}

impl fmt::Display for PairLink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_uri())
    }
}

/// Characters a node id is allowed to hold: base32 (iroh's textual form) plus
/// the separators a future encoding might use. Anything else means the code was
/// mangled in transit.
fn is_node_id_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
}

/// Percent-encode everything outside RFC 3986's unreserved set.
fn encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push('%');
                encoded.push_str(&format!("{byte:02X}"));
            }
        }
    }
    encoded
}

/// Decode `%XX` escapes; malformed escapes are kept verbatim so a stray `%` in
/// a device name cannot fail the parse.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: &str = "6e1a8a52e4f1e2b4c3d5a6f7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7";

    #[test]
    fn canonical_link_round_trips() {
        let link = PairLink::new(NODE, Some("Studio".to_string()));
        assert_eq!(
            link.to_uri(),
            format!("livtet://sync?v=2&nodeId={NODE}&name=Studio")
        );
        assert_eq!(PairLink::parse(&link.to_uri()).unwrap(), link);
    }

    #[test]
    fn a_link_without_a_name_has_no_name_parameter() {
        let link = PairLink::new(NODE, None);
        assert_eq!(link.to_uri(), format!("livtet://sync?v=2&nodeId={NODE}"));
        assert_eq!(PairLink::parse(&link.to_uri()).unwrap().name, None);
    }

    #[test]
    fn a_name_is_encoded_and_decoded() {
        let name = "Jacky's desk & phone/2 + café";
        let link = PairLink::new(NODE, Some(name.to_string()));
        assert!(!link.to_uri().contains(' '), "spaces must be escaped");
        assert_eq!(PairLink::parse(&link.to_uri()).unwrap().name.as_deref(), Some(name));
    }

    #[test]
    fn an_empty_name_is_treated_as_absent() {
        let link = PairLink::new(NODE, Some(String::new()));
        assert_eq!(link.name, None);
        assert_eq!(link.to_uri(), format!("livtet://sync?v=2&nodeId={NODE}"));
    }

    #[test]
    fn the_legacy_phone_form_parses() {
        let link = PairLink::parse(&format!("livtet://pair?node={NODE}")).unwrap();
        assert_eq!(link.node_id, NODE);
        assert_eq!(link.version, PROTOCOL_VERSION, "no v= means this version");
        assert_eq!(link.name, None);
    }

    #[test]
    fn the_web_form_parses_for_both_hosts() {
        for raw in [
            format!("https://livtet.olamaelcu.net/pair?node={NODE}"),
            format!("https://livtet.olamaelcu.net/sync?v=2&nodeId={NODE}"),
            format!("https://livtet.olamaelcu.net/pair/?node={NODE}"),
        ] {
            let link = PairLink::parse(&raw).unwrap();
            assert_eq!(link.node_id, NODE, "{raw}");
        }
    }

    #[test]
    fn the_web_form_is_emitted_with_https() {
        let web = PairLink::new(NODE, None).to_web_uri();
        assert_eq!(
            web,
            format!("https://livtet.olamaelcu.net/sync?v=2&nodeId={NODE}")
        );
        assert_eq!(PairLink::parse(&web).unwrap().node_id, NODE);
    }

    #[test]
    fn surrounding_whitespace_and_case_do_not_matter() {
        let raw = format!("  LIVTET://SYNC?v=2&nodeid={NODE}  ");
        assert_eq!(PairLink::parse(&raw).unwrap().node_id, NODE);
    }

    #[test]
    fn a_link_naming_no_node_is_rejected() {
        assert_eq!(
            PairLink::parse("livtet://sync?v=2"),
            Err(PairLinkError::MissingNodeId)
        );
        assert_eq!(
            PairLink::parse("livtet://sync?v=2&nodeId="),
            Err(PairLinkError::InvalidNodeId(String::new()))
        );
    }

    #[test]
    fn a_node_id_with_foreign_characters_is_rejected() {
        assert_eq!(
            PairLink::parse("livtet://sync?nodeId=not%20a%20node"),
            Err(PairLinkError::InvalidNodeId("not a node".to_string()))
        );
    }

    #[test]
    fn another_protocol_version_is_refused_rather_than_guessed() {
        assert_eq!(
            PairLink::parse(&format!("livtet://sync?v=3&nodeId={NODE}")),
            Err(PairLinkError::UnsupportedVersion(3))
        );
        assert_eq!(
            PairLink::parse(&format!("livtet://sync?v=two&nodeId={NODE}")),
            Err(PairLinkError::MalformedVersion("two".to_string()))
        );
    }

    #[test]
    fn foreign_schemes_and_hosts_are_refused() {
        assert!(matches!(
            PairLink::parse(&format!("https://evil.example/sync?nodeId={NODE}")),
            Err(PairLinkError::UnsupportedHost(_))
        ));
        assert!(matches!(
            PairLink::parse("https://livtet.olamaelcu.net/book/abc"),
            Err(PairLinkError::UnsupportedHost(_))
        ));
        assert!(matches!(
            PairLink::parse(&format!("ftp://sync?nodeId={NODE}")),
            Err(PairLinkError::UnsupportedScheme(_))
        ));
        assert!(matches!(
            PairLink::parse("just some text"),
            Err(PairLinkError::NotALink(_))
        ));
        assert!(matches!(
            PairLink::parse(&format!("livtet://settings?nodeId={NODE}")),
            Err(PairLinkError::UnsupportedHost(_))
        ));
    }

    #[test]
    fn unknown_parameters_are_ignored() {
        let raw = format!("livtet://sync?v=2&nodeId={NODE}&future=1");
        assert_eq!(PairLink::parse(&raw).unwrap().node_id, NODE);
    }
}
