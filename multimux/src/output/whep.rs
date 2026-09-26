//! WHEP (draft-ietf-wish-whep) egress output (issue #743).
//!
//! Accepts an inbound WHEP viewer: an HTTP `POST` carrying an SDP offer
//! (`application/sdp`), answered with a `201 Created` carrying this side's
//! SDP answer, after which this route's `media_plane::Trunk` samples flow
//! out to the viewer over ICE + DTLS-SRTP —
//! [`webrtc_runtime::media::MediaTransport`] is the transport this module
//! drives, exactly like `crate::source::whip` on the ingest side. **This
//! module is that module's mirror**: read `crate::source::whip`'s own
//! module doc first — every structural decision documented there (why SDP
//! itself is hand-rolled rather than routed through
//! `webrtc_runtime::whep::server::WhepSession`, why the ICE/DTLS/SRTP
//! machinery lives in this module's own per-session read loop rather than
//! in a `Stage` impl, the `OPTIONS` CORS preflight handling, the bind-once
//! listener shape) applies here too, un-repeated.
//!
//! # Direction: egress, not ingest
//!
//! Where WHIP ingest depacketises inbound SRTP into `Sample`s for a
//! `Trunk`, WHEP egress does the reverse: it reads `Sample`s a `Trunk`
//! already has (published by whatever `InputSpec` is feeding that route —
//! RTSP, RTMP, WHIP, TS/UDP, …) and packetises them into outbound SRTP RTP.
//! Concretely: `crate::origin::spawn_whep_outputs` awaits the route's first
//! `Trunk` (via `RouteHandle::await_first_trunk`, exactly like a push
//! output) *before* [`run_whep`] ever binds a socket — by the time a
//! viewer's first `POST` can be answered, the `Trunk` this module reads
//! `Sample`s and the real (already-known) `TrackSpec` — including real
//! SPS/PPS — from already exists. This is a **simplification** relative to
//! WHIP ingest's own deferred-`avcC`-capture dance (that module's own doc):
//! there is no "wait for a real IDR to learn the codec config" step here,
//! because the codec config was already captured by whichever ingest is
//! feeding this route.
//!
//! # Scope: video (H.264) only, no trickle ICE, no renegotiation
//!
//! Mirrors `crate::source::whip`'s own "video only" scope for the identical
//! reason in reverse: this workspace has no RTP/Opus **packetiser** (only
//! the reverse, a depacketiser, matters for WHIP ingest — this module needs
//! the packetiser direction, and none exists for Opus). A viewer's offer
//! containing no `m=video` section — or more than one `m=` section at all —
//! is rejected with [`MultimuxError::Sdp`], same as WHIP. Only the initial
//! offer/answer exchange is implemented: no `PATCH` (trickle ICE / ICE
//! restart) endpoint exists, matching WHIP ingest's own identical omission
//! (see that module's doc) — a viewer must complete non-trickle ICE
//! gathering before `POST`ing, exactly like `tests/assets/whip_publish.mjs`
//! already does for the ingest side.
//!
//! # Why the codec config comes straight off the `Trunk`, not the bitstream
//!
//! `crate::source::whip::WhipIngestSession` scans a real IDR for SPS/PPS
//! because a WHIP publisher's SDP offer generally omits
//! `sprop-parameter-sets` (browsers carry parameter sets in-band). This
//! module has no such problem: the route's `Trunk` already carries a real
//! [`transmux::pipeline::TrackSpec`] with a real `CodecConfig::Avc` (whatever
//! upstream ingest produced it already did the same "read the real
//! bitstream" work, once, at ingest time) — `handle_whep_connection` reads
//! it directly off `Trunk::tracks()` and uses it both to build the answer's
//! `sprop-parameter-sets`/`profile-level-id` and to drive
//! [`transmux::RtpPacketiser::packetise_video`]'s per-sample STAP-A
//! parameter-set aggregation.
//!
//! # Continuous RTP state across [`transmux::RtpPacketiser`] calls
//!
//! [`transmux::RtpPacketiser::packetise_video`] is a **batch** API: called
//! once per (track, all-its-samples) with its own fresh sequence-number
//! counter starting at 0 and a timestamp computed relative to that batch's
//! first sample. A live egress session instead has to packetise one
//! just-arrived `Sample` at a time, with sequence numbers and timestamps
//! that stay continuous across calls (RFC 3550 §5.1) — the reason this
//! module still calls `packetise_video` (reusing its already-correct RFC
//! 6184 single-NAL/STAP-A/FU-A framing rather than re-implementing it) but
//! then **patches** the two fields that batch call got wrong for a
//! streaming caller: `patch_seq_and_timestamp` overwrites the RTP fixed
//! header's sequence-number and timestamp bytes (always at a fixed byte
//! offset, RFC 3550 §5.1) with this session's own running counters before
//! handing the packet to [`MediaTransport::encrypt_rtp`] (which needs the
//! typed `rtp_packet::RtpPacket`, so the patched bytes are re-parsed rather
//! than hand-assembled). Those two are the *only* fields patched — SSRC and
//! payload type are already this session's, straight from the
//! `RtpPacketiser` it was built with; see that function's own doc for the
//! full "what is deliberately not patched" list.
//!
//! # Peer address: learned, not looked up
//!
//! [`MediaTransport`] has no public "current remote address" getter (by
//! design — see that type's own module doc: it owns no socket). Exactly
//! like `crate::source::whip::read_one`'s own `peer` parameter, this
//! module learns the viewer's UDP source address from the first inbound
//! datagram `run_whep_session` observes (a STUN binding request, in
//! practice) and reuses that address for every outbound RTP send for the
//! rest of the session — a WHEP viewer's ICE candidate is a single stable
//! 5-tuple once connectivity checks pick it, so one learned address is
//! sufficient (no candidate-pair migration handling in this cut).

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use broadcast_auth::{AuthResult, Verifier};
use broadcast_common::Parse;
use media_plane::trunk::{SampleCursorItem, Trunk};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Mutex as TokioMutex, mpsc};
use tokio_util::sync::CancellationToken;

use transmux::ir::Track;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::{DEFAULT_AUDIO_PT, DEFAULT_MTU, RtpPacketiser, VIDEO_CLOCK_RATE};

use webrtc_runtime::media::{
    Datagram, MediaEvent, MediaTransport, MediaTransportConfig, SetupRole, parse_remote_fingerprint,
};
use webrtc_runtime::whep::{content_type, status};

use crate::error::{MultimuxError, Result};

/// Default cap on concurrently admitted WHEP viewers per route — mirrors
/// `crate::source::whip::DEFAULT_WHIP_MAX_SESSIONS`'s own reasoning:
/// generous for a real audience while still bounding an unbounded flood of
/// inbound viewer connections.
pub const DEFAULT_WHEP_MAX_SESSIONS: usize = 64;

/// How often `run_whep_session`'s read loop times out waiting for an
/// inbound datagram before checking the `Trunk` cursor for new samples to
/// send — see `crate::source::whip`'s `ACCEPT_POLL_INTERVAL` for the
/// identical "bounded poll, not a busy loop" reasoning.
const SESSION_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Bound on the accept-pump's admission channel — see
/// `crate::source::whip::ACCEPT_QUEUE_CAPACITY`'s identical reasoning.
const ACCEPT_QUEUE_CAPACITY: usize = 32;

/// Max UDP datagram this output reads in one `recv` — matches
/// `crate::source::whip::MAX_UDP_DATAGRAM`.
const MAX_UDP_DATAGRAM: usize = 65_536;

/// Ends a WHEP session (freeing its `active_sessions` slot) once no inbound
/// datagram — STUN, DTLS, or SRTP/RTCP, valid or not — has arrived for this
/// long. RFC 7675 (ICE consent freshness) uses the same 30s figure for how
/// long a live session may go without a fresh consent check before treating
/// the peer as gone; a viewer that vanished (network loss, tab closed
/// without a clean teardown) would otherwise hold its slot forever.
const WHEP_INBOUND_SILENCE_TIMEOUT: Duration = Duration::from_secs(30);

/// A WHEP egress route: binds an HTTP listen socket and answers viewers
/// against it. See the module doc.
#[derive(Debug, Clone)]
pub struct WhepRoute {
    listen: String,
    max_sessions: usize,
}

impl WhepRoute {
    /// Build a route whose WHEP viewer endpoint listens on `listen` (e.g.
    /// `"0.0.0.0:8081"`, or `"127.0.0.1:0"` for an ephemeral test port). A
    /// viewer `POST`s its SDP offer to `http://<listen>/` (any path is
    /// accepted — this is a single-route listener, not a multi-tenant path
    /// router, exactly like `crate::source::whip::WhipRoute`).
    pub fn new(listen: impl Into<String>) -> Self {
        WhepRoute {
            listen: listen.into(),
            max_sessions: DEFAULT_WHEP_MAX_SESSIONS,
        }
    }

    /// Overrides [`DEFAULT_WHEP_MAX_SESSIONS`].
    #[must_use]
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = max_sessions;
        self
    }

    /// The configured `host:port` this route binds.
    pub fn listen(&self) -> &str {
        &self.listen
    }
}

/// One admitted WHEP viewer, handed from the HTTP accept-pump to
/// [`run_whep`]'s admission loop: the SDP exchange is already complete by
/// the time this exists (see the module doc) — what remains is the media
/// session itself.
struct AdmittedWhep {
    socket: Arc<UdpSocket>,
    media: Arc<TokioMutex<MediaTransport>>,
    /// The real, already-known `TrackSpec` (with real SPS/PPS) this
    /// session was negotiated against.
    spec: TrackSpec,
    /// The RTP dynamic payload type this session negotiated (echoed from
    /// the viewer's offer).
    pt: u8,
    /// Fixed SSRC for this session's one outbound video stream.
    ssrc: u32,
}

/// One parsed WHEP SDP offer — the viewer-side analogue of
/// `crate::source::whip::ParsedOffer`, minus the codec-parameter echo (this
/// side decides the codec parameters itself, from the real `Trunk` track).
struct ParsedWhepOffer {
    remote_ufrag: String,
    remote_pwd: String,
    /// The offer's `a=fingerprint` value (RFC 8122 §5) — required; without
    /// it there is nothing to authenticate the peer's DTLS certificate
    /// against (RFC 5764 §5), so an offer lacking one is rejected like any
    /// other malformed offer.
    remote_fingerprint: String,
    mid: String,
    candidates: Vec<String>,
    /// The offer's `a=setup` value (`"active"`/`"passive"`/`"actpass"`), if
    /// present — drives [`choose_setup_role`].
    setup: Option<String>,
    /// The payload type this route will answer with: the first `m=video`
    /// `fmt` entry whose `a=rtpmap` names H.264.
    payload_type: u8,
}

/// Parses a WHEP viewer offer for its single supported case: exactly one
/// `m=video` section naming at least one H.264 payload type — see the
/// module doc's "Scope" section.
fn parse_whep_offer(offer: &str) -> Result<ParsedWhepOffer> {
    let session = sdp_types::Session::parse(offer.as_bytes()).map_err(|e| MultimuxError::Sdp {
        reason: format!("whep: parse offer: {e}"),
    })?;
    let video_medias: Vec<_> = session
        .medias
        .iter()
        .filter(|m| m.media == "video")
        .collect();
    if session.medias.len() != video_medias.len() || video_medias.len() != 1 {
        return Err(MultimuxError::Sdp {
            reason: format!(
                "whep: this route accepts exactly one m=video section and nothing else \
                 (Opus audio has no RTP packetiser in this workspace yet); offer had {} \
                 total section(s), {} of them video",
                session.medias.len(),
                video_medias.len()
            ),
        });
    }
    let media = video_medias[0];

    let remote_ufrag =
        sdp_attr_anywhere(offer, "a=ice-ufrag:").ok_or_else(|| MultimuxError::Sdp {
            reason: "whep: offer has no a=ice-ufrag".into(),
        })?;
    let remote_pwd = sdp_attr_anywhere(offer, "a=ice-pwd:").ok_or_else(|| MultimuxError::Sdp {
        reason: "whep: offer has no a=ice-pwd".into(),
    })?;
    let remote_fingerprint = parse_remote_fingerprint(offer).ok_or_else(|| MultimuxError::Sdp {
        reason: "whep: offer has no a=fingerprint".into(),
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
    let setup = sdp_attr_anywhere(offer, "a=setup:");

    let mut payload_type = None;
    for tok in media.fmt.split_whitespace() {
        let Ok(pt) = tok.parse::<u8>() else { continue };
        let rtpmap = offer
            .lines()
            .find(|l| l.starts_with(&format!("a=rtpmap:{pt} ")));
        let Some(rtpmap) = rtpmap else { continue };
        if rtpmap.to_ascii_uppercase().contains("H264") {
            payload_type = Some(pt);
            break;
        }
    }
    let payload_type = payload_type.ok_or_else(|| MultimuxError::Sdp {
        reason: "whep: m=video has no H.264 (a=rtpmap naming H264) payload type".into(),
    })?;

    Ok(ParsedWhepOffer {
        remote_ufrag,
        remote_pwd,
        remote_fingerprint,
        mid,
        candidates,
        setup,
        payload_type,
    })
}

/// Finds the first `a=<prefix>` line's value anywhere in `sdp` — see
/// `crate::source::whip::sdp_attr_anywhere`'s identical reasoning.
fn sdp_attr_anywhere(sdp: &str, prefix: &str) -> Option<String> {
    sdp.lines()
        .find_map(|l| l.strip_prefix(prefix))
        .map(|v| v.trim().to_string())
}

/// Resolves this side's [`SetupRole`] from the viewer offer's `a=setup`
/// value (RFC 8842 §4.1): the answerer's role must be the complement of an
/// explicit `"active"`/`"passive"` offer, and defaults to
/// [`SetupRole::Passive`] (this side is the DTLS server) for `"actpass"` or
/// a missing attribute — the same default `crate::source::whip` hardcodes,
/// and what every real WHEP viewer (an ordinary `RTCPeerConnection` offer)
/// actually sends. [`SetupRole::Active`] is reachable here only for the
/// rare/defensive case of a viewer offer that explicitly pins `"passive"`.
fn choose_setup_role(offer_setup: Option<&str>) -> SetupRole {
    match offer_setup {
        Some("active") => SetupRole::Passive,
        Some("passive") => SetupRole::Active,
        _ => SetupRole::Passive,
    }
}

/// Builds this side's SDP answer: `a=sendonly` (a WHEP viewer endpoint only
/// ever receives media), `a=rtcp-mux` (required, see
/// `crate::source::whip::build_answer`'s identical note), and the real
/// codec parameters (`sprop-parameter-sets`/`profile-level-id`) read
/// straight off the `Trunk`'s own `TrackSpec` — see the module doc.
fn build_whep_answer(
    parsed: &ParsedWhepOffer,
    media: &MediaTransport,
    local_addr: SocketAddr,
    local_ice_ufrag: &str,
    local_ice_pwd: &str,
    setup_role: SetupRole,
    config: &transmux::AVCDecoderConfigurationRecord,
) -> String {
    let pt = parsed.payload_type;
    let profile_level_id = format!(
        "{:02X}{:02X}{:02X}",
        config.profile_indication, config.profile_compatibility, config.level_indication
    );
    let mut sprop = String::new();
    for (i, sps) in config.sps.iter().enumerate() {
        if i > 0 {
            sprop.push(',');
        }
        sprop.push_str(&transmux::rtp::base64_encode(&sps.0));
    }
    for pps in &config.pps {
        if !sprop.is_empty() {
            sprop.push(',');
        }
        sprop.push_str(&transmux::rtp::base64_encode(&pps.0));
    }

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
    answer.push_str(&format!("m=video 9 UDP/TLS/RTP/SAVPF {pt}\r\n"));
    answer.push_str("c=IN IP4 127.0.0.1\r\n");
    answer.push_str("a=rtcp:9 IN IP4 0.0.0.0\r\n");
    answer.push_str(&format!("a=rtpmap:{pt} H264/{VIDEO_CLOCK_RATE}\r\n"));
    answer.push_str(&format!(
        "a=fmtp:{pt} packetization-mode=1;profile-level-id={profile_level_id};\
         sprop-parameter-sets={sprop}\r\n"
    ));
    answer.push_str("a=sendonly\r\n");
    answer.push_str(&format!("a=mid:{}\r\n", parsed.mid));
    answer.push_str("a=rtcp-mux\r\n");
    answer.push_str(&format!("a=ice-ufrag:{local_ice_ufrag}\r\n"));
    answer.push_str(&format!("a=ice-pwd:{local_ice_pwd}\r\n"));
    answer.push_str(&format!(
        "a=fingerprint:sha-256 {}\r\n",
        media.local_fingerprint()
    ));
    answer.push_str(&format!("a=setup:{}\r\n", setup_role.name()));
    answer.push_str(&format!("a=candidate:{candidate_line}\r\n"));
    answer.push_str("a=end-of-candidates\r\n");
    answer
}

/// A short pseudo-random token — see `crate::source::whip::rand_token`'s
/// identical technique (OS-random `RandomState`, no `rand` dependency).
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

/// A pseudo-random 32-bit SSRC for one session's outbound video stream —
/// same `RandomState` technique as [`rand_token`], just hashed to a `u32`
/// instead of rendered to a token alphabet.
fn rand_ssrc() -> u32 {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    let state = RandomState::new();
    (state.hash_one(Instant::now()) as u32) | 1
}

/// Verifies one request against `output_auth`, exactly mirroring
/// `crate::origin::output_auth_gate`'s own decision: `None` when
/// `output_auth` isn't configured or the method is `OPTIONS` (a CORS
/// preflight never carries the real request's credentials — see that
/// function's own doc), the same `broadcast_auth::RequestContext`
/// (method/URI/headers/peer address, no body — `output_auth_gate` never
/// feeds one in either), and the same `401` + `WWW-Authenticate` challenge
/// on anything other than `AuthResult::Ok` (default-deny, since
/// `AuthResult` is `#[non_exhaustive]`). `Some(response)` is the full
/// HTTP/1.1 response to write back and close; `None` means "authorized,
/// proceed".
fn check_output_auth(
    output_auth: &Option<Arc<Verifier>>,
    method: &str,
    uri: &str,
    headers: &[(String, String)],
    peer_addr: Option<SocketAddr>,
) -> Option<String> {
    let verifier = output_auth.as_ref()?;
    if method.eq_ignore_ascii_case("OPTIONS") {
        return None;
    }
    let header_pairs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut ctx = broadcast_auth::RequestContext::new(method, uri).with_headers(&header_pairs);
    if let Some(peer) = peer_addr {
        ctx = ctx.with_peer_addr(peer);
    }
    match verifier.verify(&ctx) {
        AuthResult::Ok => None,
        _ => Some(format!(
            "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: {}\r\nContent-Length: 0\r\n\r\n",
            verifier.challenge()
        )),
    }
}

/// Handles one accepted TCP connection end-to-end: read the POST, verify
/// output auth, parse and answer the offer against the route's real `Trunk`
/// track, bind this session's own ephemeral media socket, build the
/// [`MediaTransport`], write the `201 Created` response, and hand the
/// negotiated session off to `tx`. An `OPTIONS` preflight gets a permissive
/// CORS response and no session — see
/// `crate::source::whip::handle_whip_connection`'s identical shape.
async fn handle_whep_connection(
    mut stream: TcpStream,
    trunk: &Trunk,
    tx: &mpsc::Sender<AdmittedWhep>,
    active_sessions: &Arc<AtomicUsize>,
    max_sessions: usize,
    output_auth: &Option<Arc<Verifier>>,
) -> Result<()> {
    let peer_addr = stream.peer_addr().ok();
    let crate::webrtc_http::HttpRequest {
        request_line,
        headers,
        body,
    } = match crate::webrtc_http::read_http_request(&mut stream).await {
        Ok(r) => r,
        Err(crate::webrtc_http::ReadRequestError::HeadersTooLarge) => {
            let resp = "HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes()).await;
            return Ok(());
        }
        Err(crate::webrtc_http::ReadRequestError::BodyTooLarge) => {
            let resp = "HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes()).await;
            return Ok(());
        }
        Err(crate::webrtc_http::ReadRequestError::BadRequest) => {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes()).await;
            return Ok(());
        }
        Err(crate::webrtc_http::ReadRequestError::LengthRequired) => {
            let resp = "HTTP/1.1 411 Length Required\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes()).await;
            return Ok(());
        }
        Err(crate::webrtc_http::ReadRequestError::Timeout) => {
            return Err(MultimuxError::Connect {
                reason: "whep: read request: timed out".into(),
            });
        }
        Err(crate::webrtc_http::ReadRequestError::Io(e)) => {
            return Err(MultimuxError::Connect {
                reason: format!("whep: read request: {e}"),
            });
        }
    };
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

    // Output auth (issue r07-C11): verified before any work — same gate a
    // GET on the media routes goes through, applied here since this raw
    // listener has no axum middleware stack of its own.
    let mut method_and_uri = request_line.split_whitespace();
    let method = method_and_uri.next().unwrap_or_default();
    let uri = method_and_uri.next().unwrap_or_default();
    if let Some(resp) = check_output_auth(output_auth, method, uri, &headers, peer_addr) {
        let _ = stream.write_all(resp.as_bytes()).await;
        return Ok(());
    }

    let offer_sdp = String::from_utf8_lossy(&body).into_owned();
    let parsed = parse_whep_offer(&offer_sdp)?;

    let spec = trunk
        .tracks()
        .iter()
        .find(|t| matches!(t.config, CodecConfig::Avc { .. }))
        .cloned();
    let Some(spec) = spec else {
        let resp = format!(
            "HTTP/1.1 {} Conflict\r\nRetry-After: 2\r\nContent-Length: 0\r\n\r\n",
            status::CONFLICT
        );
        let _ = stream.write_all(resp.as_bytes()).await;
        return Ok(());
    };
    let CodecConfig::Avc { config, .. } = &spec.config else {
        unreachable!("filtered to CodecConfig::Avc above");
    };

    // Capacity check (issue #743's mirror of `crate::source::whip`'s own
    // `max_sessions`): reject before answering, not after — an admitted
    // session that never gets read is a leaked socket/ICE agent for no
    // benefit.
    //
    // `slot` is an RAII guard (issue r07-C11 follow-up): every `?` between
    // here and the successful `tx.send` below releases it automatically on
    // drop, so a client that resets the connection mid-setup can no longer
    // leak the reservation forever. `slot.disarm()` right before returning
    // `Ok` hands the slot's lifetime off to the admitted session —
    // `run_whep_session`'s own `active_sessions.fetch_sub` at the end of its
    // loop is the new owner from that point on.
    let Some(slot) = crate::webrtc_http::SessionSlot::acquire(active_sessions, max_sessions) else {
        let resp = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes()).await;
        return Ok(());
    };

    // See `crate::source::whip::handle_whip_connection`'s identical
    // "advertise the signalling connection's own local address" reasoning.
    let advertise_ip = stream
        .local_addr()
        .map(|a| a.ip())
        .map_err(|e| MultimuxError::Connect {
            reason: format!("whep: signalling connection local_addr: {e}"),
        })?;
    let socket = UdpSocket::bind((advertise_ip, 0))
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("whep: bind media socket on {advertise_ip}: {e}"),
        })?;
    let local_addr = socket.local_addr().map_err(|e| MultimuxError::Connect {
        reason: format!("whep: media socket local_addr: {e}"),
    })?;

    let setup_role = choose_setup_role(parsed.setup.as_deref());
    let local_ice_ufrag = rand_token(8);
    let local_ice_pwd = rand_token(24);
    let mut media = MediaTransport::new(MediaTransportConfig {
        local_addr,
        local_ice_ufrag: local_ice_ufrag.clone(),
        local_ice_pwd: local_ice_pwd.clone(),
        remote_ice_ufrag: parsed.remote_ufrag.clone(),
        remote_ice_pwd: parsed.remote_pwd.clone(),
        remote_fingerprint: parsed.remote_fingerprint.clone(),
        is_controlling: false,
        local_setup: setup_role,
        stun_server: None,
    })
    .map_err(|e| MultimuxError::Connect {
        reason: format!("whep: build media transport: {e}"),
    })?;

    for raw in &parsed.candidates {
        let _ = media.add_remote_candidate(raw);
    }

    let answer = build_whep_answer(
        &parsed,
        &media,
        local_addr,
        &local_ice_ufrag,
        &local_ice_pwd,
        setup_role,
        &config.config,
    );
    let resp = format!(
        "HTTP/1.1 201 Created\r\n\
         Content-Type: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Expose-Headers: Location\r\n\
         Location: /whep/session\r\n\
         Content-Length: {}\r\n\r\n{}",
        content_type::SDP,
        answer.len(),
        answer
    );
    stream
        .write_all(resp.as_bytes())
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("whep: write answer: {e}"),
        })?;
    let _ = stream.shutdown().await;

    let admitted = AdmittedWhep {
        socket: Arc::new(socket),
        media: Arc::new(TokioMutex::new(media)),
        spec,
        pt: parsed.payload_type,
        ssrc: rand_ssrc(),
    };
    if tx.send(admitted).await.is_ok() {
        // Ownership of the slot passes to the admitted session's own
        // `run_whep_session` loop from here — it decrements the same
        // counter once, when the session ends.
        slot.disarm();
    }
    // If the send failed (channel closed), `slot` drops here and releases
    // the slot itself — nothing else will ever drive this session.
    Ok(())
}

/// Rescales `ticks` (in `from_timescale` ticks/sec) to the fixed 90 kHz RTP
/// video clock (RFC 6184) — the streaming analogue of
/// `transmux::rtp`'s own (private) `rescale_ts`, computed in `u128` to
/// avoid overflow for a `from_timescale` far below 90 kHz on a
/// long-running session's large absolute tick count.
fn rescale_to_90k(ticks: u64, from_timescale: u32) -> u32 {
    if from_timescale == 0 || from_timescale == VIDEO_CLOCK_RATE {
        return ticks as u32;
    }
    (((ticks as u128) * VIDEO_CLOCK_RATE as u128 + (from_timescale as u128) / 2)
        / from_timescale as u128) as u32
}

/// Byte offset of the `sequence number` field within the RTP fixed header
/// (RFC 3550 §5.1: the 16-bit field immediately after the `V`/`P`/`X`/`CC`
/// and `M`/`PT` octets).
const RTP_SEQ_OFFSET: usize = 2;
/// Width of the RTP `sequence number` field, in bytes (16 bits, RFC 3550
/// §5.1).
const RTP_SEQ_LEN: usize = 2;
/// Byte offset of the `timestamp` field within the RTP fixed header
/// (RFC 3550 §5.1: immediately after the sequence number).
const RTP_TIMESTAMP_OFFSET: usize = RTP_SEQ_OFFSET + RTP_SEQ_LEN;
/// Width of the RTP `timestamp` field, in bytes (32 bits, RFC 3550 §5.1).
const RTP_TIMESTAMP_LEN: usize = 4;

/// Overwrites an RTP packet's `sequence number` and `timestamp` fields on
/// an already-serialized wire packet — see the module doc's "Continuous RTP
/// state" section for why this is necessary at all. The RTP fixed header's
/// layout (RFC 3550 §5.1) puts both fields at a fixed offset regardless of
/// payload format (single-NAL / STAP-A / FU-A), so this needs no awareness
/// of which one `contiguous` is.
///
/// # What is deliberately *not* patched
///
/// Only these two fields are wrong for a streaming caller. Everything else
/// [`RtpPacketiser`] wrote is already correct for this session and must be
/// left alone:
///
/// - **`SSRC`** (bytes `[8:12]`) — [`send_sample`] constructs its
///   `RtpPacketiser` with `ssrc: session.ssrc`, and every packetise path
///   (single-NAL, STAP-A, FU-A) writes it through `transmux::rtp`'s own
///   `rtp_header` helper, so the emitted SSRC is already this session's.
/// - **`payload type`** (low 7 bits of byte 1) — likewise set from
///   `video_pt: session.pt`, the payload type echoed from the viewer's own
///   offer.
/// - **`marker`** (high bit of byte 1) — RFC 6184's per-access-unit
///   semantics, which the packetiser is the only thing positioned to get
///   right.
///
/// # Errors
///
/// Returns `None` for a `contiguous` shorter than
/// [`rtp_packet::FIXED_HEADER_LEN`], which cannot carry the fields this
/// patches. **Unreachable in practice** — [`RtpPacketiser`] always emits at
/// least a full RFC 3550 §5.1 fixed header — but it is reported rather than
/// tolerated: silently returning the buffer *unpatched* would put a packet
/// on the wire carrying the packetiser's own per-batch sequence number and
/// timestamp instead of this session's running ones, which is the exact bug
/// this function exists to prevent. The caller ([`send_sample`]) drops such
/// a packet loudly instead of sending a wrong one.
fn patch_seq_and_timestamp(contiguous: &[u8], seq: u16, timestamp: u32) -> Option<Vec<u8>> {
    if contiguous.len() < rtp_packet::FIXED_HEADER_LEN {
        return None;
    }
    let mut v = contiguous.to_vec();
    v[RTP_SEQ_OFFSET..RTP_SEQ_OFFSET + RTP_SEQ_LEN].copy_from_slice(&seq.to_be_bytes());
    v[RTP_TIMESTAMP_OFFSET..RTP_TIMESTAMP_OFFSET + RTP_TIMESTAMP_LEN]
        .copy_from_slice(&timestamp.to_be_bytes());
    Some(v)
}

/// The per-session negotiated state [`send_sample`] needs beyond the
/// socket/transport handles — grouped into one struct purely to keep that
/// function's argument count down; every field is set once at admission
/// time and never changes for the session's lifetime.
struct SessionMedia<'a> {
    /// The negotiated RTP dynamic payload type.
    pt: u8,
    /// This session's fixed SSRC.
    ssrc: u32,
    /// The real, already-known `TrackSpec` (with real SPS/PPS) this
    /// session was negotiated against.
    spec: &'a TrackSpec,
}

/// Packetises one `Sample` into RTP (via [`RtpPacketiser::packetise_video`],
/// patched per the module doc), encrypts each packet, and sends it to
/// `peer`. `next_seq` is this session's running sequence-number counter,
/// advanced by exactly the number of RTP packets this one sample produced
/// (1 for a small frame, more for STAP-A-led or FU-A-fragmented frames).
async fn send_sample(
    socket: &UdpSocket,
    media: &TokioMutex<MediaTransport>,
    peer: SocketAddr,
    next_seq: &mut u16,
    session: &SessionMedia<'_>,
    sample: &Sample,
) {
    let Some(dts) = sample.dts else {
        return;
    };
    let timestamp = rescale_to_90k(dts.max(0) as u64, session.spec.timescale);

    let packetiser = RtpPacketiser {
        mtu: DEFAULT_MTU,
        video_pt: session.pt,
        audio_pt: DEFAULT_AUDIO_PT,
        ssrc: session.ssrc,
        // Lead each sync (IDR) sample with a fresh STAP-A of the real
        // SPS/PPS (RFC 6184 §5.7.1) so a viewer that joins mid-stream, or
        // that lost the very first one, can still resynchronize at the next
        // keyframe — see the module doc.
        stap_a_parameter_sets: sample.flags.is_sync,
    };
    let track = Track::new(session.spec.clone(), vec![sample.clone()]);
    let packets = match packetiser.packetise_video(&track, session.pt) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "whep: packetise_video failed; dropping sample");
            return;
        }
    };

    let mut guard = media.lock().await;
    for pkt in &packets {
        let contiguous = pkt.as_contiguous();
        // This packet's sequence number, burnt from the session counter
        // *before* any of the drop paths below — RFC 3550 §5.1 sequence
        // numbers count packets emitted for the stream, so a number burnt
        // on a packet that never made it out reads to the viewer as
        // ordinary loss (which it is), rather than being silently reused
        // by the next packet and hiding the gap.
        let seq = *next_seq;
        *next_seq = next_seq.wrapping_add(1);
        let Some(patched) = patch_seq_and_timestamp(&contiguous, seq, timestamp) else {
            tracing::error!(
                len = contiguous.len(),
                min = rtp_packet::FIXED_HEADER_LEN,
                "whep: packetiser emitted a packet shorter than the RFC 3550 §5.1 fixed \
                 header; dropping it rather than sending one still carrying the \
                 packetiser's own sequence number/timestamp"
            );
            continue;
        };
        let wire = match rtp_packet::RtpPacket::parse(&patched) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "whep: re-parsing patched RTP packet failed");
                continue;
            }
        };
        match guard.encrypt_rtp(&wire) {
            Ok(protected) => {
                let _ = socket.send_to(&protected, peer).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "whep: encrypt_rtp failed");
            }
        }
    }
}

/// Whether one [`MediaEvent`] proves the far end is actually still there
/// (issue r07-C11 follow-up), for [`run_whep_session`]'s own
/// `last_inbound` reset: a decrypted RTP or RTCP packet (the far end's
/// decoder/receiver-report machinery is live, RFC 3550 §6.2) — including
/// an authenticated SRTCP packet `rtcp-packet` can't decode
/// (`RtcpUnsupported`: the RFC 4585 PLI/NACK/REMB feedback that is most of
/// what a real browser viewer sends; excluding it ended every real viewer's
/// session after 30s) — or the one-time DTLS handshake completion. A
/// datagram failing SRTP/SRTCP authentication is an `Err`, never an event,
/// so a stray still cannot count. Deliberately excludes
/// `IceStateChanged`/`LocalCandidateGathered` — real, `Ok`-returning ICE
/// bookkeeping that says nothing about whether the far end is still
/// decoding anything.
fn is_liveness_event(event: &MediaEvent) -> bool {
    matches!(
        event,
        MediaEvent::Rtp(_)
            | MediaEvent::Rtcp(_)
            | MediaEvent::RtcpUnsupported(_)
            | MediaEvent::DtlsHandshakeComplete
    )
}

/// Drives one admitted viewer's media session for its whole lifetime:
/// alternates between servicing inbound ICE/DTLS-SRTP datagrams (draining
/// [`MediaTransport::poll_transmit`] after each) and — once the DTLS
/// handshake has completed and a peer address is known — draining newly
/// published `Trunk` samples for this session's track, packetising and
/// sending each as SRTP. Returns when the socket errors, the session is
/// cancelled, the process is shutting down, or [`WHEP_INBOUND_SILENCE_TIMEOUT`]
/// elapses with no *authenticated* inbound datagram (see the loop's own
/// `last_inbound` doc for what that means here).
async fn run_whep_session(
    admitted: AdmittedWhep,
    trunk: Arc<Trunk>,
    cancel: CancellationToken,
    active_sessions: Arc<AtomicUsize>,
) {
    run_whep_session_with_silence_timeout(
        admitted,
        trunk,
        cancel,
        active_sessions,
        WHEP_INBOUND_SILENCE_TIMEOUT,
    )
    .await;
}

/// [`run_whep_session`], with the silence bound as a parameter rather than
/// baked to [`WHEP_INBOUND_SILENCE_TIMEOUT`] (30s) — exists so a test can
/// prove a session actually ends after a period of silence using a short
/// real duration instead of waiting out the real constant.
async fn run_whep_session_with_silence_timeout(
    admitted: AdmittedWhep,
    trunk: Arc<Trunk>,
    cancel: CancellationToken,
    active_sessions: Arc<AtomicUsize>,
    silence_timeout: Duration,
) {
    let AdmittedWhep {
        socket,
        media,
        spec,
        pt,
        ssrc,
    } = admitted;
    let session = SessionMedia {
        pt,
        ssrc,
        spec: &spec,
    };
    let mut cursor = trunk.subscribe();
    let mut peer_addr: Option<SocketAddr> = None;
    let mut handshake_done = false;
    let mut next_seq: u16 = 0;
    let mut buf = vec![0u8; MAX_UDP_DATAGRAM];
    // RFC 7675 consent-freshness bound (issue r07-C11) — updated only when
    // `handle_datagram` returns `Ok` *and* [`is_liveness_event`] accepts at
    // least one of the produced events (a decrypted RTP/RTCP packet, or the
    // DTLS handshake-complete event). Neither `Ok` alone nor "produced some
    // event" is that signal: after the webrtc-runtime 0.2.0 change, a DTLS
    // datagram from any address other than the ICE-selected pair returns
    // `Ok(())` with no events, but ordinary ICE bookkeeping (consent
    // binding requests/responses, a gathered candidate, an ICE connection
    // state transition) legitimately returns `Ok` *with* events too —
    // `MediaEvent::IceStateChanged`/`LocalCandidateGathered`, neither of
    // which says the far end is still decoding anything. A live viewer's
    // decoder sends SRTCP receiver reports every few seconds (RFC 3550
    // §6.2), so "decrypted an RTP/RTCP packet" (or the one-time handshake
    // completion) is the actual liveness signal; STUN consent checks alone
    // deliberately do not count, matching or not.
    let mut last_inbound = Instant::now();

    loop {
        if cancel.is_cancelled() {
            break;
        }
        match tokio::time::timeout(SESSION_POLL_INTERVAL, socket.recv_from(&mut buf)).await {
            Ok(Ok((n, peer))) => {
                peer_addr = Some(peer);
                let mut guard = media.lock().await;
                match guard.handle_datagram(Instant::now(), peer, &buf[..n]) {
                    Ok(events) => {
                        if events.iter().any(is_liveness_event) {
                            last_inbound = Instant::now();
                        }
                        if events
                            .iter()
                            .any(|e| matches!(e, MediaEvent::DtlsHandshakeComplete))
                        {
                            handshake_done = true;
                        }
                        while let Some(Datagram { peer, bytes }) = guard.poll_transmit() {
                            let _ = socket.send_to(&bytes, peer).await;
                        }
                    }
                    Err(e) => {
                        // Post-handshake this socket accepts datagrams from
                        // anyone (no source-address/ICE-pair check), so a
                        // failure once live is an unauthenticated stray, not
                        // a dead session — logged and skipped, matching
                        // `crate::source::whip::read_one`'s identical
                        // reasoning. Before the handshake ever completes,
                        // though, this session can never make progress
                        // again: end it now rather than waiting out the
                        // full silence timeout.
                        if handshake_done {
                            tracing::debug!(error = %e, "whep: datagram handling failed");
                        } else {
                            tracing::warn!(error = %e, "whep: dtls setup failed; ending session");
                            break;
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "whep: session socket read failed");
                break;
            }
            Err(_) => {
                if last_inbound.elapsed() >= silence_timeout {
                    tracing::info!("whep: session ending after inbound silence timeout");
                    break;
                }
                // Timeout: still drive ICE/DTLS timers (retransmits, etc.)
                // before checking for new samples below.
                let mut guard = media.lock().await;
                guard.handle_timeout(Instant::now());
                while let Some(Datagram { peer, bytes }) = guard.poll_transmit() {
                    let _ = socket.send_to(&bytes, peer).await;
                }
            }
        }

        if handshake_done && let Some(peer) = peer_addr {
            while let Some(item) = cursor.poll() {
                match item {
                    SampleCursorItem::Timed { track_id, sample }
                    | SampleCursorItem::Sparse { track_id, sample }
                        if track_id == session.spec.track_id =>
                    {
                        send_sample(&socket, &media, peer, &mut next_seq, &session, &sample).await;
                    }
                    _ => {}
                }
            }
        }
    }
    active_sessions.fetch_sub(1, Ordering::Relaxed);
}

/// Binds `route`'s listen socket and admits/drives up to
/// [`WhepRoute::with_max_sessions`] WHEP viewers **concurrently** until
/// cancelled — mirrors `crate::source::whip::run_whip`'s shape, but as an
/// egress driver spawned once per configured `OutputKind::Whep` output
/// (`crate::origin::spawn_whep_outputs`) rather than a supervised ingest
/// task. Returns once `cancel` fires or the listen socket fails.
pub async fn run_whep(
    route: &WhepRoute,
    trunk: Arc<Trunk>,
    cancel: CancellationToken,
    output_auth: Option<Arc<Verifier>>,
) {
    let listener = match TcpListener::bind(&route.listen).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(listen = %route.listen, error = %e, "whep: bind failed");
            return;
        }
    };

    let (admit_tx, mut admit_rx) = mpsc::channel::<AdmittedWhep>(ACCEPT_QUEUE_CAPACITY);
    let active_sessions = Arc::new(AtomicUsize::new(0));
    let max_sessions = route.max_sessions;

    let accept_trunk = Arc::clone(&trunk);
    let accept_cancel = cancel.clone();
    let accept_active = Arc::clone(&active_sessions);
    // Caps per-connection tasks in flight — see
    // `crate::source::whip::WhipRoute::ensure_infra`'s identical
    // `accept_semaphore` reasoning.
    let accept_semaphore = Arc::new(tokio::sync::Semaphore::new(
        crate::webrtc_http::MAX_PENDING_HTTP_CONNECTIONS,
    ));
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = accept_cancel.cancelled() => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let tx = admit_tx.clone();
                            let trunk = Arc::clone(&accept_trunk);
                            let active = Arc::clone(&accept_active);
                            let output_auth = output_auth.clone();
                            let permit = Arc::clone(&accept_semaphore)
                                .acquire_owned()
                                .await
                                .expect("accept_semaphore is never closed");
                            tokio::spawn(async move {
                                let _permit = permit;
                                if let Err(e) = handle_whep_connection(
                                    stream,
                                    &trunk,
                                    &tx,
                                    &active,
                                    max_sessions,
                                    &output_auth,
                                )
                                .await
                                {
                                    tracing::warn!(
                                        error = %e,
                                        "whep: signalling connection failed"
                                    );
                                }
                            });
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "whep: accept-pump ending after a listen-socket error"
                            );
                            break;
                        }
                    }
                }
            }
        }
    });

    let mut sessions: VecDeque<tokio::task::JoinHandle<()>> = VecDeque::new();
    loop {
        // Reap finished session tasks so `sessions` doesn't grow unbounded
        // over a long-running route's lifetime.
        sessions.retain(|h| !h.is_finished());
        tokio::select! {
            () = cancel.cancelled() => break,
            admitted = admit_rx.recv() => {
                match admitted {
                    Some(a) => {
                        let trunk = Arc::clone(&trunk);
                        let session_cancel = cancel.clone();
                        let active = Arc::clone(&active_sessions);
                        sessions.push_back(tokio::spawn(run_whep_session(
                            a, trunk, session_cancel, active,
                        )));
                    }
                    None => break,
                }
            }
            () = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
    for h in sessions {
        h.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed SHA-256 SDP fingerprint (RFC 8122 §5) — the shape
    /// `MediaTransport::new` validates; see `crate::source::whip`'s
    /// identical `OFFER_FINGERPRINT`.
    const OFFER_FINGERPRINT: &str = "sha-256 \
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:\
99:aa:bb:cc:dd:ee:ff";

    const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=candidate:1 1 udp 2130706431 10.0.0.5 54321 typ host\r\n";

    #[test]
    fn parses_video_only_offer() {
        let parsed = parse_whep_offer(OFFER).expect("parse");
        assert_eq!(parsed.payload_type, 96);
        assert_eq!(parsed.remote_ufrag, "abcd");
        assert_eq!(parsed.remote_pwd, "abcdefghijklmnopqrstuvwx");
        assert_eq!(parsed.remote_fingerprint, OFFER_FINGERPRINT);
        assert_eq!(parsed.mid, "0");
        assert_eq!(parsed.candidates.len(), 1);
        assert_eq!(parsed.setup.as_deref(), Some("actpass"));
    }

    /// RFC 5764 §5 / RFC 8122 §5: an offer with no `a=fingerprint` gives
    /// nothing to authenticate the peer's DTLS certificate against —
    /// rejected like any other malformed offer, never admitted unverified.
    #[test]
    fn rejects_offer_with_no_fingerprint() {
        let offer = OFFER.replace(
            "a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n",
            "",
        );
        assert!(parse_whep_offer(&offer).is_err());
    }

    #[test]
    fn rejects_offer_with_no_video() {
        let offer = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=ice-ufrag:x\r\na=ice-pwd:xxxxxxxxxxxxxxxxxxxxxxxx\r\n";
        assert!(parse_whep_offer(offer).is_err());
    }

    #[test]
    fn rejects_offer_with_audio_and_video() {
        let offer = OFFER.replace("m=video 9", "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nm=video 9");
        assert!(parse_whep_offer(&offer).is_err());
    }

    /// MUTATION-CHECKED: swapping the two match arms below (`"active" =>
    /// Active`, `"passive" => Passive`) makes this test fail — the offer's
    /// role and this side's chosen role would then be identical instead of
    /// complementary, which is exactly the RFC 8842 §4.1 violation this
    /// pins. Restored afterward.
    #[test]
    fn choose_setup_role_picks_the_complementary_role() {
        assert_eq!(choose_setup_role(Some("active")), SetupRole::Passive);
        assert_eq!(choose_setup_role(Some("passive")), SetupRole::Active);
        assert_eq!(choose_setup_role(Some("actpass")), SetupRole::Passive);
        assert_eq!(choose_setup_role(None), SetupRole::Passive);
    }

    #[test]
    fn rescale_to_90k_is_identity_at_90k() {
        assert_eq!(rescale_to_90k(12345, VIDEO_CLOCK_RATE), 12345);
    }

    #[test]
    fn rescale_to_90k_scales_a_different_timescale() {
        // 1 second at a 1000 Hz timescale -> 1 second at 90 kHz.
        assert_eq!(rescale_to_90k(1000, 1000), VIDEO_CLOCK_RATE);
    }

    /// MUTATION-CHECKED: replacing the seq/timestamp patch ranges with a
    /// no-op (`let _ = (seq, timestamp);`) makes this test fail — the
    /// patched bytes would still read the original (wrong) values instead
    /// of the ones this test asserts. Restored afterward.
    #[test]
    fn patch_seq_and_timestamp_overwrites_the_right_bytes() {
        // A minimal 12-byte RTP fixed header: V=2, PT=96, seq=1, ts=1000,
        // ssrc=0xDEADBEEF, followed by 2 bytes of "payload".
        let mut original = vec![0x80u8, 96];
        original.extend_from_slice(&1u16.to_be_bytes());
        original.extend_from_slice(&1000u32.to_be_bytes());
        original.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        original.extend_from_slice(&[0xAA, 0xBB]);

        let patched = patch_seq_and_timestamp(&original, 42, 90_000).expect("full header patches");
        assert_eq!(u16::from_be_bytes([patched[2], patched[3]]), 42);
        assert_eq!(
            u32::from_be_bytes([patched[4], patched[5], patched[6], patched[7]]),
            90_000
        );
        // Everything else is untouched — in particular the SSRC, which the
        // packetiser already wrote as this session's own (see
        // `patch_seq_and_timestamp`'s "what is deliberately not patched").
        assert_eq!(patched[0], original[0]);
        assert_eq!(patched[1], original[1]);
        assert_eq!(&patched[8..12], &original[8..12]);
        assert_eq!(&patched[12..], &original[12..]);
    }

    /// A packet too short to carry the fields being patched is refused
    /// (`None`), never returned unpatched — see
    /// `patch_seq_and_timestamp`'s `# Errors` section for why silently
    /// handing the buffer back would be the exact bug that function exists
    /// to prevent (a packet on the wire carrying the *packetiser's* own
    /// per-batch sequence number and timestamp instead of the session's).
    ///
    /// MUTATION-CHECKED: restoring the old tolerant form — `if v.len() >= 8
    /// { ...patch... } v` returning the buffer unconditionally — makes this
    /// test fail on the `is_none()` assertion, because a short packet would
    /// come back as a `Some` the caller then sends. Restored afterward.
    #[test]
    fn patch_seq_and_timestamp_refuses_a_packet_shorter_than_the_fixed_header() {
        // One byte short of the RFC 3550 §5.1 fixed header. Note this is
        // also long enough to have satisfied the old `>= 8` threshold,
        // which would have "patched" it into a packet with a truncated
        // SSRC — the second half of the defect.
        let short = vec![0u8; rtp_packet::FIXED_HEADER_LEN - 1];
        assert!(
            patch_seq_and_timestamp(&short, 42, 90_000).is_none(),
            "a packet shorter than the fixed header must be refused, not returned unpatched"
        );

        // Exactly the fixed header (no payload) is the shortest acceptable
        // input — the boundary itself is inclusive.
        let exact = vec![0u8; rtp_packet::FIXED_HEADER_LEN];
        let patched = patch_seq_and_timestamp(&exact, 42, 90_000)
            .expect("a bare fixed header is long enough to patch");
        assert_eq!(u16::from_be_bytes([patched[2], patched[3]]), 42);
        assert_eq!(
            u32::from_be_bytes([patched[4], patched[5], patched[6], patched[7]]),
            90_000
        );
    }

    fn nz(n: usize) -> std::num::NonZeroUsize {
        std::num::NonZeroUsize::new(n).unwrap()
    }

    fn empty_trunk() -> Arc<Trunk> {
        Trunk::new(media_plane::trunk::TrunkConfig::new(
            nz(4),
            nz(4),
            nz(4),
            nz(4),
            nz(4),
        ))
    }

    fn basic_verifier(username: &str, password: &str) -> Arc<Verifier> {
        Arc::new(Verifier::new(
            broadcast_auth::Credentials::Basic {
                username: username.to_string(),
                password: password.to_string(),
            },
            "whep-test",
        ))
    }

    /// PRE-FIX FAILURE OBSERVED: `run_whep`/`handle_whep_connection` had no
    /// `output_auth` parameter at all — an unauthenticated `POST` reached
    /// the offer parser unconditionally (the same response an authenticated
    /// request would eventually get), so the assertion that it must
    /// *instead* get a `401` failed immediately (the response started with
    /// `HTTP/1.1 409`, the empty-trunk "no track yet" response, not `401`).
    #[tokio::test]
    async fn whep_output_auth_rejects_unauthenticated_and_accepts_authenticated() {
        use base64::Engine as _;
        use tokio::io::AsyncReadExt;

        let verifier = basic_verifier("alice", "s3cret");
        let trunk = empty_trunk();
        let (tx, _rx) = mpsc::channel::<AdmittedWhep>(1);
        let active_sessions = Arc::new(AtomicUsize::new(0));

        // Unauthenticated request: must be 401, never reaching the offer
        // parser at all.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let request = format!(
            "POST /whep HTTP/1.1\r\nContent-Length: {}\r\n\r\n{OFFER}",
            OFFER.len()
        );
        client.write_all(request.as_bytes()).await.unwrap();
        handle_whep_connection(
            server,
            &trunk,
            &tx,
            &active_sessions,
            64,
            &Some(Arc::clone(&verifier)),
        )
        .await
        .expect("a rejected request is a normal 401 response, not a hard error");
        let mut resp = Vec::new();
        client.read_to_end(&mut resp).await.unwrap();
        let resp = String::from_utf8_lossy(&resp);
        assert!(
            resp.starts_with("HTTP/1.1 401"),
            "unauthenticated POST must be rejected: {resp}"
        );
        assert!(resp.contains("WWW-Authenticate"));

        // Correctly authenticated request: must get past the auth gate (the
        // empty trunk then answers 409 "no track yet" — the point under
        // test is that it is *not* 401).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let credential = base64::engine::general_purpose::STANDARD.encode("alice:s3cret");
        let request = format!(
            "POST /whep HTTP/1.1\r\nAuthorization: Basic {credential}\r\nContent-Length: {}\r\n\r\n{OFFER}",
            OFFER.len()
        );
        client.write_all(request.as_bytes()).await.unwrap();
        handle_whep_connection(server, &trunk, &tx, &active_sessions, 64, &Some(verifier))
            .await
            .expect("handle_whep_connection");
        let mut resp = Vec::new();
        client.read_to_end(&mut resp).await.unwrap();
        let resp = String::from_utf8_lossy(&resp);
        assert!(
            !resp.starts_with("HTTP/1.1 401"),
            "correctly authenticated POST must not be rejected: {resp}"
        );
    }

    fn test_track_spec() -> TrackSpec {
        TrackSpec::new(
            1,
            VIDEO_CLOCK_RATE,
            CodecConfig::Avc {
                config: transmux::AVCConfigurationBox::new(
                    transmux::AVCDecoderConfigurationRecord {
                        configuration_version: 1,
                        profile_indication: 0x42,
                        profile_compatibility: 0,
                        level_indication: 0x1f,
                        length_size_minus_one: 3,
                        sps: Vec::new(),
                        pps: Vec::new(),
                        chroma_format: None,
                        bit_depth_luma_minus8: None,
                        bit_depth_chroma_minus8: None,
                        sps_ext: Vec::new(),
                    },
                ),
                width: 0,
                height: 0,
            },
        )
    }

    async fn test_admitted_whep() -> AdmittedWhep {
        let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        s.set_nonblocking(true).unwrap();
        let media = MediaTransport::new(MediaTransportConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            local_ice_ufrag: rand_token(8),
            local_ice_pwd: rand_token(24),
            remote_ice_ufrag: rand_token(8),
            remote_ice_pwd: rand_token(24),
            is_controlling: false,
            local_setup: SetupRole::Passive,
            stun_server: None,
            remote_fingerprint: OFFER_FINGERPRINT.into(),
        })
        .unwrap();
        AdmittedWhep {
            socket: Arc::new(UdpSocket::from_std(s).unwrap()),
            media: Arc::new(TokioMutex::new(media)),
            spec: test_track_spec(),
            pt: 96,
            ssrc: rand_ssrc(),
        }
    }

    /// PRE-FIX FAILURE OBSERVED: `run_whep_session` had no silence bound at
    /// all — a session with no inbound datagram ran forever, so
    /// `active_sessions` never dropped back to 0 within any bounded wait;
    /// asserting it reached 0 within `2 * silence_timeout` timed out
    /// (the assertion below is what now catches that: the loop kept
    /// running past the deadline with the counter still at 1).
    #[tokio::test]
    async fn whep_session_ends_after_inbound_silence_timeout() {
        let admitted = test_admitted_whep().await;
        let trunk = empty_trunk();
        let cancel = CancellationToken::new();
        let active_sessions = Arc::new(AtomicUsize::new(1));
        let silence_timeout = Duration::from_millis(100);

        let handle = tokio::spawn(run_whep_session_with_silence_timeout(
            admitted,
            trunk,
            cancel,
            Arc::clone(&active_sessions),
            silence_timeout,
        ));

        tokio::time::timeout(silence_timeout * 10, handle)
            .await
            .expect("the session must end on its own within a few silence timeouts")
            .expect("run_whep_session_with_silence_timeout must not panic");

        assert_eq!(
            active_sessions.load(Ordering::Relaxed),
            0,
            "a session with no inbound traffic must free its slot after the silence timeout"
        );
    }

    fn trunk_with_avc_track() -> Arc<Trunk> {
        let trunk = empty_trunk();
        trunk.writer().unwrap().set_tracks(vec![test_track_spec()]);
        trunk
    }

    /// PRE-FIX FAILURE OBSERVED: with the manual `fetch_add`/`fetch_sub`
    /// this replaced (no decrement on any `?` between the capacity check
    /// and the successful `tx.send`), this assertion failed with
    /// `active_sessions.load() == 1` — the malformed-fingerprint failure
    /// left the reserved slot permanently held. The `SessionSlot` guard
    /// fixes every `?` on that path at once (`local_addr`, `UdpSocket::bind`,
    /// `MediaTransport::new`, the answer `write_all`), not just the one this
    /// specific test happens to exercise.
    #[tokio::test]
    async fn capacity_slot_is_released_when_media_transport_build_fails() {
        let offer = OFFER.replace(
            "a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n",
            "a=fingerprint:sha-256 00:11\r\n",
        );
        let trunk = trunk_with_avc_track();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let request = format!(
            "POST /whep HTTP/1.1\r\nContent-Length: {}\r\n\r\n{offer}",
            offer.len()
        );
        client.write_all(request.as_bytes()).await.unwrap();

        let (tx, _rx) = mpsc::channel::<AdmittedWhep>(1);
        let active_sessions = Arc::new(AtomicUsize::new(0));
        let result = handle_whep_connection(server, &trunk, &tx, &active_sessions, 4, &None).await;
        assert!(
            result.is_err(),
            "a malformed fingerprint must surface as an error, not a 201"
        );
        assert_eq!(
            active_sessions.load(Ordering::SeqCst),
            0,
            "the capacity slot must be released when setup fails after it was reserved"
        );
    }

    /// A session admitted successfully hands its slot off to the
    /// admitted-session channel's receiver, which decrements the counter
    /// exactly once when it later reaps that session — never twice.
    #[tokio::test]
    async fn capacity_slot_is_decremented_exactly_once_after_a_normal_session() {
        let trunk = trunk_with_avc_track();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let request = format!(
            "POST /whep HTTP/1.1\r\nContent-Length: {}\r\n\r\n{OFFER}",
            OFFER.len()
        );
        client.write_all(request.as_bytes()).await.unwrap();

        let (tx, mut rx) = mpsc::channel::<AdmittedWhep>(1);
        let active_sessions = Arc::new(AtomicUsize::new(0));
        handle_whep_connection(server, &trunk, &tx, &active_sessions, 4, &None)
            .await
            .expect("a well-formed offer is admitted");
        assert_eq!(
            active_sessions.load(Ordering::SeqCst),
            1,
            "the slot stays reserved for the live session"
        );

        // Stands in for `run_whep_session`'s own decrement once the session
        // ends — the one place that owns the slot from here on.
        let _admitted = rx.recv().await.expect("the session was admitted");
        active_sessions.fetch_sub(1, Ordering::SeqCst);
        assert_eq!(
            active_sessions.load(Ordering::SeqCst),
            0,
            "exactly one decrement must bring the counter back to 0 — no leak, no double-release"
        );
    }

    /// Builds a STUN Binding Request (RFC 5389 §6) carrying a `USERNAME`
    /// attribute (§15.3, `"{local_ufrag}:{remote_ufrag}"`) and, if
    /// `integrity` is `Some`, a syntactically-complete but wrong
    /// `MESSAGE-INTEGRITY` attribute (§15.4: type `0x0008`, a 20-byte
    /// HMAC-SHA1 that is simply never the real one) — real enough for the
    /// ICE agent to recognize as addressed to it, wrong enough to fail
    /// authentication either way.
    fn stun_binding_request(username: &[u8], integrity: bool) -> Vec<u8> {
        let mut pkt = Vec::new();
        pkt.extend_from_slice(&[0x00, 0x01]); // Binding Request
        let username_attr_len = 4 + username.len().div_ceil(4) * 4;
        let total_attr_len = if integrity {
            username_attr_len + 4 + 20
        } else {
            username_attr_len
        };
        pkt.extend_from_slice(&(total_attr_len as u16).to_be_bytes());
        pkt.extend_from_slice(&[0x21, 0x12, 0xA4, 0x42]); // magic cookie
        pkt.extend_from_slice(&[0u8; 12]); // transaction id
        pkt.extend_from_slice(&[0x00, 0x06]); // USERNAME
        pkt.extend_from_slice(&(username.len() as u16).to_be_bytes());
        pkt.extend_from_slice(username);
        while pkt.len() % 4 != 0 {
            pkt.push(0);
        }
        if integrity {
            pkt.extend_from_slice(&[0x00, 0x08, 0x00, 0x14]); // MESSAGE-INTEGRITY, len 20
            pkt.extend_from_slice(&[0xEE; 20]); // wrong HMAC-SHA1
        }
        pkt
    }

    /// PINS real `webrtc-runtime` behavior this crate's fix relies on
    /// (issue r07-C11 follow-up) — a coordinator review asked whether a
    /// STUN message that fails message-integrity is "swallowed as `Ok`" by
    /// the ICE agent the same way an off-pair DTLS datagram is. It is not:
    /// both a `USERNAME`-only request (missing the required
    /// `MESSAGE-INTEGRITY` attribute entirely) and one carrying a
    /// syntactically complete but wrong `MESSAGE-INTEGRITY` come back as
    /// `Err`, handled by `run_whep_session`'s *existing*, already-correct
    /// pre-handshake-failure-is-fatal path — never by the `Ok(events)`
    /// branch [`is_liveness_event`] guards. Only a datagram in the DTLS
    /// content-type band from an address that isn't the ICE-selected pair
    /// is the confirmed `Ok(())`-with-no-event case (`handle_dtls_datagram`
    /// says so directly), which is why the sibling test below drives its
    /// stray traffic with that shape (and a plain out-of-range byte)
    /// instead of a forged STUN message.
    #[test]
    fn stun_integrity_failures_surface_as_err_not_ok() {
        let mut media = MediaTransport::new(MediaTransportConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            local_ice_ufrag: "localuf01".into(),
            local_ice_pwd: "local-ice-password-000000".into(),
            remote_ice_ufrag: "remoteuf01".into(),
            remote_ice_pwd: "remote-ice-password-00000".into(),
            is_controlling: false,
            local_setup: SetupRole::Passive,
            stun_server: None,
            remote_fingerprint: OFFER_FINGERPRINT.into(),
        })
        .unwrap();
        let from: SocketAddr = "127.0.0.1:9".parse().unwrap();
        let username = b"localuf01:remoteuf01";

        assert!(
            media
                .handle_datagram(Instant::now(), from, &stun_binding_request(username, false))
                .is_err(),
            "a Binding Request missing MESSAGE-INTEGRITY entirely must be Err"
        );
        assert!(
            media
                .handle_datagram(Instant::now(), from, &stun_binding_request(username, true))
                .is_err(),
            "a Binding Request with a wrong MESSAGE-INTEGRITY must be Err"
        );
    }

    /// Pins [`is_liveness_event`]'s exact decision — the fix this round
    /// makes — deterministically, against hand-constructed events, rather
    /// than relying only on the integration test's ability to provoke the
    /// right event shape from a real transport. `MediaEvent::Rtcp` isn't
    /// exercised here (building one needs a real `rtcp_packet::
    /// CompoundPacket`, which would add a new direct dependency this crate
    /// doesn't otherwise need); `Rtp`/`DtlsHandshakeComplete` and the two
    /// ICE-bookkeeping variants cover the actual decision boundary.
    #[test]
    fn is_liveness_event_matches_only_rtp_rtcp_and_handshake_complete() {
        let rtp = MediaEvent::Rtp(webrtc_runtime::media::DecryptedRtp {
            marker: false,
            payload_type: 96,
            sequence_number: 1,
            timestamp: 0,
            ssrc: 1,
            csrc: Vec::new(),
            payload: vec![0xAA],
        });
        assert!(is_liveness_event(&rtp), "a decrypted RTP packet counts");
        assert!(
            is_liveness_event(&MediaEvent::DtlsHandshakeComplete),
            "the one-time handshake completion counts"
        );
        assert!(
            !is_liveness_event(&MediaEvent::IceStateChanged("connected".into())),
            "an ICE connection-state transition alone must not count"
        );
        assert!(
            !is_liveness_event(&MediaEvent::LocalCandidateGathered(
                "1 1 udp 2130706431 127.0.0.1 9 typ srflx".into()
            )),
            "gathering a local candidate alone must not count"
        );
    }

    /// A datagram from a third party — never the session's real (and here,
    /// never-established) peer — must not keep the session alive. Sent
    /// continuously for `3 * silence_timeout`, concurrently with the
    /// session task itself (started at the same instant, so a bug that
    /// treats either shape as liveness would need the *whole* spam window
    /// plus another `silence_timeout` to end — well past `assert_bound`
    /// below): a DTLS Handshake content-type byte (`handle_dtls_datagram`
    /// rejects any address but the selected pair with `Ok(())`, no event —
    /// this session never even ran ICE, so `selected_pair_addr` is always
    /// `None`) and a plain out-of-range byte (no defined meaning in any
    /// demux band, silently ignored). See
    /// `stun_integrity_failures_surface_as_err_not_ok` for why a forged
    /// STUN message isn't in this rotation: it surfaces as `Err`, which
    /// (correctly, and not what this test is about) already ends a
    /// pre-handshake session immediately via the sibling branch.
    ///
    /// PRE-FIX FAILURE OBSERVED (round 3): reverting `is_liveness_event`'s
    /// call site back to round 2's `!events.is_empty()` did **not** make
    /// this test fail — confirming, empirically, that neither stray shape
    /// here ever produced a non-empty `Ok` event set even under that
    /// broader condition (both are the same `Ok(())`/silently-ignored
    /// cases either way). The real round-2 gap (`IceStateChanged`/
    /// `LocalCandidateGathered` from *genuine* ICE bookkeeping counting as
    /// liveness) is not reachable by any datagram an outside party can
    /// forge — see this test's own doc for the two shapes that were tried
    /// and ruled out. `is_liveness_event`'s own unit tests below pin the
    /// exact decision this round's fix makes, deterministically.
    #[tokio::test]
    async fn stray_datagrams_from_another_address_do_not_extend_the_silence_timeout() {
        let admitted = test_admitted_whep().await;
        let session_addr = admitted.socket.local_addr().unwrap();
        let trunk = empty_trunk();
        let cancel = CancellationToken::new();
        let active_sessions = Arc::new(AtomicUsize::new(1));
        let silence_timeout = Duration::from_millis(200);
        // Comfortably more than the correct end time (~`silence_timeout`)
        // and comfortably less than what a bug extending life via the spam
        // would need (`spam_duration + silence_timeout`) — see this test's
        // own doc.
        let assert_bound = Duration::from_millis(400);
        let spam_duration = silence_timeout * 3;

        let handle = tokio::spawn(run_whep_session_with_silence_timeout(
            admitted,
            trunk,
            cancel,
            Arc::clone(&active_sessions),
            silence_timeout,
        ));

        // Random bytes whose first byte falls outside every demux range
        // (STUN <=1, DTLS 20..=63, RTP/RTCP 128..=191) — "no defined
        // meaning", silently ignored.
        const RANDOM_BYTES: &[u8] = &[0x05, 0xAA, 0xBB, 0xCC];
        // A DTLS Handshake content-type byte (RFC 6347 §4.1): always
        // rejected — this session never selected any pair, let alone one
        // at the stray socket's address.
        const DTLS_LOOKING: &[u8] = &[0x16, 0x00, 0, 0, 0, 0, 0, 0];

        // Spams concurrently with the session task above (both started at
        // the same instant) rather than before awaiting it — the whole
        // point is that `assert_bound` below is too tight for a bug that
        // extends life on each stray to still finish in time.
        let spam = tokio::spawn(async move {
            let stray_std = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            stray_std.set_nonblocking(true).unwrap();
            let stray = UdpSocket::from_std(stray_std).unwrap();
            let deadline = Instant::now() + spam_duration;
            let mut i: u32 = 0;
            while Instant::now() < deadline {
                let payload = if i.is_multiple_of(2) {
                    RANDOM_BYTES
                } else {
                    DTLS_LOOKING
                };
                let _ = stray.send_to(payload, session_addr).await;
                i = i.wrapping_add(1);
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        });

        tokio::time::timeout(assert_bound, handle)
            .await
            .expect(
                "the session must end within assert_bound despite continued stray traffic from \
                 another address",
            )
            .expect("run_whep_session_with_silence_timeout must not panic");

        assert_eq!(
            active_sessions.load(Ordering::Relaxed),
            0,
            "stray datagrams from another address must not keep a session alive past its \
             silence timeout"
        );
        spam.abort();
    }
}
