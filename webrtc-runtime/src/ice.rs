//! ICE server configuration from WHIP/WHEP Link headers.
//!
//! Parses and serializes `Link` header values carrying STUN/TURN server
//! configuration per RFC 9725 section 4.4.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Error;

/// A STUN or TURN server discovered via a `Link` header.
///
/// Corresponds to the `rel="ice-server"` link relation defined in RFC 9725 section 4.4:
///
/// ```text
/// Link: <stun:stun.example.com>; rel="ice-server"
/// Link: <turn:turn.example.com?transport=udp>; rel="ice-server";
///       username="user"; credential="pass"
/// ```
#[derive(Clone, PartialEq, Eq)]
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

/// Manual `Debug`: the TURN `credential` never renders (issue #1142).
impl core::fmt::Debug for IceServer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IceServer")
            .field("url", &self.url)
            .field("username", &self.username)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
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
/// # Errors
///
/// [`Error::InvalidHeader`] (`"Link"`) when a `url` would end the `<...>`
/// value or start a new link (a control character, `<`, `>` or `"`), or when
/// a `username`/`credential` holds a control character, which cannot appear in
/// an RFC 8288 quoted-string.
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
/// let header = format_ice_server_links(&servers).expect("safe link values");
/// assert!(header.contains("stun:stun.example.com"));
/// assert!(header.contains(r#"username="user""#));
/// ```
pub fn format_ice_server_links(servers: &[IceServer]) -> Result<String, Error> {
    let mut parts: Vec<String> = Vec::with_capacity(servers.len());
    for s in servers {
        checked_uri(&s.url)?;
        let mut link = alloc::format!("<{}>; rel=\"{}\"", s.url, ICE_SERVER_REL);
        if let Some(ref u) = s.username {
            checked_quoted(u)?;
            link.push_str("; username=");
            link.push_str(&quote_escape(u));
        }
        if let Some(ref c) = s.credential {
            checked_quoted(c)?;
            link.push_str("; credential=");
            link.push_str(&quote_escape(c));
        }
        parts.push(link);
    }
    Ok(parts.join(", "))
}

/// A `Link` URI-reference: no control character, and none of `<`, `>` or `"`
/// that would end the `<...>` value or forge the next link.
fn checked_uri(url: &str) -> Result<(), Error> {
    if url.is_empty()
        || url
            .chars()
            .any(|c| c.is_ascii_control() || matches!(c, '<' | '>' | '"'))
    {
        return Err(Error::InvalidHeader { header: "Link" });
    }
    Ok(())
}

/// A quoted `Link` parameter value: a control character is not a
/// `quoted-string` character (RFC 8288 §3); `"` and `\` are escaped by
/// [`quote_escape`].
fn checked_quoted(value: &str) -> Result<(), Error> {
    if value.chars().any(|c| c.is_ascii_control()) {
        return Err(Error::InvalidHeader { header: "Link" });
    }
    Ok(())
}

/// Quotes `value` as an RFC 8288 `quoted-string`, backslash-escaping any `"`
/// or `\` it contains (audit run-09 W22) — a static TURN operator password
/// can legally contain either, unlike a TURN REST credential (a base64
/// HMAC), so `format -> parse` must round-trip it rather than silently
/// truncating or corrupting the value.
fn quote_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Removes the surrounding `" "` (if present) and un-escapes `\X` -> `X`,
/// the inverse of [`quote_escape`].
fn unquote(value: &str) -> String {
    let inner = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(escaped) = chars.next()
        {
            out.push(escaped);
        } else {
            out.push(c);
        }
    }
    out
}

/// Split `s` on `sep`, ignoring any `sep` that falls inside an RFC 8288
/// `quoted-string` (`"..."`, with `\"` an escaped quote that does not end
/// it) — used for both the entry-separating comma and the
/// parameter-separating semicolon (audit run-09 W22). `angle_brackets`
/// additionally protects a comma inside the URI's `< >` delimiters, which
/// only the entry-level split needs.
fn split_respecting_quotes(s: &str, sep: char, angle_brackets: bool) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_quotes = false;
    let mut angle_depth = 0u32;
    let mut start = 0;
    let mut chars = s.char_indices();

    while let Some((i, ch)) = chars.next() {
        match ch {
            '\\' if in_quotes => {
                // Skip the escaped character so a `\"` inside the string
                // doesn't end it early.
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            '<' if angle_brackets && !in_quotes => angle_depth += 1,
            '>' if angle_brackets && !in_quotes => angle_depth = angle_depth.saturating_sub(1),
            c if c == sep && !in_quotes && angle_depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start <= s.len() {
        parts.push(&s[start..]);
    }
    parts
}

/// Split a `Link` header value into individual link-value entries.
///
/// A comma inside `< >` brackets (part of the URI) or inside a quoted
/// parameter value is not a separator.
fn split_link_entries(header: &str) -> Vec<&str> {
    split_respecting_quotes(header, ',', true)
}

/// Parse a single link-value (e.g. `<url>; rel="ice-server"; username="u"`).
fn parse_single_link(entry: &str) -> Option<IceServer> {
    let uri_start = entry.find('<')? + 1;
    let uri_end = entry[uri_start..].find('>')? + uri_start;
    let url = entry[uri_start..uri_end].trim();

    let params_str = &entry[uri_end + 1..];
    let params = parse_params(params_str);

    // Only keep entries with `rel` naming "ice-server" (RFC 8288: `rel` is
    // case-insensitive and MAY be a space-separated list of relation types,
    // audit run-09 W22 — a strict `rel != "ice-server"` missed both).
    let rel = params
        .iter()
        .find(|(k, _)| *k == "rel")
        .map(|(_, v)| v.as_str())?;
    if !rel
        .split_whitespace()
        .any(|token| token.eq_ignore_ascii_case(ICE_SERVER_REL))
    {
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
///
/// A `;` inside a quoted value (e.g. a TURN operator password containing a
/// literal `;`) is not a separator (audit run-09 W22).
fn parse_params(s: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for part in split_respecting_quotes(s, ';', false) {
        let part = part.trim();
        if let Some(eq) = part.find('=') {
            let key = part[..eq].trim().to_ascii_lowercase();
            let val = unquote(part[eq + 1..].trim());
            result.push((key, val));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

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

        let header = format_ice_server_links(&servers).expect("safe link values");
        let parsed = parse_ice_server_links(&header);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], servers[0]);
        assert_eq!(parsed[1], servers[1]);
    }

    #[test]
    fn format_empty() {
        assert_eq!(format_ice_server_links(&[]).expect("no servers"), "");
    }

    #[test]
    fn format_stun_only() {
        let servers = vec![IceServer {
            url: "stun:s.example.com".into(),
            username: None,
            credential: None,
        }];
        let header = format_ice_server_links(&servers).expect("safe link values");
        assert_eq!(header, r#"<stun:s.example.com>; rel="ice-server""#);
    }

    // Regression (audit run-09 W22): a credential containing characters
    // that are legal inside an RFC 8288 quoted-string (`;`, `,`, `"`) must
    // round-trip through format -> parse byte-for-byte, not be split or
    // truncated. A static operator password (unlike a base64 TURN REST
    // credential) can contain any of these.
    #[test]
    fn credential_with_semicolon_comma_and_quote_round_trips() {
        let servers = vec![IceServer {
            url: "turn:turn.example.com".into(),
            username: Some("user".into()),
            credential: Some(r#"pa;ss,word"with\backslash"#.into()),
        }];
        let header = format_ice_server_links(&servers).expect("safe link values");
        let parsed = parse_ice_server_links(&header);
        assert_eq!(
            parsed.len(),
            1,
            "the embedded `,`/`;` must not split entries: {header}"
        );
        assert_eq!(parsed[0], servers[0]);
    }

    // Regression (audit run-09 W22): `rel` is case-insensitive (RFC 8288)
    // and may be a space-separated list of relation types.
    #[test]
    fn rel_matching_is_case_insensitive_and_accepts_a_token_list() {
        let header = r#"<stun:s.example.com>; rel="ICE-SERVER", <stun:s2.example.com>; rel="other ice-server""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 2, "{header}");
        assert_eq!(servers[0].url, "stun:s.example.com");
        assert_eq!(servers[1].url, "stun:s2.example.com");
    }

    #[test]
    fn format_turn_with_creds() {
        let servers = vec![IceServer {
            url: "turn:t.example.com".into(),
            username: Some("u".into()),
            credential: Some("c".into()),
        }];
        let header = format_ice_server_links(&servers).expect("safe link values");
        assert_eq!(
            header,
            r#"<turn:t.example.com>; rel="ice-server"; username="u"; credential="c""#
        );
    }

    /// #2859: a `url` was written inside `<...>` raw, so `>` or a CR/LF in a
    /// configured server forged the rest of the `Link` header.
    #[test]
    fn formatting_rejects_link_breaking_values() {
        for url in [
            "stun:s.example.com>\r\nX-Injected: 1",
            "stun:<s>.example.com",
            "stun:s.example.com\"",
        ] {
            let servers = vec![IceServer {
                url: url.into(),
                username: None,
                credential: None,
            }];
            let err =
                format_ice_server_links(&servers).expect_err("a link-breaking URL must be refused");
            assert!(
                matches!(err, Error::InvalidHeader { header: "Link" }),
                "url {url:?}: {err:?}"
            );
        }
        let servers = vec![IceServer {
            url: "turn:t.example.com".into(),
            username: Some("user\r\nX-Injected: 1".into()),
            credential: None,
        }];
        let err = format_ice_server_links(&servers)
            .expect_err("a control character in a credential must be refused");
        assert!(matches!(err, Error::InvalidHeader { header: "Link" }));
    }

    // -----------------------------------------------------------------------
    // RFC 8288 `Link` splitting (audit-22 #3006). The header a TURN/STUN
    // discovery response carries is one comma-separated list, so a comma
    // inside the URI's `< >` delimiters or inside a `quoted-string` (where
    // `\"` is an escaped quote) is part of the value, never a separator.
    // -----------------------------------------------------------------------

    /// A comma in the URI and an escaped quote in a credential.
    ///
    /// Bite tests: dropping the `< >` tracking splits the URI (`?x=1,2`);
    /// treating `\"` as a closing quote ends the credential early, after
    /// which the entry comma is swallowed.
    #[test]
    fn uri_commas_and_escaped_quotes_do_not_split_entries() {
        let header = r#"<turn:a.example?x=1,2>; rel="ice-server"; username="a,b\"c", <stun:b.example>; rel="ice-server""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 2, "{header}: {servers:?}");
        assert_eq!(servers[0].url, "turn:a.example?x=1,2");
        assert_eq!(servers[0].username.as_deref(), Some("a,b\"c"));
        assert_eq!(servers[1].url, "stun:b.example");
    }

    /// A `<` inside a quoted parameter value is a character, not an opening
    /// angle bracket: counting it would leave the depth raised and swallow
    /// the next entry's comma.
    #[test]
    fn a_lt_inside_a_quoted_parameter_does_not_open_a_bracket() {
        let header = r#"<stun:s.example>; rel="ice-server"; username="a<b", <stun:s2.example>; rel="ice-server""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 2, "{header}: {servers:?}");
        assert_eq!(servers[0].username.as_deref(), Some("a<b"));
        assert_eq!(servers[1].url, "stun:s2.example");
    }

    /// A `>` inside a quoted parameter value never closes an angle bracket.
    /// With an unmatched `<` still open, a quoted `>` must not re-open the
    /// comma as a separator.
    #[test]
    fn a_gt_inside_a_quoted_parameter_does_not_close_a_bracket() {
        let header = r#"<turn:a.example?x=<1>; rel="ice-server"; username="q>z", <stun:b.example>; rel="ice-server""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(
            servers.len(),
            1,
            "an unclosed `<` must keep the entry unsplit: {servers:?}"
        );
        assert_eq!(servers[0].url, "turn:a.example?x=<1");
    }

    /// An escaped quote sits outside a quoted-string only as an ordinary
    /// character (RFC 9110 §5.6.4's `quoted-pair` applies inside quotes
    /// alone), so a `\` cannot hide the entry-separating comma after it.
    #[test]
    fn a_backslash_outside_quotes_does_not_escape_the_separator() {
        let header =
            r#"<stun:s.example>; rel="ice-server"; note=a\, <stun:s2.example>; rel="ice-server""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 2, "{header}: {servers:?}");
        assert_eq!(servers[0].url, "stun:s.example");
        assert_eq!(servers[1].url, "stun:s2.example");
    }

    /// An unbalanced `>` before any `<` must not make the parser panic or
    /// invent a server: only the well-formed entry remains.
    #[test]
    fn an_unbalanced_gt_yields_no_phantom_server() {
        let header = r#"> junk, <stun:s.example>; rel="ice-server""#;
        let servers = parse_ice_server_links(header);
        assert_eq!(servers.len(), 1, "{servers:?}");
        assert_eq!(servers[0].url, "stun:s.example");
    }
}
