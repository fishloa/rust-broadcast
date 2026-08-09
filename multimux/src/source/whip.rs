//! WHIP (RFC 9725) push ingest source (issue #740).
//!
//! Accepts an inbound WHIP publisher: an HTTP `POST` carrying an SDP offer
//! (`application/sdp`), answered with a `201 Created` carrying this side's
//! SDP answer, after which media flows over ICE + DTLS-SRTP —
//! [`webrtc_runtime::media::MediaTransport`] is the transport this module
//! drives; SDP itself (offer parsing, answer construction) is out of that
//! crate's scope (see its module doc) and is handled here, over
//! [`sdp_types::Session`] (the same SDP crate `crate::source::sdp` uses for
//! RTSP DESCRIBE bodies) for structure, plus a small amount of manual line
//! scanning for the WebRTC-specific attributes (`a=ice-ufrag`, `a=setup`,
//! `a=candidate`, …) neither `sdp_types` nor `crate::source::sdp` models.
//!
//! # Scope: video (H.264) only in this cut
//!
//! A WHIP publisher's audio track is essentially always Opus (RFC 7587) —
//! browsers do not send AAC over RTP. `transmux::RtpStreamDepacketiser` only
//! depayloads RFC 6184 (H.264) and RFC 3640 (AAC); there is no Opus RTP
//! depacketiser anywhere in this workspace, and adding one is out of scope
//! for this crate (`transmux` is not touched here). An offer containing no
//! `m=video` section — or more than one `m=` section at all — is rejected
//! with [`MultimuxError::Sdp`] rather than silently dropping the media it
//! can't carry. Follow-up: an RFC 7587 Opus depacketiser in `transmux`.
//!
//! # Why the `avcC` config is captured from the bitstream, not the SDP
//!
//! [`crate::source::sdp::parse_sdp_tracks`] (RTSP/raw-RTP-over-UDP) builds
//! `CodecConfig::Avc` from the SDP's `a=fmtp` `sprop-parameter-sets`
//! (RFC 6184 §8.2) — the RTSP-world convention. A browser's WHIP offer
//! generally omits it: WebRTC senders carry SPS/PPS **in-band** instead, as
//! a STAP-A aggregation packet ahead of each IDR (the same RFC 6184 §5.1,
//! just the other of its two documented ways to convey parameter sets).
//! Requiring `sprop-parameter-sets` here would reject a real browser's offer
//! outright. Instead, [`WhipIngestSession`] depayloads immediately (its
//! `avcC` config is never consulted by `RtpStreamDepacketiser::push` itself —
//! only carried through to the caller, per that type's own doc), and defers
//! announcing [`SessionEvent::NewProgram`] until it has scanned a real IDR's
//! length-prefixed NAL units for a genuine SPS (type 7) + PPS (type 8) pair,
//! from which [`transmux::avc_config_from_sps_pps`] builds the real
//! [`transmux::AVCConfigurationBox`] — mirrors `crate::source::rtmp`'s own
//! "`Established` gates on the first `Sample`" precedent (see that module's
//! doc), just gated on "first sample with real parameter sets" rather than
//! merely "first sample". Every sample observed before that gate fires is
//! buffered and re-emitted immediately after, exactly like RTMP's
//! `newly_seen_samples` — nothing is ever dropped waiting for it. This
//! crate's own engineering discipline (never fabricate a spec value) rules
//! out shipping a placeholder/empty `avcC` in the announced `TrackSpec`.
//!
//! # Listener shape
//!
//! Exactly like `crate::source::rtmp`: WHIP is an inbound publisher, so this
//! is a [`Listener`], not a `media_plane::ingress::Dialer`. The HTTP
//! POST/201 exchange is the "handshake"; `WhipRoute::ensure_infra` (private)
//! binds the listen socket once and spawns an accept-pump task, mirroring
//! `RtmpRoute::ensure_infra` almost exactly. Unlike RTMP, the accepted
//! "connection" the pump hands off is not a live socket still being read —
//! by the time an admitted session reaches the channel, the SDP exchange is
//! already complete (a real HTTP request/response, entirely orthogonal to
//! the media session that follows) and what is queued is the *result*: a
//! bound ephemeral [`UdpSocket`], the negotiated [`MediaTransport`], and the
//! offer's track set.
//!
//! [`WhipIngestSession::feed`]'s `Stage::In` is `&'a [u8]` — one *decrypted*
//! RTP packet's reconstructed wire bytes (this module's own private
//! `rebuild_rtp_wire`), the same shape `crate::source::rtp_udp` feeds its
//! depacketiser with — so, unlike RTMP (`Stage::In = &'a [ServerEvent]`),
//! [`run_whip`] can use [`ListenDriver::feed`]'s plain `&[u8]` convenience
//! wrapper rather than the `driver_mut`/`driver`/`reap_if_terminal` triple.
//! The ICE/DTLS/SRTP machinery that produces those bytes from a raw UDP
//! datagram lives entirely in [`run_whip`]'s own private per-session read
//! loop (`read_one`), never in the [`Stage`] impl itself — exactly the same
//! split RTMP draws between `RtmpConnection::next_events` (I/O) and
//! `RtmpIngestSession::feed` (sans-IO translation).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use broadcast_common::{Demand, Stage, Timestamp};
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Mutex as TokioMutex, OnceCell, mpsc};

use media_plane::ingress::{
    AcceptOutcome, HandshakePolicy, IngestSession, ListenDriver, Listener, ProgramId, SessionEvent,
    SessionId,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};

use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, RtpStreamDepacketiser,
    RtpStreamTrack, avc_config_from_sps_pps,
};

use webrtc_runtime::media::{
    Datagram, MediaEvent, MediaTransport, MediaTransportConfig, SetupRole,
};

use crate::error::{MultimuxError, Result};
use crate::route::RouteHandle;
use crate::source::{DriverProgress, IngestTimeouts, Source};

/// Unknown coded dimensions — the SDP/RTP path gives no frame geometry at
/// all, matching `crate::source::sdp`'s own `UNKNOWN_DIMENSION` placeholder
/// for `CodecConfig::Avc` (that field genuinely isn't derivable here either;
/// this is the same accepted convention, not a new fabrication).
const UNKNOWN_DIMENSION: u16 = 0;

/// H.264 NAL unit type field mask (ISO/IEC 14496-10 §7.3.1 `nal_unit_type`,
/// 5 bits).
const NAL_TYPE_MASK: u8 = 0x1F;
/// Sequence parameter set NAL unit type (ISO/IEC 14496-10 Table 7-1).
const NAL_TYPE_SPS: u8 = 7;
/// Picture parameter set NAL unit type (ISO/IEC 14496-10 Table 7-1).
const NAL_TYPE_PPS: u8 = 8;
/// AVCC-style NAL length-prefix width (`length_size_minus_one` = 3, i.e. a
/// 4-byte length) — matches `transmux::rtp::reassemble_video`'s own encoding
/// of the AUs [`RtpStreamDepacketiser::push`] emits.
const NAL_LENGTH_PREFIX: usize = 4;

/// RFC 3550 §5.1 fixed RTP header length (before any CSRC/extension) — used
/// only to read the payload-type byte before depayloading.
const RTP_MIN_HEADER_LEN: usize = 12;
/// Mask for the 7-bit payload-type field (RTP header byte 1, bit 7 is the
/// marker bit).
const RTP_PT_MASK: u8 = 0x7F;
/// Mask for the RTP version+padding+extension+CSRC-count byte (header byte
/// 0) this module preserves verbatim when rebuilding wire bytes from a
/// decrypted, already-parsed [`webrtc_runtime::media::DecryptedRtp`] — always
/// `2` (version) with no padding/extension/CSRC beyond what the CSRC list
/// itself already carries, per RFC 3550 §5.1.
const RTP_VERSION_BYTE: u8 = 0x80;

/// Default cap on concurrently admitted WHIP publishers per route, mirroring
/// [`crate::source::rtmp::DEFAULT_RTMP_MAX_SESSIONS`]'s own reasoning: generous
/// for one origin's worth of encoders while still bounding a flood of inbound
/// connections.
pub const DEFAULT_WHIP_MAX_SESSIONS: usize = 16;

/// How often [`run_whip`]'s driving loop polls [`ListenDriver::poll_accept`]
/// while no session has a read in flight to race it against — see
/// `crate::source::rtmp::ACCEPT_POLL_INTERVAL`'s identical reasoning.
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Bound on the accept-pump task's channel — see
/// `crate::source::rtmp::ACCEPT_QUEUE_CAPACITY`'s identical reasoning.
const ACCEPT_QUEUE_CAPACITY: usize = 32;

/// Max UDP datagram this source reads in one `recv` — matches
/// `crate::source::rtp_udp::MAX_UDP_DATAGRAM`.
const MAX_UDP_DATAGRAM: usize = 65_536;

/// One negotiated WHIP session, handed from the HTTP accept-pump to
/// [`WhipListener::poll_accept`]: the SDP exchange is already complete by
/// the time this exists (see the module doc) — what remains is the media
/// session itself.
struct AdmittedWhip {
    socket: Arc<UdpSocket>,
    media: Arc<TokioMutex<MediaTransport>>,
    tracks: Vec<WhipTrack>,
}

/// One `m=video` track resolved from a WHIP offer.
#[derive(Clone)]
struct WhipTrack {
    track_id: u32,
    payload_type: u8,
    clock_rate: u32,
}

/// Bind-once, reuse-forever infrastructure — see [`WhipRoute::ensure_infra`]
/// and `crate::source::rtmp::RtmpInfra`'s identical shape.
struct WhipInfra {
    accept_rx: Arc<StdMutex<mpsc::Receiver<AdmittedWhip>>>,
}

/// A WHIP push-ingest route: binds an HTTP listen socket once and accepts
/// publishers against it. See the module doc.
pub struct WhipRoute {
    name: String,
    listen: String,
    timeouts: IngestTimeouts,
    max_sessions: usize,
    infra: OnceCell<WhipInfra>,
}

impl std::fmt::Debug for WhipRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhipRoute")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("max_sessions", &self.max_sessions)
            .finish()
    }
}

impl WhipRoute {
    /// Build a route whose WHIP publish endpoint listens on `listen` (e.g.
    /// `"0.0.0.0:8080"`, or `"127.0.0.1:0"` for an ephemeral test port). A
    /// publisher `POST`s its SDP offer to `http://<listen>/` (any path is
    /// accepted — this is a single-route listener, not a multi-tenant path
    /// router).
    pub fn new(name: impl Into<String>, listen: impl Into<String>) -> Self {
        WhipRoute {
            name: name.into(),
            listen: listen.into(),
            timeouts: IngestTimeouts::default(),
            max_sessions: DEFAULT_WHIP_MAX_SESSIONS,
            infra: OnceCell::new(),
        }
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Overrides [`DEFAULT_WHIP_MAX_SESSIONS`].
    #[must_use]
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = max_sessions;
        self
    }

    /// Binds the HTTP listen socket and spawns the accept-pump task on the
    /// first call only — see `crate::source::rtmp::RtmpRoute::ensure_infra`'s
    /// identical "bind once" reasoning.
    async fn ensure_infra(&self) -> Result<Arc<StdMutex<mpsc::Receiver<AdmittedWhip>>>> {
        let infra =
            self.infra
                .get_or_try_init(|| async {
                    let listener = TcpListener::bind(&self.listen).await.map_err(|e| {
                        MultimuxError::Connect {
                            reason: format!("whip: bind {}: {e}", self.listen),
                        }
                    })?;
                    let (tx, rx) = mpsc::channel(ACCEPT_QUEUE_CAPACITY);
                    tokio::spawn(async move {
                        loop {
                            match listener.accept().await {
                                Ok((stream, _peer)) => {
                                    let tx = tx.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = handle_whip_connection(stream, &tx).await {
                                            tracing::warn!(
                                                error = %e,
                                                "whip: signalling connection failed"
                                            );
                                        }
                                    });
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "whip: accept-pump ending after a listen-socket error"
                                    );
                                    break;
                                }
                            }
                        }
                    });
                    Ok::<WhipInfra, MultimuxError>(WhipInfra {
                        accept_rx: Arc::new(StdMutex::new(rx)),
                    })
                })
                .await?;
        Ok(Arc::clone(&infra.accept_rx))
    }
}

impl Source for WhipRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// Reads one HTTP/1.1 request (request line + `Content-Length` body) off
/// `stream` — deliberately minimal (no chunked transfer-encoding, no
/// persistent-connection reuse): a WHIP client's POST is a single
/// request/response, and this listener serves exactly one route.
async fn read_http_request(stream: &mut TcpStream) -> std::io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok((String::new(), Vec::new()));
        }
        buf.extend_from_slice(&tmp[..n]);
        let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buf[..pos]).to_string();
        let mut lines = head.lines();
        let request_line = lines.next().unwrap_or_default().to_string();
        let content_length: usize = lines
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|v| v.trim().parse().ok())
            })
            .unwrap_or(0);
        let body_start = pos + 4;
        while buf.len() < body_start + content_length {
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        let body_end = (body_start + content_length).min(buf.len());
        return Ok((request_line, buf[body_start..body_end].to_vec()));
    }
}

/// One parsed WHIP SDP offer: session-level ICE credentials + candidates
/// (taken from wherever in the SDP text they appear — a bundled offer
/// signals `a=ice-ufrag`/`a=ice-pwd` identically on every `m=` section, and
/// this cut requires exactly one anyway), plus the single `m=video` track.
struct ParsedOffer {
    remote_ufrag: String,
    remote_pwd: String,
    mid: String,
    candidates: Vec<String>,
    payload_type: u8,
    clock_rate: u32,
    /// The `m=video …` line and every `a=rtpmap`/`a=fmtp`/`a=rtcp-fb` line
    /// naming `payload_type`, verbatim — echoed into the answer so the
    /// negotiated codec parameters match the offer exactly (RFC 3264 §6.1:
    /// an answer's media format list is a subset of the offer's).
    m_line: String,
    codec_lines: Vec<String>,
}

/// Parses a WHIP offer for its single supported case: exactly one `m=video`
/// section, H.264 payload types only (matched by SDP encoding name, not
/// assumed) — see the module doc's "Scope" section for why audio (Opus) and
/// multi-track offers are rejected rather than silently dropped.
fn parse_whip_offer(offer: &str) -> Result<ParsedOffer> {
    let session = sdp_types::Session::parse(offer.as_bytes()).map_err(|e| MultimuxError::Sdp {
        reason: format!("whip: parse offer: {e}"),
    })?;
    let video_medias: Vec<_> = session
        .medias
        .iter()
        .filter(|m| m.media == "video")
        .collect();
    if session.medias.len() != video_medias.len() || video_medias.len() != 1 {
        return Err(MultimuxError::Sdp {
            reason: format!(
                "whip: this route accepts exactly one m=video section and nothing else \
                 (Opus audio has no RTP depacketiser in this workspace yet); offer had {} \
                 total section(s), {} of them video",
                session.medias.len(),
                video_medias.len()
            ),
        });
    }
    let media = video_medias[0];

    let remote_ufrag =
        sdp_attr_anywhere(offer, "a=ice-ufrag:").ok_or_else(|| MultimuxError::Sdp {
            reason: "whip: offer has no a=ice-ufrag".into(),
        })?;
    let remote_pwd = sdp_attr_anywhere(offer, "a=ice-pwd:").ok_or_else(|| MultimuxError::Sdp {
        reason: "whip: offer has no a=ice-pwd".into(),
    })?;
    let mid = media
        .get_first_attribute_value("mid")
        .ok()
        .flatten()
        .unwrap_or("0")
        .to_string();
    let candidates: Vec<String> = offer
        .lines()
        .filter_map(|l| l.strip_prefix("a=candidate:"))
        .map(str::to_string)
        .collect();

    // Pick the first payload type in `m=video`'s `fmt` list whose
    // `a=rtpmap` names H.264 — skips a paired `rtx` payload type (RFC 4588)
    // offered alongside it, which this route neither requests nor handles.
    let mut chosen: Option<(u8, u32)> = None;
    for tok in media.fmt.split_whitespace() {
        let Ok(pt) = tok.parse::<u8>() else { continue };
        let rtpmap = offer
            .lines()
            .find(|l| l.starts_with(&format!("a=rtpmap:{pt} ")));
        let Some(rtpmap) = rtpmap else { continue };
        if !rtpmap.to_ascii_uppercase().contains("H264") {
            continue;
        }
        let clock_rate =
            transmux::rtpmap_clock_rate(rtpmap.strip_prefix("a=rtpmap:").unwrap_or(rtpmap))
                .unwrap_or(90_000);
        chosen = Some((pt, clock_rate));
        break;
    }
    let (payload_type, clock_rate) = chosen.ok_or_else(|| MultimuxError::Sdp {
        reason: "whip: m=video has no H.264 (a=rtpmap naming H264) payload type".into(),
    })?;

    let codec_lines: Vec<String> = offer
        .lines()
        .filter(|l| {
            let matches_pt = |prefix: &str| {
                l.strip_prefix(prefix)
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|pt| pt.parse::<u8>().ok())
                    == Some(payload_type)
            };
            matches_pt("a=rtpmap:") || matches_pt("a=fmtp:") || matches_pt("a=rtcp-fb:")
        })
        .map(str::to_string)
        .collect();

    let m_line = offer
        .lines()
        .find(|l| l.starts_with("m=video"))
        .unwrap_or("m=video 9 UDP/TLS/RTP/SAVPF")
        .to_string();

    Ok(ParsedOffer {
        remote_ufrag,
        remote_pwd,
        mid,
        candidates,
        payload_type,
        clock_rate,
        m_line,
        codec_lines,
    })
}

/// Finds the first `a=<prefix>` line's value anywhere in `sdp` — WHIP
/// offers signal ICE credentials once (session-level, or identically on
/// every bundled `m=` section), so unlike [`crate::source::sdp`]'s
/// per-media attribute lookups, a flat text scan is both sufficient and
/// simpler; mirrors `webrtc-runtime`'s own `whip_media_smoke` example.
fn sdp_attr_anywhere(sdp: &str, prefix: &str) -> Option<String> {
    sdp.lines()
        .find_map(|l| l.strip_prefix(prefix))
        .map(|v| v.trim().to_string())
}

/// Builds this side's SDP answer: [`SetupRole::Passive`] (this route is
/// always the DTLS server), `a=recvonly` (a WHIP publish endpoint never
/// sends media back), `a=rtcp-mux` (required — [`MediaTransport`] demuxes
/// RTP/RTCP on one 5-tuple per RFC 5761 §4, never a separate RTCP port).
fn build_answer(
    offer: &ParsedOffer,
    media: &MediaTransport,
    local_addr: std::net::SocketAddr,
    local_ice_ufrag: &str,
    local_ice_pwd: &str,
) -> String {
    let candidate_line = format!(
        "0 1 udp 2130706431 {} {} typ host",
        local_addr.ip(),
        local_addr.port()
    );
    let mut answer = String::new();
    answer.push_str("v=0\r\n");
    answer.push_str("o=- 0 0 IN IP4 127.0.0.1\r\n");
    answer.push_str("s=-\r\n");
    answer.push_str("t=0 0\r\n");
    answer.push_str(&format!("{}\r\n", offer.m_line));
    answer.push_str("c=IN IP4 127.0.0.1\r\n");
    answer.push_str("a=rtcp:9 IN IP4 0.0.0.0\r\n");
    for l in &offer.codec_lines {
        answer.push_str(l);
        answer.push_str("\r\n");
    }
    answer.push_str("a=recvonly\r\n");
    answer.push_str(&format!("a=mid:{}\r\n", offer.mid));
    answer.push_str("a=rtcp-mux\r\n");
    answer.push_str(&format!("a=ice-ufrag:{local_ice_ufrag}\r\n"));
    answer.push_str(&format!("a=ice-pwd:{local_ice_pwd}\r\n"));
    answer.push_str(&format!(
        "a=fingerprint:sha-256 {}\r\n",
        media.local_fingerprint()
    ));
    answer.push_str("a=setup:passive\r\n");
    answer.push_str(&format!("a=candidate:{candidate_line}\r\n"));
    answer.push_str("a=end-of-candidates\r\n");
    answer
}

/// Handles one accepted TCP connection end-to-end: read the POST, parse and
/// answer the offer, bind this session's own ephemeral media socket, build
/// the [`MediaTransport`], write the `201 Created` response, and hand the
/// negotiated session off to `tx`. An `OPTIONS` preflight (browsers send one
/// ahead of a cross-origin `POST` with a non-simple `Content-Type`) gets a
/// permissive CORS response and no session.
async fn handle_whip_connection(
    mut stream: TcpStream,
    tx: &mpsc::Sender<AdmittedWhip>,
) -> Result<()> {
    let (request_line, body) =
        read_http_request(&mut stream)
            .await
            .map_err(|e| MultimuxError::Connect {
                reason: format!("whip: read request: {e}"),
            })?;
    if request_line.starts_with("OPTIONS") {
        let resp = "HTTP/1.1 204 No Content\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Access-Control-Allow-Methods: POST, OPTIONS\r\n\
             Access-Control-Allow-Headers: Content-Type\r\n\
             Content-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes()).await;
        return Ok(());
    }
    if !request_line.starts_with("POST") {
        let resp = "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes()).await;
        return Ok(());
    }

    let offer_sdp = String::from_utf8_lossy(&body).into_owned();
    let parsed = parse_whip_offer(&offer_sdp)?;

    // The IP to advertise as this session's ICE host candidate: not
    // `0.0.0.0` (unreachable — that would be the address on the wire if the
    // media socket bound the wildcard address and this just echoed it back),
    // but the local side of the TCP connection the client used to reach this
    // *signalling* endpoint — a real, client-reachable address by
    // construction (whatever routing/NAT let this very connection through
    // already applies to it). No STUN/TURN reflexive/relay candidate is
    // gathered here (`MediaTransportConfig::stun_server: None` below), so a
    // publisher behind NAT from this route's listener is out of scope for
    // this cut, same as `webrtc_runtime`'s own `whip_media_smoke` example.
    let advertise_ip = stream
        .local_addr()
        .map(|a| a.ip())
        .map_err(|e| MultimuxError::Connect {
            reason: format!("whip: signalling connection local_addr: {e}"),
        })?;
    let socket = UdpSocket::bind((advertise_ip, 0))
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("whip: bind media socket on {advertise_ip}: {e}"),
        })?;
    let local_addr = socket.local_addr().map_err(|e| MultimuxError::Connect {
        reason: format!("whip: media socket local_addr: {e}"),
    })?;

    let local_ice_ufrag = rand_token(8);
    let local_ice_pwd = rand_token(24);
    let mut media = MediaTransport::new(MediaTransportConfig {
        local_addr,
        local_ice_ufrag: local_ice_ufrag.clone(),
        local_ice_pwd: local_ice_pwd.clone(),
        remote_ice_ufrag: parsed.remote_ufrag.clone(),
        remote_ice_pwd: parsed.remote_pwd.clone(),
        is_controlling: false,
        local_setup: SetupRole::Passive,
        stun_server: None,
    })
    .map_err(|e| MultimuxError::Connect {
        reason: format!("whip: build media transport: {e}"),
    })?;

    for raw in &parsed.candidates {
        // A candidate this side can't parse is skipped, not fatal — mirrors
        // `whip_media_smoke`'s own handling; ICE connectivity checks simply
        // never nominate a pair for it.
        let _ = media.add_remote_candidate(raw);
    }

    let answer = build_answer(
        &parsed,
        &media,
        local_addr,
        &local_ice_ufrag,
        &local_ice_pwd,
    );
    let resp = format!(
        "HTTP/1.1 201 Created\r\n\
         Content-Type: application/sdp\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Expose-Headers: Location\r\n\
         Location: /whip/session\r\n\
         Content-Length: {}\r\n\r\n{}",
        answer.len(),
        answer
    );
    stream
        .write_all(resp.as_bytes())
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("whip: write answer: {e}"),
        })?;
    let _ = stream.shutdown().await;

    let admitted = AdmittedWhip {
        socket: Arc::new(socket),
        media: Arc::new(TokioMutex::new(media)),
        tracks: vec![WhipTrack {
            track_id: 1,
            payload_type: parsed.payload_type,
            clock_rate: parsed.clock_rate,
        }],
    };
    let _ = tx.send(admitted).await;
    Ok(())
}

/// A short pseudo-random token for ICE ufrag/pwd — see
/// `webrtc_runtime`'s `whip_media_smoke` example for the identical
/// technique (OS-random `RandomState`, no `rand` dependency for one call
/// site).
fn rand_token(len: usize) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let state = RandomState::new();
    (0..len)
        .map(|i| {
            let idx = (state.hash_one(i) as usize) % CHARS.len();
            CHARS[idx] as char
        })
        .collect()
}

/// The non-blocking [`Listener`] bridge over the accept-pump's channel — see
/// the module doc and `crate::source::rtmp::RtmpListener`'s identical shape.
struct WhipListener {
    accept_rx: Arc<StdMutex<mpsc::Receiver<AdmittedWhip>>>,
    max_sessions: usize,
}

impl Listener for WhipListener {
    type Session = WhipIngestSession;
    type Error = MultimuxError;

    fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    fn poll_accept(&mut self) -> Result<Option<WhipIngestSession>> {
        let mut rx = self
            .accept_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match rx.try_recv() {
            Ok(admitted) => Ok(Some(WhipIngestSession::new(admitted))),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(MultimuxError::Connect {
                reason: "whip: accept-pump task ended (listen socket failure)".into(),
            }),
        }
    }
}

/// This track's captured real decoder config, once seen — `None` until a
/// real SPS+PPS pair has been scanned out of an actual IDR (see the module
/// doc's "Why the `avcC` config is captured from the bitstream" section).
type CapturedConfig = Option<AVCConfigurationBox>;

/// An admitted WHIP publisher: the depacketiser plus payload-type routing
/// (identical shape to `crate::source::rtp_udp::RtpUdpIngestSession`), plus
/// the deferred-`avcC`-capture state this route needs because the SDP offer
/// alone doesn't carry real parameter sets (see the module doc). Also holds
/// the shared handles [`run_whip`]'s own read loop needs — the socket and
/// the sans-IO [`MediaTransport`] — mirroring
/// `crate::source::rtmp::RtmpIngestSession::conn_handle`.
pub struct WhipIngestSession {
    socket: Arc<UdpSocket>,
    media: Arc<TokioMutex<MediaTransport>>,
    depacketiser: RtpStreamDepacketiser,
    pt_to_track: HashMap<u8, u32>,
    clock_rate_by_track: HashMap<u32, u32>,
    captured: HashMap<u32, CapturedConfig>,
    /// Samples observed before every track's config was captured — replayed
    /// immediately once the establishment gate fires. See the module doc.
    buffered: Vec<(u32, Sample)>,
    announced: bool,
    pending: std::collections::VecDeque<SessionEvent>,
}

impl WhipIngestSession {
    fn new(admitted: AdmittedWhip) -> Self {
        let mut pt_to_track = HashMap::new();
        let mut clock_rate_by_track = HashMap::new();
        let mut captured = HashMap::new();
        let mut tracks = Vec::new();
        for t in &admitted.tracks {
            pt_to_track.insert(t.payload_type, t.track_id);
            clock_rate_by_track.insert(t.track_id, t.clock_rate);
            captured.insert(t.track_id, None);
            // The placeholder handed to `RtpStreamTrack::new` is never
            // observed externally: `RtpStreamDepacketiser::push` never
            // consults a track's `config` (only carries it through to
            // `track_specs()`, which this module deliberately never calls —
            // see the module doc), and no `TrackSpec` is announced until
            // `captured` holds every track's real config.
            let placeholder = CodecConfig::Avc {
                config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
                    configuration_version: 1,
                    profile_indication: 0,
                    profile_compatibility: 0,
                    level_indication: 0,
                    length_size_minus_one: 3,
                    sps: Vec::new(),
                    pps: Vec::new(),
                    chroma_format: None,
                    bit_depth_luma_minus8: None,
                    bit_depth_chroma_minus8: None,
                    sps_ext: Vec::new(),
                }),
                width: UNKNOWN_DIMENSION,
                height: UNKNOWN_DIMENSION,
            };
            tracks.push(RtpStreamTrack::new(
                t.track_id,
                transmux::rtp::RtpMediaKind::H264,
                placeholder,
                t.clock_rate,
            ));
        }
        WhipIngestSession {
            socket: admitted.socket,
            media: admitted.media,
            depacketiser: RtpStreamDepacketiser::new(tracks),
            pt_to_track,
            clock_rate_by_track,
            captured,
            buffered: Vec::new(),
            announced: false,
            pending: std::collections::VecDeque::new(),
        }
    }

    /// A cheap `Arc` clone of the media socket — [`run_whip`]'s own
    /// per-session read task drives it concurrently with every other
    /// session's, entirely outside this sans-IO [`Stage`] impl.
    fn socket_handle(&self) -> Arc<UdpSocket> {
        Arc::clone(&self.socket)
    }

    /// A cheap `Arc` clone of the [`MediaTransport`] — see
    /// [`Self::socket_handle`].
    fn media_handle(&self) -> Arc<TokioMutex<MediaTransport>> {
        Arc::clone(&self.media)
    }

    /// Scans `sample`'s length-prefixed NAL data (see [`NAL_LENGTH_PREFIX`])
    /// for a real SPS/PPS pair and, if found, captures the real `avcC` for
    /// `track_id` — see the module doc.
    fn try_capture_config(&mut self, track_id: u32, sample: &Sample) {
        if self.captured.get(&track_id).cloned().flatten().is_some() {
            return;
        }
        let mut sps = Vec::new();
        let mut pps = Vec::new();
        let data = sample.data.as_ref();
        let mut off = 0usize;
        while off + NAL_LENGTH_PREFIX <= data.len() {
            let len = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
                as usize;
            off += NAL_LENGTH_PREFIX;
            if off + len > data.len() {
                break;
            }
            let nal = &data[off..off + len];
            off += len;
            if nal.is_empty() {
                continue;
            }
            match nal[0] & NAL_TYPE_MASK {
                NAL_TYPE_SPS => sps.push(AvcSps(nal.to_vec())),
                NAL_TYPE_PPS => pps.push(AvcPps(nal.to_vec())),
                _ => {}
            }
        }
        if sps.is_empty() || pps.is_empty() {
            return;
        }
        if let Ok(config) = avc_config_from_sps_pps(sps, pps) {
            self.captured.insert(track_id, Some(config));
        }
    }

    /// True once every track this session was constructed with has a
    /// captured real config.
    fn all_captured(&self) -> bool {
        self.captured.values().all(Option::is_some)
    }

    /// Fires the establishment gate: builds real [`TrackSpec`]s from the
    /// captured configs, announces `NewProgram`/`Established`, then replays
    /// every buffered sample — see the module doc.
    fn announce(&mut self) {
        self.announced = true;
        let mut track_ids: Vec<u32> = self.clock_rate_by_track.keys().copied().collect();
        track_ids.sort_unstable();
        let specs: Vec<TrackSpec> = track_ids
            .iter()
            .filter_map(|id| {
                let config = self.captured.get(id)?.clone()?;
                let clock_rate = *self.clock_rate_by_track.get(id)?;
                Some(TrackSpec::new(
                    *id,
                    clock_rate,
                    CodecConfig::Avc {
                        config,
                        width: UNKNOWN_DIMENSION,
                        height: UNKNOWN_DIMENSION,
                    },
                ))
            })
            .collect();
        self.pending.push_back(SessionEvent::NewProgram {
            program: ProgramId(0),
            tracks: specs,
        });
        self.pending.push_back(SessionEvent::Established);
        for (track_id, sample) in std::mem::take(&mut self.buffered) {
            self.pending.push_back(SessionEvent::Sample {
                program: ProgramId(0),
                track_id,
                retention: RetentionClass::Timed,
                sample,
            });
        }
    }

    fn observe_sample(&mut self, track_id: u32, sample: Sample) {
        if !self.announced {
            self.try_capture_config(track_id, &sample);
            self.buffered.push((track_id, sample));
            if self.all_captured() {
                self.announce();
            }
            return;
        }
        self.pending.push_back(SessionEvent::Sample {
            program: ProgramId(0),
            track_id,
            retention: RetentionClass::Timed,
            sample,
        });
    }
}

impl Stage for WhipIngestSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = MultimuxError;

    /// `input` is one already-decrypted RTP packet's reconstructed wire
    /// bytes (see this module's private `rebuild_rtp_wire`) — [`run_whip`]'s
    /// read loop performs the ICE/DTLS/SRTP decrypt before ever calling
    /// this.
    fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<()> {
        let Some(track_id) =
            payload_type_of(input).and_then(|pt| self.pt_to_track.get(&pt).copied())
        else {
            return Ok(());
        };
        let samples =
            self.depacketiser
                .push(track_id, input)
                .map_err(|e| MultimuxError::Depay {
                    reason: e.to_string(),
                })?;
        for sample in samples {
            self.observe_sample(track_id, sample);
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(MAX_UDP_DATAGRAM)
    }
}

impl IngestSession for WhipIngestSession {
    /// Uninhabited: every outbound datagram this session needs sent
    /// (STUN/DTLS/SRTP) is written directly by [`run_whip`]'s read loop via
    /// [`MediaTransport::poll_transmit`] — never through this trait, exactly
    /// like `crate::source::rtmp::RtmpIngestSession`'s identical rationale.
    type Request = std::convert::Infallible;
}

/// Extracts the RTP payload-type field (RFC 3550 §5.1, header byte 1 bits
/// `[6:0]`) — see `crate::source::rtp_udp::payload_type_of`'s identical
/// helper.
fn payload_type_of(packet: &[u8]) -> Option<u8> {
    if packet.len() < RTP_MIN_HEADER_LEN {
        return None;
    }
    Some(packet[1] & RTP_PT_MASK)
}

/// Rebuilds a wire-format RTP packet (RFC 3550 §5.1 fixed header, CSRC list,
/// and payload, concatenated) from a [`webrtc_runtime::media::DecryptedRtp`]
/// — the type [`MediaTransport`] hands back has already promoted the header
/// fields to typed values and decrypted the payload, but
/// `RtpStreamDepacketiser::push` (like every RTP depacketiser in this
/// workspace) takes the raw wire packet, so this reconstructs it rather than
/// growing a second, parsed-header entry point on that shared type just for
/// this one caller. No extension header: [`MediaTransport`] never reports
/// one having been present (its `DecryptedRtp` has no such field), so the
/// extension bit is correctly always clear here.
fn rebuild_rtp_wire(pkt: &webrtc_runtime::media::DecryptedRtp) -> Vec<u8> {
    let csrc_count = pkt.csrc.len().min(0x0F) as u8;
    let mut out = Vec::with_capacity(12 + 4 * csrc_count as usize + pkt.payload.len());
    out.push(RTP_VERSION_BYTE | csrc_count);
    out.push(if pkt.marker {
        0x80 | (pkt.payload_type & RTP_PT_MASK)
    } else {
        pkt.payload_type & RTP_PT_MASK
    });
    out.extend_from_slice(&pkt.sequence_number.to_be_bytes());
    out.extend_from_slice(&pkt.timestamp.to_be_bytes());
    out.extend_from_slice(&pkt.ssrc.to_be_bytes());
    for csrc in pkt.csrc.iter().take(csrc_count as usize) {
        out.extend_from_slice(&csrc.to_be_bytes());
    }
    out.extend_from_slice(&pkt.payload);
    out
}

/// What one [`read_one`] call observed — mirrors
/// `crate::source::rtmp::ReadOutcome`'s shape.
enum ReadOutcome {
    /// Zero or more decrypted RTP packets, each ready to
    /// [`ListenDriver::feed`] in order.
    Events(Vec<Vec<u8>>),
    /// No datagram within [`IngestTimeouts::read`] — treated as ending this
    /// session, same as `crate::source::rtmp`'s own "timeouts still bound
    /// reads" policy.
    TimedOut,
    /// The underlying socket read failed.
    TransportError(String),
}

type BoxedRead = Pin<Box<dyn Future<Output = (SessionId, ReadOutcome)> + Send>>;

/// Awaits the next UDP datagram for session `id`, bounded by `read_timeout`:
/// decrypts it through `media` (ICE/DTLS/SRTP), drains and sends every
/// outbound datagram [`MediaTransport::poll_transmit`] now wants sent, and
/// reconstructs wire bytes for every [`MediaEvent::Rtp`] observed. Boxed so
/// [`run_whip`] can hold many of these, one per admitted session, in one
/// [`FuturesUnordered`] — mirrors `crate::source::rtmp::read_one`.
fn read_one(
    id: SessionId,
    socket: Arc<UdpSocket>,
    media: Arc<TokioMutex<MediaTransport>>,
    read_timeout: Duration,
) -> BoxedRead {
    Box::pin(async move {
        let mut buf = [0u8; MAX_UDP_DATAGRAM];
        let outcome = match tokio::time::timeout(read_timeout, socket.recv_from(&mut buf)).await {
            Ok(Ok((n, peer))) => {
                let mut guard = media.lock().await;
                match guard.handle_datagram(Instant::now(), peer, &buf[..n]) {
                    Ok(events) => {
                        while let Some(Datagram { peer, bytes }) = guard.poll_transmit() {
                            let _ = socket.send_to(&bytes, peer).await;
                        }
                        let wire: Vec<Vec<u8>> = events
                            .into_iter()
                            .filter_map(|e| match e {
                                MediaEvent::Rtp(pkt) => Some(rebuild_rtp_wire(&pkt)),
                                _ => None,
                            })
                            .collect();
                        ReadOutcome::Events(wire)
                    }
                    Err(e) => ReadOutcome::TransportError(e.to_string()),
                }
            }
            Ok(Err(e)) => ReadOutcome::TransportError(e.to_string()),
            Err(_) => {
                // A read timeout still lets the transport act on the passage
                // of time (ICE retransmits, etc.) before this session ends.
                let mut guard = media.lock().await;
                guard.handle_timeout(Instant::now());
                while let Some(Datagram { peer, bytes }) = guard.poll_transmit() {
                    let _ = socket.send_to(&bytes, peer).await;
                }
                ReadOutcome::TimedOut
            }
        };
        (id, outcome)
    })
}

/// Per-session [`DriverProgress`] bookkeeping — mirrors
/// `crate::source::rtmp::ProgressBySession`.
type ProgressBySession = HashMap<SessionId, DriverProgress>;

/// Publishes session `id`'s progress and, if that left it terminal, reaps
/// it — mirrors `crate::source::rtmp::report_and_maybe_reap` exactly
/// (`WhipIngestSession::Stage::In` being `&[u8]` doesn't change this half:
/// [`ListenDriver::feed`] itself already does the reap-on-terminal step for
/// the actual feed call in [`run_whip`]'s loop, but the accept/timeout paths
/// still need the same driver/reap access this helper wraps).
fn report_and_maybe_reap(
    driver: &mut ListenDriver<WhipListener>,
    id: SessionId,
    route_handle: &Arc<RouteHandle>,
    progress: &mut ProgressBySession,
) -> bool {
    if let Some(d) = driver.driver(id) {
        crate::source::advance_route(d, route_handle, progress.entry(id).or_default());
    }
    let reaped = driver.reap_if_terminal(id).is_some();
    if reaped {
        progress.remove(&id);
    }
    reaped
}

/// Binds `route` (once ever), then admits and drives up to
/// [`Listener::max_sessions`] WHIP publishers **concurrently** until a
/// listen-socket-level failure occurs — mirrors
/// `crate::source::rtmp::run_rtmp`'s identical shape and "never returns in
/// ordinary operation" contract.
pub async fn run_whip(
    route: &WhipRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &Arc<RouteHandle>,
) -> MultimuxError {
    let accept_rx = match route.ensure_infra().await {
        Ok(rx) => rx,
        Err(e) => return e,
    };
    let listener = WhipListener {
        accept_rx,
        max_sessions: route.max_sessions,
    };
    let mut driver: ListenDriver<WhipListener> = ListenDriver::new(
        listener,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let start = Instant::now();
    let read_timeout = route.timeouts.read;

    let mut progress: ProgressBySession = HashMap::new();
    let mut reads: FuturesUnordered<BoxedRead> = FuturesUnordered::new();

    loop {
        tokio::select! {
            () = tokio::time::sleep(ACCEPT_POLL_INTERVAL) => {
                loop {
                    match driver.poll_accept() {
                        AcceptOutcome::Idle => break,
                        AcceptOutcome::Refused => {
                            tracing::warn!("whip: connection refused, max_sessions reached");
                        }
                        AcceptOutcome::Error(e) => return e,
                        AcceptOutcome::Admitted(id) => {
                            let session = driver
                                .driver(id)
                                .expect("just admitted by poll_accept")
                                .session();
                            let socket = session.socket_handle();
                            let media = session.media_handle();
                            progress.insert(id, DriverProgress::new());
                            reads.push(read_one(id, socket, media, read_timeout));
                        }
                        _ => break,
                    }
                }
            }
            Some((id, outcome)) = reads.next(), if !reads.is_empty() => {
                let now = Timestamp::from_instant(start, Instant::now());
                match outcome {
                    ReadOutcome::Events(events) => {
                        for wire in &events {
                            driver.feed(id, wire, now);
                        }
                        let reaped =
                            report_and_maybe_reap(&mut driver, id, route_handle, &mut progress);
                        if !reaped
                            && let Some(d) = driver.driver(id)
                        {
                            let socket = d.session().socket_handle();
                            let media = d.session().media_handle();
                            reads.push(read_one(id, socket, media, read_timeout));
                        }
                    }
                    ReadOutcome::TimedOut => {
                        tracing::warn!("whip: session idle past read timeout");
                        if let Some(d) = driver.driver_mut(id) {
                            d.finish();
                        }
                        report_and_maybe_reap(&mut driver, id, route_handle, &mut progress);
                    }
                    ReadOutcome::TransportError(reason) => {
                        tracing::warn!(error = %reason, "whip: session read failed");
                        if let Some(d) = driver.driver_mut(id) {
                            d.finish();
                        }
                        report_and_maybe_reap(&mut driver, id, route_handle, &mut progress);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=candidate:1 1 udp 2130706431 10.0.0.5 54321 typ host\r\n";

    #[test]
    fn parses_video_only_offer() {
        let parsed = parse_whip_offer(OFFER).expect("parse");
        assert_eq!(parsed.payload_type, 96);
        assert_eq!(parsed.clock_rate, 90_000);
        assert_eq!(parsed.remote_ufrag, "abcd");
        assert_eq!(parsed.remote_pwd, "abcdefghijklmnopqrstuvwx");
        assert_eq!(parsed.mid, "0");
        assert_eq!(parsed.candidates.len(), 1);
    }

    #[test]
    fn rejects_offer_with_no_video() {
        let offer = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=ice-ufrag:x\r\na=ice-pwd:xxxxxxxxxxxxxxxxxxxxxxxx\r\n";
        assert!(parse_whip_offer(offer).is_err());
    }

    #[test]
    fn rejects_offer_with_audio_and_video() {
        // Adds an `m=audio` section ahead of the existing `m=video` one;
        // minimally well-formed is enough — the point under test is the
        // section-count rejection, not full audio semantics.
        let offer = OFFER.replace("m=video 9", "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nm=video 9");
        assert!(parse_whip_offer(&offer).is_err());
    }

    #[test]
    fn payload_type_of_extracts_pt() {
        let mut pkt = vec![0x80u8, 96];
        pkt.extend_from_slice(&[0u8; 10]);
        assert_eq!(payload_type_of(&pkt), Some(96));
        assert_eq!(payload_type_of(&[0x80]), None);
    }

    #[test]
    fn rebuild_rtp_wire_round_trips_header_fields() {
        let pkt = webrtc_runtime::media::DecryptedRtp {
            marker: true,
            payload_type: 96,
            sequence_number: 1234,
            timestamp: 90_000,
            ssrc: 0xDEAD_BEEF,
            csrc: vec![0x1111_2222],
            payload: vec![0xAA, 0xBB, 0xCC],
        };
        let wire = rebuild_rtp_wire(&pkt);
        // Version=2, one CSRC.
        assert_eq!(wire[0], 0x80 | 1);
        assert_eq!(wire[1], 0x80 | 96, "marker bit set, PT 96");
        assert_eq!(u16::from_be_bytes([wire[2], wire[3]]), 1234);
        assert_eq!(
            u32::from_be_bytes([wire[4], wire[5], wire[6], wire[7]]),
            90_000
        );
        assert_eq!(
            u32::from_be_bytes([wire[8], wire[9], wire[10], wire[11]]),
            0xDEAD_BEEF
        );
        assert_eq!(
            u32::from_be_bytes([wire[12], wire[13], wire[14], wire[15]]),
            0x1111_2222
        );
        assert_eq!(&wire[16..], &[0xAA, 0xBB, 0xCC]);
    }

    /// MUTATION-CHECKED: change `try_capture_config`'s NAL-type match to
    /// require only `NAL_TYPE_SPS` (drop the PPS requirement) and this test
    /// starts capturing a config from an SPS-only sample — fails because
    /// `avc_config_from_sps_pps` is never even reached (the early-return
    /// guard `sps.is_empty() || pps.is_empty()` is the thing under test);
    /// restoring the `&&`-equivalent guard makes it pass again.
    #[tokio::test]
    async fn try_capture_config_requires_both_sps_and_pps() {
        let admitted = AdmittedWhip {
            socket: {
                let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
                s.set_nonblocking(true).unwrap();
                Arc::new(UdpSocket::from_std(s).unwrap())
            },
            media: Arc::new(TokioMutex::new(
                MediaTransport::new(MediaTransportConfig {
                    local_addr: "127.0.0.1:0".parse().unwrap(),
                    local_ice_ufrag: rand_token(8),
                    local_ice_pwd: rand_token(24),
                    remote_ice_ufrag: rand_token(8),
                    remote_ice_pwd: rand_token(24),
                    is_controlling: false,
                    local_setup: SetupRole::Passive,
                    stun_server: None,
                })
                .unwrap(),
            )),
            tracks: vec![WhipTrack {
                track_id: 1,
                payload_type: 96,
                clock_rate: 90_000,
            }],
        };
        let mut session = WhipIngestSession::new(admitted);

        // SPS only (type 7, 4 bytes -- the minimum `avc_config_from_sps_pps`
        // reads profile/compat/level from), no PPS: real (small,
        // hand-built-for-the-test) NAL header + profile/level bytes --
        // `try_capture_config` only reads the type nibble, never decodes the
        // RBSP any further than that.
        let sps_only = [0u8, 0, 0, 4, 0x67, 0x42, 0x00, 0x1F];
        let sample = Sample::new(
            bytes::Bytes::copy_from_slice(&sps_only),
            None,
            None,
            None,
            true,
        );
        session.try_capture_config(1, &sample);
        assert!(
            session.captured.get(&1).cloned().flatten().is_none(),
            "SPS alone must not capture a config"
        );

        // Now a real SPS+PPS pair (types 7 and 8).
        let mut both = Vec::new();
        both.extend_from_slice(&4u32.to_be_bytes());
        both.extend_from_slice(&[0x67, 0x42, 0x00, 0x1F]);
        both.extend_from_slice(&2u32.to_be_bytes());
        both.extend_from_slice(&[0x68, 0xCE]);
        let sample = Sample::new(bytes::Bytes::copy_from_slice(&both), None, None, None, true);
        session.try_capture_config(1, &sample);
        assert!(
            session.captured.get(&1).cloned().flatten().is_some(),
            "SPS+PPS must capture a config"
        );
    }
}
