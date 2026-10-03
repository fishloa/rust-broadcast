//! [`MediaTransport`] — see the `media` module doc for the full picture.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use broadcast_common::{Parse, Serialize};
use bytes::BytesMut;
use rtc_dtls::config::{ClientAuthType, ConfigBuilder, HandshakeConfig, VerifyPeerCertificateFn};
use rtc_dtls::crypto::Certificate;
use rtc_dtls::crypto_provider::{RTCCryptoProvider, default_provider};
use rtc_dtls::endpoint::{Endpoint as DtlsEndpoint, EndpointEvent};
use rtc_dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use rtc_ice::agent::agent_config::AgentConfig;
use rtc_ice::agent::{Agent as IceAgent, Event as IceAgentEvent};
use rtc_ice::candidate::candidate_host::CandidateHostConfig;
use rtc_ice::candidate::candidate_server_reflexive::CandidateServerReflexiveConfig;
use rtc_ice::candidate::{CandidateConfig, CandidateType, unmarshal_candidate};
use rtc_ice::mdns::MulticastDnsMode;
use rtc_shared::error::Error as SharedError;
use rtc_shared::{EcnCodepoint, TaggedBytesMut, TransportContext, TransportProtocol};
use rtc_srtp::context::Context as SrtpContext;
use rtc_srtp::protection_profile::ProtectionProfile;
use sansio::Protocol;
use sha2::{Digest, Sha256};

use crate::Error;
use crate::media::gather::StunGather;

// ---------------------------------------------------------------------------
// Demux constants — see the `media` module doc for the RFC 5764 §5.1.2 /
// RFC 5761 §4 citations these encode.
// ---------------------------------------------------------------------------

/// RFC 5764 §5.1.2: a first byte of 0 or 1 is STUN.
const DEMUX_STUN_MAX: u8 = 1;
/// RFC 5764 §5.1.2: a first byte of 20-63 (inclusive) is DTLS.
const DEMUX_DTLS_MIN: u8 = 20;
const DEMUX_DTLS_MAX: u8 = 63;
/// RFC 5764 §5.1.2: a first byte of 128-191 (inclusive) is RTP or RTCP.
const DEMUX_RTP_MIN: u8 = 128;
const DEMUX_RTP_MAX: u8 = 191;
/// RFC 5761 §4: RTCP packet types occupy `[192, 223]` today, with `[224,
/// 254]` reserved as a SHOULD-only-when-exhausted fallback band for future
/// IANA allocations — see the `media` module doc and
/// `docs/rfc5761-rtcp-mux.md` §3 for the full citation. This range is
/// widened to include `[224, 254]` (issue #948 item 3): a legitimately
/// registered future RTCP packet type in that band would otherwise be
/// silently misclassified as RTP by [`is_rtcp_packet_type`].
///
/// `[1, 191]` (the *other* SHOULD-only-when-exhausted band RFC 5761 §4
/// names) is deliberately **not** folded in here: unlike `[224, 254]`, that
/// range fully overlaps the RTP marker-bit-clear byte value (`M=0` ⇒
/// `byte1 == PT`, `PT` in `0..=127`), so treating it as RTCP would
/// misclassify essentially all unmarked RTP traffic — a live break, not a
/// theoretical one.
///
/// `[224, 254]` was tried and REVERTED. RFC 5761 §4 does list it as a valid
/// RTCP band, but it is a "SHOULD only be used when other values have been
/// exhausted" last resort with nothing registered in it — while the aliasing
/// it collides with is ubiquitous. A dynamic RTP payload type in `96..=126`
/// with the marker bit set produces `byte1` in `224..=254`: e.g. Opus at
/// PT 111 with `M=1` is `0x80 | 111 = 239`. That is the exact packet shape
/// this crate has verified decrypting from a real browser, and widening the
/// range routed it to `decrypt_rtcp`, breaking it.
///
/// So the demux deliberately covers `[192, 223]` only: the band where RTCP is
/// actually assigned, and the one RFC 5761 §4 protects by forbidding RTP
/// payload types `64..=95`. An RTCP type in `[224, 254]` would be
/// misclassified — accepted, because none exists and the alternative breaks
/// live traffic.
const RTCP_MUX_TYPE_MIN: u8 = 192;
const RTCP_MUX_TYPE_MAX: u8 = 223;

/// RFC 5761 §4: true if `byte1` (the second octet of a packet already known
/// to be in the RTP/RTCP-multiplexed SRTP/SRTCP band, see [`DEMUX_RTP_MIN`]/
/// [`DEMUX_RTP_MAX`]) falls in the range reserved for RTCP packet types
/// rather than an RTP marker-bit + payload-type byte.
fn is_rtcp_packet_type(byte1: u8) -> bool {
    (RTCP_MUX_TYPE_MIN..=RTCP_MUX_TYPE_MAX).contains(&byte1)
}

/// The SRTP protection profile this transport offers in its DTLS handshake
/// (RFC 5764 §4.1.2): `SRTP_AES128_CM_HMAC_SHA1_80`, the one profile every
/// WebRTC implementation is required to support (see
/// `rtc_srtp::protection_profile::ProtectionProfile::Aes128CmHmacSha1_80`'s
/// own doc). Not configurable in this cut — see [`MediaTransportConfig`].
const OFFERED_SRTP_PROFILE: SrtpProtectionProfile =
    SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80;

/// The label used to export SRTP keying material from a completed DTLS
/// handshake (RFC 5764 §4.2).
const SRTP_KEYING_MATERIAL_LABEL: &str = "EXTRACTOR-dtls_srtp";

// ---------------------------------------------------------------------------
// SDP certificate fingerprint (RFC 8122 §5) — the identity WebRTC actually
// authenticates peers with, in place of a CA chain (RFC 5764 §5 "Identity
// Checks"). See the `media` module doc.
// ---------------------------------------------------------------------------

/// The hash-function token for the SHA-256 fingerprint (RFC 8122 §5,
/// RFC 8827 §5). SHA-256 is the one mandatory-to-implement algorithm
/// (RFC 8827 §6.5), so it is the only token accepted here.
const FINGERPRINT_HASH_TOKEN: &str = "sha-256";

/// The SHA-256 digest length in bytes — also the number of colon-separated
/// hex bytes an `a=fingerprint:sha-256` value carries (RFC 8122 §5).
const FINGERPRINT_LEN: usize = 32;

/// The recommended cap on the number of remote ICE candidates one
/// [`MediaTransport`] admits (RFC 8445 §6.1.2.4-6.1.2.5: 100 candidate pairs
/// per checklist set, "specifically to bound" the amplification-style issue
/// in §19.5.1). The spec requires this limit be configurable, so it is not
/// hardcoded: it is only the default a caller passes as
/// [`MediaTransportConfig::max_remote_candidates`]. `rtc-ice` 0.20.0 has no
/// pair cap of its own, so it is enforced here on remote candidates instead:
/// since local candidates are the small, fixed set this crate gathers per
/// transport (host + at most one server-reflexive candidate), bounding
/// remote candidates bounds the pair count too.
pub const MAX_REMOTE_CANDIDATES: usize = 100;

/// Parses an SDP `a=fingerprint` attribute *value* (`"sha-256 AB:CD:…"`,
/// RFC 8122 §5) into its raw digest: the hash-function token (case-
/// insensitive, and only [`FINGERPRINT_HASH_TOKEN`] is accepted — SHA-256
/// is mandatory per RFC 8827 §6.5), then exactly 32 colon-separated hex
/// bytes (case-insensitive). Anything else is an `Err` describing why.
fn parse_fingerprint_value(value: &str) -> Result<[u8; FINGERPRINT_LEN], String> {
    let mut parts = value.split_whitespace();
    let token = parts.next().unwrap_or_default();
    if !token.eq_ignore_ascii_case(FINGERPRINT_HASH_TOKEN) {
        return Err(format!(
            "remote_fingerprint {value:?} must use the {FINGERPRINT_HASH_TOKEN} hash function \
             (RFC 8122 §5, RFC 8827 §6.5)"
        ));
    }
    let Some(hex) = parts.next() else {
        return Err(format!(
            "remote_fingerprint {value:?} has no digest after the {FINGERPRINT_HASH_TOKEN} token"
        ));
    };
    if parts.next().is_some() {
        return Err(format!("remote_fingerprint {value:?} has trailing data"));
    }
    let bytes: Vec<&str> = hex.split(':').collect();
    if bytes.len() != FINGERPRINT_LEN {
        return Err(format!(
            "remote_fingerprint digest must be exactly {FINGERPRINT_LEN} colon-separated hex \
             bytes, got {}",
            bytes.len()
        ));
    }
    let mut digest = [0u8; FINGERPRINT_LEN];
    for (slot, part) in digest.iter_mut().zip(&bytes) {
        if part.len() != 2 {
            return Err(format!(
                "remote_fingerprint digest byte {part:?} is not exactly two hex digits"
            ));
        }
        *slot = u8::from_str_radix(part, 16)
            .map_err(|e| format!("remote_fingerprint digest byte {part:?}: {e}"))?;
    }
    Ok(digest)
}

/// Equality of two digests without an early-exit loop: fold the XOR of every
/// byte pair into one accumulator and test it only at the end, so the time
/// taken cannot reveal how many leading bytes of a guess were right.
fn digests_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut accumulator = 0u8;
    for (x, y) in a.iter().zip(b) {
        accumulator |= x ^ y;
    }
    accumulator == 0
}

/// True iff `peer_certs` is non-empty and the SHA-256 of its leaf
/// certificate (`peer_certs[0]`, RFC 8122 §4: the fingerprint is of the
/// leaf, the chain follows it) equals `expected`. An empty list fails —
/// there is nothing to authenticate against.
fn peer_cert_fingerprint_ok(peer_certs: &[Vec<u8>], expected: &[u8; FINGERPRINT_LEN]) -> bool {
    peer_certs
        .first()
        .is_some_and(|leaf| digests_equal(&Sha256::digest(leaf), expected))
}

/// Read the remote SDP's DTLS certificate fingerprint: the value of its
/// first `a=fingerprint:` line (RFC 8122 §5), e.g. `"sha-256 AB:CD:…"`.
///
/// Media-level sections are searched before session-level lines: a bundled
/// offer signals the fingerprint per `m=` section and those are the ones a
/// peer actually commits to, so a media-level value wins when both exist
/// (the attribute is legal at either level, RFC 8866 §5.13). Returns `None`
/// only when the SDP carries no `a=fingerprint` anywhere — callers must
/// then reject the session rather than build a transport that could never
/// be verified. Pair with [`MediaTransportConfig::remote_fingerprint`],
/// which validates the returned value's shape at construction time.
pub fn parse_remote_fingerprint(sdp: &str) -> Option<String> {
    let mut media_level: Option<String> = None;
    let mut session_level: Option<String> = None;
    let mut in_media = false;
    for line in sdp.lines() {
        if let Some(value) = line.strip_prefix("a=fingerprint:") {
            if in_media {
                if media_level.is_none() {
                    media_level = Some(value.trim().to_string());
                }
            } else if session_level.is_none() {
                session_level = Some(value.trim().to_string());
            }
        } else if line.starts_with("m=") {
            in_media = true;
        }
    }
    media_level.or(session_level)
}

// ---------------------------------------------------------------------------
// Key lifetime / rekey (issue #948 item 3) — RFC 3711 §8.2/§9.2, RFC 5764
// §4.4/§5.2. See the `media` module doc's "Key lifetime and rekey" section
// for the full picture, including the one place this deliberately falls
// short of §5.2's illustrative in-band-renegotiation text (this crate uses
// §4.4's own "a new DTLS session SHOULD be used" mechanism instead —
// `rtc-dtls` 0.20.0 exposes no API for the former).
// ---------------------------------------------------------------------------

/// RFC 3711 §8.2's Key Management Parameters table lists two *separate*
/// per-master-key packet-count limits: `SRTP-packets-max-lifetime = 2^48`
/// and `SRTCP-packets-max-lifetime = 2^31`. §9.2 ("Key Usage") explains why
/// only one of them needs tracking in practice when (as here, the default
/// per §4.3) SRTP and SRTCP session keys share one master key:
///
/// > when 2^48 SRTP packets or 2^31 SRTCP packets have been secured with
/// > the same key (whichever occurs before), the key management MUST be
/// > called to provide new master key(s) (previously stored and used keys
/// > MUST NOT be used again), or the session MUST be terminated.
///
/// i.e. the *binding* limit is `min(2^48, 2^31) == 2^31`, always. This
/// crate applies that single 2^31 figure as [`MediaTransport::needs_rekey`]'s
/// trigger for both the RTP and RTCP packet counters — exact for RTCP,
/// conservative for RTP (whose own bound is the looser 2^48, never actually
/// reached first per §9.2's own worked example). RFC 5764 §4.4 ("Key Usage
/// Limitations") names this same figure `maximum_lifetime` and points at its
/// own §4.1.2 protection-profile table (`docs/rfc5764-dtls-srtp.md` §2.1),
/// which independently tabulates 2^31 as "max lifetime" for every profile it
/// defines, including [`OFFERED_SRTP_PROFILE`].
const MAXIMUM_LIFETIME_PACKETS: u64 = 1 << 31;

/// RFC 5764 §5.2 ("Rehandshake and Rekey"):
///
/// > Because of packet reordering, packets protected by the previous set
/// > of keys can appear on the wire after the handshake has completed. To
/// > compensate for this fact, receivers SHOULD maintain both sets of keys
/// > for some time in order to be able to decrypt and verify older
/// > packets. The keys should be maintained for the duration of the
/// > maximum segment lifetime (MSL).
///
/// RFC 5764 does not itself give MSL a numeric value. This crate uses the
/// same 2-minute figure RFC 793 §3.3 assumes for TCP's own MSL — the
/// conventional reading of "MSL" industry-wide when the citing spec (as
/// here) doesn't repeat the number.
const RETIRED_KEY_RETENTION: Duration = Duration::from_secs(120);

/// True once `count` has reached [`MAXIMUM_LIFETIME_PACKETS`] — pulled out
/// of [`MediaTransport::needs_rekey`] as a pure function so a test can hit
/// the exact boundary without actually sending two billion packets.
fn exceeds_maximum_lifetime(count: u64) -> bool {
    count >= MAXIMUM_LIFETIME_PACKETS
}

/// The DTLS-SRTP "setup" role (RFC 8842 §4.1, obsoleting RFC 4145 §5): which
/// side of the DTLS handshake a peer takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SetupRole {
    /// This side is the DTLS client (`a=setup:active`).
    Active,
    /// This side is the DTLS server (`a=setup:passive`).
    Passive,
    /// Either role is acceptable. Valid only in an SDP *offer*'s `a=setup`
    /// value (RFC 8842 §4.1) — never a value [`MediaTransport`] can
    /// actually be built with (see [`MediaTransport::new`]).
    ActPass,
}

impl SetupRole {
    /// The SDP `a=setup` attribute token (RFC 8842 §4.1).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Passive => "passive",
            Self::ActPass => "actpass",
        }
    }
}

broadcast_common::impl_spec_display!(SetupRole);

/// Configuration for a [`MediaTransport`].
///
/// SDP parsing/generation is out of scope for this crate (see the `media`
/// module doc): every field is a value the caller already pulled out of a
/// negotiated SDP offer/answer.
#[derive(Debug, Clone)]
pub struct MediaTransportConfig {
    /// The local UDP socket address media is sent from/received on.
    pub local_addr: SocketAddr,
    /// This side's ICE username fragment (RFC 8445 §5.3), signalled as
    /// `a=ice-ufrag` in the local SDP.
    pub local_ice_ufrag: String,
    /// This side's ICE password (RFC 8445 §5.3), signalled as `a=ice-pwd`.
    pub local_ice_pwd: String,
    /// The remote peer's `a=ice-ufrag` value.
    pub remote_ice_ufrag: String,
    /// The remote peer's `a=ice-pwd` value.
    pub remote_ice_pwd: String,
    /// Whether this side is the ICE controlling agent (RFC 8445 §4). The
    /// WHIP/WHEP offerer is conventionally controlling; a media server
    /// answering an offer is conventionally controlled (`false`).
    pub is_controlling: bool,
    /// This side's DTLS role.
    ///
    /// [`SetupRole::Passive`] (the DTLS server) and [`SetupRole::Active`]
    /// (the DTLS client) are both implemented by [`MediaTransport::new`];
    /// [`SetupRole::ActPass`] is not — it is only ever valid as an SDP
    /// *offer's* `a=setup` value (RFC 8842 §4.1), never a role either side
    /// actually settles into once the answer picks a concrete side.
    pub local_setup: SetupRole,
    /// The remote peer's DTLS certificate fingerprint from its SDP
    /// `a=fingerprint` attribute (RFC 8122 §5), e.g. `"sha-256 AB:CD:…"`.
    /// The DTLS handshake is rejected unless the peer's leaf certificate
    /// hashes to exactly this value.
    pub remote_fingerprint: String,
    /// A STUN server to gather a server-reflexive candidate from, if any
    /// (RFC 8445 §5.1.1.2). `None` gathers a host candidate only.
    pub stun_server: Option<SocketAddr>,
    /// The cap on remote ICE candidates [`MediaTransport::add_remote_candidate`]
    /// admits before it starts rejecting further ones (RFC 8445 §6.1.2.5,
    /// which requires a configurable, enforced limit — see
    /// [`MAX_REMOTE_CANDIDATES`] for the recommended default and the
    /// amplification concern it bounds).
    pub max_remote_candidates: usize,
}

/// The RFC 3550 §5.3.1 header extension carried by a [`DecryptedRtp`]
/// packet, if `X=1`: an owned copy of [`rtp_packet::HeaderExtension`], whose
/// `data` borrows from the (transient, decrypt-local) plaintext buffer —
/// `DecryptedRtp` itself is fully owned, so it needs its own copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedRtpExtension {
    /// `defined by profile` — a 16-bit identifier whose meaning is entirely
    /// profile-specific (§5.3.1), e.g. RFC 8285 one-/two-byte multiplexing,
    /// or a single-purpose extension such as RFC 5285's predecessor uses
    /// (CVO video orientation, AV1 dependency descriptor, `mid`/`rid`).
    pub profile_id: u16,
    /// The extension data, opaque at this layer (RFC 3550 §5.3.1: "the
    /// actual format of the extension is specified by the profile").
    pub data: Vec<u8>,
}

/// An RTP packet decrypted from an inbound SRTP packet (RFC 3711), with its
/// RFC 3550 §5.1 fixed-header fields promoted to typed fields. `payload` is
/// the still-opaque coded media — this crate never decodes it, the same
/// convention `rtp-packet` itself documents for [`rtp_packet::RtpPacket`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedRtp {
    /// `marker (M)` (RFC 3550 §5.1).
    pub marker: bool,
    /// `payload type (PT)`, 7 bits (RFC 3550 §5.1).
    pub payload_type: u8,
    /// `sequence number` (RFC 3550 §5.1).
    pub sequence_number: u16,
    /// `timestamp` (RFC 3550 §5.1).
    pub timestamp: u32,
    /// `SSRC` — synchronization source identifier (RFC 3550 §5.1).
    pub ssrc: u32,
    /// The CSRC identifier list (RFC 3550 §5.1).
    pub csrc: Vec<u32>,
    /// The §5.3.1 header extension, if `X=1` (RFC 8285 CVO/AV1-dependency-
    /// descriptor/`mid`/`rid` and similar are carried this way; audit
    /// run-09 W20 — this used to be dropped between decrypt and the caller).
    pub extension: Option<DecryptedRtpExtension>,
    /// The opaque coded media payload.
    pub payload: Vec<u8>,
}

/// One outbound UDP datagram [`MediaTransport::poll_transmit`] wants sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Datagram {
    /// Destination address.
    pub peer: SocketAddr,
    /// The datagram bytes.
    pub bytes: Vec<u8>,
}

/// Events [`MediaTransport`] reports to its caller.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum MediaEvent {
    /// A new local candidate finished gathering (currently: the
    /// server-reflexive candidate, once STUN resolves it). The string is
    /// the ICE candidate-attribute body (RFC 8839 §5.1) *without* the
    /// leading `a=candidate:` — [`rtc_ice::candidate::Candidate::marshal`]'s
    /// own format — for the caller to fold into a Trickle-ICE fragment.
    LocalCandidateGathered(String),
    /// The ICE agent's connection state changed. Carries
    /// `rtc_ice::state::ConnectionState`'s own `Display` text (that type is
    /// `rtc-ice`'s, not this crate's, so it does not get its own
    /// `name()`/`impl_spec_display!` pair here).
    IceStateChanged(String),
    /// The DTLS handshake completed and the SRTP decrypt context is ready;
    /// inbound [`MediaEvent::Rtp`]/[`MediaEvent::Rtcp`] events can now be
    /// produced.
    DtlsHandshakeComplete,
    /// A decrypted, parsed inbound RTP packet.
    Rtp(DecryptedRtp),
    /// A decrypted inbound RTCP compound packet (RFC 3550 §6.1), parsed by
    /// the workspace's own `rtcp-packet` crate.
    Rtcp(rtcp_packet::CompoundPacket),
    /// An inbound SRTCP packet that decrypted and **authenticated** under
    /// the negotiated key (RFC 3711 §3.4), but whose plaintext is not a
    /// well-formed RFC 3550 §6 compound `rtcp-packet` can parse at all —
    /// e.g. a bad version field, a truncated packet, or a leading
    /// SDES/BYE/APP not preceded by a report. It is a genuine packet from
    /// the authenticated peer (so, e.g., proof the peer is still live), not
    /// a transport error; carries the parse error that explains why it was
    /// not surfaced as [`MediaEvent::Rtcp`].
    ///
    /// This is now the **rare** case: the RFC 4585 RTPFB/PSFB feedback
    /// (NACK, PLI, REMB, transport-cc) or RFC 3611 XR packets a browser
    /// receiver sends continuously — including as the *only* packet in a
    /// Reduced-Size RTCP datagram (RFC 5506 §4.1) — decode fine as
    /// [`MediaEvent::Rtcp`] via `rtcp_packet::RtcpPacket::Unknown`, so an SR/RR
    /// sharing that datagram is no longer discarded along with them (#1071).
    RtcpUnsupported(rtcp_packet::Error),
    /// An inbound datagram in the RFC 5764 §5.1.2 SRTP/SRTCP band
    /// (first byte 128..=191) failed authentication under both the
    /// current and (if any) retired read key.
    ///
    /// This is the EXPECTED outcome for spoofed or garbage traffic, not a
    /// transport error: post-handshake, this crate accepts datagrams from
    /// any source address (RFC 8445 gives no further authentication once a
    /// pair is selected — SRTP's own auth tag is the check), so anyone can
    /// send a byte in this range to the port. Before this event existed,
    /// [`MediaTransport::handle_datagram`] returned `Err` for this case,
    /// making it a fatal-looking result for every caller (audit run-09 W20;
    /// `multimux` already had to special-case exactly this after it tore
    /// down live WHIP ingests on the first stray datagram).
    AuthFailure {
        /// `true` for an RFC 5761 §4 RTCP packet type, `false` for RTP.
        is_rtcp: bool,
        /// The decrypt/parse failure, for logging.
        reason: String,
    },
    /// [`MediaTransport::handle_timeout`]'s underlying ICE or DTLS timer
    /// drive returned an error (audit run-09 W20). Before this event
    /// existed these were silently discarded (`let _ =`), so a failed DTLS
    /// handshake (fatal alert, retransmit exhaustion) or an ICE agent error
    /// showed up only as silence — the caller never learned anything had
    /// gone wrong.
    TimerError(String),
}

/// The ICE + DTLS-SRTP media transport for one peer connection.
///
/// Owns no socket: [`Self::poll_transmit`] / [`Self::handle_datagram`] /
/// [`Self::handle_timeout`] are the whole IO surface — see the `media`
/// module doc for the push/pull shape and the demultiplexing rules applied
/// in [`Self::handle_datagram`].
pub struct MediaTransport {
    local_addr: SocketAddr,
    local_fingerprint: String,
    /// The parsed [`MediaTransportConfig::remote_fingerprint`] digest: the
    /// SHA-256 the peer's leaf certificate must hash to (RFC 8122 §5,
    /// RFC 5764 §5). Checked in the DTLS verify callback and re-checked on
    /// every handshake completion before any SRTP key is derived.
    remote_fingerprint_digest: [u8; FINGERPRINT_LEN],
    local_setup: SetupRole,
    ice: IceAgent,
    dtls: DtlsEndpoint,
    /// Set only for [`SetupRole::Active`]: the client-role handshake config
    /// [`Self::maybe_start_active_dtls`] hands to [`DtlsEndpoint::connect`]
    /// once ICE has nominated a pair (see that method's doc). `None` for
    /// [`SetupRole::Passive`], which never dials out.
    dtls_client_config: Option<Arc<HandshakeConfig>>,
    /// The peer address of the current (or most recent) DTLS association,
    /// set once a handshake completes. [`Self::rekey`] needs it to tear
    /// down that association and, for [`SetupRole::Active`], redial a
    /// fresh one.
    dtls_peer: Option<SocketAddr>,
    /// The remote address of the selected ICE candidate pair (RFC 8445
    /// §6.2.1), recorded from the same `SelectedCandidatePairChange` event
    /// that starts an Active-role dial. DTLS is accepted only from this
    /// address — see [`Self::handle_dtls_datagram`]. `None` until ICE
    /// selects a pair, which means all DTLS datagrams are dropped.
    selected_pair_addr: Option<SocketAddr>,
    srtp_read: Option<SrtpContext>,
    srtp_write: Option<SrtpContext>,
    /// RFC 3711 §9.2 packet counters for the *current* `srtp_write`/
    /// `srtp_read` key epoch — see [`MAXIMUM_LIFETIME_PACKETS`] and
    /// [`Self::needs_rekey`]. Reset to zero by [`Self::rekey`].
    write_rtp_count: u64,
    write_rtcp_count: u64,
    read_rtp_count: u64,
    read_rtcp_count: u64,
    /// RFC 5764 §5.2: the read context [`Self::rekey`] just retired,
    /// paired with the [`Instant`] it must be dropped by (the rekey time
    /// plus [`RETIRED_KEY_RETENTION`]) so a reordered packet keyed under
    /// it can still be decrypted in the meantime. Purged by
    /// [`Self::purge_expired_retired_key`].
    retired_srtp_read: Option<(SrtpContext, Instant)>,
    gather: Option<StunGather>,
    /// The crypto provider (`rtc-crypto` 0.21 takes it explicitly everywhere:
    /// ICE agent, DTLS config/certificate, SRTP contexts), resolved once from
    /// the feature-selected default at construction.
    crypto_provider: Arc<dyn RTCCryptoProvider>,
    /// Remote candidates admitted so far via [`Self::add_remote_candidate`],
    /// capped at [`Self::max_remote_candidates`]. `rtc-ice`'s `IceAgent`
    /// keeps its own remote-candidate list privately (no public getter), so
    /// this is the only place that count is observable from here.
    remote_candidate_count: usize,
    /// This transport's copy of [`MediaTransportConfig::max_remote_candidates`]
    /// (RFC 8445 §6.1.2.5's configurable cap).
    max_remote_candidates: usize,
    /// Every distinct remote `SocketAddr` already counted toward
    /// [`Self::max_remote_candidates`]: the address of each explicitly added
    /// remote candidate ([`Self::add_remote_candidate`]) plus every new STUN
    /// source address [`Self::handle_stun_datagram`] has let through to the
    /// ICE agent.
    ///
    /// `rtc-ice` 0.20.0 creates its own peer-reflexive remote candidate (and
    /// a new candidate pair) for *every* authenticated STUN Binding Request
    /// from a source address it does not already recognize — see
    /// `agent::mod::handle_inbound`'s `remote_candidate_index.is_none()`
    /// branch — entirely bypassing `add_remote_candidate`'s own cap, since
    /// that path is never called. RFC 8445 §19.5.1's amplification concern
    /// applies directly: the peer is untrusted (any WHEP viewer) and knows
    /// ice-ufrag/ice-pwd from the negotiated SDP, so it can authenticate as
    /// many Binding Requests as it likes from as many source ports as it
    /// likes. Since `rtc-ice` keeps its remote-candidate list private (no
    /// public getter), gating new source addresses here, before they ever
    /// reach the agent, is the only enforcement point available.
    known_remote_addrs: HashSet<SocketAddr>,
}

impl MediaTransport {
    /// Build a transport for one peer connection.
    ///
    /// Generates a fresh self-signed DTLS certificate — WebRTC authenticates
    /// peers by the SDP-signalled fingerprint (RFC 8122), not a CA chain, so
    /// self-signed is the norm — and a host ICE candidate from
    /// `config.local_addr`. If `config.stun_server` is set, gathering a
    /// server-reflexive candidate begins immediately; watch for
    /// [`MediaEvent::LocalCandidateGathered`] from [`Self::handle_datagram`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Media`] if `config.remote_fingerprint` is not a
    /// well-formed SHA-256 SDP fingerprint (RFC 8122 §5 — see
    /// [`parse_remote_fingerprint`] for producing one from an SDP body), if
    /// `config.local_setup` is [`SetupRole::ActPass`] (never a role a
    /// concrete transport can be built with — see
    /// [`MediaTransportConfig::local_setup`]), or if certificate generation
    /// or ICE/DTLS setup fails.
    pub fn new(config: MediaTransportConfig) -> Result<Self, Error> {
        let remote_fingerprint_digest =
            parse_fingerprint_value(&config.remote_fingerprint).map_err(Error::Media)?;
        if config.local_setup == SetupRole::ActPass {
            return Err(Error::Media(
                "local_setup ActPass is only valid as an SDP offer's a=setup value; the \
                 answer (or this side's own choice, if answering) must resolve to Active or \
                 Passive before a MediaTransport can be built"
                    .to_string(),
            ));
        }

        // Generates a fresh self-signed certificate — WebRTC authenticates
        // peers by the SDP-signalled fingerprint (RFC 8122), not a CA chain,
        // so self-signed is the norm.
        let crypto_provider = default_provider()
            .map_err(|e| Error::Media(format!("resolve default crypto provider: {e}")))?;
        let certificate = Certificate::generate_self_signed(
            vec!["localhost".to_string()],
            crypto_provider.crypto(),
        )
        .map_err(|e| Error::Media(format!("generate self-signed certificate: {e}")))?;
        Self::with_certificate(
            config,
            remote_fingerprint_digest,
            certificate,
            crypto_provider,
        )
    }

    /// The shared construction body. Split out from [`Self::new`] only so a
    /// test can supply the certificate (and thereby give a peer pair each
    /// other's genuine fingerprint before construction — impossible through
    /// the public API, since `new` generates the certificate itself); see
    /// the loopback tests in the test module.
    fn with_certificate(
        config: MediaTransportConfig,
        remote_fingerprint_digest: [u8; FINGERPRINT_LEN],
        certificate: Certificate,
        crypto_provider: Arc<dyn RTCCryptoProvider>,
    ) -> Result<Self, Error> {
        let is_client = config.local_setup == SetupRole::Active;

        let local_fingerprint = sha256_fingerprint(certificate.certificate[0].as_ref());

        let agent_config = AgentConfig {
            local_ufrag: config.local_ice_ufrag.clone(),
            local_pwd: config.local_ice_pwd.clone(),
            is_controlling: config.is_controlling,
            multicast_dns_mode: MulticastDnsMode::Disabled,
            candidate_types: vec![CandidateType::Host, CandidateType::ServerReflexive],
            ..Default::default()
        };
        let mut ice = IceAgent::new(
            Instant::now(),
            Arc::new(agent_config),
            crypto_provider.clone(),
        )
        .map_err(|e| Error::Media(format!("new ice agent: {e}")))?;

        let host_candidate = CandidateHostConfig {
            base_config: CandidateConfig {
                network: "udp".to_string(),
                address: config.local_addr.ip().to_string(),
                port: config.local_addr.port(),
                component: 1,
                ..Default::default()
            },
            ..Default::default()
        }
        .new_candidate_host()
        .map_err(|e| Error::Media(format!("build host candidate: {e}")))?;
        ice.add_local_candidate(host_candidate)
            .map_err(|e| Error::Media(format!("add host candidate: {e}")))?;

        ice.start_connectivity_checks(
            Instant::now(),
            config.is_controlling,
            config.remote_ice_ufrag.clone(),
            config.remote_ice_pwd.clone(),
        )
        .map_err(|e| Error::Media(format!("start connectivity checks: {e}")))?;

        // RFC 5764 §4.1: is_client picks which handshake role rtc-dtls
        // builds this config for. `remote_addr: None` — this cut never sets
        // an explicit `server_name`. `with_insecure_skip_verify(true)` only
        // disables CA-chain verification (WebRTC has none — peers are
        // authenticated by the SDP-signalled fingerprint, RFC 8122 §5 /
        // RFC 5764 §5); the actual identity check is the verify callback
        // below, which runs for both roles regardless of that flag and
        // accepts only a leaf certificate hashing to the configured
        // remote-fingerprint digest.
        let expected_remote_digest = remote_fingerprint_digest;
        let verify_peer_certificate: VerifyPeerCertificateFn = Arc::new(move |certs, _chains| {
            if peer_cert_fingerprint_ok(certs, &expected_remote_digest) {
                Ok(())
            } else {
                Err(SharedError::ErrInvalidCertificate)
            }
        });
        let builder = ConfigBuilder::default()
            .with_crypto_provider(crypto_provider.clone())
            .with_certificates(vec![certificate])
            .with_srtp_protection_profiles(vec![OFFERED_SRTP_PROFILE])
            .with_insecure_skip_verify(true)
            .with_verify_peer_certificate(Some(verify_peer_certificate));
        // The passive/server role must REQUIRE a client certificate: the
        // fingerprint check is the only authentication either side gets, so
        // without a client certificate there would be nothing to verify and
        // an anonymous peer could complete the handshake.
        let builder = if is_client {
            builder
        } else {
            builder.with_client_auth(ClientAuthType::RequireAnyClientCert)
        };
        let handshake_config = Arc::new(
            builder
                .build(is_client, None)
                .map_err(|e| Error::Media(format!("build dtls handshake config: {e}")))?,
        );

        // Passive (DTLS server): the config is installed as the endpoint's
        // server_config, so an inbound ClientHello from the peer implicitly
        // starts an association (RFC 5764 §5.1.2's "forward to DTLS" band).
        // Active (DTLS client): no server_config — this side must dial out
        // itself via `DtlsEndpoint::connect`, which happens once ICE
        // nominates a pair (see `Self::maybe_start_active_dtls`); the
        // config is retained in `dtls_client_config` for that call.
        let (dtls_server_config, dtls_client_config) = if is_client {
            (None, Some(handshake_config))
        } else {
            (Some(handshake_config), None)
        };
        let dtls = DtlsEndpoint::new(
            config.local_addr,
            TransportProtocol::UDP,
            dtls_server_config,
        );

        let gather = match config.stun_server {
            Some(server) => Some(StunGather::new(Instant::now(), config.local_addr, server)?),
            None => None,
        };

        Ok(Self {
            local_addr: config.local_addr,
            local_fingerprint,
            remote_fingerprint_digest,
            local_setup: config.local_setup,
            ice,
            dtls,
            dtls_client_config,
            dtls_peer: None,
            selected_pair_addr: None,
            srtp_read: None,
            srtp_write: None,
            write_rtp_count: 0,
            write_rtcp_count: 0,
            read_rtp_count: 0,
            read_rtcp_count: 0,
            retired_srtp_read: None,
            gather,
            crypto_provider,
            remote_candidate_count: 0,
            max_remote_candidates: config.max_remote_candidates,
            known_remote_addrs: HashSet::new(),
        })
    }

    /// The SHA-256 fingerprint of this side's self-signed DTLS certificate
    /// (RFC 8122), colon-hex formatted exactly as the SDP `a=fingerprint`
    /// value expects after its `sha-256 ` prefix.
    pub fn local_fingerprint(&self) -> &str {
        &self.local_fingerprint
    }

    /// This side's DTLS role, as given to [`MediaTransportConfig::local_setup`].
    pub fn local_setup(&self) -> SetupRole {
        self.local_setup
    }

    /// Add a remote ICE candidate (the candidate-attribute body, e.g. from
    /// `a=candidate:<this>` in the remote's SDP or a Trickle-ICE fragment).
    ///
    /// # Errors
    ///
    /// Also returns [`Error::Media`] once
    /// [`MediaTransportConfig::max_remote_candidates`] remote candidates
    /// have already been admitted (RFC 8445 §6.1.2.5) — every candidate past
    /// the cap is rejected, never silently dropped.
    pub fn add_remote_candidate(&mut self, candidate: &str) -> Result<(), Error> {
        if self.remote_candidate_count >= self.max_remote_candidates {
            let cap = self.max_remote_candidates;
            return Err(Error::Media(format!(
                "remote candidate count exceeds the {cap}-candidate cap (RFC 8445 §6.1.2.5)"
            )));
        }
        let c = unmarshal_candidate(candidate)
            .map_err(|e| Error::Media(format!("unmarshal remote candidate {candidate:?}: {e}")))?;
        // Counted here, before the `rtc-ice` call: this bounds the number of
        // candidates handed to the agent's connectivity-check machinery
        // regardless of whether the agent itself treats one as a duplicate
        // or otherwise filters it (`rtc-ice` 0.20.0 keeps its own remote
        // candidate list privately, so this is the only cap available).
        self.remote_candidate_count += 1;
        self.known_remote_addrs.insert(c.addr());
        self.ice
            .add_remote_candidate(c)
            .map_err(|e| Error::Media(format!("add remote candidate: {e}")))?;
        Ok(())
    }

    /// The next outbound datagram to send, if any. Drains the ICE agent's,
    /// the DTLS endpoint's, and (while gathering) the STUN client's write
    /// queues, in that order.
    pub fn poll_transmit(&mut self) -> Option<Datagram> {
        if let Some(msg) = Protocol::poll_write(&mut self.ice) {
            return Some(Datagram {
                peer: msg.transport.peer_addr,
                bytes: msg.message.to_vec(),
            });
        }
        if let Some(msg) = self.dtls.poll_transmit() {
            return Some(Datagram {
                peer: msg.transport.peer_addr,
                bytes: msg.message.to_vec(),
            });
        }
        if let Some(gather) = &mut self.gather
            && let Some(msg) = gather.poll_transmit()
        {
            return Some(Datagram {
                peer: msg.transport.peer_addr,
                bytes: msg.message.to_vec(),
            });
        }
        None
    }

    /// Drive ICE/DTLS/STUN-gather timers. Call periodically (the underlying
    /// `rtc-ice`/`rtc-stun` retransmission schedules are sub-second) even
    /// when no datagrams are arriving.
    ///
    /// Returns any [`MediaEvent::TimerError`]s the drive produced. Before
    /// this returned anything, a failed ICE or DTLS timer drive (handshake
    /// failure, a fatal alert) was silently discarded (`let _ =`), so the
    /// caller never learned anything had gone wrong — it just saw silence
    /// (audit run-09 W20).
    pub fn handle_timeout(&mut self, now: Instant) -> Vec<MediaEvent> {
        let mut events = Vec::new();
        if let Err(e) = Protocol::handle_timeout(&mut self.ice, now) {
            events.push(MediaEvent::TimerError(format!("ice handle_timeout: {e}")));
        }
        let peers: Vec<SocketAddr> = self.dtls.get_connections_keys().copied().collect();
        for peer in peers {
            if let Err(e) = self.dtls.handle_timeout(peer, now) {
                events.push(MediaEvent::TimerError(format!(
                    "dtls handle_timeout ({peer}): {e}"
                )));
            }
        }
        if let Some(gather) = &mut self.gather {
            gather.handle_timeout(now);
        }
        if self.gather.as_ref().is_some_and(StunGather::done) {
            self.gather = None;
        }
        self.purge_expired_retired_key(now);
        events
    }

    /// RFC 5764 §5.2's retention window on the previous read key
    /// ([`Self::rekey`]) has an end, not just a beginning: once `now` is
    /// past it, drop the retired context so it does not sit around
    /// forever as a permanent second decrypt attempt for garbage traffic.
    fn purge_expired_retired_key(&mut self, now: Instant) {
        if self
            .retired_srtp_read
            .as_ref()
            .is_some_and(|(_, deadline)| now >= *deadline)
        {
            self.retired_srtp_read = None;
        }
    }

    /// Feed one inbound UDP datagram from `peer`, demultiplexing it as
    /// STUN/DTLS/SRTP per the `media` module doc and returning whatever
    /// events it produced.
    pub fn handle_datagram(
        &mut self,
        now: Instant,
        peer: SocketAddr,
        data: &[u8],
    ) -> Result<Vec<MediaEvent>, Error> {
        let mut events = Vec::new();
        let Some(&first) = data.first() else {
            return Ok(events);
        };

        if first <= DEMUX_STUN_MAX {
            self.handle_stun_datagram(now, peer, data, &mut events)?;
        } else if (DEMUX_DTLS_MIN..=DEMUX_DTLS_MAX).contains(&first) {
            self.handle_dtls_datagram(now, peer, data, &mut events)?;
        } else if (DEMUX_RTP_MIN..=DEMUX_RTP_MAX).contains(&first) {
            self.handle_srtp_datagram(now, data, &mut events);
        }
        // Any other first byte has no defined meaning on this flow (see the
        // module doc's demux table) and is silently ignored.

        Ok(events)
    }

    fn handle_stun_datagram(
        &mut self,
        now: Instant,
        peer: SocketAddr,
        data: &[u8],
        events: &mut Vec<MediaEvent>,
    ) -> Result<(), Error> {
        let from_gather_server = self.gather.as_ref().map(StunGather::server) == Some(peer);
        if from_gather_server {
            let srflx = self
                .gather
                .as_mut()
                .expect("checked Some above")
                .handle_datagram(now, self.local_addr, data)?;
            if let Some(mapped) = srflx {
                let candidate = self.add_server_reflexive_candidate(mapped, peer)?;
                events.push(MediaEvent::LocalCandidateGathered(candidate));
            }
            return Ok(());
        }

        // RFC 8445 §19.5.1 / §6.1.2.5: gate new source addresses against
        // the same cap `add_remote_candidate` enforces, before the datagram
        // ever reaches the ICE agent — see `known_remote_addrs`'s doc for
        // why this is the only place that can be done. An address already
        // admitted (an explicit candidate or a previously admitted STUN
        // source) keeps working unconditionally; only a *new* source
        // address is subject to the cap.
        if !self.known_remote_addrs.contains(&peer) {
            if self.known_remote_addrs.len() >= self.max_remote_candidates {
                return Ok(());
            }
            self.known_remote_addrs.insert(peer);
        }

        let tagged = TaggedBytesMut {
            now,
            transport: TransportContext {
                local_addr: self.local_addr,
                peer_addr: peer,
                ecn: None,
                transport_protocol: TransportProtocol::UDP,
            },
            message: BytesMut::from(data),
        };
        Protocol::handle_read(&mut self.ice, tagged)
            .map_err(|e| Error::Media(format!("ice handle_read: {e}")))?;

        while let Some(tagged_evt) = Protocol::poll_event(&mut self.ice) {
            let evt = tagged_evt.event;
            if let IceAgentEvent::SelectedCandidatePairChange(_, remote) = &evt {
                // RFC 5764 §5's identity check is only meaningful against
                // the peer ICE actually nominated: record its address as
                // the sole source DTLS will be accepted from (see
                // `handle_dtls_datagram`), then dial if this side is the
                // DTLS client.
                self.selected_pair_addr = Some(remote.addr());
                self.maybe_start_active_dtls(now, remote.addr())?;
            }
            if let Some(mapped) = map_ice_event(evt)? {
                events.push(mapped);
            }
        }
        Ok(())
    }

    /// [`SetupRole::Active`]: once ICE nominates a candidate pair (the
    /// [`IceAgentEvent::SelectedCandidatePairChange`] that fires the first
    /// time `set_selected_pair` runs — RFC 8445's connectivity-check
    /// result), dial the DTLS handshake to the peer's now-known address.
    ///
    /// A no-op for [`SetupRole::Passive`] (`dtls_client_config` is `None`)
    /// and for a second nomination on the same peer address
    /// (`DtlsEndpoint::connect` only inserts a new association into a
    /// vacant `remote` entry, per its own doc — calling it again on an
    /// address that already has an association is harmless).
    fn maybe_start_active_dtls(
        &mut self,
        now: Instant,
        remote_addr: SocketAddr,
    ) -> Result<(), Error> {
        let Some(client_config) = self.dtls_client_config.clone() else {
            return Ok(());
        };
        self.dtls
            .connect(now, remote_addr, client_config, None)
            .map_err(|e| Error::Media(format!("dtls connect (active/client role): {e}")))?;
        Ok(())
    }

    fn handle_dtls_datagram(
        &mut self,
        now: Instant,
        peer: SocketAddr,
        data: &[u8],
        events: &mut Vec<MediaEvent>,
    ) -> Result<(), Error> {
        // Accept DTLS only from the remote address of the selected ICE
        // candidate pair (RFC 8445 §6.2.1). Before any pair is selected —
        // and from any other address afterwards — drop the datagram with no
        // event: an off-path sender must not be able to open a DTLS
        // association on this port at all, let alone complete one and have
        // its SRTP keys installed (RFC 5764 §5's "establish a DTLS
        // association ... with the ICE-authenticated peer").
        if self.selected_pair_addr != Some(peer) {
            return Ok(());
        }
        let dtls_events = self
            .dtls
            .read(now, peer, None::<EcnCodepoint>, BytesMut::from(data))
            .map_err(|e| Error::Media(format!("dtls read: {e}")))?;
        for ev in dtls_events {
            match ev {
                EndpointEvent::HandshakeComplete => {
                    self.on_dtls_handshake_complete(peer)?;
                    events.push(MediaEvent::DtlsHandshakeComplete);
                }
                EndpointEvent::ApplicationData(_) => {
                    // DTLS application data (e.g. SCTP data channels) is out
                    // of scope for this cut — see the crate README.
                }
                // `EndpointEvent` is `#[non_exhaustive]` in rtc-dtls 0.21. An
                // event this crate does not understand is surfaced as an
                // error (never silently dropped, which could stall a
                // handshake the caller thinks is progressing).
                _ => {
                    return Err(Error::Media(
                        "unrecognised rtc-dtls endpoint event".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Decrypt one inbound SRTP/SRTCP datagram, per RFC 5764 §5.2's
    /// two-key-set reordering rule: try the current `srtp_read` context
    /// first, and only fall back to the retired one (whatever
    /// [`Self::rekey`] most recently retired) — a packet that arrived
    /// late, still keyed under the old master key — if that fails or
    /// there is no current context yet.
    fn handle_srtp_datagram(&mut self, now: Instant, data: &[u8], events: &mut Vec<MediaEvent>) {
        self.purge_expired_retired_key(now);

        let is_rtcp = data.get(1).is_some_and(|&pt| is_rtcp_packet_type(pt));

        match self
            .srtp_read
            .as_mut()
            .map(|ctx| decrypt_srtp(ctx, is_rtcp, data))
        {
            Some(Ok(event)) => {
                if is_rtcp {
                    self.read_rtcp_count += 1;
                } else {
                    self.read_rtp_count += 1;
                }
                events.push(event);
            }
            Some(Err(current_err)) => {
                // The current context rejected the packet — exactly what
                // §5.2 says to expect from a packet reordered across a
                // rekey, still keyed under the master key `Self::rekey`
                // just retired. Try that before giving up.
                if let Some((ctx, _deadline)) = self.retired_srtp_read.as_mut()
                    && let Ok(event) = decrypt_srtp(ctx, is_rtcp, data)
                {
                    events.push(event);
                    return;
                }
                // Retired didn't save it either (or there wasn't one).
                // Anyone can send a byte in the RFC 5764 §5.1.2 SRTP band
                // (128..=191) to this port — post-handshake, datagrams are
                // accepted from any source address — so this is the
                // EXPECTED outcome for spoofed/garbage traffic, not an
                // exceptional one: surface it as an event (audit run-09
                // W20), not `Err`.
                events.push(MediaEvent::AuthFailure {
                    is_rtcp,
                    reason: current_err.to_string(),
                });
            }
            None => {
                // No current context at all (no handshake yet, or
                // mid-rekey) — try the retired one; if that fails too,
                // drop the packet silently, the same tolerance this
                // method always had for "SRTP arrived before the DTLS
                // handshake finished".
                if let Some((ctx, _deadline)) = self.retired_srtp_read.as_mut()
                    && let Ok(event) = decrypt_srtp(ctx, is_rtcp, data)
                {
                    events.push(event);
                }
            }
        }
    }

    /// Install the SRTP keys of a just-completed DTLS association, after
    /// two guards on top of the handshake itself:
    ///
    /// - Defence in depth for RFC 5764 §5 / RFC 8122 §5: re-check the
    ///   association's stored peer certificate against the configured
    ///   remote-fingerprint digest before deriving any key from it, even
    ///   though the verify callback already ran during the handshake.
    /// - Once SRTP keys are installed for a peer, a `HandshakeComplete`
    ///   from a *different* address never replaces them: this method
    ///   returns [`Error::Media`] and installs nothing (the caller sees the
    ///   failure; the live session's keys are untouched). Rekeying is the
    ///   supported way to fresh keys for the same peer — [`Self::rekey`]
    ///   clears the write context first, which re-opens this path for that
    ///   address. A second association completing at a different address is
    ///   by construction not the peer of the live one, so its keys must not
    ///   silently hijack the session (the SRTP contexts would otherwise be
    ///   swapped and the real peer's media would start failing
    ///   authentication).
    fn on_dtls_handshake_complete(&mut self, peer: SocketAddr) -> Result<(), Error> {
        let state = self.dtls.get_connection_state(peer).ok_or_else(|| {
            Error::Media("dtls handshake completed but no connection state for peer".to_string())
        })?;

        if !peer_cert_fingerprint_ok(&state.peer_certificates, &self.remote_fingerprint_digest) {
            return Err(Error::Media(
                "dtls handshake completed but the peer certificate does not match the \
                 configured remote_fingerprint: no SRTP keys installed"
                    .to_string(),
            ));
        }

        if self.srtp_write.is_some() && self.dtls_peer != Some(peer) {
            return Err(Error::Media(format!(
                "ignoring dtls handshake completed from {peer}: srtp keys are already installed \
                 for {:?} and are never replaced by another association",
                self.dtls_peer
            )));
        }

        self.dtls_peer = Some(peer);

        let srtp_profile = to_srtp_profile(state.srtp_protection_profile())?;
        let key_len = srtp_profile.key_len();
        let salt_len = srtp_profile.salt_len();
        let material = state
            .export_keying_material(SRTP_KEYING_MATERIAL_LABEL, &[], 2 * (key_len + salt_len))
            .map_err(|e| Error::Media(format!("export srtp keying material: {e}")))?;

        let material = material.as_ref();
        let client_key = &material[0..key_len];
        let server_key = &material[key_len..2 * key_len];
        let client_salt = &material[2 * key_len..2 * key_len + salt_len];
        let server_salt = &material[2 * key_len + salt_len..2 * key_len + 2 * salt_len];

        let ((read_key, read_salt), (write_key, write_salt)) = select_read_write_material(
            state.is_client(),
            (client_key, client_salt),
            (server_key, server_salt),
        );

        let read_ctx = SrtpContext::new(
            read_key,
            read_salt,
            srtp_profile,
            None,
            None,
            self.crypto_provider.crypto(),
        )
        .map_err(|e| Error::Media(format!("build srtp decrypt context: {e}")))?;
        let write_ctx = SrtpContext::new(
            write_key,
            write_salt,
            srtp_profile,
            None,
            None,
            self.crypto_provider.crypto(),
        )
        .map_err(|e| Error::Media(format!("build srtp encrypt context: {e}")))?;
        self.srtp_read = Some(read_ctx);
        self.srtp_write = Some(write_ctx);
        Ok(())
    }

    /// Encrypt an outbound RTP packet (RFC 3711) with the write-side SRTP
    /// context built once the DTLS handshake completes, ready to send as a
    /// UDP datagram to the peer.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Media`] if the DTLS handshake has not completed yet
    /// (no write context exists) or if `rtc-srtp` itself rejects the
    /// packet.
    pub fn encrypt_rtp(&mut self, packet: &rtp_packet::RtpPacket<'_>) -> Result<Vec<u8>, Error> {
        let ctx = self.srtp_write.as_mut().ok_or_else(|| {
            Error::Media("no srtp write context yet: dtls handshake has not completed".to_string())
        })?;
        let plaintext = packet.to_bytes();
        let protected = ctx
            .encrypt_rtp(&plaintext)
            .map_err(|e| Error::Media(format!("srtp encrypt: {e}")))?;
        self.write_rtp_count += 1;
        Ok(protected.to_vec())
    }

    /// Encrypt an outbound RTCP compound packet (RFC 3711 §3.4 / RFC 3550
    /// §6.1) with the same write-side context [`Self::encrypt_rtp`] uses
    /// (RFC 3711 §3.2.1: master keys may be shared between SRTP/SRTCP,
    /// session keys are kept distinct by `rtc-srtp` internally per
    /// §4.3.2's separate labels), ready to send as a UDP datagram to the
    /// peer.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Media`] if the DTLS handshake has not completed yet
    /// (no write context exists) or if `rtc-srtp` itself rejects the
    /// packet.
    pub fn encrypt_rtcp(&mut self, packet: &rtcp_packet::CompoundPacket) -> Result<Vec<u8>, Error> {
        let ctx = self.srtp_write.as_mut().ok_or_else(|| {
            Error::Media("no srtp write context yet: dtls handshake has not completed".to_string())
        })?;
        let plaintext = packet.to_bytes();
        let protected = ctx
            .encrypt_rtcp(&plaintext)
            .map_err(|e| Error::Media(format!("srtcp encrypt: {e}")))?;
        self.write_rtcp_count += 1;
        Ok(protected.to_vec())
    }

    /// True once any RFC 3711 §9.2 packet counter for the current key
    /// epoch — write or read, RTP or RTCP — has reached
    /// `MAXIMUM_LIFETIME_PACKETS`. The caller's cue to call
    /// [`Self::rekey`]; [`MediaTransport`] never calls it on its own (it is
    /// sans-IO and has no clock/scheduler of its own to decide "now" with).
    pub fn needs_rekey(&self) -> bool {
        exceeds_maximum_lifetime(self.write_rtp_count)
            || exceeds_maximum_lifetime(self.write_rtcp_count)
            || exceeds_maximum_lifetime(self.read_rtp_count)
            || exceeds_maximum_lifetime(self.read_rtcp_count)
    }

    /// Retire the current SRTP/SRTCP keys and start a fresh DTLS handshake
    /// to replace them — the rekey path RFC 3711 §9.2's `maximum_lifetime`
    /// trigger ([`Self::needs_rekey`]) exists to reach.
    ///
    /// The write context is dropped immediately: RFC 3711 §9.2's
    /// "previously stored and used keys MUST NOT be used again" applies to
    /// sending unconditionally, and until the new handshake completes,
    /// [`Self::encrypt_rtp`]/[`Self::encrypt_rtcp`] will error exactly as
    /// they do before the *first* handshake. The current read context is
    /// **not** dropped — it moves to the retired slot, retained for
    /// `RETIRED_KEY_RETENTION` so [`Self::handle_datagram`] can still
    /// decrypt a packet reordered across the boundary (RFC 5764 §5.2).
    ///
    /// # Which RFC 5764 rekey mechanism this is
    ///
    /// RFC 5764 §4.4 ("Key Usage Limitations") is what this method actually
    /// implements: "When \[`maximum_lifetime`\] is reached, **a new DTLS
    /// session SHOULD be used** to establish replacement keys" — exactly
    /// [`Endpoint::stop`](rtc_dtls::endpoint::Endpoint::stop) followed by a
    /// fresh [`Endpoint::connect`](rtc_dtls::endpoint::Endpoint::connect)
    /// (or wait-for-ClientHello) below. §5.2 ("Rehandshake and Rekey")
    /// separately illustrates an *alternative* mechanism — "a new
    /// handshake over the **existing** DTLS channel" (in-band
    /// renegotiation, same association) — that this method does **not**
    /// use: `rtc-dtls` 0.20.0 — checked directly, both `endpoint.rs`'s
    /// public surface (`connect`/`stop`/`close`/`read`/`write`/
    /// `handle_timeout`/`poll_timeout`, nothing else) and `conn/mod.rs`
    /// (whose only reference to rehandshaking is a comment citing RFC 6347
    /// §4.1.0's record-sequence-number-overflow rule, implementing only its
    /// "abandon" branch, never its "rehandshake" one) — exposes no API for
    /// it. For [`SetupRole::Active`] this method dials the brand-new
    /// session itself; for [`SetupRole::Passive`], it waits for the peer to
    /// send a fresh ClientHello (the same asymmetry
    /// `maybe_start_active_dtls` already documents for the *first*
    /// handshake). §5.2's actual receiver-side purpose — tolerating packets
    /// reordered across the rekey boundary — is unaffected by *which* of
    /// the two mechanisms produced the new keys, since SRTP/SRTCP packets
    /// carry no DTLS epoch of their own; that guarantee is fully
    /// implemented here via `retired_srtp_read`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Media`] if no DTLS handshake has ever completed
    /// (nothing to rekey), if redialling the fresh handshake
    /// ([`SetupRole::Active`] only) fails, or if this side is
    /// [`SetupRole::Passive`] (audit run-09 W19): `maybe_start_active_dtls`
    /// is a no-op for Passive (it only ever dials out for Active), so tearing
    /// down the association here would leave the Passive side waiting for a
    /// fresh `ClientHello` a browser or OBS peer never sends without a new
    /// SDP offer — silently ending media for good. The caller must instead
    /// drive an ICE restart or SDP renegotiation to get a new DTLS handshake.
    pub fn rekey(&mut self, now: Instant) -> Result<(), Error> {
        let Some(peer) = self.dtls_peer else {
            return Err(Error::Media(
                "rekey: no dtls handshake has completed yet; nothing to rekey".to_string(),
            ));
        };
        if self.local_setup == SetupRole::Passive {
            return Err(Error::Media(
                "rekey: this side is SetupRole::Passive, which never dials out a new DTLS \
                 session on its own; drive an ICE restart or SDP renegotiation instead"
                    .to_string(),
            ));
        }

        if let Some(old_read) = self.srtp_read.take() {
            self.retired_srtp_read = Some((old_read, now + RETIRED_KEY_RETENTION));
        }
        self.srtp_write = None;
        self.write_rtp_count = 0;
        self.write_rtcp_count = 0;
        self.read_rtp_count = 0;
        self.read_rtcp_count = 0;

        let _ = self.dtls.stop(now, peer);
        self.maybe_start_active_dtls(now, peer)?;
        Ok(())
    }

    fn add_server_reflexive_candidate(
        &mut self,
        mapped: SocketAddr,
        stun_server: SocketAddr,
    ) -> Result<String, Error> {
        let candidate = CandidateServerReflexiveConfig {
            base_config: CandidateConfig {
                network: "udp".to_string(),
                address: mapped.ip().to_string(),
                port: mapped.port(),
                component: 1,
                ..Default::default()
            },
            rel_addr: self.local_addr.ip().to_string(),
            rel_port: self.local_addr.port(),
            url: Some(format!("stun:{stun_server}")),
        }
        .new_candidate_server_reflexive()
        .map_err(|e| Error::Media(format!("build server-reflexive candidate: {e}")))?;

        let marshaled = candidate.marshal();
        self.ice
            .add_local_candidate(candidate)
            .map_err(|e| Error::Media(format!("add server-reflexive candidate: {e}")))?;
        Ok(marshaled)
    }
}

/// Decrypt one SRTP or SRTCP datagram with `ctx` and parse the result into
/// the corresponding [`MediaEvent`]. Pulled out of
/// [`MediaTransport::handle_srtp_datagram`] as a free function so that
/// method can try it against the current context and, on failure, the
/// retired one, without duplicating the decrypt-then-parse logic (RFC 3711
/// / RFC 5764 §5.2 — see the `media` module doc's demux/rekey sections).
fn decrypt_srtp(ctx: &mut SrtpContext, is_rtcp: bool, data: &[u8]) -> Result<MediaEvent, Error> {
    if is_rtcp {
        let plaintext = ctx
            .decrypt_rtcp(data)
            .map_err(|e| Error::Media(format!("srtcp decrypt: {e}")))?;
        // Authentication already passed above. `rtcp_packet::CompoundPacket`
        // now decodes RFC 4585/3611 feedback (and RFC 5506 Reduced-Size RTCP)
        // via `RtcpPacket::Unknown`, so a parse failure from here on means the
        // plaintext genuinely isn't a well-formed RTCP compound packet, not a
        // forged or corrupt datagram — see [`MediaEvent::RtcpUnsupported`].
        Ok(match rtcp_packet::CompoundPacket::parse(&plaintext) {
            Ok(compound) => MediaEvent::Rtcp(compound),
            Err(e) => MediaEvent::RtcpUnsupported(e),
        })
    } else {
        let plaintext = ctx
            .decrypt_rtp(data)
            .map_err(|e| Error::Media(format!("srtp decrypt: {e}")))?;
        let pkt = rtp_packet::RtpPacket::parse(&plaintext)
            .map_err(|e| Error::Media(format!("rtp parse: {e}")))?;
        Ok(MediaEvent::Rtp(DecryptedRtp {
            marker: pkt.marker,
            payload_type: pkt.payload_type,
            sequence_number: pkt.sequence_number,
            timestamp: pkt.timestamp,
            ssrc: pkt.ssrc,
            csrc: pkt.csrc.clone(),
            extension: pkt.extension.map(|ext| DecryptedRtpExtension {
                profile_id: ext.profile_id,
                data: ext.data.to_vec(),
            }),
            payload: pkt.payload.to_vec(),
        }))
    }
}

fn map_ice_event(evt: IceAgentEvent) -> Result<Option<MediaEvent>, Error> {
    match evt {
        IceAgentEvent::ConnectionStateChange(state) => {
            Ok(Some(MediaEvent::IceStateChanged(state.to_string())))
        }
        IceAgentEvent::SelectedCandidatePairChange(..) | IceAgentEvent::RoleChange(_) => Ok(None),
        // `Event` is `#[non_exhaustive]` in rtc-ice 0.21. An unknown event is
        // surfaced as an error, never silently dropped.
        _ => Err(Error::Media("unrecognised rtc-ice event".to_string())),
    }
}

/// A `(master_key, master_salt)` pair sliced from the RFC 5764 §4.2
/// exporter's output.
type KeyMaterial<'a> = (&'a [u8], &'a [u8]);

/// RFC 5764 §4.2: which of the exporter's four `(master_key, master_salt)`
/// pairs a side reads inbound traffic with vs. writes outbound traffic
/// with, given whether this side is the DTLS client.
///
/// > the server MUST only use \[`client_write_*`\] keys to decrypt inbound
/// > traffic ... the client MUST only use \[`server_write_*`\] keys to
/// > decrypt inbound traffic
///
/// i.e. each side reads with the *other* side's write material and writes
/// with its *own*. Returns `(read, write)`.
fn select_read_write_material<'a>(
    is_client: bool,
    client: KeyMaterial<'a>,
    server: KeyMaterial<'a>,
) -> (KeyMaterial<'a>, KeyMaterial<'a>) {
    if is_client {
        (server, client)
    } else {
        (client, server)
    }
}

fn to_srtp_profile(profile: SrtpProtectionProfile) -> Result<ProtectionProfile, Error> {
    match profile {
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80 => {
            Ok(ProtectionProfile::Aes128CmHmacSha1_80)
        }
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_32 => {
            Ok(ProtectionProfile::Aes128CmHmacSha1_32)
        }
        SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm => Ok(ProtectionProfile::AeadAes128Gcm),
        SrtpProtectionProfile::Srtp_Aead_Aes_256_Gcm => Ok(ProtectionProfile::AeadAes256Gcm),
        other => Err(Error::Media(format!(
            "negotiated an unsupported SRTP protection profile: {other:?}"
        ))),
    }
}

/// The RFC 8122 certificate fingerprint: the SHA-256 digest of the DER
/// certificate, colon-hex formatted (e.g. `AB:CD:...`), matching the value
/// expected after `a=fingerprint:sha-256 ` in SDP.
fn sha256_fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    /// The feature-selected default `rtc-crypto` provider (ring), as
    /// `MediaTransport::new` resolves it.
    fn test_crypto() -> Arc<dyn RTCCryptoProvider> {
        default_provider().expect("default crypto provider")
    }

    use super::*;

    #[test]
    fn setup_role_display_matches_sdp_token() {
        assert_eq!(SetupRole::Active.to_string(), "active");
        assert_eq!(SetupRole::Passive.to_string(), "passive");
        assert_eq!(SetupRole::ActPass.to_string(), "actpass");
    }

    #[test]
    fn fingerprint_is_colon_hex_sha256() {
        // A fixed input so the expected digest can be checked against an
        // independently computed SHA-256 (verifies formatting, not a live
        // certificate).
        let fp = sha256_fingerprint(b"");
        // SHA-256("") is the well-known empty-string digest.
        assert_eq!(
            fp,
            "E3:B0:C4:42:98:FC:1C:14:9A:FB:F4:C8:99:6F:B9:24:27:AE:41:E4:64:9B:93:4C:A4:95:99:1B:78:52:B8:55"
        );
    }

    /// A well-formed SHA-256 fingerprint (RFC 8122 §5) that matches no
    /// certificate any test generates — the placeholder for configs whose
    /// handshake is never expected to complete, or which swap in each
    /// peer's real digest via same-module access (see the loopback tests).
    const DUMMY_REMOTE_FINGERPRINT: &str = "sha-256 \
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:\
99:aa:bb:cc:dd:ee:ff";

    fn test_config(local_setup: SetupRole) -> MediaTransportConfig {
        // RFC 8445 §5.3: ufrag >= 24 bits (4 chars), pwd >= 128 bits (22
        // chars) of ICE-char. `rtc-ice` enforces the ufrag minimum at
        // construction, which is why these are longer than the crate's
        // pre-existing "u"/"p" placeholders (those never actually built an
        // agent far enough to hit the check, since `SetupRole::Active`
        // always errored out first).
        MediaTransportConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            local_ice_ufrag: "localufrag0".into(),
            local_ice_pwd: "localicepassword1234567".into(),
            remote_ice_ufrag: "remoteufrag0".into(),
            remote_ice_pwd: "remoteicepassword123456".into(),
            is_controlling: false,
            local_setup,
            stun_server: None,
            remote_fingerprint: DUMMY_REMOTE_FINGERPRINT.into(),
            max_remote_candidates: MAX_REMOTE_CANDIDATES,
        }
    }

    #[test]
    fn new_rejects_malformed_remote_fingerprint() {
        for bad in ["md5 00:11", "sha-256 00:11", "", "sha-256"] {
            let mut cfg = test_config(SetupRole::Passive);
            cfg.remote_fingerprint = bad.to_string();
            match MediaTransport::new(cfg) {
                Err(Error::Media(_)) => {}
                Err(other) => panic!("expected Error::Media for {bad:?}, got {other:?}"),
                Ok(_) => panic!("remote_fingerprint {bad:?} must be rejected by new()"),
            }
        }
    }

    #[test]
    fn parse_fingerprint_value_accepts_case_insensitive_sha256() {
        // Bite test: drop the `eq_ignore_ascii_case` (exact match) or the
        // per-byte `from_str_radix`, and one of these arms fails.
        let expected = [
            0xab, 0xcd, 0xef, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa,
            0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
            0x0a, 0x0b, 0x0c, 0x0d,
        ];
        let upper = "SHA-256 AB:CD:EF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:\
                     DD:EE:FF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D";
        let lower = upper.to_ascii_lowercase();
        assert_eq!(parse_fingerprint_value(upper).unwrap(), expected);
        assert_eq!(parse_fingerprint_value(&lower).unwrap(), expected);

        // Rejections: wrong token, missing digest, wrong byte count,
        // non-hex digits.
        for bad in [
            "md5 AB:CD",
            "sha-256",
            "sha-256 AB:CD",
            "sha-256 AB:CD:",
            "sha-256 ABCD",
            "sha-256 ZZ:CD:EF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:\
             DD:EE:FF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D",
        ] {
            assert!(
                parse_fingerprint_value(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn digests_equal_is_length_strict_and_orderly() {
        let a = [1u8; 32];
        assert!(digests_equal(&a, &[1u8; 32]));
        assert!(!digests_equal(&a, &[2u8; 32]));
        assert!(!digests_equal(&a, &[1u8; 31]));
    }

    #[test]
    fn parse_remote_fingerprint_media_beats_session() {
        let sdp = "v=0\r\na=fingerprint:sha-256 AA\r\nm=audio 9 RTP/AVP\r\n\
                   a=fingerprint:sha-256 BB\r\n";
        assert_eq!(parse_remote_fingerprint(sdp).as_deref(), Some("sha-256 BB"));
        let sdp = "v=0\r\na=fingerprint:sha-256 AA \r\nm=audio 9 RTP/AVP\r\n";
        assert_eq!(parse_remote_fingerprint(sdp).as_deref(), Some("sha-256 AA"));
        assert_eq!(parse_remote_fingerprint("v=0\r\n"), None);
    }

    #[test]
    fn rejects_actpass_setup_role() {
        match MediaTransport::new(test_config(SetupRole::ActPass)) {
            Err(Error::Media(_)) => {}
            Err(other) => panic!("expected Error::Media, got {other:?}"),
            Ok(_) => panic!("expected an error for local_setup: ActPass"),
        }
    }

    #[test]
    fn accepts_active_setup_role() {
        // Issue #948 item 2: SetupRole::Active used to be rejected
        // unconditionally. `Endpoint::connect` (rtc-dtls) genuinely
        // supports the DTLS-client role sans-IO, so this must now build.
        let mt = MediaTransport::new(test_config(SetupRole::Active))
            .expect("Active (DTLS client) role must now be buildable");
        assert!(
            mt.dtls_client_config.is_some(),
            "Active role must retain a client handshake config for maybe_start_active_dtls"
        );
    }

    #[test]
    fn accepts_passive_setup_role_with_no_client_config() {
        let mt = MediaTransport::new(test_config(SetupRole::Passive))
            .expect("Passive (DTLS server) role must still build");
        assert!(
            mt.dtls_client_config.is_none(),
            "Passive role must never dial out, so it must retain no client handshake config"
        );
    }

    #[test]
    fn add_remote_candidate_enforces_max_cap() {
        // RFC 8445 §6.1.2.5: the candidate-pair cap MUST be enforced. Feed
        // more than MAX_REMOTE_CANDIDATES distinct, well-formed host
        // candidate lines and check every one past the cap is rejected, and
        // that the transport's admitted count never exceeds the cap.
        let mut mt =
            MediaTransport::new(test_config(SetupRole::Passive)).expect("Passive role must build");

        let extra = 5;
        let mut accepted = 0usize;
        let mut rejected = 0usize;
        for i in 0..(MAX_REMOTE_CANDIDATES + extra) {
            // Distinct port per candidate so none is treated as a duplicate
            // by the ICE agent itself (which would otherwise mask the cap).
            let port = 10_000 + i as u16;
            let line = format!("1 1 udp 2130706431 127.0.0.1 {port} typ host");
            match mt.add_remote_candidate(&line) {
                Ok(()) => accepted += 1,
                Err(Error::Media(_)) => rejected += 1,
                Err(other) => panic!("unexpected error variant: {other:?}"),
            }
        }

        assert_eq!(
            accepted, MAX_REMOTE_CANDIDATES,
            "exactly MAX_REMOTE_CANDIDATES candidates must be admitted"
        );
        assert_eq!(
            rejected, extra,
            "every candidate past the cap must be rejected, not silently dropped"
        );
        assert_eq!(
            mt.remote_candidate_count, MAX_REMOTE_CANDIDATES,
            "the transport's admitted remote-candidate count must never exceed the cap"
        );
    }

    #[test]
    fn add_remote_candidate_honours_a_smaller_configured_cap() {
        // RFC 8445 §6.1.2.5 requires the limit to be configurable, not just
        // a fixed default: a caller that sets a stricter
        // `max_remote_candidates` than [`MAX_REMOTE_CANDIDATES`] must have
        // it enforced, not silently widened back to the default.
        let mut config = test_config(SetupRole::Passive);
        let small_cap = 3;
        config.max_remote_candidates = small_cap;
        let mut mt = MediaTransport::new(config).expect("Passive role must build");

        let mut accepted = 0usize;
        let mut rejected = 0usize;
        for i in 0..(small_cap + 2) {
            let port = 20_000 + i as u16;
            let line = format!("1 1 udp 2130706431 127.0.0.1 {port} typ host");
            match mt.add_remote_candidate(&line) {
                Ok(()) => accepted += 1,
                Err(Error::Media(_)) => rejected += 1,
                Err(other) => panic!("unexpected error variant: {other:?}"),
            }
        }

        assert_eq!(
            accepted, small_cap,
            "exactly the configured cap must be admitted, not MAX_REMOTE_CANDIDATES"
        );
        assert_eq!(
            rejected, 2,
            "every candidate past the configured cap must be rejected"
        );
        assert_eq!(mt.remote_candidate_count, small_cap);
    }

    // -----------------------------------------------------------------------
    // RFC 5764 §4.2 read/write key-material selection (item 1: outbound
    // SRTP). Bite test: swap the `if is_client` branches in
    // `select_read_write_material`, this test fails (both assertions
    // invert); restore, it passes again.
    // -----------------------------------------------------------------------

    #[test]
    fn read_write_material_selection_matches_rfc5764_4_2() {
        let client = (&b"CLIENT_KEY______"[..], &b"CLIENT_SALT___"[..]);
        let server = (&b"SERVER_KEY______"[..], &b"SERVER_SALT___"[..]);

        // We are the DTLS server (is_client = false): decrypt what the
        // client wrote, encrypt with our own (server) write material.
        let (read, write) = select_read_write_material(false, client, server);
        assert_eq!(read, client, "server must decrypt with client_write_*");
        assert_eq!(write, server, "server must encrypt with server_write_*");

        // We are the DTLS client (is_client = true): decrypt what the
        // server wrote, encrypt with our own (client) write material.
        let (read, write) = select_read_write_material(true, client, server);
        assert_eq!(read, server, "client must decrypt with server_write_*");
        assert_eq!(write, client, "client must encrypt with client_write_*");
    }

    // -----------------------------------------------------------------------
    // RFC 5761 §4 RTCP-mux demux (item 3). Bite test: change
    // `RTCP_MUX_TYPE_MAX` back to 223, `type_230_in_should_last_resort_
    // band_is_rtcp` fails; restore to 254, it passes.
    // -----------------------------------------------------------------------

    #[test]
    fn currently_registered_rtcp_types_are_rtcp() {
        // SR, RR, SDES, BYE, APP, RTPFB, PSFB, XR, AVB, RTPS — the IANA
        // registry's live allocations, all inside [192, 223].
        for pt in [200u8, 201, 202, 203, 204, 205, 206, 207, 208, 209] {
            assert!(
                is_rtcp_packet_type(pt),
                "RTCP type {pt} must classify as RTCP"
            );
        }
    }

    #[test]
    fn marked_dynamic_rtp_payload_types_are_not_rtcp() {
        // REGRESSION GUARD (issue #948 item 3). The demux was briefly widened
        // to RFC 5761 §4's [224, 254] last-resort RTCP band. That routed real
        // RTP to `decrypt_rtcp`: a dynamic payload type in 96..=126 with the
        // marker bit set lands in exactly that range.
        //
        // Opus at PT 111 with M=1 is `0x80 | 111 == 239` — the precise packet
        // shape this crate verified decrypting from a live browser. Widening
        // broke it.
        for pt in 96u8..=126 {
            let byte1 = 0x80 | pt; // marker bit set
            assert!(
                !is_rtcp_packet_type(byte1),
                "marked RTP PT {pt} (byte1 {byte1}) must classify as RTP, not RTCP"
            );
        }
    }

    #[test]
    fn unmarked_dynamic_rtp_payload_type_is_not_rtcp() {
        // byte1 == 96 is RTP with M=0, PT=96 (a common dynamic payload
        // type) — must never be swept into the widened RTCP band.
        assert!(
            !is_rtcp_packet_type(96),
            "an unmarked RTP payload-type byte must not classify as RTCP"
        );
    }

    #[test]
    fn band_1_to_191_is_deliberately_excluded_from_rtcp() {
        // See RTCP_MUX_TYPE_MIN/MAX's doc: folding this band in would
        // misclassify virtually all unmarked RTP traffic (M=0 => byte1 ==
        // PT, PT in 0..=127), so it must stay outside the RTCP band despite
        // RFC 5761 §4 naming it as a last-resort RTCP allocation range.
        assert!(!is_rtcp_packet_type(1));
        assert!(!is_rtcp_packet_type(96));
        assert!(!is_rtcp_packet_type(191));
    }

    // -----------------------------------------------------------------------
    // RFC 3711 Appendix B.3 outbound SRTP (item 1). Mirrors
    // `tests/srtp_rfc3711_vectors.rs`'s
    // `srtp_context_reproduces_appendix_b3_ciphertext` (the existing
    // decrypt-direction fixture test), but exercises `MediaTransport`'s own
    // new `encrypt_rtp`/`encrypt_rtcp` — not just the underlying
    // `rtc_srtp::context::Context` the fixture test already covers
    // directly.
    // -----------------------------------------------------------------------

    /// RFC 3711 Appendix B.3 master key/salt — same values as
    /// `tests/srtp_rfc3711_vectors.rs`'s `B3_MASTER_KEY`/`B3_MASTER_SALT`.
    const APPENDIX_B3_MASTER_KEY: [u8; 16] = [
        0xE1, 0xF9, 0x7A, 0x0D, 0x3E, 0x01, 0x8B, 0xE0, 0xD6, 0x4F, 0xA3, 0x2C, 0x06, 0xDE, 0x41,
        0x39,
    ];
    const APPENDIX_B3_MASTER_SALT: [u8; 14] = [
        0x0E, 0xC6, 0x75, 0xAD, 0x49, 0x8A, 0xFE, 0xEB, 0xB6, 0x96, 0x0B, 0x3A, 0xAB, 0xE6,
    ];

    /// Builds a `MediaTransport` via the real constructor, then seeds
    /// `srtp_write` directly with the Appendix B.3 vectors — same-module
    /// test access to the private field stands in for a completed DTLS
    /// handshake, which `select_read_write_material`'s own test above
    /// already covers independently.
    fn transport_with_appendix_b3_write_context() -> MediaTransport {
        let mut mt = MediaTransport::new(test_config(SetupRole::Passive)).unwrap();
        mt.srtp_write = Some(
            SrtpContext::new(
                &APPENDIX_B3_MASTER_KEY,
                &APPENDIX_B3_MASTER_SALT,
                ProtectionProfile::Aes128CmHmacSha1_80,
                None,
                None,
                test_crypto().crypto(),
            )
            .unwrap(),
        );
        mt
    }

    #[test]
    fn encrypt_rtp_reproduces_appendix_b3_ciphertext() {
        let mut mt = transport_with_appendix_b3_write_context();
        let packet = rtp_packet::RtpPacket {
            marker: false,
            payload_type: 96,
            sequence_number: 0,
            timestamp: 0,
            ssrc: 0,
            csrc: Vec::new(),
            extension: None,
            padding: None,
            payload: &[0xAAu8; 32],
        };

        let protected = mt.encrypt_rtp(&packet).expect("encrypt_rtp");

        // Independent oracle: a freshly-built `rtc_srtp::context::Context`
        // keyed identically, not the same object `encrypt_rtp` used
        // internally — proves `MediaTransport::encrypt_rtp` (typed-packet
        // serialization + delegation) reproduces the same Appendix B.3
        // ciphertext, not just that the library agrees with itself.
        let mut oracle_ctx = SrtpContext::new(
            &APPENDIX_B3_MASTER_KEY,
            &APPENDIX_B3_MASTER_SALT,
            ProtectionProfile::Aes128CmHmacSha1_80,
            None,
            None,
            test_crypto().crypto(),
        )
        .unwrap();
        let expected = oracle_ctx.encrypt_rtp(&packet.to_bytes()).unwrap();
        assert_eq!(protected, expected.to_vec());

        // And it must decrypt back to the original plaintext.
        let mut dec_ctx = SrtpContext::new(
            &APPENDIX_B3_MASTER_KEY,
            &APPENDIX_B3_MASTER_SALT,
            ProtectionProfile::Aes128CmHmacSha1_80,
            None,
            None,
            test_crypto().crypto(),
        )
        .unwrap();
        assert_eq!(
            dec_ctx.decrypt_rtp(&protected).unwrap().to_vec(),
            packet.to_bytes()
        );
    }

    #[test]
    fn encrypt_rtcp_reproduces_appendix_b3_behaviour() {
        let mut mt = transport_with_appendix_b3_write_context();
        let compound =
            rtcp_packet::CompoundPacket::new(vec![rtcp_packet::RtcpPacket::SenderReport(
                rtcp_packet::SenderReport {
                    ssrc: 0,
                    ntp_msw: 0,
                    ntp_lsw: 0,
                    rtp_timestamp: 0,
                    packet_count: 0,
                    octet_count: 0,
                    report_blocks: Vec::new(),
                },
            )])
            .unwrap();

        let protected = mt.encrypt_rtcp(&compound).expect("encrypt_rtcp");

        let mut dec_ctx = SrtpContext::new(
            &APPENDIX_B3_MASTER_KEY,
            &APPENDIX_B3_MASTER_SALT,
            ProtectionProfile::Aes128CmHmacSha1_80,
            None,
            None,
            test_crypto().crypto(),
        )
        .unwrap();
        assert_eq!(
            dec_ctx.decrypt_rtcp(&protected).unwrap().to_vec(),
            compound.to_bytes()
        );
    }

    #[test]
    fn encrypt_rtp_before_handshake_errors() {
        let mut mt = MediaTransport::new(test_config(SetupRole::Passive)).unwrap();
        let packet = rtp_packet::RtpPacket {
            marker: false,
            payload_type: 96,
            sequence_number: 0,
            timestamp: 0,
            ssrc: 0,
            csrc: Vec::new(),
            extension: None,
            padding: None,
            payload: &[0u8; 4],
        };
        match mt.encrypt_rtp(&packet) {
            Err(Error::Media(_)) => {}
            Err(other) => panic!("expected Error::Media, got {other:?}"),
            Ok(_) => panic!("expected an error: no srtp write context before handshake"),
        }
    }

    // -----------------------------------------------------------------------
    // Key lifetime / rekey (issue #948 item 3): RFC 3711 §8.2/§9.2's
    // `maximum_lifetime` trigger and RFC 5764 §5.2's key-retention rekey
    // path.
    // -----------------------------------------------------------------------

    /// A second master key/salt, distinct from [`APPENDIX_B3_MASTER_KEY`]/
    /// [`APPENDIX_B3_MASTER_SALT`], standing in for the fresh key set a
    /// completed rehandshake would install — used to prove the *retired*
    /// (old) context, not the current one, is what recovers a
    /// reordered/old-keyed packet.
    const OTHER_MASTER_KEY: [u8; 16] = [0xFF; 16];
    const OTHER_MASTER_SALT: [u8; 14] = [0xFF; 14];

    fn b3_srtp_context() -> SrtpContext {
        SrtpContext::new(
            &APPENDIX_B3_MASTER_KEY,
            &APPENDIX_B3_MASTER_SALT,
            ProtectionProfile::Aes128CmHmacSha1_80,
            None,
            None,
            test_crypto().crypto(),
        )
        .unwrap()
    }

    fn other_srtp_context() -> SrtpContext {
        SrtpContext::new(
            &OTHER_MASTER_KEY,
            &OTHER_MASTER_SALT,
            ProtectionProfile::Aes128CmHmacSha1_80,
            None,
            None,
            test_crypto().crypto(),
        )
        .unwrap()
    }

    /// Builds a `MediaTransport` via the real constructor, then seeds
    /// `dtls_peer` + both SRTP contexts directly — same-module test access
    /// standing in for a completed handshake, exactly as
    /// `transport_with_appendix_b3_write_context` already does for the
    /// write-only tests above.
    fn transport_with_completed_handshake(setup: SetupRole) -> (MediaTransport, SocketAddr) {
        let mut mt = MediaTransport::new(test_config(setup)).unwrap();
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        mt.dtls_peer = Some(peer);
        mt.srtp_read = Some(b3_srtp_context());
        mt.srtp_write = Some(b3_srtp_context());
        (mt, peer)
    }

    fn rtp_test_packet(sequence_number: u16) -> rtp_packet::RtpPacket<'static> {
        rtp_packet::RtpPacket {
            marker: false,
            payload_type: 96,
            sequence_number,
            timestamp: 0,
            ssrc: 0,
            csrc: Vec::new(),
            extension: None,
            padding: None,
            payload: &[0xAA; 32],
        }
    }

    #[test]
    fn exceeds_maximum_lifetime_boundary() {
        // Bite test: change `count >= MAXIMUM_LIFETIME_PACKETS` to `>` and
        // the middle assertion flips to failing.
        assert!(!exceeds_maximum_lifetime(MAXIMUM_LIFETIME_PACKETS - 1));
        assert!(exceeds_maximum_lifetime(MAXIMUM_LIFETIME_PACKETS));
        assert!(exceeds_maximum_lifetime(MAXIMUM_LIFETIME_PACKETS + 1));
    }

    #[test]
    fn needs_rekey_reports_threshold_crossing_on_every_counter() {
        let mut mt = MediaTransport::new(test_config(SetupRole::Passive)).unwrap();
        assert!(!mt.needs_rekey());

        mt.write_rtp_count = MAXIMUM_LIFETIME_PACKETS - 1;
        assert!(!mt.needs_rekey(), "one below the limit must not trigger");
        mt.write_rtp_count = MAXIMUM_LIFETIME_PACKETS;
        assert!(
            mt.needs_rekey(),
            "write_rtp_count at the limit must trigger"
        );
        mt.write_rtp_count = 0;

        mt.write_rtcp_count = MAXIMUM_LIFETIME_PACKETS;
        assert!(
            mt.needs_rekey(),
            "write_rtcp_count at the limit must trigger"
        );
        mt.write_rtcp_count = 0;

        mt.read_rtp_count = MAXIMUM_LIFETIME_PACKETS;
        assert!(mt.needs_rekey(), "read_rtp_count at the limit must trigger");
        mt.read_rtp_count = 0;

        mt.read_rtcp_count = MAXIMUM_LIFETIME_PACKETS;
        assert!(
            mt.needs_rekey(),
            "read_rtcp_count at the limit must trigger"
        );
    }

    #[test]
    fn encrypt_rtp_and_rtcp_increment_write_counters() {
        let mut mt = transport_with_appendix_b3_write_context();
        assert_eq!(mt.write_rtp_count, 0);
        assert_eq!(mt.write_rtcp_count, 0);

        mt.encrypt_rtp(&rtp_test_packet(0)).expect("encrypt_rtp");
        assert_eq!(mt.write_rtp_count, 1);
        mt.encrypt_rtp(&rtp_test_packet(1)).expect("encrypt_rtp");
        assert_eq!(mt.write_rtp_count, 2);
        assert_eq!(
            mt.write_rtcp_count, 0,
            "RTP encryption must not touch the RTCP counter"
        );

        let compound =
            rtcp_packet::CompoundPacket::new(vec![rtcp_packet::RtcpPacket::SenderReport(
                rtcp_packet::SenderReport {
                    ssrc: 0,
                    ntp_msw: 0,
                    ntp_lsw: 0,
                    rtp_timestamp: 0,
                    packet_count: 0,
                    octet_count: 0,
                    report_blocks: Vec::new(),
                },
            )])
            .unwrap();
        mt.encrypt_rtcp(&compound).expect("encrypt_rtcp");
        assert_eq!(mt.write_rtcp_count, 1);
        assert_eq!(
            mt.write_rtp_count, 2,
            "RTCP encryption must not touch the RTP counter"
        );
    }

    #[test]
    fn handle_datagram_increments_read_counter_on_successful_decrypt() {
        let (mut mt, peer) = transport_with_completed_handshake(SetupRole::Passive);
        let mut oracle = b3_srtp_context();
        let ciphertext = oracle
            .encrypt_rtp(&rtp_test_packet(0).to_bytes())
            .unwrap()
            .to_vec();

        assert_eq!(mt.read_rtp_count, 0);
        let now = Instant::now();
        let events = mt
            .handle_datagram(now, peer, &ciphertext)
            .expect("handle_datagram");
        assert_eq!(
            events.len(),
            1,
            "must decrypt to exactly one MediaEvent::Rtp"
        );
        assert_eq!(mt.read_rtp_count, 1);
    }

    // Regression (audit run-09 W20): the RFC 3550 §5.3.1 header extension
    // (RFC 8285 CVO/AV1-dependency-descriptor/`mid`/`rid` and similar all
    // ride on it) must survive decrypt into `DecryptedRtp`, not be dropped.
    #[test]
    fn decrypted_rtp_preserves_the_header_extension() {
        let (mut mt, peer) = transport_with_completed_handshake(SetupRole::Passive);
        let mut oracle = b3_srtp_context();
        let mut pkt = rtp_test_packet(0);
        pkt.extension = Some(rtp_packet::HeaderExtension {
            profile_id: 0xBEDE,
            data: &[0xDE, 0xAD, 0xBE, 0xEF],
        });
        let ciphertext = oracle.encrypt_rtp(&pkt.to_bytes()).unwrap().to_vec();

        let events = mt
            .handle_datagram(Instant::now(), peer, &ciphertext)
            .expect("handle_datagram");
        match &events[..] {
            [MediaEvent::Rtp(rtp)] => {
                let ext = rtp
                    .extension
                    .as_ref()
                    .expect("extension must not be dropped");
                assert_eq!(ext.profile_id, 0xBEDE);
                assert_eq!(ext.data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
            }
            other => panic!("expected exactly one MediaEvent::Rtp, got {other:?}"),
        }
    }

    /// r14-RTCP-C1 (#1071): a browser WHEP viewer's SRTCP is mostly RFC 4585
    /// feedback (here a bare PSFB PLI, RFC 4585 §6.3.1, PT=206 FMT=1, sent
    /// alone as Reduced-Size RTCP per RFC 5506 §4.1) — `rtcp-packet` now
    /// decodes it as `RtcpPacket::Unknown` inside `MediaEvent::Rtcp`, not
    /// `RtcpUnsupported`, since `CompoundPacket` no longer requires an
    /// unrecognized leading PT to be SR/RR. Still surfaces as an event, not
    /// the `Err` a forged/stray datagram gets (which multimux's WHEP silence
    /// timer relies on to tell a live viewer from noise).
    #[test]
    fn authenticated_bare_feedback_srtcp_decodes_as_rtcp_unknown_forged_is_an_error() {
        const PLI: [u8; 12] = [
            0x81, 0xCE, 0x00, 0x02, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
        ];
        let (mut mt, peer) = transport_with_completed_handshake(SetupRole::Passive);

        // SRTCP under the wrong key must still fail authentication — but as
        // an `AuthFailure` event, not `Err` (audit run-09 W20): anyone can
        // send a byte in the SRTP/SRTCP band, so this is the expected
        // outcome for spoofed traffic, not a transport error.
        let mut forger = other_srtp_context();
        let forged = forger.encrypt_rtcp(&PLI).unwrap().to_vec();
        let events = mt
            .handle_datagram(Instant::now(), peer, &forged)
            .expect("an authentication failure is an event, not Err");
        assert!(
            matches!(&events[..], [MediaEvent::AuthFailure { is_rtcp: true, .. }]),
            "expected exactly one AuthFailure(is_rtcp: true), got {events:?}"
        );

        let mut oracle = b3_srtp_context();
        let genuine = oracle.encrypt_rtcp(&PLI).unwrap().to_vec();
        let events = mt
            .handle_datagram(Instant::now(), peer, &genuine)
            .expect("an authenticated feedback packet is not an error");
        match &events[..] {
            [MediaEvent::Rtcp(compound)] => {
                assert_eq!(compound.packets.len(), 1);
                match &compound.packets[0] {
                    rtcp_packet::RtcpPacket::Unknown {
                        packet_type,
                        payload,
                        ..
                    } => {
                        assert_eq!(*packet_type, 206);
                        assert_eq!(payload, &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
                    }
                    other => panic!("expected RtcpPacket::Unknown, got {other:?}"),
                }
            }
            other => panic!("expected exactly one MediaEvent::Rtcp, got {other:?}"),
        }
        assert_eq!(mt.read_rtcp_count, 1);
    }

    /// r14-RTCP-C1 (#1071): before the fix, a compound packet carrying a real
    /// SR immediately followed by RFC 4585 PSFB feedback failed
    /// `CompoundPacket::parse` on the unrecognized PT and surfaced as
    /// `RtcpUnsupported`, discarding the parsed SR along with it — the caller
    /// never saw the sender's own stats, only proof of life. After the fix,
    /// the SR is delivered intact and the PSFB packet decodes as
    /// `RtcpPacket::Unknown`, both inside one `MediaEvent::Rtcp`.
    #[test]
    fn compound_sr_then_unrecognized_feedback_still_delivers_the_sr() {
        #[rustfmt::skip]
        const SR_THEN_PLI: [u8; 40] = [
            0x80, 0xC8, 0x00, 0x06, // SR header: V=2, RC=0, PT=200, length=6
            0x11, 0x22, 0x33, 0x44, // SSRC
            0x00, 0x00, 0x00, 0x00, // NTP MSW
            0x00, 0x00, 0x00, 0x00, // NTP LSW
            0x00, 0x00, 0x00, 0x00, // RTP timestamp
            0x00, 0x00, 0x00, 0x00, // packet_count
            0x00, 0x00, 0x00, 0x00, // octet_count
            // PSFB PLI (RFC 4585 §6.3.1, PT=206 FMT=1) immediately follows.
            0x81, 0xCE, 0x00, 0x02, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
        ];
        let (mut mt, peer) = transport_with_completed_handshake(SetupRole::Passive);
        let mut oracle = b3_srtp_context();
        let genuine = oracle.encrypt_rtcp(&SR_THEN_PLI).unwrap().to_vec();

        let events = mt
            .handle_datagram(Instant::now(), peer, &genuine)
            .expect("SR + unrecognized feedback must decrypt");
        let compound = match &events[..] {
            [MediaEvent::Rtcp(cp)] => cp,
            other => panic!("expected exactly one MediaEvent::Rtcp, got {other:?}"),
        };
        assert_eq!(compound.packets.len(), 2);
        match &compound.packets[0] {
            rtcp_packet::RtcpPacket::SenderReport(sr) => assert_eq!(sr.ssrc, 0x1122_3344),
            other => panic!("expected the SR to survive, got {other:?}"),
        }
        match &compound.packets[1] {
            rtcp_packet::RtcpPacket::Unknown { packet_type, .. } => {
                assert_eq!(*packet_type, 206);
            }
            other => panic!("expected the PSFB as Unknown, got {other:?}"),
        }
    }

    /// `RtcpUnsupported` still exists for SRTCP that authenticates but is not
    /// a *valid ordering* of RTCP packets — a lone SDES (a recognized RFC
    /// 3550 §6 type, not SR/RR, and not `RtcpPacket::Unknown`) leading with no
    /// preceding report — distinct from the now-decodable bare-feedback case
    /// above. (A bad version field can't reach this path at all: the SRTP
    /// layer itself validates the RTCP version before it will encrypt.)
    #[test]
    fn authenticated_but_genuinely_malformed_srtcp_is_rtcp_unsupported() {
        // Lone SDES, SC=1, one empty chunk (SSRC + terminator + padding):
        // V=2, PT=202, length=2 -> total_len=12 bytes (the SRTP layer
        // requires a minimum RTCP length, so an empty SC=0 packet is too
        // short to even reach the crypto layer, let alone this crate).
        #[rustfmt::skip]
        const LONE_SDES: [u8; 12] = [
            0x81, 0xCA, 0x00, 0x02, // header: V=2, SC=1, PT=202, length=2
            0x11, 0x22, 0x33, 0x44, // chunk SSRC
            0x00, 0x00, 0x00, 0x00, // item-list terminator + padding
        ];
        let (mut mt, peer) = transport_with_completed_handshake(SetupRole::Passive);
        let mut oracle = b3_srtp_context();
        let genuine = oracle.encrypt_rtcp(&LONE_SDES).unwrap().to_vec();

        let events = mt
            .handle_datagram(Instant::now(), peer, &genuine)
            .expect("an authenticated-but-malformed packet is not an error");
        assert!(
            matches!(&events[..], [MediaEvent::RtcpUnsupported(_)]),
            "expected exactly one RtcpUnsupported, got {events:?}"
        );
        assert_eq!(mt.read_rtcp_count, 1);
    }

    #[test]
    fn rekey_before_any_handshake_errors() {
        let mut mt = MediaTransport::new(test_config(SetupRole::Passive)).unwrap();
        match mt.rekey(Instant::now()) {
            Err(Error::Media(_)) => {}
            Err(other) => panic!("expected Error::Media, got {other:?}"),
            Ok(()) => panic!("expected an error: no dtls handshake has ever completed"),
        }
    }

    #[test]
    fn rekey_on_passive_errors_without_tearing_down_the_association() {
        // Regression (audit run-09 W19): `rekey()` on `SetupRole::Passive`
        // must not tear down the working association. `maybe_start_active_dtls`
        // is a no-op for Passive — it only ever dials out for Active — so
        // doing the teardown anyway would leave the Passive side waiting
        // forever for a `ClientHello` a browser/OBS peer never sends without
        // a new SDP offer, silently ending media for good.
        let (mut mt, peer) = transport_with_completed_handshake(SetupRole::Passive);
        mt.write_rtp_count = 5;
        mt.read_rtp_count = 7;

        match mt.rekey(Instant::now()) {
            Err(Error::Media(_)) => {}
            Err(other) => panic!("expected Error::Media, got {other:?}"),
            Ok(()) => panic!("expected Passive rekey to be refused"),
        }

        // Nothing was torn down: the existing keys and counters are intact,
        // and the still-working association keeps decrypting.
        assert!(mt.srtp_write.is_some(), "write context must not be dropped");
        assert!(
            mt.srtp_read.is_some(),
            "read context must not be moved to retired"
        );
        assert!(mt.retired_srtp_read.is_none());
        assert_eq!(mt.write_rtp_count, 5);
        assert_eq!(mt.read_rtp_count, 7);

        let mut oracle = b3_srtp_context();
        let packet = oracle
            .encrypt_rtp(&rtp_test_packet(0).to_bytes())
            .unwrap()
            .to_vec();
        let events = mt
            .handle_datagram(Instant::now(), peer, &packet)
            .expect("the untouched association keeps decrypting after a refused rekey");
        assert_eq!(events.len(), 1, "expected the RTP packet to still decrypt");
    }

    #[test]
    fn rekey_retires_read_context_and_drops_write_context() {
        // Active, not Passive: Passive's rekey is refused outright (see
        // `rekey_on_passive_errors_without_tearing_down_the_association`
        // above) rather than tearing anything down, so this teardown
        // behaviour is exercised on the role that actually redials.
        let (mut mt, _peer) = transport_with_completed_handshake(SetupRole::Active);
        mt.write_rtp_count = 5;
        mt.read_rtp_count = 7;

        mt.rekey(Instant::now()).expect("rekey");

        assert!(
            mt.srtp_write.is_none(),
            "RFC 3711 §9.2: the write key MUST NOT be reused past rekey"
        );
        assert!(
            mt.srtp_read.is_none(),
            "the read context moves to the retired slot, it is not left in place"
        );
        assert!(
            mt.retired_srtp_read.is_some(),
            "RFC 5764 §5.2: the old read key must be retained, not dropped outright"
        );
        assert_eq!(mt.write_rtp_count, 0);
        assert_eq!(mt.read_rtp_count, 0);
    }

    /// The central bite test for issue #948 item 3, in two parts. Comment
    /// out the `self.retired_srtp_read = Some(...)` line in `rekey` (i.e.
    /// disable the RFC 5764 §5.2 retention this test exists to check), and
    /// part 1 fails: with no current *and* no retired read context, the
    /// reordered packet is silently dropped (0 events) instead of
    /// decrypted.
    #[test]
    fn retired_read_context_decrypts_reordered_packet_until_msl_elapses_then_stops() {
        // Active, not Passive: Passive's rekey is refused outright (see
        // `rekey_on_passive_errors_without_tearing_down_the_association`),
        // so the "no current read context yet" case is exercised on the
        // role that actually redials — `rekey` itself only ever starts a
        // new handshake, it doesn't complete one synchronously, so
        // `srtp_read` is `None` here too until a later handshake completes.
        let (mut mt, peer) = transport_with_completed_handshake(SetupRole::Active);
        let mut oracle = b3_srtp_context();
        // The "in-flight when the rekey happened" packet: encrypted under
        // the about-to-be-retired key, arrives only after `rekey` below.
        let reordered_packet = oracle
            .encrypt_rtp(&rtp_test_packet(0).to_bytes())
            .unwrap()
            .to_vec();

        let t0 = Instant::now();
        mt.rekey(t0).expect("rekey");
        // `rekey` only starts the new handshake (redials); it never
        // completes synchronously, so `srtp_read` stays `None` until a
        // later `handle_datagram` finishes it (never, in this test) —
        // decrypting the reordered packet now depends entirely on
        // `retired_srtp_read`.
        assert!(mt.srtp_read.is_none());

        // Part 1: within the retention window, the retired key recovers it.
        let events = mt
            .handle_datagram(t0, peer, &reordered_packet)
            .expect("handle_datagram within retention window");
        assert_eq!(events.len(), 1, "the reordered packet must still decrypt");
        match &events[0] {
            MediaEvent::Rtp(rtp) => assert_eq!(rtp.sequence_number, 0),
            other => panic!("expected MediaEvent::Rtp, got {other:?}"),
        }

        // Part 2: once RETIRED_KEY_RETENTION has elapsed, the key is
        // purged (via `handle_timeout`) and the same key material can no
        // longer decrypt anything — a *different* reordered packet (a
        // fresh sequence number, so this isn't a replay-window rejection
        // masquerading as "no key") is now dropped silently, same as the
        // pre-handshake case.
        let t1 = t0 + RETIRED_KEY_RETENTION + Duration::from_secs(1);
        mt.handle_timeout(t1);
        assert!(
            mt.retired_srtp_read.is_none(),
            "the retired key must be purged after MSL"
        );

        let another_reordered_packet = oracle
            .encrypt_rtp(&rtp_test_packet(1).to_bytes())
            .unwrap()
            .to_vec();
        let events_after_expiry = mt
            .handle_datagram(t1, peer, &another_reordered_packet)
            .expect("handle_datagram after retention expiry");
        assert!(
            events_after_expiry.is_empty(),
            "past the MSL, there is no key left to decrypt with"
        );
    }

    /// The RFC 5764 §5.2 scenario in its most literal form: a packet
    /// reordered across the rekey boundary arrives *after* a new
    /// handshake has already installed fresh keys, so it fails
    /// authentication under the current context — and only then should
    /// the retired context be tried. Bite test: skip straight to `Err`
    /// on a current-context failure (i.e. never fall back to
    /// `retired_srtp_read`), and this fails — `handle_datagram` would
    /// return `Err` instead of the one decrypted `MediaEvent::Rtp`.
    #[test]
    fn retired_read_context_recovers_packet_that_fails_the_new_key() {
        let (mut mt, peer) = transport_with_completed_handshake(SetupRole::Passive);
        // Simulate "a rekey already happened, and a new handshake already
        // completed": `srtp_read` is the *new* key, `retired_srtp_read` is
        // the *old* one, both present at once — the exact window RFC 5764
        // §5.2 describes.
        mt.srtp_read = Some(other_srtp_context());
        mt.retired_srtp_read = Some((b3_srtp_context(), Instant::now() + RETIRED_KEY_RETENTION));

        let mut oracle = b3_srtp_context();
        let old_keyed_packet = oracle
            .encrypt_rtp(&rtp_test_packet(0).to_bytes())
            .unwrap()
            .to_vec();

        let events = mt
            .handle_datagram(Instant::now(), peer, &old_keyed_packet)
            .expect("must recover via the retired context, not error out");
        assert_eq!(events.len(), 1);
        match &events[0] {
            MediaEvent::Rtp(rtp) => assert_eq!(rtp.sequence_number, 0),
            other => panic!("expected MediaEvent::Rtp, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // DTLS peer-certificate fingerprint verification (RFC 8122 §5, RFC 5764
    // §5). These two run the real handshake between two `MediaTransport`s in
    // process. They live here rather than in `tests/dtls_fingerprint.rs`
    // because a certificate is generated inside `MediaTransport::new`, so
    // through the public API alone side A can only learn B's fingerprint by
    // constructing B — and vice versa, one of them necessarily holds a stale
    // value; these tests pre-generate both certificates and build through
    // the private `with_certificate`. Everything observable from outside
    // (wrong-fingerprint rejection, malformed fingerprints at `new`, the
    // address gate) is covered there.
    // -----------------------------------------------------------------------

    fn reserve_loopback_addr() -> SocketAddr {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        drop(socket);
        addr
    }

    /// The candidate-attribute body `unmarshal_candidate` expects (the SDP
    /// line minus its `a=` prefix).
    fn host_candidate_line(addr: SocketAddr) -> String {
        format!("1 1 udp 2130706431 {} {} typ host", addr.ip(), addr.port())
    }

    struct LoopbackPair {
        a: MediaTransport,
        b: MediaTransport,
        a_addr: SocketAddr,
        b_addr: SocketAddr,
    }

    /// Two transports wired together in-process with each other's genuine
    /// fingerprint: A is the DTLS client + ICE controlling agent, B the
    /// server + controlled — the same role split as a WHIP publisher facing
    /// this crate's passive ingest side. Both certificates are pre-generated
    /// so each side can be constructed already holding the other's real
    /// digest (the verify closure captures it at construction time).
    fn loopback_pair() -> LoopbackPair {
        loopback_pair_with_max_remote(MAX_REMOTE_CANDIDATES)
    }

    /// Same as [`loopback_pair`], but with both sides'
    /// [`MediaTransportConfig::max_remote_candidates`] set to `cap` instead
    /// of the default — for tests that need to exhaust a small cap (e.g.
    /// the STUN-source-address one below).
    fn loopback_pair_with_max_remote(cap: usize) -> LoopbackPair {
        let a_addr = reserve_loopback_addr();
        let b_addr = reserve_loopback_addr();
        let cert_a = Certificate::generate_self_signed(
            vec!["localhost".to_string()],
            test_crypto().crypto(),
        )
        .unwrap();
        let cert_b = Certificate::generate_self_signed(
            vec!["localhost".to_string()],
            test_crypto().crypto(),
        )
        .unwrap();
        let fp_a = sha256_fingerprint(cert_a.certificate[0].as_ref());
        let fp_b = sha256_fingerprint(cert_b.certificate[0].as_ref());
        let mut a = MediaTransport::with_certificate(
            MediaTransportConfig {
                local_addr: a_addr,
                local_ice_ufrag: "loopa0ufrag".into(),
                local_ice_pwd: "loopa-ice-password-0000000".into(),
                remote_ice_ufrag: "loopb0ufrag".into(),
                remote_ice_pwd: "loopb-ice-password-0000000".into(),
                is_controlling: true,
                local_setup: SetupRole::Active,
                stun_server: None,
                remote_fingerprint: format!("sha-256 {fp_b}"),
                max_remote_candidates: cap,
            },
            parse_fingerprint_value(&format!("sha-256 {fp_b}")).unwrap(),
            cert_a,
            test_crypto(),
        )
        .unwrap();
        let mut b = MediaTransport::with_certificate(
            MediaTransportConfig {
                local_addr: b_addr,
                local_ice_ufrag: "loopb0ufrag".into(),
                local_ice_pwd: "loopb-ice-password-0000000".into(),
                remote_ice_ufrag: "loopa0ufrag".into(),
                remote_ice_pwd: "loopa-ice-password-0000000".into(),
                is_controlling: false,
                local_setup: SetupRole::Passive,
                stun_server: None,
                remote_fingerprint: format!("sha-256 {fp_a}"),
                max_remote_candidates: cap,
            },
            parse_fingerprint_value(&format!("sha-256 {fp_a}")).unwrap(),
            cert_b,
            test_crypto(),
        )
        .unwrap();
        a.add_remote_candidate(&host_candidate_line(b_addr))
            .unwrap();
        b.add_remote_candidate(&host_candidate_line(a_addr))
            .unwrap();
        LoopbackPair {
            a,
            b,
            a_addr,
            b_addr,
        }
    }

    /// Pump datagrams between the pair until both report
    /// `DtlsHandshakeComplete` (panicking after `budget`), recording every
    /// DTLS-band datagram A sent — its first is the ClientHello, reused by
    /// the address-gate test.
    fn pump_until_both_complete(pair: &mut LoopbackPair, budget: Duration) -> Vec<Vec<u8>> {
        let deadline = Instant::now() + budget;
        let mut a_dtls_datagrams = Vec::new();
        let (mut a_done, mut b_done) = (false, false);
        while Instant::now() < deadline && !(a_done && b_done) {
            let now = Instant::now();
            let mut progressed = false;
            while let Some(dgram) = pair.a.poll_transmit() {
                if dgram.bytes.first().is_some_and(|&b| (20..=63).contains(&b)) {
                    a_dtls_datagrams.push(dgram.bytes.clone());
                }
                for event in pair
                    .b
                    .handle_datagram(now, pair.a_addr, &dgram.bytes)
                    .expect("B feed")
                {
                    if matches!(event, MediaEvent::DtlsHandshakeComplete) {
                        b_done = true;
                    }
                }
                progressed = true;
            }
            while let Some(dgram) = pair.b.poll_transmit() {
                for event in pair
                    .a
                    .handle_datagram(now, pair.b_addr, &dgram.bytes)
                    .expect("A feed")
                {
                    if matches!(event, MediaEvent::DtlsHandshakeComplete) {
                        a_done = true;
                    }
                }
                progressed = true;
            }
            pair.a.handle_timeout(now);
            pair.b.handle_timeout(now);
            if !progressed {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        assert!(
            a_done && b_done,
            "two MediaTransports holding each other's real fingerprint must complete the DTLS handshake"
        );
        a_dtls_datagrams
    }

    #[test]
    fn dtls_accepts_matching_fingerprint() {
        let mut pair = loopback_pair();
        pump_until_both_complete(&mut pair, Duration::from_secs(8));

        // The happy path still works end to end: media flows in both
        // directions under the keys the completed handshake installed.
        let protected = pair.a.encrypt_rtp(&rtp_test_packet(7)).unwrap();
        let events = pair
            .b
            .handle_datagram(Instant::now(), pair.a_addr, &protected)
            .unwrap();
        assert!(
            matches!(&events[..], [MediaEvent::Rtp(rtp)] if rtp.sequence_number == 7),
            "B must decrypt A's RTP: {events:?}"
        );

        let protected = pair.b.encrypt_rtp(&rtp_test_packet(8)).unwrap();
        let events = pair
            .a
            .handle_datagram(Instant::now(), pair.b_addr, &protected)
            .unwrap();
        assert!(
            matches!(&events[..], [MediaEvent::Rtp(rtp)] if rtp.sequence_number == 8),
            "A must decrypt B's RTP: {events:?}"
        );
    }

    #[test]
    fn dtls_from_non_selected_address_is_ignored() {
        let mut pair = loopback_pair();
        let a_dtls_datagrams = pump_until_both_complete(&mut pair, Duration::from_secs(8));
        assert!(!a_dtls_datagrams.is_empty());
        let intruder_addr: SocketAddr = "127.0.0.1:59999".parse().unwrap();

        // A DTLS ClientHello from an address that is not the selected ICE
        // pair's produces no event, and nothing is ever sent back to it.
        let events = pair
            .a
            .handle_datagram(Instant::now(), intruder_addr, &a_dtls_datagrams[0])
            .unwrap();
        assert!(
            events.is_empty(),
            "DTLS from a non-selected address must produce no events"
        );
        while let Some(dgram) = pair.a.poll_transmit() {
            assert_ne!(
                dgram.peer, intruder_addr,
                "must not answer DTLS at a non-selected address"
            );
        }

        // The SRTP keys are unchanged: the real peer's media still decrypts.
        let protected = pair.b.encrypt_rtp(&rtp_test_packet(9)).unwrap();
        let events = pair
            .a
            .handle_datagram(Instant::now(), pair.b_addr, &protected)
            .unwrap();
        assert!(
            matches!(&events[..], [MediaEvent::Rtp(rtp)] if rtp.sequence_number == 9),
            "media from the real peer must still decrypt afterwards: {events:?}"
        );

        // And before any pair is selected at all, DTLS datagrams are dropped
        // too — a fresh transport never even starts an association.
        let mut fresh = MediaTransport::new(test_config(SetupRole::Passive)).unwrap();
        let events = fresh
            .handle_datagram(Instant::now(), pair.b_addr, &a_dtls_datagrams[0])
            .unwrap();
        assert!(events.is_empty());
        while let Some(dgram) = fresh.poll_transmit() {
            assert_ne!(
                dgram.peer, pair.b_addr,
                "no association may start before ICE selects a pair"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Remote-candidate cap vs. `rtc-ice`'s own peer-reflexive candidate
    // creation. RFC 8445 §19.5.1 / §6.1.2.5: `rtc-ice` 0.20.0 authenticates
    // (USERNAME + MESSAGE-INTEGRITY) and admits a STUN Binding Request from
    // *any* source address, creating a new peer-reflexive remote candidate
    // for one it doesn't already recognize — entirely bypassing
    // `add_remote_candidate`'s own cap, since that method is never called
    // for it. The untrusted peer (any WHEP viewer) already knows
    // ice-ufrag/ice-pwd from the negotiated SDP, so it can authenticate as
    // many requests as it likes from as many source ports as it likes.
    // -----------------------------------------------------------------------

    /// Build an authenticated STUN Binding Request (RFC 8489 §7.3, class
    /// `CLASS_REQUEST`) exactly as `rtc-ice`'s own inbound check
    /// (`agent::mod::handle_inbound`'s `assert_inbound_username` /
    /// `assert_inbound_message_integrity`) verifies one: USERNAME
    /// `"{local_ufrag}:{remote_ufrag}"` (the receiving side's own ufrag,
    /// then the ufrag it was configured to expect from its peer) and
    /// MESSAGE-INTEGRITY keyed with the receiving side's local ICE
    /// password — the two credentials an SDP-holding peer already has.
    fn authenticated_binding_request(
        local_ufrag: &str,
        remote_ufrag: &str,
        local_pwd: &str,
    ) -> Vec<u8> {
        use rtc_stun::attributes::ATTR_USERNAME;
        use rtc_stun::fingerprint::FINGERPRINT;
        use rtc_stun::integrity::MessageIntegrity;
        use rtc_stun::message::{BINDING_REQUEST, Message, TransactionId};
        use rtc_stun::textattrs::Username;

        let username = format!("{local_ufrag}:{remote_ufrag}");
        let crypto = test_crypto();
        let mut msg = Message::new();
        msg.build(&[
            Box::new(BINDING_REQUEST),
            Box::new(TransactionId::new()),
            Box::new(Username::new(ATTR_USERNAME, username)),
            Box::new(MessageIntegrity::new_short_term_integrity_with_provider(
                local_pwd.to_string(),
                crypto.crypto(),
            )),
            Box::new(FINGERPRINT),
        ])
        .expect("build authenticated stun binding request");
        msg.raw
    }

    #[test]
    fn stun_from_new_addresses_is_capped_and_original_peer_still_flows() {
        // Bite test: run this against the unfixed code (no
        // `known_remote_addrs` gate in `handle_stun_datagram`) and it fails.
        // `pair.b.ice.get_remote_candidates_stats(Instant::now())` is `rtc-ice`'s own
        // public stats accessor — it walks the agent's private
        // `remote_candidates` list directly, so it observes the *real*
        // peer-reflexive-candidate growth this fix is supposed to stop, not
        // just this crate's own bookkeeping. Pre-fix, this reached 11 (1
        // explicit + 10 authenticated peer-reflexive admissions), not
        // `small_cap`.
        let small_cap = 3;
        let mut pair = loopback_pair_with_max_remote(small_cap);
        pump_until_both_complete(&mut pair, Duration::from_secs(8));

        // B already knows exactly one remote candidate (A's host address,
        // added by `loopback_pair_with_max_remote`).
        assert_eq!(
            pair.b.ice.get_remote_candidates_stats(Instant::now()).len(),
            1
        );
        assert_eq!(pair.b.known_remote_addrs.len(), 1);

        // Any WHEP viewer already knows B's ufrag/pwd (both are plain SDP
        // attributes) and B's expected remote ufrag (ditto) — but not a real
        // ICE candidate address: ten new local UDP sockets stand in for ten
        // distinct, never-negotiated source ports.
        let new_source_request = authenticated_binding_request(
            "loopb0ufrag",
            "loopa0ufrag",
            "loopb-ice-password-0000000",
        );
        for _ in 0..10 {
            let new_source_addr = reserve_loopback_addr();
            pair.b
                .handle_datagram(Instant::now(), new_source_addr, &new_source_request)
                .expect("a capped-and-dropped datagram must not error");
        }

        let remote_candidates = pair.b.ice.get_remote_candidates_stats(Instant::now()).len();
        assert!(
            remote_candidates <= small_cap,
            "at most {small_cap} remote candidates may ever be tracked by the agent, got {remote_candidates}"
        );
        assert!(
            pair.b.known_remote_addrs.len() <= small_cap,
            "at most {small_cap} remote candidates may ever be tracked, got {}",
            pair.b.known_remote_addrs.len()
        );

        // Media from the original, legitimate peer must still flow — the
        // cap must reject only the new/unauthorized addresses, not disturb
        // the already-established session.
        let protected = pair.a.encrypt_rtp(&rtp_test_packet(42)).unwrap();
        let events = pair
            .b
            .handle_datagram(Instant::now(), pair.a_addr, &protected)
            .unwrap();
        assert!(
            matches!(&events[..], [MediaEvent::Rtp(rtp)] if rtp.sequence_number == 42),
            "original peer's media must still flow after the flood of new-source requests: {events:?}"
        );
    }
}
