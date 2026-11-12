//! The `srt://` URL a caller dials (draft-sharabayko-srt-01 §3.2.1.3 Stream
//! ID; the scheme itself is SRT's registered URL form).
//!
//! An SRT endpoint is a `host:port` plus the query overrides the Stream ID
//! handshake extension carries: `streamid` (opaque, passed through to the
//! handshake), `latency`, `mode` and `passphrase`. This module parses that
//! form into a typed [`SrtUrl`]; *which* values an endpoint accepts is policy
//! and belongs to the caller (the `multimux` push, for one, dials only in
//! Caller mode and rejects a passphrase).
//!
//! `std` only: the authority and query are parsed with the `url` crate and
//! `streamid` values are percent-decoded with `percent_encoding`.
//!
//! # Accepted shapes
//!
//! Both `srt://host:port?…` and a scheme-less `host:port?…` are accepted (an
//! operator's configured remote is often just `host:port`). An authority may
//! be `host`, `host:port`, `[v6]` or `[v6]:port`; a userinfo prefix is
//! rejected rather than silently dropped, because an SRT authority has no
//! credentials and a stream key written after `@` must not vanish into a bare
//! `host:port`. A bare, unbracketed IPv6 literal is rejected: `::1:9000` is
//! ambiguous (is `9000` a port or the last group?), so the operator brackets
//! it.
//!
//! The query is split off the raw text rather than read through
//! `Url::query_pairs`, a documented exception to the workspace's de-hand-roll
//! rule (spec §9): a Haivision `streamid=#!::r=…` value contains a `#` that a
//! `Url` treats as a fragment delimiter, which would cut the value short
//! unless it were percent-encoded. Splitting first keeps the whole value.

use alloc::string::{String, ToString};
use core::net::{IpAddr, SocketAddr};

/// SRT's registered default port, so a URL that omits the port still names a
/// dialable endpoint.
pub const DEFAULT_PORT: u16 = 9000;

/// The largest `latency` the draft treats as practical
/// (draft-sharabayko-srt-01 §3.2.1.1), in milliseconds. A larger value is
/// rejected rather than clamped: a wrong latency is a real misconfiguration.
pub const MAX_LATENCY_MS: u16 = 8000;

/// Which side opens the SRT handshake (`draft-sharabayko-srt-01` §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SrtMode {
    /// The caller dials a listener.
    #[default]
    Caller,
    /// The endpoint binds and accepts inbound callers.
    Listener,
}

impl UrlRejection {
    /// The `name()` label convention (#204).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            UrlRejection::Authority => "authority",
            UrlRejection::LatencyNotANumber => "latency-not-a-number",
            UrlRejection::LatencyTooLarge => "latency-too-large",
            UrlRejection::Mode => "mode",
        }
    }
}

impl SrtMode {
    /// The `name()` label convention (#204).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            SrtMode::Caller => "caller",
            SrtMode::Listener => "listener",
        }
    }
}

broadcast_common::impl_spec_display!(SrtMode);
broadcast_common::impl_spec_display!(UrlRejection);

/// A parsed `srt://` URL: the endpoint to dial and its Stream ID overrides.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SrtUrl {
    /// The endpoint's host (a domain name or a bracketed IP literal).
    pub host: String,
    /// The endpoint's port, [`DEFAULT_PORT`] when the URL omits it.
    pub port: u16,
    /// The opaque `streamid` value, percent-decoded.
    pub stream_id: Option<String>,
    /// The `latency` value in milliseconds, at most [`MAX_LATENCY_MS`].
    pub latency_ms: Option<u16>,
    /// The `mode` value; [`SrtMode::Caller`] when absent.
    pub mode: SrtMode,
    /// The `passphrase` value (payload encryption).
    pub passphrase: Option<String>,
}

/// Why an `srt://` URL was rejected: the failing part of the grammar, so a
/// caller can word its own error while the grammar stays in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UrlRejection {
    /// The authority is not a `host[:port]` — no host, a bad port, a userinfo
    /// prefix, or an ambiguous bare IPv6 literal.
    Authority,
    /// A `latency` that is not whole milliseconds.
    LatencyNotANumber,
    /// A `latency` above [`MAX_LATENCY_MS`].
    LatencyTooLarge,
    /// A `mode` other than `caller` or `listener`.
    Mode,
}

/// Manual `Debug`: a `passphrase` is a secret and never renders.
impl core::fmt::Debug for SrtUrl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SrtUrl")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("stream_id", &self.stream_id)
            .field("latency_ms", &self.latency_ms)
            .field("mode", &self.mode)
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl SrtUrl {
    /// Parse an `srt://` URL; see this module's own doc for the accepted
    /// shapes.
    ///
    /// An absent `latency`, `mode` or `passphrase` leaves the corresponding
    /// field unset, so the caller keeps whatever its handshake config already
    /// holds. Unknown query keys are ignored.
    ///
    /// # Errors
    ///
    /// [`UrlRejection::Authority`] for a bad authority,
    /// [`UrlRejection::LatencyNotANumber`]/[`UrlRejection::LatencyTooLarge`]
    /// for a `latency`, and [`UrlRejection::Mode`] for an unknown `mode`.
    pub fn parse(url: &str) -> Result<Self, UrlRejection> {
        let (authority, query) = match url.split_once("srt://") {
            Some(("", rest)) => match rest.split_once('?') {
                Some((authority, query)) => (authority, Some(query)),
                None => (rest, None),
            },
            _ => match url.split_once('?') {
                Some((authority, query)) => (authority, Some(query)),
                None => (url, None),
            },
        };
        let (host, port) = parse_authority(authority).ok_or(UrlRejection::Authority)?;

        let mut parsed = Self {
            host,
            port,
            stream_id: None,
            latency_ms: None,
            mode: SrtMode::Caller,
            passphrase: None,
        };
        if let Some(query) = query {
            for pair in query.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                match key {
                    "streamid" => parsed.stream_id = Some(percent_decode(value)),
                    "latency" => {
                        parsed.latency_ms = Some(parse_latency(value)?);
                    }
                    "mode" => parsed.mode = parse_mode(value).ok_or(UrlRejection::Mode)?,
                    "passphrase" => parsed.passphrase = Some(percent_decode(value)),
                    _ => {}
                }
            }
        }
        Ok(parsed)
    }

    /// The handshake's dial address: `host:port`, or the literal socket
    /// address when the host already is an IP.
    #[must_use]
    pub fn dial_addr(&self) -> String {
        match self.host.parse::<IpAddr>() {
            Ok(ip) => SocketAddr::new(ip, self.port).to_string(),
            Err(_) => format!("{}:{}", self.host, self.port),
        }
    }
}

/// Parse an SRT authority into `(host, port)`, or `None` when it is not one.
fn parse_authority(authority: &str) -> Option<(String, u16)> {
    let authority = authority.trim_end_matches('/');
    if authority.is_empty() {
        return None;
    }
    let url = url::Url::parse(&format!("srt://{authority}")).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?.to_string();
    Some((host, url.port().unwrap_or(DEFAULT_PORT)))
}

/// Parse a `latency` value: whole milliseconds, at most [`MAX_LATENCY_MS`].
fn parse_latency(value: &str) -> Result<u16, UrlRejection> {
    let ms = value
        .parse::<u16>()
        .map_err(|_| UrlRejection::LatencyNotANumber)?;
    if ms > MAX_LATENCY_MS {
        return Err(UrlRejection::LatencyTooLarge);
    }
    Ok(ms)
}

/// Parse a `mode` value.
fn parse_mode(value: &str) -> Option<SrtMode> {
    match value {
        "caller" => Some(SrtMode::Caller),
        "listener" => Some(SrtMode::Listener),
        _ => None,
    }
}

/// Percent-decode a query value. A stray `%` with no valid hex pair is kept
/// literally rather than dropped.
fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_the_passphrase() {
        let url = SrtUrl::parse("srt://h?passphrase=SECRET").expect("passphrase");
        let rendered = format!("{url:?}");
        assert!(!rendered.contains("SECRET"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn host_only_takes_the_default_port() {
        let url = SrtUrl::parse("srt://example.com").expect("host only");
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, DEFAULT_PORT);
        assert_eq!(url.mode, SrtMode::Caller);
    }

    #[test]
    fn bracketed_ipv6_keeps_its_brackets_off_the_host() {
        let url = SrtUrl::parse("srt://[::1]:9001").expect("bracketed v6");
        assert_eq!(url.host, "[::1]");
        assert_eq!(url.dial_addr(), "[::1]:9001");
    }

    #[test]
    fn a_bare_ipv6_literal_is_ambiguous_not_guessed() {
        assert_eq!(SrtUrl::parse("::1:9000"), Err(UrlRejection::Authority));
    }

    #[test]
    fn the_latency_cap_is_the_drafts() {
        assert!(SrtUrl::parse("srt://h?latency=8000").is_ok());
        assert_eq!(
            SrtUrl::parse("srt://h?latency=8001"),
            Err(UrlRejection::LatencyTooLarge)
        );
        assert_eq!(
            SrtUrl::parse("srt://h?latency=late"),
            Err(UrlRejection::LatencyNotANumber)
        );
    }
}
