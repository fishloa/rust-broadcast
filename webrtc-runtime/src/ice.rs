//! ICE server configuration from WHIP/WHEP Link headers.
//!
//! Parses and serializes `Link` header values carrying STUN/TURN server
//! configuration per RFC 9725 section 4.4.

use alloc::string::String;
use alloc::vec::Vec;

/// A STUN or TURN server discovered via a `Link` header.
///
/// Corresponds to the `rel="ice-server"` link relation defined in RFC 9725 section 4.4:
///
/// ```text
/// Link: <stun:stun.example.com>; rel="ice-server"
/// Link: <turn:turn.example.com?transport=udp>; rel="ice-server";
///       username="user"; credential="pass"
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IceServer {
    /// URI of the ICE server (e.g. `stun:stun.example.com`,
    /// `turn:turn.example.com?transport=udp`,
    /// `turns:turn.example.com?transport=tcp`).
    pub url: String,
    /// Optional username for TURN authentication.
    pub username: Option<String>,
    /// Optional credential for TURN authentication.
    pub credential: Option<String>,
}

/// ICE server Link header relation value (RFC 9725 section 4.4).
const ICE_SERVER_REL: &str = "ice-server";

/// Parse ICE server entries from one or more `Link` header values.
///
/// Each `Link` value may contain multiple comma-separated link entries.
/// Only entries with `rel="ice-server"` are returned; others are silently
/// skipped (per RFC 9725 section 6: "client MUST ignore unknown rel values").
///
/// # Example
///
/// ```
/// use webrtc_runtime::ice::{parse_ice_server_links, IceServer};
///
/// let header = r#"<stun:stun.l.google.com:19302>; rel="ice-server""#;
/// let servers = parse_ice_server_links(header);
/// assert_eq!(servers.len(), 1);
/// assert_eq!(servers[0].url, "stun:stun.l.google.com:19302");
/// assert_eq!(servers[0].username, None);
/// ```
pub fn parse_ice_server_links(link_header: &str) -> Vec<IceServer> {
    let mut servers = Vec::new();

    // Split on commas that are outside angle brackets (separates multiple
    // link-values in a single header). A simplistic approach: track bracket
    // depth.
    for entry in split_link_entries(link_header) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some(server) = parse_single_link(entry) {
            servers.push(server);
        }
    }

    servers
}

/// Serialize a slice of `IceServer`s back to a combined `Link` header value.
///
/// Each server produces one link-value; they are joined by `, `.
///
/// # Example
///
/// ```
/// use webrtc_runtime::ice::{format_ice_server_links, IceServer};
///
/// let servers = vec![
///     IceServer {
///         url: "stun:stun.example.com".into(),
///         username: None,
///         credential: None,
///     },
///     IceServer {
///         url: "turn:turn.example.com?transport=udp".into(),
///         username: Some("user".into()),
///         credential: Some("pass".into()),
///     },
/// ];
/// let header = format_ice_server_links(&servers);
/// assert!(header.contains("stun:stun.example.com"));
/// assert!(header.contains(r#"username="user""#));
/// ```
pub fn format_ice_server_links(servers: &[IceServer]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(servers.len());
    for s in servers {
        let mut link = alloc::format!("<{}>; rel=\"{}\"", s.url, ICE_SERVER_REL);
        if let Some(ref u) = s.username {
            link.push_str(&alloc::format!("; username=\"{u}\""));
        }
        if let Some(ref c) = s.credential {
            link.push_str(&alloc::format!("; credential=\"{c}\""));
        }
        parts.push(link);
    }
    parts.join(", ")
}

/// Split a `Link` header value into individual link-value entries.
///
/// Commas inside `< >` brackets are part of the URI and must not be treated
/// as separators.
fn split_link_entries(header: &str) -> Vec<&str> {
    let mut entries = Vec::new();
    let mut depth = 0u32;
    let mut start = 0;

    for (i, ch) in header.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                entries.push(&header[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    // Push the trailing segment.
    if start < header.len() {
        entries.push(&header[start..]);
    }
    entries
}

/// Parse a single link-value (e.g. `<url>; rel="ice-server"; username="u"`).
fn parse_single_link(entry: &str) -> Option<IceServer> {
    // Extract the URI from < >.
    let uri_start = entry.find('<')? + 1;
    let uri_end = entry[uri_start..].find('>')? + uri_start;
    let url = entry[uri_start..uri_end].trim();

    // Parse the parameters after `>`.
    let params_str = &entry[uri_end + 1..];
    let params = parse_params(params_str);

    // Only keep entries with rel="ice-server".
    let rel = params
        .iter()
        .find(|(k, _)| *k == "rel")
        .map(|(_, v)| v.as_str())?;
    if rel != ICE_SERVER_REL {
        return None;
    }

    let username = params
        .iter()
        .find(|(k, _)| *k == "username")
        .map(|(_, v)| v.clone());
    let credential = params
        .iter()
        .find(|(k, _)| *k == "credential")
        .map(|(_, v)| v.clone());

    Some(IceServer {
        url: url.into(),
        username,
        credential,
    })
}

/// Parse semicolon-delimited `key="value"` or `key=value` parameters.
fn parse_params(s: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for part in s.split(';') {
        let part = part.trim();
        if let Some(eq) = part.find('=') {
            let key = part[..eq].trim().to_ascii_lowercase();
            let val = part[eq + 1..].trim().trim_matches('"');
            result.push((key, val.into()));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stun_only() {
        let header = r#"<stun:stun.l.google.com:19302>; rel="ice-server""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].url, "stun:stun.l.google.com:19302");
        assert_eq!(servers[0].username, None);
        assert_eq!(servers[0].credential, None);
    }

    #[test]
    fn parse_turn_with_credentials() {
        let header = r#"<turn:turn.example.com?transport=udp>; rel="ice-server"; username="user"; credential="pass""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].url, "turn:turn.example.com?transport=udp");
        assert_eq!(servers[0].username.as_deref(), Some("user"));
        assert_eq!(servers[0].credential.as_deref(), Some("pass"));
    }

    #[test]
    fn parse_turns_tcp() {
        let header = r#"<turns:turn.example.com?transport=tcp>; rel="ice-server"; username="u"; credential="c""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].url, "turns:turn.example.com?transport=tcp");
    }

    #[test]
    fn parse_multiple_servers_comma_separated() {
        let header = r#"<stun:s1.example.com>; rel="ice-server", <turn:t1.example.com>; rel="ice-server"; username="u"; credential="c""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].url, "stun:s1.example.com");
        assert_eq!(servers[1].url, "turn:t1.example.com");
        assert_eq!(servers[1].username.as_deref(), Some("u"));
    }

    #[test]
    fn skip_non_ice_server_rel() {
        let header = r#"<https://example.com/ext>; rel="urn:ietf:params:whip:ext:core:layer", <stun:s.example.com>; rel="ice-server""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].url, "stun:s.example.com");
    }

    #[test]
    fn empty_header() {
        let servers = parse_ice_server_links("");
        assert!(servers.is_empty());
    }

    #[test]
    fn format_round_trip() {
        let servers = vec![
            IceServer {
                url: "stun:stun.example.com".into(),
                username: None,
                credential: None,
            },
            IceServer {
                url: "turn:turn.example.com?transport=udp".into(),
                username: Some("user".into()),
                credential: Some("pass".into()),
            },
        ];

        let header = format_ice_server_links(&servers);
        let parsed = parse_ice_server_links(&header);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], servers[0]);
        assert_eq!(parsed[1], servers[1]);
    }

    #[test]
    fn format_empty() {
        assert_eq!(format_ice_server_links(&[]), "");
    }

    #[test]
    fn format_stun_only() {
        let servers = vec![IceServer {
            url: "stun:s.example.com".into(),
            username: None,
            credential: None,
        }];
        let header = format_ice_server_links(&servers);
        assert_eq!(header, r#"<stun:s.example.com>; rel="ice-server""#);
    }

    #[test]
    fn format_turn_with_creds() {
        let servers = vec![IceServer {
            url: "turn:t.example.com".into(),
            username: Some("u".into()),
            credential: Some("c".into()),
        }];
        let header = format_ice_server_links(&servers);
        assert_eq!(
            header,
            r#"<turn:t.example.com>; rel="ice-server"; username="u"; credential="c""#
        );
    }
}
