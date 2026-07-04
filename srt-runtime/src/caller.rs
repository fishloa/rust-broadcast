//! Caller-side handshake engine — `draft-sharabayko-srt-01` §4.3.1
//! (Caller-Listener Handshake), Caller role.
//!
//! [`CallerHandshake`] is a driveable, sans-IO state machine:
//! [`CallerHandshake::start`] returns the INDUCTION handshake bytes to send;
//! [`CallerHandshake::feed`] consumes an inbound (already-parsed)
//! [`ControlPacket`] and returns the [`HandshakeOutput`]s produced — further
//! bytes to send, the negotiated parameters on success, or a rejection. No
//! sockets, no clock: retransmit timing is driven by
//! [`CallerHandshake::tick`] calls from the caller.
//!
//! Flow implemented (§4.3.1.1 / §4.3.1.2):
//!
//! 1. `start()`: send INDUCTION (Version 4, Encryption Field 0, Extension
//!    Field 2, SYN Cookie 0).
//! 2. `feed()` the Listener's INDUCTION response: validate Version 5 and the
//!    SRT magic code `0x4A17`, capture the cookie and Listener's Socket ID,
//!    then send CONCLUSION (Version 5, the captured cookie, HSREQ + optional
//!    Stream ID / Group extensions).
//! 3. `feed()` the Listener's CONCLUSION response: validate it, decode its
//!    Handshake Extension Message, and reach [`CallerHandshakeState::Connected`]
//!    with [`NegotiatedParams`] — or [`CallerHandshakeState::Rejected`] on any
//!    validation failure or explicit peer rejection.

use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::handshake_sm::{
    self, HANDSHAKE_VERSION_4, HANDSHAKE_VERSION_5, HandshakeConfig, HandshakeOutput,
    INDUCTION_LEGACY_SOCKET_TYPE, NegotiatedParams, RejectionReason, SRT_MAGIC_CODE,
};
use crate::packet::{
    ControlPacket, EncryptionField, ExtensionType, HandshakeExtensionFlags, HandshakeExtensions,
    HandshakePacket, HandshakeType, HsExtMessage,
};

/// Caller-side handshake lifecycle state (`draft-sharabayko-srt-01` §4.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CallerHandshakeState {
    /// No handshake message sent yet.
    Idle,
    /// INDUCTION sent; awaiting the Listener's INDUCTION response.
    AwaitingInductionResponse,
    /// CONCLUSION sent; awaiting the Listener's CONCLUSION response.
    AwaitingConclusionResponse,
    /// The handshake completed; [`NegotiatedParams`] are available.
    Connected,
    /// The handshake was rejected (locally, or by the peer).
    Rejected,
    /// No response arrived after the configured retry budget.
    TimedOut,
}

impl CallerHandshakeState {
    /// A short label for this state.
    pub fn name(&self) -> &'static str {
        match self {
            CallerHandshakeState::Idle => "Idle",
            CallerHandshakeState::AwaitingInductionResponse => "AwaitingInductionResponse",
            CallerHandshakeState::AwaitingConclusionResponse => "AwaitingConclusionResponse",
            CallerHandshakeState::Connected => "Connected",
            CallerHandshakeState::Rejected => "Rejected",
            CallerHandshakeState::TimedOut => "TimedOut",
        }
    }
}

impl core::fmt::Display for CallerHandshakeState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// A driveable Caller-side SRT handshake (`draft-sharabayko-srt-01` §4.3.1).
#[derive(Debug)]
pub struct CallerHandshake {
    own_socket_id: u32,
    config: HandshakeConfig,
    state: CallerHandshakeState,
    peer_socket_id: u32,
    syn_cookie: u32,
    last_sent: Option<Vec<u8>>,
    ticks_since_send: u32,
    retries: u32,
    negotiated: Option<NegotiatedParams>,
}

impl CallerHandshake {
    /// Creates a fresh Caller handshake in [`CallerHandshakeState::Idle`].
    pub fn new(own_socket_id: u32, config: HandshakeConfig) -> Self {
        CallerHandshake {
            own_socket_id,
            config,
            state: CallerHandshakeState::Idle,
            peer_socket_id: 0,
            syn_cookie: 0,
            last_sent: None,
            ticks_since_send: 0,
            retries: 0,
            negotiated: None,
        }
    }

    /// The current state.
    pub fn state(&self) -> CallerHandshakeState {
        self.state
    }

    /// The negotiated parameters, once [`CallerHandshakeState::Connected`].
    pub fn negotiated(&self) -> Option<&NegotiatedParams> {
        self.negotiated.as_ref()
    }

    /// Builds the initial INDUCTION handshake (§4.3.1.1) and transitions to
    /// [`CallerHandshakeState::AwaitingInductionResponse`].
    ///
    /// # Errors
    /// [`Error::HandshakeOutOfSequence`] if called more than once.
    pub fn start(&mut self) -> Result<Vec<u8>> {
        if self.state != CallerHandshakeState::Idle {
            return Err(Error::HandshakeOutOfSequence {
                state: self.state.name(),
                reason: "start() called after the handshake already began",
            });
        }
        let hp = HandshakePacket {
            timestamp: 0,
            dest_socket_id: 0, // §4.3.1.1: 0 is interpreted as a connection request.
            version: HANDSHAKE_VERSION_4,
            encryption_field: EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(INDUCTION_LEGACY_SOCKET_TYPE),
            initial_seq_number: self.config.initial_seq_number,
            mtu: self.config.mtu,
            max_flow_window_size: self.config.max_flow_window_size,
            handshake_type: HandshakeType::Induction,
            srt_socket_id: self.own_socket_id,
            syn_cookie: 0,
            peer_ip: self.config.local_ip,
            extensions: HandshakeExtensions(&[]),
        };
        let bytes = handshake_sm::build_bytes(hp)?;
        self.last_sent = Some(bytes.clone());
        self.ticks_since_send = 0;
        self.state = CallerHandshakeState::AwaitingInductionResponse;
        Ok(bytes)
    }

    /// Feeds an inbound control packet. Only meaningful while awaiting the
    /// Listener's INDUCTION or CONCLUSION response.
    ///
    /// # Errors
    /// [`Error::UnexpectedControlPacket`] if `packet` is not a Handshake
    /// packet; [`Error::HandshakeOutOfSequence`] if fed outside those two
    /// states (a driver bug, not a peer failure).
    pub fn feed(&mut self, packet: &ControlPacket<'_>) -> Result<Vec<HandshakeOutput>> {
        let hp = match packet {
            ControlPacket::Handshake(hp) => hp,
            other => {
                return Err(Error::UnexpectedControlPacket {
                    actual: other.control_type().name(),
                });
            }
        };
        match self.state {
            CallerHandshakeState::AwaitingInductionResponse => self.on_induction_response(hp),
            CallerHandshakeState::AwaitingConclusionResponse => self.on_conclusion_response(hp),
            _ => Err(Error::HandshakeOutOfSequence {
                state: self.state.name(),
                reason: "not awaiting a handshake response",
            }),
        }
    }

    /// Convenience wrapper: parses `bytes` as an [`ControlPacket`] then
    /// [`Self::feed`]s it.
    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Result<Vec<HandshakeOutput>> {
        let packet = ControlPacket::parse(bytes)?;
        self.feed(&packet)
    }

    /// Advances retransmit timing by one caller-defined tick. If the
    /// handshake is waiting on a peer response and
    /// [`HandshakeConfig::retransmit_after_ticks`] have elapsed with none
    /// arriving, re-emits the last sent packet; after
    /// [`HandshakeConfig::max_retries`] such retransmissions, transitions to
    /// [`CallerHandshakeState::TimedOut`].
    pub fn tick(&mut self) -> Vec<HandshakeOutput> {
        if !matches!(
            self.state,
            CallerHandshakeState::AwaitingInductionResponse
                | CallerHandshakeState::AwaitingConclusionResponse
        ) {
            return Vec::new();
        }
        self.ticks_since_send += 1;
        if self.ticks_since_send < self.config.retransmit_after_ticks {
            return Vec::new();
        }
        self.ticks_since_send = 0;
        self.retries += 1;
        if self.retries > self.config.max_retries {
            self.state = CallerHandshakeState::TimedOut;
            return vec![HandshakeOutput::TimedOut];
        }
        match self.last_sent.clone() {
            Some(bytes) => vec![HandshakeOutput::Send(bytes)],
            None => Vec::new(),
        }
    }

    fn on_induction_response(&mut self, hp: &HandshakePacket<'_>) -> Result<Vec<HandshakeOutput>> {
        if let Some(reason) = RejectionReason::from_handshake_type(hp.handshake_type) {
            self.state = CallerHandshakeState::Rejected;
            return Ok(vec![HandshakeOutput::Rejected(reason)]);
        }
        if hp.handshake_type != HandshakeType::Induction {
            self.state = CallerHandshakeState::Rejected;
            return Ok(vec![HandshakeOutput::Rejected(RejectionReason::Rogue)]);
        }
        if hp.version != HANDSHAKE_VERSION_5 {
            // Not an SRT party (or an incompatible version) — §4.3.1.1.
            self.state = CallerHandshakeState::Rejected;
            return Ok(vec![HandshakeOutput::Rejected(RejectionReason::Version)]);
        }
        if hp.extension_field.0 != SRT_MAGIC_CODE {
            // §4.3.1.1: "whether the Extension Flags contains the magic
            // value 0x4A17; otherwise the connection is rejected."
            self.state = CallerHandshakeState::Rejected;
            return Ok(vec![HandshakeOutput::Rejected(RejectionReason::Rogue)]);
        }

        self.peer_socket_id = hp.srt_socket_id;
        self.syn_cookie = hp.syn_cookie;

        let hs_msg = HsExtMessage {
            srt_version: self.config.srt_version,
            srt_flags: self.config.flags,
            receiver_tsbpd_delay_ms: self.config.latency_ms,
            sender_tsbpd_delay_ms: self.config.latency_ms,
        };
        let (ext_bytes, ext_flags) = handshake_sm::build_conclusion_extensions(
            ExtensionType::HsReq,
            &hs_msg,
            self.config.stream_id.as_deref(),
            self.config.group,
        )?;

        let hp_out = HandshakePacket {
            timestamp: 0,
            // §4.3.1.2: the socket ID previously received in the induction phase.
            dest_socket_id: self.peer_socket_id,
            version: HANDSHAKE_VERSION_5,
            encryption_field: self.config.encryption_field,
            extension_field: HandshakeExtensionFlags(ext_flags),
            initial_seq_number: self.config.initial_seq_number,
            mtu: self.config.mtu,
            max_flow_window_size: self.config.max_flow_window_size,
            handshake_type: HandshakeType::Conclusion,
            srt_socket_id: self.own_socket_id,
            syn_cookie: self.syn_cookie,
            peer_ip: self.config.local_ip,
            extensions: HandshakeExtensions(&ext_bytes),
        };
        let bytes = handshake_sm::build_bytes(hp_out)?;
        self.last_sent = Some(bytes.clone());
        self.ticks_since_send = 0;
        self.retries = 0;
        self.state = CallerHandshakeState::AwaitingConclusionResponse;
        Ok(vec![HandshakeOutput::Send(bytes)])
    }

    fn on_conclusion_response(&mut self, hp: &HandshakePacket<'_>) -> Result<Vec<HandshakeOutput>> {
        if let Some(reason) = RejectionReason::from_handshake_type(hp.handshake_type) {
            self.state = CallerHandshakeState::Rejected;
            return Ok(vec![HandshakeOutput::Rejected(reason)]);
        }
        if hp.handshake_type != HandshakeType::Conclusion {
            self.state = CallerHandshakeState::Rejected;
            return Ok(vec![HandshakeOutput::Rejected(RejectionReason::Rogue)]);
        }
        if hp.version != HANDSHAKE_VERSION_5 {
            self.state = CallerHandshakeState::Rejected;
            return Ok(vec![HandshakeOutput::Rejected(RejectionReason::Version)]);
        }

        let parsed = match handshake_sm::parse_peer_extensions(hp) {
            Ok(p) => p,
            Err(_) => {
                self.state = CallerHandshakeState::Rejected;
                return Ok(vec![HandshakeOutput::Rejected(RejectionReason::Rogue)]);
            }
        };
        let peer_msg = match parsed.hs_msg {
            Some(m) => m,
            None => {
                self.state = CallerHandshakeState::Rejected;
                return Ok(vec![HandshakeOutput::Rejected(RejectionReason::Rogue)]);
            }
        };

        let negotiated = NegotiatedParams {
            version: HANDSHAKE_VERSION_5,
            flags: crate::packet::HandshakeExtensionMessageFlags(
                self.config.flags.0 & peer_msg.srt_flags.0,
            ),
            latency_ms: handshake_sm::negotiate_latency_ms(self.config.latency_ms, &peer_msg),
            own_socket_id: self.own_socket_id,
            peer_socket_id: self.peer_socket_id,
            stream_id: self.config.stream_id.clone(),
            group: self.config.group,
        };
        self.negotiated = Some(negotiated.clone());
        self.state = CallerHandshakeState::Connected;
        Ok(vec![HandshakeOutput::Connected(negotiated)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::handshake::{HANDSHAKE_TYPE_INDUCTION, HS_EXT_FLAG_HSREQ};

    #[test]
    fn start_is_idempotent_guard() {
        let mut c = CallerHandshake::new(1, HandshakeConfig::default());
        assert!(c.start().is_ok());
        assert!(c.start().is_err());
    }

    #[test]
    fn induction_wire_values_match_draft_4_3_1_1() {
        let mut c = CallerHandshake::new(0xAAAA_BBBB, HandshakeConfig::default());
        let bytes = c.start().unwrap();
        let pkt = ControlPacket::parse(&bytes).unwrap();
        match pkt {
            ControlPacket::Handshake(hp) => {
                assert_eq!(hp.version, HANDSHAKE_VERSION_4);
                assert_eq!(hp.encryption_field, EncryptionField::NoEncryption);
                assert_eq!(hp.extension_field.0, INDUCTION_LEGACY_SOCKET_TYPE);
                assert_eq!(hp.handshake_type.to_bits(), HANDSHAKE_TYPE_INDUCTION);
                assert_eq!(hp.srt_socket_id, 0xAAAA_BBBB);
                assert_eq!(hp.syn_cookie, 0);
                assert_eq!(hp.dest_socket_id, 0);
            }
            _ => panic!("expected handshake"),
        }
        assert_eq!(c.state(), CallerHandshakeState::AwaitingInductionResponse);
    }

    fn induction_response(cookie: u32, listener_id: u32) -> ControlPacket<'static> {
        ControlPacket::Handshake(HandshakePacket {
            timestamp: 0,
            dest_socket_id: 0xAAAA_BBBB,
            version: HANDSHAKE_VERSION_5,
            encryption_field: EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(SRT_MAGIC_CODE),
            initial_seq_number: 0,
            mtu: 1500,
            max_flow_window_size: 8192,
            handshake_type: HandshakeType::Induction,
            srt_socket_id: listener_id,
            syn_cookie: cookie,
            peer_ip: [0; 4],
            extensions: HandshakeExtensions(&[]),
        })
    }

    #[test]
    fn conclusion_carries_the_captured_cookie_and_hsreq() {
        let mut c = CallerHandshake::new(0xAAAA_BBBB, HandshakeConfig::default());
        c.start().unwrap();
        let outputs = c
            .feed(&induction_response(0xC0FF_EE00, 0x1111_2222))
            .unwrap();
        assert_eq!(outputs.len(), 1);
        let bytes = match &outputs[0] {
            HandshakeOutput::Send(b) => b.clone(),
            other => panic!("expected Send, got {other:?}"),
        };
        let pkt = ControlPacket::parse(&bytes).unwrap();
        match pkt {
            ControlPacket::Handshake(hp) => {
                assert_eq!(hp.version, HANDSHAKE_VERSION_5);
                assert_eq!(hp.handshake_type, HandshakeType::Conclusion);
                assert_eq!(hp.syn_cookie, 0xC0FF_EE00);
                assert_eq!(hp.dest_socket_id, 0x1111_2222);
                assert_eq!(hp.extension_field.0 & HS_EXT_FLAG_HSREQ, HS_EXT_FLAG_HSREQ);
                let blocks: Vec<_> = hp.extensions.iter().map(|b| b.unwrap()).collect();
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].ext_type, ExtensionType::HsReq);
            }
            _ => panic!("expected handshake"),
        }
        assert_eq!(c.state(), CallerHandshakeState::AwaitingConclusionResponse);
    }

    #[test]
    fn induction_response_bad_magic_is_rejected() {
        let mut c = CallerHandshake::new(1, HandshakeConfig::default());
        c.start().unwrap();
        let mut bad = induction_response(1, 2);
        if let ControlPacket::Handshake(hp) = &mut bad {
            hp.extension_field = HandshakeExtensionFlags(0x0000);
        }
        let outputs = c.feed(&bad).unwrap();
        assert_eq!(
            outputs,
            vec![HandshakeOutput::Rejected(RejectionReason::Rogue)]
        );
        assert_eq!(c.state(), CallerHandshakeState::Rejected);
    }

    #[test]
    fn induction_response_bad_version_is_rejected() {
        let mut c = CallerHandshake::new(1, HandshakeConfig::default());
        c.start().unwrap();
        let mut bad = induction_response(1, 2);
        if let ControlPacket::Handshake(hp) = &mut bad {
            hp.version = HANDSHAKE_VERSION_4;
        }
        let outputs = c.feed(&bad).unwrap();
        assert_eq!(
            outputs,
            vec![HandshakeOutput::Rejected(RejectionReason::Version)]
        );
        assert_eq!(c.state(), CallerHandshakeState::Rejected);
    }

    #[test]
    fn explicit_peer_rejection_is_surfaced() {
        let mut c = CallerHandshake::new(1, HandshakeConfig::default());
        c.start().unwrap();
        let mut rejected = induction_response(1, 2);
        if let ControlPacket::Handshake(hp) = &mut rejected {
            hp.handshake_type = RejectionReason::Backlog.to_handshake_type();
        }
        let outputs = c.feed(&rejected).unwrap();
        assert_eq!(
            outputs,
            vec![HandshakeOutput::Rejected(RejectionReason::Backlog)]
        );
        assert_eq!(c.state(), CallerHandshakeState::Rejected);
    }

    #[test]
    fn feed_before_start_is_out_of_sequence() {
        let mut c = CallerHandshake::new(1, HandshakeConfig::default());
        let resp = induction_response(1, 2);
        assert!(matches!(
            c.feed(&resp),
            Err(Error::HandshakeOutOfSequence { .. })
        ));
    }

    #[test]
    fn feed_rejects_non_handshake_packets() {
        use crate::packet::misc::KeepAlivePacket;
        let mut c = CallerHandshake::new(1, HandshakeConfig::default());
        c.start().unwrap();
        let ka = ControlPacket::KeepAlive(KeepAlivePacket {
            timestamp: 0,
            dest_socket_id: 0,
        });
        assert!(matches!(
            c.feed(&ka),
            Err(Error::UnexpectedControlPacket { .. })
        ));
    }

    #[test]
    fn tick_retransmits_then_times_out() {
        let config = HandshakeConfig {
            retransmit_after_ticks: 2,
            max_retries: 1,
            ..HandshakeConfig::default()
        };
        let mut c = CallerHandshake::new(1, config);
        let first = c.start().unwrap();

        assert_eq!(c.tick(), Vec::new()); // 1 tick, threshold 2: nothing yet
        let out = c.tick(); // 2 ticks: retransmit #1
        assert_eq!(out, vec![HandshakeOutput::Send(first.clone())]);

        assert_eq!(c.tick(), Vec::new());
        let out = c.tick(); // retransmit #2 exceeds max_retries=1
        assert_eq!(out, vec![HandshakeOutput::TimedOut]);
        assert_eq!(c.state(), CallerHandshakeState::TimedOut);
    }
}
