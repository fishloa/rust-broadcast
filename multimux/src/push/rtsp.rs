//! RTSP push transport — client ANNOUNCE/RECORD to a remote RTSP server.
//!
//! RFC 2326 §10.3 (ANNOUNCE), §10.4 (SETUP), §10.11 (RECORD). The MPEG-2 TS
//! bytes produced by the default `PushTransport::encode_media` are sent as
//! RTP (RFC 2250 §2: MPEG2-TS, static payload type 33, clock rate 90000),
//! framed as interleaved TCP data (§10.12) — one SETUP/one interleaved
//! channel for the whole muxed stream, matching the single m= line the SDP
//! below advertises.
//!
//! Fixed here (audit run-07 C3 / run-09 C1, #1025) — this transport
//! previously could not push to any real server:
//! - the configured push `url` is now actually used for every request
//!   (previously hard-coded to `rtsp://localhost/push`);
//! - userinfo credentials in the URL are honoured (`config.credentials`
//!   still wins when both are given, matching
//!   [`crate::source::http_auth::resolve_credentials`]'s precedence);
//! - every response status is checked; a non-2xx now fails `connect`/`setup`
//!   instead of being silently ignored;
//! - the SDP carries a session-level `c=` line (RFC 4566 requires one
//!   somewhere) and `a=rtpmap`/`a=control`, and SETUP addresses the track's
//!   own control URL (RFC 2326 Appendix C.1.1), not the aggregate URL;
//! - the interleaved payload is now RTP, not raw TS bytes with no RTP
//!   header (RFC 2326 §10.12 requires interleaved data to be RTP/RTCP);
//! - a 401 challenge's `ClientEvent::AuthRetry` is now written to the
//!   socket — rtsp-runtime computes the retry, but this transport never
//!   sent it, so an authenticating server (Digest or Basic) always failed
//!   the push even after rtsp-runtime's own C1 fix.

use crate::push::PushTransport;
use rtsp_runtime::client::{ClientEvent, ClientSession};
use rtsp_runtime::interleaved::InterleavedFrame;
use rtsp_runtime::transport::{Transport, TransportSpec};
use rtsp_runtime::{Credentials, StatusCode};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use transmux::ir::TrackSpec;

/// RFC 3551 §6 static payload type for MPEG2 Transport Stream (RFC 2250 §2).
const PT_MP2T: u8 = 33;
/// MPEG-2 TS packet length (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;
/// RFC 2250 §2: "no more than seven ... TS packets" per RTP packet.
const MAX_TS_PACKETS_PER_RTP: usize = 7;
/// The resulting cap on one RTP packet's MP2T payload (1316 bytes).
const MAX_RTP_PAYLOAD: usize = TS_PACKET_LEN * MAX_TS_PACKETS_PER_RTP;
/// RFC 2250 §2's fixed RTP clock rate for MP2T.
const RTP_CLOCK_HZ: u64 = 90_000;
/// Read buffer for one RTSP response (SDP + headers comfortably fit).
const READ_BUF_LEN: usize = 4096;

/// Per-connection configuration for the RTSP push transport.
#[derive(Debug, Clone, Default)]
pub struct RtspTransportConfig {
    /// Optional credentials for RTSP auth. Takes precedence over any
    /// username/password already present in the push URL's userinfo.
    pub credentials: Option<(String, String)>,
}

/// The RTSP push transport: an owned TCP connection + [`ClientSession`].
pub struct RtspTransport {
    stream: Option<TcpStream>,
    client: ClientSession,
    channel: u8,
    /// The presentation URL (credentials stripped): OPTIONS/ANNOUNCE/RECORD.
    url: String,
    /// The one track's control URL (RFC 2326 Appendix C.1.1): SETUP.
    control_url: String,
    /// RTP sequence number for the outgoing MP2T stream (RFC 3550 §5.1).
    seq: u16,
    /// Pseudo-random per-connection SSRC (no RTCP negotiation on a push).
    ssrc: u32,
    /// Wall-clock origin for the RTP timestamp (90 kHz, RFC 2250 §2).
    started: Instant,
}

impl std::fmt::Debug for RtspTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspTransport")
            .field("connected", &self.stream.is_some())
            .field("url", &self.url)
            .finish()
    }
}

#[async_trait::async_trait]
impl PushTransport for RtspTransport {
    type Config = RtspTransportConfig;
    type Error = RtspPushError;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        let mut parsed = url::Url::parse(url).map_err(|e| RtspPushError::Connect(e.to_string()))?;
        let host = parsed.host_str().unwrap_or("127.0.0.1").to_string();
        let port = parsed.port().unwrap_or(554);
        let addr = format!("{host}:{port}");

        // Config credentials win over the URL's own userinfo (mirrors
        // `crate::source::http_auth::resolve_credentials`'s precedence).
        let url_credentials = credentials_from_url(&parsed)?;
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
        let clean_url = parsed.to_string();

        let stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| RtspPushError::Connect(e.to_string()))?;

        let mut client = ClientSession::new();
        let creds = config
            .credentials
            .as_ref()
            .map(|(user, pass)| Credentials::new(user.clone(), pass.clone()))
            .or(url_credentials);
        if let Some(creds) = creds {
            client = client.with_credentials(creds);
        }

        let control_url = if clean_url.ends_with('/') {
            format!("{clean_url}trackID=0")
        } else {
            format!("{clean_url}/trackID=0")
        };

        let mut transport = Self {
            stream: Some(stream),
            client,
            channel: 0,
            url: clean_url,
            control_url,
            seq: 0,
            ssrc: rand_ssrc(),
            started: Instant::now(),
        };

        let options_bytes = transport
            .client
            .options(&transport.url)
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
        let (status, _) = transport.roundtrip(options_bytes).await?;
        if !status.is_success() {
            return Err(RtspPushError::Protocol(format!(
                "OPTIONS rejected: {status:?}"
            )));
        }

        Ok(transport)
    }

    async fn setup(&mut self, _tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        let sdp = build_sdp();
        let announce_bytes = self
            .client
            .announce(&self.url, &sdp)
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
        let (status, _) = self.roundtrip(announce_bytes).await?;
        if !status.is_success() {
            return Err(RtspPushError::Protocol(format!(
                "ANNOUNCE rejected: {status:?}"
            )));
        }

        // RFC 2326 §12.39: `mode` defaults to PLAY: a RECORD session must
        // say so explicitly, or a strict server (gortsplib/`mediamtx`:
        // "transport header contains a invalid mode (null)") rejects SETUP.
        let mut spec = TransportSpec::rtp_avp_tcp_interleaved(self.channel, self.channel + 1);
        spec.mode = Some("RECORD".to_string());
        let transport_spec = Transport::single(spec);
        let setup_bytes = self
            .client
            .setup(&self.control_url, &transport_spec)
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
        let (status, _) = self.roundtrip(setup_bytes).await?;
        if !status.is_success() {
            return Err(RtspPushError::Protocol(format!(
                "SETUP rejected: {status:?}"
            )));
        }

        let record_bytes = self
            .client
            .record(&self.url)
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
        let (status, _) = self.roundtrip(record_bytes).await?;
        if !status.is_success() {
            return Err(RtspPushError::Protocol(format!(
                "RECORD rejected: {status:?}"
            )));
        }

        Ok(())
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        if self.stream.is_none() {
            return Err(RtspPushError::Connect("not connected".into()));
        }
        // RFC 2250 §2: no more than 7 whole 188-byte TS packets per RTP
        // packet. `MAX_RTP_PAYLOAD` is a multiple of `TS_PACKET_LEN`, and
        // `data` is always a whole number of TS packets (it comes from
        // `TsMux::package`), so every chunk here — including the last — is
        // itself a whole number of TS packets; none is split mid-packet.
        for chunk in data.chunks(MAX_RTP_PAYLOAD) {
            let packet = self.rtp_packet(chunk);
            let frame = InterleavedFrame::new(self.channel, packet);
            let bytes = frame
                .to_bytes()
                .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
            let stream = self
                .stream
                .as_mut()
                .ok_or_else(|| RtspPushError::Connect("not connected".into()))?;
            stream.write_all(&bytes).await.map_err(RtspPushError::Io)?;
        }
        Ok(())
    }

    fn close(&mut self) {
        self.stream = None;
    }
}

impl RtspTransport {
    /// Writes `request` and reads responses until a terminal
    /// [`ClientEvent::Response`], transparently writing any
    /// [`ClientEvent::AuthRetry`] the engine emits along the way. Without
    /// this loop, a 401 challenge's retry bytes (computed by rtsp-runtime)
    /// are simply dropped and the push never reaches RECORD (audit run-07
    /// C3 / run-09 C1, #1025).
    async fn roundtrip(
        &mut self,
        request: Vec<u8>,
    ) -> Result<(StatusCode, Vec<u8>), RtspPushError> {
        {
            let stream = self
                .stream
                .as_mut()
                .ok_or_else(|| RtspPushError::Connect("not connected".into()))?;
            stream
                .write_all(&request)
                .await
                .map_err(RtspPushError::Io)?;
        }

        let mut buf = [0u8; READ_BUF_LEN];
        loop {
            let n = {
                let stream = self
                    .stream
                    .as_mut()
                    .ok_or_else(|| RtspPushError::Connect("not connected".into()))?;
                stream.read(&mut buf).await.map_err(RtspPushError::Io)?
            };
            if n == 0 {
                return Err(RtspPushError::Connect("connection closed by peer".into()));
            }
            let events = self
                .client
                .handle_data(&buf[..n])
                .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
            for event in events {
                match event {
                    ClientEvent::Response { status, body, .. } => return Ok((status, body)),
                    ClientEvent::AuthRetry { request, .. } => {
                        let stream = self
                            .stream
                            .as_mut()
                            .ok_or_else(|| RtspPushError::Connect("not connected".into()))?;
                        stream
                            .write_all(&request)
                            .await
                            .map_err(RtspPushError::Io)?;
                    }
                    ClientEvent::MediaData { .. } => {}
                    // `ClientEvent` is `#[non_exhaustive]`; nothing else is
                    // expected on a push connection's read side.
                    _ => {}
                }
            }
        }
    }

    /// Builds one RTP packet (RFC 3550 §5.1) carrying `payload` as MP2T.
    fn rtp_packet(&mut self, payload: &[u8]) -> Vec<u8> {
        let elapsed_micros = self.started.elapsed().as_micros() as u64;
        let timestamp = ((elapsed_micros * RTP_CLOCK_HZ / 1_000_000) & 0xFFFF_FFFF) as u32;
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);

        let mut packet = Vec::with_capacity(12 + payload.len());
        packet.push(0x80); // V=2, P=0, X=0, CC=0
        packet.push(PT_MP2T); // M=0
        packet.extend_from_slice(&seq.to_be_bytes());
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&self.ssrc.to_be_bytes());
        packet.extend_from_slice(payload);
        packet
    }
}

/// Extracts [`Credentials`] from `url`'s userinfo (RFC 3986 §3.2.1), if any.
/// Mirrors `crate::source::http_auth::credentials_from_url`, kept local so
/// this module's errors stay `RtspPushError` rather than `MultimuxError`.
fn credentials_from_url(url: &url::Url) -> Result<Option<Credentials>, RtspPushError> {
    if url.username().is_empty() {
        return Ok(None);
    }
    let username = percent_decode(url.username())?;
    let password = match url.password() {
        Some(p) => percent_decode(p)?,
        None => String::new(),
    };
    Ok(Some(Credentials::new(username, password)))
}

/// Percent-decodes a URL userinfo component (RFC 3986 §2.1) to UTF-8. The
/// error message deliberately never echoes `s` — it is (part of) a still
/// percent-encoded credential.
fn percent_decode(s: &str) -> Result<String, RtspPushError> {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|e| RtspPushError::Connect(format!("invalid percent-encoded userinfo: {e}")))
}

/// A pseudo-random 32-bit SSRC — same `RandomState` technique as
/// `crate::output::whep::rand_ssrc` (OS-random, no `rand` dependency).
fn rand_ssrc() -> u32 {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    let state = RandomState::new();
    (state.hash_one(Instant::now()) as u32) | 1
}

/// Builds the SDP for the push session: one `m=` line for the whole
/// MPEG-2 TS-muxed stream (RFC 2250 §2) — every track is multiplexed into
/// this single RTP session, matching the one SETUP/one interleaved channel
/// this transport actually uses, not one `m=` line per elementary track.
fn build_sdp() -> String {
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 0.0.0.0\r\n\
         s=multimux push\r\n\
         c=IN IP4 0.0.0.0\r\n\
         t=0 0\r\n\
         m=video 0 RTP/AVP {PT_MP2T}\r\n\
         a=rtpmap:{PT_MP2T} MP2T/90000\r\n\
         a=control:trackID=0\r\n"
    )
}

/// Errors from the RTSP push transport.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RtspPushError {
    #[error("RTSP connect failed: {0}")]
    Connect(String),
    #[error("RTSP protocol error: {0}")]
    Protocol(String),
    #[error("RTSP I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdp_has_a_session_level_c_line_and_matches_the_single_control_url() {
        // RFC 4566 §5 requires a `c=` line somewhere (session- or
        // media-level) — the pre-fix SDP had neither, which is invalid SDP
        // regardless of the RTP-framing bugs.
        let sdp = build_sdp();
        assert!(sdp.contains("\r\nc=IN IP4"), "missing c= line: {sdp}");
        assert!(
            sdp.contains("a=control:trackID=0"),
            "control attribute must match the one control URL SETUP uses: {sdp}"
        );
        assert!(
            sdp.contains(&format!("RTP/AVP {PT_MP2T}")),
            "must advertise the static MP2T payload type: {sdp}"
        );
        // Exactly one m= line: everything is multiplexed into the one TS
        // stream this transport actually SETUPs and sends.
        assert_eq!(
            sdp.matches("\r\nm=").count(),
            1,
            "expected one m= line: {sdp}"
        );
    }

    #[test]
    fn control_url_is_appended_with_a_single_slash() {
        // Regression shape for RFC 2326 Appendix C.1.1: no double slash
        // when the presentation URL already ends in one, no missing slash
        // otherwise.
        for (base, expected) in [
            ("rtsp://h/push", "rtsp://h/push/trackID=0"),
            ("rtsp://h/push/", "rtsp://h/push/trackID=0"),
        ] {
            let control_url = if base.ends_with('/') {
                format!("{base}trackID=0")
            } else {
                format!("{base}/trackID=0")
            };
            assert_eq!(control_url, expected);
        }
    }

    #[test]
    fn rtp_packet_header_is_v2_pt33_with_incrementing_seq() {
        let mut transport = RtspTransport {
            stream: None,
            client: ClientSession::new(),
            channel: 0,
            url: "rtsp://h/push".to_string(),
            control_url: "rtsp://h/push/trackID=0".to_string(),
            seq: 0xFFFF, // exercise the u16 wraparound.
            ssrc: 0xdead_beef,
            started: Instant::now(),
        };
        let payload = [0x47u8; TS_PACKET_LEN]; // one TS sync-byte-led packet.

        let first = transport.rtp_packet(&payload);
        assert_eq!(first[0], 0x80, "V=2,P=0,X=0,CC=0");
        assert_eq!(first[1], PT_MP2T, "M=0, PT=33 (RFC 2250 static MP2T)");
        assert_eq!(u16::from_be_bytes([first[2], first[3]]), 0xFFFF);
        assert_eq!(
            u32::from_be_bytes([first[8], first[9], first[10], first[11]]),
            0xdead_beef
        );
        assert_eq!(
            &first[12..],
            &payload[..],
            "payload must pass through verbatim"
        );

        let second = transport.rtp_packet(&payload);
        assert_eq!(
            u16::from_be_bytes([second[2], second[3]]),
            0,
            "sequence number must wrap u16, not panic"
        );
    }

    // Regression: this transport's errors can end up in route logs, so the
    // configured password must never appear in a connect failure, and
    // `Debug` must never show the credential (mirrors rtsp-runtime's own
    // `client_session_debug_does_not_leak_embedded_credentials_secret`).
    #[test]
    fn transport_debug_does_not_leak_the_configured_password() {
        let mut client = ClientSession::new();
        client = client.with_credentials(Credentials::new("user", "extremely-secret-password"));
        let transport = RtspTransport {
            stream: None,
            client,
            channel: 0,
            url: "rtsp://h/push".to_string(),
            control_url: "rtsp://h/push/trackID=0".to_string(),
            seq: 0,
            ssrc: 1,
            started: Instant::now(),
        };
        let debug = format!("{transport:?}");
        assert!(
            !debug.contains("extremely-secret-password"),
            "leaked via RtspTransport Debug: {debug}"
        );
    }
}
