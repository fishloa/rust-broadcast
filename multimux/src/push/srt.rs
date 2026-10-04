//! The SRT push transport (issue #744 Phase 1) — a thin wrapper over
//! [`srt_runtime::io::SrtSocket`] in **Caller** mode: dials out to a
//! downstream SRT Listener and pushes muxed TS payloads to it.
//!
//! SRT is the first concrete [`PushTransport`] (see `super`): the simplest
//! push wire protocol this crate could start with — a single
//! [`srt_runtime::io::SrtSocket::connect`] handshake and a
//! [`send`](SrtTransport#method.send) per muxed batch, with no session
//! protocol (no RTMP connect/publish, no RTSP ANNOUNCE/RECORD) to run before
//! the first payload.

use crate::push::PushTransport;
use srt_runtime::HandshakeConfig;
use srt_runtime::io::SrtSocket;

/// Per-connection configuration for the SRT push transport — the SRT
/// handshake config dialed out with (latency, MTU, …).
#[derive(Debug, Clone, Default)]
pub struct SrtTransportConfig {
    /// The SRT handshake configuration used for
    /// [`SrtSocket::connect`](srt_runtime::io::SrtSocket::connect).
    pub srt_config: HandshakeConfig,
}

/// The SRT push transport: an owned [`SrtSocket`] in Caller mode.
///
/// The socket is held as `Option` so [`close`](SrtTransport::close) can
/// actually tear the connection down (dropping the owned handle aborts the
/// SRT driver task) rather than only dropping at the enclosing scope.
pub struct SrtTransport {
    socket: Option<SrtSocket>,
}

// (No URL is stored, so nothing here can leak one.)
impl std::fmt::Debug for SrtTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SrtTransport")
            .field("connected", &self.socket.is_some())
            .finish()
    }
}

/// Query-string overrides parsed from an `srt://` URL.
#[doc(hidden)]
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SrtUrlOverrides {
    /// The opaque `streamid` query value (percent-decoded), if present.
    pub stream_id: Option<String>,
    /// The `latency` query value in milliseconds, if present.
    pub latency_ms: Option<u16>,
}

/// Parse an SRT URL into the `host:port` to dial and its query-string
/// overrides (audit run 7, W17).
///
/// Accepts both `srt://host:port?…` and a **scheme-less** `host:port?…`
/// (the pre-fix code accepted the latter via `unwrap_or(url)`; routing it
/// through `url::Url::parse` made `host:9000` parse as scheme `"host"` with
/// no host, breaking it). IPv6 literals are accepted bracketed (`[::1]:9000`)
/// or bare (`::1:9000`).
///
/// The query is split off **manually** (first `?`), not via `url`'s
/// `query_pairs()`, because a Haivision `streamid=#!::r=…` value contains a
/// `#` that a `Url` parser treats as a fragment delimiter, cutting the value
/// short unless it is percent-encoded. Splitting on the raw string keeps the
/// whole value.
///
/// Recognised keys: `streamid` (opaque, passed to the handshake) and `latency`
/// (milliseconds, `0..=MAX_SRT_LATENCY_MS`; an out-of-range or unparseable
/// value is rejected, not silently ignored — a wrong latency is a real
/// misconfiguration). `mode` is validated to be `caller` (this transport only
/// dials); `passphrase` is rejected as unsupported (SRT encryption is not
/// implemented). Unknown keys are ignored.
fn parse_srt_url(url: &str) -> Result<(String, SrtUrlOverrides), srt_runtime::Error> {
    let invalid = |what: &'static str| srt_runtime::Error::Io {
        kind: std::io::ErrorKind::InvalidInput,
        context: what,
    };
    let stripped = url.strip_prefix("srt://").unwrap_or(url);
    let (authority, query) = match stripped.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (stripped, None),
    };
    let authority = authority.trim_end_matches('/');
    let addr = normalize_srt_authority(authority).ok_or_else(|| invalid("srt url: bad host"))?;

    let mut overrides = SrtUrlOverrides::default();
    if let Some(query) = query {
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (key, value) = match pair.split_once('=') {
                Some((k, v)) => (k, v),
                None => (pair, ""),
            };
            match key {
                "streamid" => overrides.stream_id = Some(percent_decode(value)),
                "latency" => {
                    let ms = value.parse::<u16>().map_err(|_| {
                        invalid("srt url: latency must be an integer number of milliseconds")
                    })?;
                    if ms > MAX_SRT_LATENCY_MS {
                        return Err(invalid("srt url: latency exceeds the 8000 ms maximum"));
                    }
                    overrides.latency_ms = Some(ms);
                }
                "mode" if value != "caller" => {
                    return Err(invalid(
                        "srt url: only mode=caller is supported for a push output",
                    ));
                }
                "passphrase" => {
                    return Err(invalid(
                        "srt url: passphrase/encryption is not supported for a push output",
                    ));
                }
                _ => {}
            }
        }
    }
    Ok((addr, overrides))
}

/// `#[doc(hidden)]` test seam over [`parse_srt_url`] (the real push path).
#[doc(hidden)]
pub fn parse_srt_url_for_test(url: &str) -> Result<(String, SrtUrlOverrides), srt_runtime::Error> {
    parse_srt_url(url)
}

/// Normalise an SRT authority into a `host:port` string, or `None` if it has
/// no host. Accepts `host`, `host:port`, `[v6]`, `[v6]:port`, and a bare
/// `v6` literal (bracketed for `ToSocketAddrs`); defaults the port.
///
/// The split is done by the `url` crate (via a throwaway `srt://` prefix), so
/// a bracketed IPv6 literal is handled by the parser rather than by hand. A
/// **bare** IPv6 literal (`::1:9000`) fails `Url::parse` — the address is
/// ambiguous (is `9000` a port or the last group?), so the operator must
/// bracket it to disambiguate.
fn normalize_srt_authority(authority: &str) -> Option<String> {
    if authority.is_empty() {
        return None;
    }
    let url = url::Url::parse(&format!("srt://{authority}")).ok()?;
    // Reject a userinfo prefix rather than silently dropping it: an SRT
    // authority is `host[:port]`, and a caller that put a credential there
    // (a stream key after `@`) must not have it vanish into a bare host:port.
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host()?;
    let port = url.port().unwrap_or(DEFAULT_SRT_PORT);
    Some(format!("{host}:{port}"))
}

/// Percent-decode an SRT query value (`%XX` → byte). A Haivision
/// `streamid=#!::r=…` value is typically left unencoded and passes through
/// literally; a percent-encoded one (e.g. a `%23` for `#`, or `%2C` for a
/// comma) is decoded. Delegates to `percent_encoding`, which leaves a stray
/// `%` with no valid hex pair literal rather than dropping it.
fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
}

/// Upper bound on an SRT `latency` query value, milliseconds
/// (draft-sharabayko-srt §3.2.1.1 practical maximum).
const MAX_SRT_LATENCY_MS: u16 = 8000;

/// Validate an `srt://` push URL at config time (audit W17), so a bad URL
/// (missing host, bad port, unsupported `mode`/`passphrase`, out-of-range
/// `latency`) surfaces as a config error — including on an admin add/reload —
/// rather than when the push task first dials.
pub(crate) fn validate_srt_url(url: &str) -> Result<(), String> {
    // The URL itself is never echoed: its `streamid`/`passphrase` are
    // credentials (audit T14, #1142). The destination (host:port) and the
    // fixed reason say what is wrong.
    parse_srt_url(url).map(|_| ()).map_err(|e| {
        format!(
            "not a valid srt:// URL ({}): {e}",
            crate::redact::redact_destination(url)
        )
    })
}

/// SRT's IANA-registered default port (RFC-style default for the `srt://`
/// scheme; draft-sharabayko-srt §Appendix).
const DEFAULT_SRT_PORT: u16 = 9000;

#[async_trait::async_trait]
impl PushTransport for SrtTransport {
    type Config = SrtTransportConfig;
    type Error = srt_runtime::Error;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        let (addr, overrides) = parse_srt_url(url)?;
        let mut srt_config = config.srt_config.clone();
        if let Some(stream_id) = overrides.stream_id {
            srt_config.stream_id = Some(stream_id);
        }
        if let Some(latency_ms) = overrides.latency_ms {
            srt_config.latency_ms = latency_ms;
        }
        let socket = SrtSocket::connect(addr, srt_config).await?;
        Ok(Self {
            socket: Some(socket),
        })
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let socket = self.socket.as_mut().ok_or(srt_runtime::Error::Io {
            kind: std::io::ErrorKind::NotConnected,
            context: "push send",
        })?;
        socket.send(data).await
    }

    fn close(&mut self) {
        // Dropping the owned SrtSocket handle aborts its driver task.
        self.socket = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit T14 (#1142): the config-time validation error says what is wrong
    /// and where, never the credentials in the URL.
    #[test]
    fn srt_validation_errors_do_not_echo_the_url() {
        for url in [
            "srt://host:9000?streamid=SECRETSTREAM&passphrase=SECRETPASS",
            "srt://host:9000?streamid=SECRETSTREAM&latency=99999",
            "srt://?streamid=SECRETSTREAM&passphrase=SECRETPASS",
        ] {
            let err = validate_srt_url(url).unwrap_err();
            for secret in ["SECRETSTREAM", "SECRETPASS", "99999"] {
                assert!(!err.contains(secret), "{secret} in {err:?} (from {url})");
            }
            assert!(err.contains("not a valid srt:// URL"), "{err}");
        }
        assert_eq!(
            validate_srt_url("srt://host:9000?passphrase=SECRETPASS").unwrap_err(),
            "not a valid srt:// URL (srt://host:9000/<redacted>): io error during srt url: \
             passphrase/encryption is not supported for a push output: InvalidInput"
        );
    }
    use srt_runtime::io::SrtListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    /// Audit run 7, W17: the query string is parsed off the address, not
    /// passed through as part of it. Before the fix
    /// `url.strip_prefix("srt://")` left `host:port?streamid=…` as the dial
    /// address, so the handshake resolved a bogus host.
    #[test]
    fn parse_srt_url_separates_address_from_query() {
        let (addr, o) =
            parse_srt_url("srt://example.com:9001?streamid=live/cam&latency=120").unwrap();
        assert_eq!(
            addr, "example.com:9001",
            "the dial address is host:port only"
        );
        assert_eq!(o.stream_id.as_deref(), Some("live/cam"));
        assert_eq!(o.latency_ms, Some(120));

        // Scheme-less `host:port` is accepted (regression: `url::Url` parsed
        // `host:9000` as scheme "host" with no host).
        let (addr, _) = parse_srt_url("example.com:9000").unwrap();
        assert_eq!(addr, "example.com:9000");
        let (addr, _) = parse_srt_url("127.0.0.1:9001").unwrap();
        assert_eq!(addr, "127.0.0.1:9001");
        // Bare host → default port.
        let (addr, o) = parse_srt_url("example.com").unwrap();
        assert_eq!(addr, "example.com:9000");
        assert_eq!(o, SrtUrlOverrides::default());

        // IPv6, bracketed and bare.
        let (addr, _) = parse_srt_url("srt://[::1]:9000").unwrap();
        assert_eq!(addr, "[::1]:9000");
        let (addr, _) = parse_srt_url("[::1]:9001").unwrap();
        assert_eq!(addr, "[::1]:9001");
        // A bare, unbracketed IPv6 authority is ambiguous → error.
        assert!(parse_srt_url("::1").is_err());
        assert!(parse_srt_url("::1:9000").is_err());
        assert!(parse_srt_url("srt://::1").is_err());

        // Haivision `#!::r=…` streamid keeps the whole value (a `#` is not a
        // fragment delimiter here); an unencoded value passes through
        // literally.
        let (_, o) = parse_srt_url("srt://h:9000?streamid=#!::r=live/cam,m=publish").unwrap();
        assert_eq!(o.stream_id.as_deref(), Some("#!::r=live/cam,m=publish"));
        // A percent-encoded streamid is decoded.
        let (_, o) = parse_srt_url("srt://h:9000?streamid=%23!::r=live%2Fcam%2Cm=publish").unwrap();
        assert_eq!(o.stream_id.as_deref(), Some("#!::r=live/cam,m=publish"));
        // A stray `%` is kept literal, not dropped.
        let (_, o) = parse_srt_url("srt://h:9000?streamid=100%").unwrap();
        assert_eq!(o.stream_id.as_deref(), Some("100%"));

        // Hostile inputs: empty host, bad port, out-of-range latency, a
        // non-caller mode, and a passphrase are all rejected, not panicked.
        assert!(parse_srt_url("").is_err());
        assert!(parse_srt_url(":9000").is_err());
        assert!(parse_srt_url("host:notaport").is_err());
        assert!(parse_srt_url("host:99999").is_err());
        assert!(parse_srt_url("host:9000?latency=notanumber").is_err());
        assert!(parse_srt_url("host:9000?latency=99999").is_err());
        assert!(parse_srt_url("host:9000?mode=listener").is_err());
        assert!(parse_srt_url("host:9000?passphrase=secret").is_err());
        // A userinfo prefix is rejected, not silently dropped (the parser
        // would otherwise reduce `user:key@host:9000` to `host:9000`).
        assert!(parse_srt_url("srt://user:key@host:9000").is_err());
        assert!(parse_srt_url("srt://user@host:9000").is_err());
        // `mode=caller` is accepted.
        assert!(parse_srt_url("host:9000?mode=caller").is_ok());
    }

    /// SRT loopback (issue #744): spawn a real test-owned `SrtListener` that
    /// accepts one Caller, then connect an `SrtTransport` (Caller mode) to it
    /// and push bytes — verifying the listener actually receives them. The
    /// `SrtSocket` data plane is delivery-guaranteed over loopback UDP, so
    /// the receiver-side `recv` poll is the assertion.
    #[tokio::test]
    async fn srt_transport_pushes_bytes_to_a_listener() {
        const PAYLOAD: &[u8] = &[0x47, 0x40, 0x00, 0x10, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00];
        let received: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let received_for_task = Arc::clone(&received);

        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let mut listener = SrtListener::bind(listener_addr, HandshakeConfig::default())
            .await
            .expect("bind");
        let bound = listener.local_addr().expect("local addr");

        let server = tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            let mut sock = match tokio::time::timeout_at(deadline, listener.accept()).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => panic!("accept failed: {e}"),
                Err(_) => panic!("accept timed out"),
            };
            // Receive until we see our payload or time out.
            while tokio::time::Instant::now() < deadline {
                match tokio::time::timeout(Duration::from_millis(200), sock.recv()).await {
                    Ok(Ok(Some(data))) if &data[..] == PAYLOAD => {
                        received_for_task.store(true, Ordering::SeqCst);
                        break;
                    }
                    Ok(Ok(Some(_))) => continue,
                    Ok(Ok(None)) => break,
                    Ok(Err(e)) => panic!("recv failed: {e}"),
                    Err(_) => continue,
                }
            }
        });

        let cfg = SrtTransportConfig::default();
        // The URL carries a query (`streamid`/`latency`) that must be parsed
        // off the address, not passed through as part of it (audit W17) —
        // `parse_srt_url` strips it, the handshake dials `host:port`.
        let url = format!("srt://{bound}?streamid=live/cam&latency=120&mode=caller");
        // HANG GUARD (workspace precedent, issue #826): bound the dial so a
        // never-connecting caller fails rather than hangs.
        let transport =
            tokio::time::timeout(Duration::from_secs(30), SrtTransport::connect(&url, &cfg))
                .await
                .expect("connect must not hang")
                .expect("connect with a query-string URL must succeed");

        let mut transport = transport;
        transport.send(PAYLOAD).await.expect("send");
        // Keep `transport` alive so its SRT driver task flushes the payload
        // to the listener (dropping the handle aborts the driver mid-flight).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !received.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        transport.close();
        server.abort();
        assert!(
            received.load(Ordering::SeqCst),
            "downstream SRT listener must receive the pushed bytes"
        );
    }
}
