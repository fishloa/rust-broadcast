//! The SRT push transport (issue #744 Phase 1) — a thin wrapper over
//! [`srt_runtime::io::SrtSocket`] in **Caller** mode: dials out to a
//! downstream SRT Listener and pushes muxed TS payloads to it.
//!
//! SRT is the first concrete [`PushTransport`] (see `super`): the simplest
//! push wire protocol this crate could start with — a single
//! [`srt_runtime::io::SrtSocket::connect`] handshake and a
//! [`send`](SrtTransport#method.send) per muxed packet, with no session
//! protocol (no RTMP connect/publish, no RTSP ANNOUNCE/RECORD) to run before
//! the first payload.

use crate::push::PushTransport;
use srt_runtime::HandshakeConfig;
use srt_runtime::SrtUrl;
use srt_runtime::io::{SrtSocket, SrtSocketConfig};
use srt_runtime::url::{SrtMode, UrlRejection};

/// Per-connection configuration for the SRT push transport — the SRT
/// handshake config dialed out with (latency, MTU, …).
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct SrtTransportConfig {
    /// The SRT handshake configuration used for
    /// [`SrtSocket::connect`](srt_runtime::io::SrtSocket::connect).
    pub srt_config: HandshakeConfig,
}

impl SrtTransportConfig {
    /// Set the SRT handshake configuration.
    #[must_use]
    pub fn with_srt_config(mut self, srt_config: HandshakeConfig) -> Self {
        self.srt_config = srt_config;
        self
    }
}

/// The SRT push transport: an owned [`SrtSocket`] in Caller mode.
///
/// The socket is held as `Option` so [`close`](SrtTransport::close) can
/// actually tear the connection down (dropping the owned handle aborts the
/// SRT driver task) rather than only dropping at the enclosing scope.
pub struct SrtTransport {
    socket: Option<SrtSocket>,
    /// The connection's long-lived TS muxer (issue #1024).
    ts_mux: crate::push::TsPushMux,
}

// (No URL is stored, so nothing here can leak one.)
impl std::fmt::Debug for SrtTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SrtTransport")
            .field("connected", &self.socket.is_some())
            .finish()
    }
}

/// Parse an `srt://` URL into the `host:port` to dial and the query overrides
/// the handshake takes (audit run 7, W17), applying this push's policy.
///
/// The URL grammar (`srt://` or scheme-less authority, bracketed IPv6,
/// percent decoding, the latency cap and the default port) is
/// [`srt_runtime::SrtUrl`](SrtUrl)'s. A push dials out, so only `mode=caller`
/// is accepted, and SRT payload encryption is not implemented here, so a
/// `passphrase` is rejected rather than silently ignored.
fn parse_srt_url(url: &str) -> Result<(String, SrtUrl), srt_runtime::Error> {
    let invalid = |what: &'static str| srt_runtime::Error::Io {
        kind: std::io::ErrorKind::InvalidInput,
        context: what,
    };
    let parsed = match SrtUrl::parse(url) {
        Ok(parsed) => parsed,
        Err(UrlRejection::LatencyTooLarge) => {
            return Err(invalid("srt url: latency exceeds the 8000 ms maximum"));
        }
        Err(UrlRejection::LatencyNotANumber) => {
            return Err(invalid(
                "srt url: latency must be an integer number of milliseconds",
            ));
        }
        Err(_) => return Err(invalid("srt url: bad host")),
    };
    if parsed.mode != SrtMode::Caller {
        return Err(invalid(
            "srt url: only mode=caller is supported for a push output",
        ));
    }
    if parsed.passphrase.is_some() {
        return Err(invalid(
            "srt url: passphrase/encryption is not supported for a push output",
        ));
    }
    Ok((parsed.dial_addr(), parsed))
}

#[cfg(any(test, feature = "test-seams"))]
#[doc(hidden)]
pub fn parse_srt_url_for_test(url: &str) -> Result<(String, SrtUrl), srt_runtime::Error> {
    parse_srt_url(url)
}

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

#[async_trait::async_trait]
impl PushTransport for SrtTransport {
    type Config = SrtTransportConfig;
    type Error = srt_runtime::Error;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        let (addr, srt_url) = parse_srt_url(url)?;
        let mut srt_config = config.srt_config.clone();
        if let Some(stream_id) = srt_url.stream_id {
            srt_config.stream_id = Some(stream_id);
        }
        if let Some(latency) = srt_url.latency {
            srt_config.latency = latency;
        }
        let socket = SrtSocket::connect(addr, SrtSocketConfig::new(srt_config)).await?;
        Ok(Self {
            socket: Some(socket),
            ts_mux: crate::push::TsPushMux::new(),
        })
    }

    fn media_muxer(&mut self) -> Option<&mut dyn crate::push::MediaMuxer> {
        Some(&mut self.ts_mux)
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let socket = self.socket.as_mut().ok_or(srt_runtime::Error::Io {
            kind: std::io::ErrorKind::NotConnected,
            context: "push send",
        })?;
        // One SRT message is one UDP datagram (no segmentation in the
        // runtime), and the receiver drops and counts any datagram over its
        // MTU-sized cap. A muxed frame is far larger than that, so split it
        // into the standard datagram payload (`mpeg_ts::TS_BYTES_PER_DATAGRAM`,
        // seven 188-byte packets);
        // before this, every frame over ~1.3 kB was silently lost (#1024).
        for chunk in data.chunks(mpeg_ts::TS_BYTES_PER_DATAGRAM) {
            socket.send(chunk).await?;
        }
        Ok(())
    }

    fn close(&mut self) {
        // Dropping the owned SrtSocket handle aborts its driver task.
        self.socket = None;
    }
}

#[cfg(all(test, feature = "test-seams"))]
mod seam_tests;

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
    /// passed through as part of it, and only this push's values survive the
    /// policy. The URL grammar itself is `srt_runtime::SrtUrl`'s
    /// (`srt-runtime/tests/it/url_table.rs`); the push applies its own rules.
    #[test]
    fn parse_srt_url_separates_address_from_query() {
        let (addr, url) =
            parse_srt_url("srt://example.com:9001?streamid=live/cam&latency=120").unwrap();
        assert_eq!(
            addr, "example.com:9001",
            "the dial address is host:port only"
        );
        assert_eq!(url.stream_id.as_deref(), Some("live/cam"));
        assert_eq!(url.latency, Some(Duration::from_millis(120)));

        // Scheme-less `host:port` is accepted, and a bare host takes the
        // default port.
        let (addr, _) = parse_srt_url("example.com:9000").unwrap();
        assert_eq!(addr, "example.com:9000");
        let (addr, url) = parse_srt_url("example.com").unwrap();
        assert_eq!(addr, "example.com:9000");
        assert_eq!(url.latency, None);

        // Haivision `#!::r=…` streamid keeps the whole value (a live stream
        // key is a credential, so only the shape is asserted here).
        let (_, url) = parse_srt_url("srt://h:9000?streamid=#!::r=live/cam,m=publish").unwrap();
        assert_eq!(url.stream_id.as_deref(), Some("#!::r=live/cam,m=publish"));

        // Hostile inputs are rejected, never panicked on: an empty host, a
        // bad port, an out-of-range or unparseable latency, a non-caller
        // mode, a passphrase, and a userinfo prefix.
        for url in [
            "",
            ":9000",
            "host:notaport",
            "host:99999",
            "host:9000?latency=notanumber",
            "host:9000?latency=99999",
            "host:9000?mode=listener",
            "host:9000?passphrase=secret",
            "srt://user:key@host:9000",
            "srt://user@host:9000",
            "::1",
            "::1:9000",
        ] {
            let Err(srt_runtime::Error::Io { .. }) = parse_srt_url(url) else {
                panic!("{url:?} must be rejected");
            };
        }
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
        let mut listener = SrtListener::bind(listener_addr, SrtSocketConfig::default())
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
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
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

    fn fixture_media() -> test_harness::collect::Collected {
        let ts = test_fixtures::read("ts/h264-aac.ts");
        let mut media = test_harness::collect::media_from_ts(&ts).expect("demux fixture");
        for track in &mut media.tracks {
            track.samples.truncate(2);
        }
        media
    }

    /// The builder keeps the handshake it is given, not the default.
    #[test]
    fn with_srt_config_keeps_the_supplied_handshake() {
        let handshake = HandshakeConfig::default().with_latency(Duration::from_millis(250));
        let cfg = SrtTransportConfig::default().with_srt_config(handshake.clone());
        assert_eq!(cfg.srt_config.latency, Duration::from_millis(250));
        assert_eq!(cfg.srt_config, handshake);
    }

    /// `Debug` names the type and its connection state.
    #[test]
    fn transport_debug_reports_the_connection_state() {
        let transport = SrtTransport {
            socket: None,
            ts_mux: crate::push::TsPushMux::new(),
        };
        let shown = format!("{transport:?}");
        assert!(shown.contains("SrtTransport"), "{shown}");
        assert!(shown.contains("connected: false"), "{shown}");
    }

    /// One pass of `media` through the transport's per-connection TS muxer,
    /// samples fed in decode order.
    fn encode_pass<T: PushTransport>(
        transport: &mut T,
        media: &test_harness::collect::Collected,
    ) -> Vec<u8> {
        let specs: Vec<transmux::TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
        transport
            .media_muxer()
            .expect("a TS push carries its own mux")
            .set_tracks(&specs)
            .expect("announce tracks");
        let mut items: Vec<(i128, transmux::TrackId, transmux::Sample)> = Vec::new();
        for track in &media.tracks {
            for sample in &track.samples {
                let dts = i128::from(sample.timing.and_then(|t| t.dts()).unwrap_or(0)) * 90_000
                    / i128::from(track.spec.timescale.get());
                items.push((dts, track.spec.track_id, sample.clone()));
            }
        }
        items.sort_by_key(|(dts, track, _)| (*dts, *track));
        let mut out = Vec::new();
        for (_, track, sample) in items {
            for message in transport
                .media_muxer()
                .expect("a TS push carries its own mux")
                .encode_sample(track, &sample)
                .expect("encode sample")
            {
                out.extend_from_slice(&message);
            }
        }
        out
    }

    /// The transport keeps its own streaming mux: a second identical pass
    /// continues the continuity counters instead of restarting them.
    #[test]
    fn the_transport_muxes_through_its_own_streaming_mux() {
        let mut transport = SrtTransport {
            socket: None,
            ts_mux: crate::push::TsPushMux::new(),
        };
        let media = fixture_media();
        let first = encode_pass(&mut transport, &media);
        let second = encode_pass(&mut transport, &media);
        assert!(!first.is_empty(), "the fixture must produce TS bytes");
        assert_ne!(
            first, second,
            "continuity counters must continue across passes, not restart at zero"
        );
    }

    /// `close` drops the socket: a later send is the not-connected error, and
    /// the listener observes the shutdown.
    #[tokio::test]
    async fn close_tears_the_transport_down_and_refuses_further_sends() {
        let mut listener = SrtListener::bind(
            "127.0.0.1:0"
                .parse::<std::net::SocketAddr>()
                .expect("loopback addr"),
            SrtSocketConfig::default(),
        )
        .await
        .expect("bind");
        let bound = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let Ok(Ok(mut sock)) =
                tokio::time::timeout(Duration::from_secs(30), listener.accept()).await
            else {
                return false;
            };
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            loop {
                match tokio::time::timeout_at(deadline, sock.recv()).await {
                    Ok(Ok(None)) => return true,
                    Ok(Ok(Some(_))) => continue,
                    Ok(Err(_)) | Err(_) => return false,
                }
            }
        });

        let mut transport = tokio::time::timeout(
            Duration::from_secs(30),
            SrtTransport::connect(
                &format!("srt://{bound}?mode=caller"),
                &SrtTransportConfig::default(),
            ),
        )
        .await
        .expect("connect must not hang")
        .expect("connect");
        transport.close();
        let err = transport
            .send(&[0x47; 188])
            .await
            .expect_err("a closed transport must refuse a send");
        assert!(
            matches!(
                &err,
                srt_runtime::Error::Io {
                    kind: std::io::ErrorKind::NotConnected,
                    ..
                }
            ),
            "closed send must be NotConnected: {err:?}"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(30), server)
                .await
                .expect("server must not hang")
                .expect("join"),
            "the listener must observe the shutdown"
        );
    }
}
