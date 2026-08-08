//! [`MediaTransport`] — see the `media` module doc for the full picture.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use broadcast_common::Parse;
use bytes::BytesMut;
use rtc_dtls::config::ConfigBuilder;
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
/// RFC 5761 §4: RTCP packet types occupy `[192, 223]` once RTP and RTCP are
/// multiplexed on one port (`a=rtcp-mux`) — see the `media` module doc.
const RTCP_MUX_TYPE_MIN: u8 = 192;
const RTCP_MUX_TYPE_MAX: u8 = 223;

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
    /// Only [`SetupRole::Passive`] is implemented by [`MediaTransport::new`]
    /// in this cut (see its doc) — it covers WHIP ingest and the common
    /// WHEP case, since browsers conventionally choose to be the DTLS
    /// client regardless of who offered.
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
    ice: IceAgent,
    dtls: DtlsEndpoint,
    srtp_read: Option<SrtpContext>,
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
    /// Returns [`Error::Media`] if `config.local_setup` is not
    /// [`SetupRole::Passive`] (the only role implemented in this cut — see
    /// [`MediaTransportConfig::local_setup`]), or if certificate generation
    /// or ICE/DTLS setup fails.
    pub fn new(config: MediaTransportConfig) -> Result<Self, Error> {
        if config.local_setup != SetupRole::Passive {
            return Err(Error::Media(format!(
                "local_setup {} not yet supported: this cut only implements the DTLS-server \
                 (passive) role, which covers WHIP ingest and the common WHEP case (browsers \
                 conventionally choose to be the DTLS client)",
                config.local_setup
            )));
        }

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

        let handshake_config = ConfigBuilder::default()
            .with_certificates(vec![certificate])
            .with_srtp_protection_profiles(vec![OFFERED_SRTP_PROFILE])
            .with_insecure_skip_verify(true)
            .build(false, None)
            .map_err(|e| Error::Media(format!("build dtls handshake config: {e}")))?;
        let dtls = DtlsEndpoint::new(
            config.local_addr,
            TransportProtocol::UDP,
            Some(Arc::new(handshake_config)),
        );

        let gather = match config.stun_server {
            Some(server) => Some(StunGather::new(config.local_addr, server)?),
            None => None,
        };

        Ok(Self {
            local_addr: config.local_addr,
            local_fingerprint,
            ice,
            dtls,
            srtp_read: None,
            gather,
        })
    }

    /// The SHA-256 fingerprint of this side's self-signed DTLS certificate
    /// (RFC 8122), colon-hex formatted exactly as the SDP `a=fingerprint`
    /// value expects after its `sha-256 ` prefix.
    pub fn local_fingerprint(&self) -> &str {
        &self.local_fingerprint
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
            if let Some(mapped) = map_ice_event(evt) {
                events.push(mapped);
            }
        }
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

        let is_rtcp = data
            .get(1)
            .is_some_and(|&pt| (RTCP_MUX_TYPE_MIN..=RTCP_MUX_TYPE_MAX).contains(&pt));

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

        // RFC 5764 §4.2: the exported material contains both sides' write
        // keys. We decrypt what the REMOTE peer wrote, so we need the
        // remote's write key — the client's key/salt if the remote is the
        // DTLS client (i.e. we are the server), or the server's if the
        // remote is the DTLS server (i.e. we are the client).
        let (read_key, read_salt) = if state.is_client() {
            (server_key, server_salt)
        } else {
            (client_key, client_salt)
        };

        let ctx = SrtpContext::new(read_key, read_salt, srtp_profile, None, None)
            .map_err(|e| Error::Media(format!("build srtp decrypt context: {e}")))?;
        self.srtp_read = Some(ctx);
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

fn map_ice_event(evt: IceAgentEvent) -> Option<MediaEvent> {
    match evt {
        IceAgentEvent::ConnectionStateChange(state) => {
            Some(MediaEvent::IceStateChanged(state.to_string()))
        }
        IceAgentEvent::SelectedCandidatePairChange(..) | IceAgentEvent::RoleChange(_) => None,
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

    #[test]
    fn rejects_non_passive_setup_role() {
        let config = MediaTransportConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            local_ice_ufrag: "u".into(),
            local_ice_pwd: "p".into(),
            remote_ice_ufrag: "ru".into(),
            remote_ice_pwd: "rp".into(),
            is_controlling: false,
            local_setup: SetupRole::Active,
            stun_server: None,
        };
        match MediaTransport::new(config) {
            Err(Error::Media(_)) => {}
            Err(other) => panic!("expected Error::Media, got {other:?}"),
            Ok(_) => panic!("expected an error for local_setup: Active"),
        }
    }
}
