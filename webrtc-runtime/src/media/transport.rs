//! [`MediaTransport`] — see the `media` module doc for the full picture.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use broadcast_common::{Parse, Serialize};
use bytes::BytesMut;
use rtc_dtls::config::{ConfigBuilder, HandshakeConfig};
use rtc_dtls::crypto::Certificate;
use rtc_dtls::endpoint::{Endpoint as DtlsEndpoint, EndpointEvent};
use rtc_dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use rtc_ice::agent::agent_config::AgentConfig;
use rtc_ice::agent::{Agent as IceAgent, Event as IceAgentEvent};
use rtc_ice::candidate::candidate_host::CandidateHostConfig;
use rtc_ice::candidate::candidate_server_reflexive::CandidateServerReflexiveConfig;
use rtc_ice::candidate::{CandidateConfig, CandidateType, unmarshal_candidate};
use rtc_ice::mdns::MulticastDnsMode;
use rtc_shared::crypto::KeyingMaterialExporter;
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
    /// A STUN server to gather a server-reflexive candidate from, if any
    /// (RFC 8445 §5.1.1.2). `None` gathers a host candidate only.
    pub stun_server: Option<SocketAddr>,
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
    local_setup: SetupRole,
    ice: IceAgent,
    dtls: DtlsEndpoint,
    /// Set only for [`SetupRole::Active`]: the client-role handshake config
    /// [`Self::maybe_start_active_dtls`] hands to [`DtlsEndpoint::connect`]
    /// once ICE has nominated a pair (see that method's doc). `None` for
    /// [`SetupRole::Passive`], which never dials out.
    dtls_client_config: Option<Arc<HandshakeConfig>>,
    srtp_read: Option<SrtpContext>,
    srtp_write: Option<SrtpContext>,
    gather: Option<StunGather>,
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
    /// Returns [`Error::Media`] if `config.local_setup` is
    /// [`SetupRole::ActPass`] (never a role a concrete transport can be
    /// built with — see [`MediaTransportConfig::local_setup`]), or if
    /// certificate generation or ICE/DTLS setup fails.
    pub fn new(config: MediaTransportConfig) -> Result<Self, Error> {
        if config.local_setup == SetupRole::ActPass {
            return Err(Error::Media(
                "local_setup ActPass is only valid as an SDP offer's a=setup value; the \
                 answer (or this side's own choice, if answering) must resolve to Active or \
                 Passive before a MediaTransport can be built"
                    .to_string(),
            ));
        }
        let is_client = config.local_setup == SetupRole::Active;

        let certificate = Certificate::generate_self_signed(vec!["localhost".to_string()])
            .map_err(|e| Error::Media(format!("generate self-signed certificate: {e}")))?;
        let local_fingerprint = sha256_fingerprint(certificate.certificate[0].as_ref());

        let agent_config = AgentConfig {
            local_ufrag: config.local_ice_ufrag.clone(),
            local_pwd: config.local_ice_pwd.clone(),
            is_controlling: config.is_controlling,
            multicast_dns_mode: MulticastDnsMode::Disabled,
            candidate_types: vec![CandidateType::Host, CandidateType::ServerReflexive],
            ..Default::default()
        };
        let mut ice = IceAgent::new(Arc::new(agent_config))
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
            config.is_controlling,
            config.remote_ice_ufrag.clone(),
            config.remote_ice_pwd.clone(),
        )
        .map_err(|e| Error::Media(format!("start connectivity checks: {e}")))?;

        // RFC 5764 §4.1: is_client picks which handshake role rtc-dtls
        // builds this config for. `remote_addr: None` — this cut never sets
        // an explicit `server_name`, so `ConfigBuilder::build` would fall
        // back to a remote IP-derived name, but `with_insecure_skip_verify`
        // above means server_name is never actually checked either way (see
        // the module doc: WebRTC authenticates by the SDP-signalled
        // fingerprint, RFC 8122, not a CA chain / hostname).
        let handshake_config = Arc::new(
            ConfigBuilder::default()
                .with_certificates(vec![certificate])
                .with_srtp_protection_profiles(vec![OFFERED_SRTP_PROFILE])
                .with_insecure_skip_verify(true)
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
            Some(server) => Some(StunGather::new(config.local_addr, server)?),
            None => None,
        };

        Ok(Self {
            local_addr: config.local_addr,
            local_fingerprint,
            local_setup: config.local_setup,
            ice,
            dtls,
            dtls_client_config,
            srtp_read: None,
            srtp_write: None,
            gather,
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
    pub fn add_remote_candidate(&mut self, candidate: &str) -> Result<(), Error> {
        let c = unmarshal_candidate(candidate)
            .map_err(|e| Error::Media(format!("unmarshal remote candidate {candidate:?}: {e}")))?;
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
        if let Some(gather) = &mut self.gather {
            if let Some(msg) = gather.poll_transmit() {
                return Some(Datagram {
                    peer: msg.transport.peer_addr,
                    bytes: msg.message.to_vec(),
                });
            }
        }
        None
    }

    /// Drive ICE/DTLS/STUN-gather timers. Call periodically (the underlying
    /// `rtc-ice`/`rtc-stun` retransmission schedules are sub-second) even
    /// when no datagrams are arriving.
    pub fn handle_timeout(&mut self, now: Instant) {
        let _ = Protocol::handle_timeout(&mut self.ice, now);
        let peers: Vec<SocketAddr> = self.dtls.get_connections_keys().copied().collect();
        for peer in peers {
            let _ = self.dtls.handle_timeout(peer, now);
        }
        if let Some(gather) = &mut self.gather {
            gather.handle_timeout(now);
        }
        if self.gather.as_ref().is_some_and(StunGather::done) {
            self.gather = None;
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
            self.handle_srtp_datagram(data, &mut events)?;
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

        while let Some(evt) = Protocol::poll_event(&mut self.ice) {
            if let IceAgentEvent::SelectedCandidatePairChange(_, remote) = &evt {
                self.maybe_start_active_dtls(remote.addr())?;
            }
            if let Some(mapped) = map_ice_event(evt) {
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
    fn maybe_start_active_dtls(&mut self, remote_addr: SocketAddr) -> Result<(), Error> {
        let Some(client_config) = self.dtls_client_config.clone() else {
            return Ok(());
        };
        self.dtls
            .connect(remote_addr, client_config, None)
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
            }
        }
        Ok(())
    }

    fn handle_srtp_datagram(
        &mut self,
        data: &[u8],
        events: &mut Vec<MediaEvent>,
    ) -> Result<(), Error> {
        let Some(ctx) = self.srtp_read.as_mut() else {
            // SRTP arrived before the DTLS handshake finished; there is no
            // context to decrypt with yet (RFC 3711), so drop it.
            return Ok(());
        };

        let is_rtcp = data.get(1).is_some_and(|&pt| is_rtcp_packet_type(pt));

        if is_rtcp {
            let plaintext = ctx
                .decrypt_rtcp(data)
                .map_err(|e| Error::Media(format!("srtcp decrypt: {e}")))?;
            let compound = rtcp_packet::CompoundPacket::parse(&plaintext)
                .map_err(|e| Error::Media(format!("rtcp parse: {e}")))?;
            events.push(MediaEvent::Rtcp(compound));
        } else {
            let plaintext = ctx
                .decrypt_rtp(data)
                .map_err(|e| Error::Media(format!("srtp decrypt: {e}")))?;
            let pkt = rtp_packet::RtpPacket::parse(&plaintext)
                .map_err(|e| Error::Media(format!("rtp parse: {e}")))?;
            events.push(MediaEvent::Rtp(DecryptedRtp {
                marker: pkt.marker,
                payload_type: pkt.payload_type,
                sequence_number: pkt.sequence_number,
                timestamp: pkt.timestamp,
                ssrc: pkt.ssrc,
                csrc: pkt.csrc.clone(),
                payload: pkt.payload.to_vec(),
            }));
        }
        Ok(())
    }

    fn on_dtls_handshake_complete(&mut self, peer: SocketAddr) -> Result<(), Error> {
        let state = self.dtls.get_connection_state(peer).ok_or_else(|| {
            Error::Media("dtls handshake completed but no connection state for peer".to_string())
        })?;

        let srtp_profile = to_srtp_profile(state.srtp_protection_profile())?;
        let key_len = srtp_profile.key_len();
        let salt_len = srtp_profile.salt_len();
        let material = state
            .export_keying_material(SRTP_KEYING_MATERIAL_LABEL, &[], 2 * (key_len + salt_len))
            .map_err(|e| Error::Media(format!("export srtp keying material: {e}")))?;

        let client_key = &material[0..key_len];
        let server_key = &material[key_len..2 * key_len];
        let client_salt = &material[2 * key_len..2 * key_len + salt_len];
        let server_salt = &material[2 * key_len + salt_len..2 * key_len + 2 * salt_len];

        let ((read_key, read_salt), (write_key, write_salt)) = select_read_write_material(
            state.is_client(),
            (client_key, client_salt),
            (server_key, server_salt),
        );

        let read_ctx = SrtpContext::new(read_key, read_salt, srtp_profile, None, None)
            .map_err(|e| Error::Media(format!("build srtp decrypt context: {e}")))?;
        let write_ctx = SrtpContext::new(write_key, write_salt, srtp_profile, None, None)
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
        Ok(protected.to_vec())
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

fn map_ice_event(evt: IceAgentEvent) -> Option<MediaEvent> {
    match evt {
        IceAgentEvent::ConnectionStateChange(state) => {
            Some(MediaEvent::IceStateChanged(state.to_string()))
        }
        IceAgentEvent::SelectedCandidatePairChange(..) | IceAgentEvent::RoleChange(_) => None,
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
        }
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
}
