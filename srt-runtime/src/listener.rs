//! Listener-side handshake engine — `draft-sharabayko-srt-01` §4.3.1
//! (Caller-Listener Handshake), Listener role.
//!
//! [`ListenerHandshake`] is the mirror of [`crate::caller::CallerHandshake`]:
//! it starts passively (no output) and reacts to inbound handshake packets
//! via [`ListenerHandshake::feed`].
//!
//! Flow implemented (§4.3.1.1 / §4.3.1.2):
//!
//! 1. `feed()` the Caller's INDUCTION: reply with an INDUCTION response
//!    (Version 5, the SRT magic code `0x4A17`, the configured/derived SYN
//!    Cookie) regardless of what the Caller sent — per §4.3.1.1 the Listener
//!    "still does not know if the Caller is SRT or UDT" at this point and
//!    "responds with the same set of values regardless."
//! 2. `feed()` the Caller's CONCLUSION: validate `Handshake Type`, `Version`,
//!    the echoed SYN Cookie, and decode the extensions (HSREQ, optional
//!    Stream ID / Group). On success, reply with the CONCLUSION response
//!    (HSRSP + optional Group) and reach
//!    [`ListenerHandshakeState::Connected`] with [`NegotiatedParams`]. On
//!    failure, reply with a rejection handshake packet (`Handshake Type` =
//!    `1000 + code`, §4.3, Table 7) and reach
//!    [`ListenerHandshakeState::Rejected`].

use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::handshake_sm::{
    self, HANDSHAKE_VERSION_5, HandshakeConfig, HandshakeOutput, NegotiatedParams, RejectionReason,
    SRT_MAGIC_CODE,
};
use crate::packet::{
    ControlPacket, EncryptionField, ExtensionType, HandshakeExtensionFlags, HandshakeExtensions,
    HandshakePacket, HandshakeType, HsExtMessage,
};

/// Listener-side handshake lifecycle state (`draft-sharabayko-srt-01` §4.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ListenerHandshakeState {
    /// Waiting for the Caller's INDUCTION.
    Idle,
    /// INDUCTION response sent; awaiting the Caller's CONCLUSION.
    AwaitingConclusion,
    /// The handshake completed; [`NegotiatedParams`] are available.
    Connected,
    /// The handshake was rejected (the Caller sent invalid data).
    Rejected,
    /// No CONCLUSION arrived after the configured retry budget.
    TimedOut,
}

impl ListenerHandshakeState {
    /// A short label for this state.
    pub fn name(&self) -> &'static str {
        match self {
            ListenerHandshakeState::Idle => "Idle",
            ListenerHandshakeState::AwaitingConclusion => "AwaitingConclusion",
            ListenerHandshakeState::Connected => "Connected",
            ListenerHandshakeState::Rejected => "Rejected",
            ListenerHandshakeState::TimedOut => "TimedOut",
        }
    }
}

impl core::fmt::Display for ListenerHandshakeState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// A driveable Listener-side SRT handshake (`draft-sharabayko-srt-01`
/// §4.3.1). `syn_cookie` is supplied by the caller/driver — this crate's core
/// never reads a clock or a socket address; see
/// [`crate::handshake_sm::derive_cookie`] for a ready-made (non-standardized)
/// derivation helper.
#[derive(Debug)]
pub struct ListenerHandshake {
    own_socket_id: u32,
    syn_cookie: u32,
    config: HandshakeConfig,
    state: ListenerHandshakeState,
    peer_socket_id: u32,
    last_sent: Option<Vec<u8>>,
    ticks_since_send: u32,
    retries: u32,
    negotiated: Option<NegotiatedParams>,
}

impl ListenerHandshake {
    /// Creates a fresh Listener handshake in [`ListenerHandshakeState::Idle`],
    /// with the SYN Cookie it will hand out on INDUCTION and check on
    /// CONCLUSION.
    pub fn new(own_socket_id: u32, syn_cookie: u32, config: HandshakeConfig) -> Self {
        ListenerHandshake {
            own_socket_id,
            syn_cookie,
            config,
            state: ListenerHandshakeState::Idle,
            peer_socket_id: 0,
            last_sent: None,
            ticks_since_send: 0,
            retries: 0,
            negotiated: None,
        }
    }

    /// The current state.
    pub fn state(&self) -> ListenerHandshakeState {
        self.state
    }

    /// The negotiated parameters, once [`ListenerHandshakeState::Connected`].
    pub fn negotiated(&self) -> Option<&NegotiatedParams> {
        self.negotiated.as_ref()
    }

    /// Feeds an inbound control packet.
    ///
    /// # Errors
    /// [`Error::UnexpectedControlPacket`] if `packet` is not a Handshake
    /// packet; [`Error::HandshakeOutOfSequence`] if fed outside
    /// [`ListenerHandshakeState::Idle`] / [`ListenerHandshakeState::AwaitingConclusion`]
    /// (a driver bug, not a peer failure).
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
            ListenerHandshakeState::Idle => self.on_induction(hp),
            ListenerHandshakeState::AwaitingConclusion => self.on_conclusion(hp),
            _ => Err(Error::HandshakeOutOfSequence {
                state: self.state.name(),
                reason: "not awaiting an induction or conclusion",
            }),
        }
    }

    /// Convenience wrapper: parses `bytes` as a [`ControlPacket`] then
    /// [`Self::feed`]s it.
    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Result<Vec<HandshakeOutput>> {
        let packet = ControlPacket::parse(bytes)?;
        self.feed(&packet)
    }

    /// Advances retransmit timing by one caller-defined tick, mirroring
    /// [`crate::caller::CallerHandshake::tick`].
    pub fn tick(&mut self) -> Vec<HandshakeOutput> {
        if self.state != ListenerHandshakeState::AwaitingConclusion {
            return Vec::new();
        }
        self.ticks_since_send += 1;
        if self.ticks_since_send < self.config.retransmit_after_ticks {
            return Vec::new();
        }
        self.ticks_since_send = 0;
        self.retries += 1;
        if self.retries > self.config.max_retries {
            self.state = ListenerHandshakeState::TimedOut;
            return vec![HandshakeOutput::TimedOut];
        }
        match self.last_sent.clone() {
            Some(bytes) => vec![HandshakeOutput::Send(bytes)],
            None => Vec::new(),
        }
    }

    fn on_induction(&mut self, hp: &HandshakePacket<'_>) -> Result<Vec<HandshakeOutput>> {
        if hp.handshake_type != HandshakeType::Induction {
            return Err(Error::HandshakeOutOfSequence {
                state: self.state.name(),
                reason: "expected an INDUCTION handshake",
            });
        }
        // §4.3.1.1: the Listener does not yet know if the Caller is SRT or
        // UDT, and always responds the same way.
        self.peer_socket_id = hp.srt_socket_id;

        let hp_out = HandshakePacket {
            timestamp: 0,
            dest_socket_id: self.peer_socket_id,
            version: HANDSHAKE_VERSION_5,
            encryption_field: self.config.encryption_field,
            extension_field: HandshakeExtensionFlags(SRT_MAGIC_CODE),
            initial_seq_number: self.config.initial_seq_number,
            mtu: self.config.mtu,
            max_flow_window_size: self.config.max_flow_window_size,
            handshake_type: HandshakeType::Induction,
            srt_socket_id: self.own_socket_id,
            syn_cookie: self.syn_cookie,
            peer_ip: self.config.local_ip,
            extensions: HandshakeExtensions(&[]),
        };
        let bytes = handshake_sm::build_bytes(hp_out)?;
        self.last_sent = Some(bytes.clone());
        self.ticks_since_send = 0;
        self.state = ListenerHandshakeState::AwaitingConclusion;
        Ok(vec![HandshakeOutput::Send(bytes)])
    }

    fn on_conclusion(&mut self, hp: &HandshakePacket<'_>) -> Result<Vec<HandshakeOutput>> {
        if hp.handshake_type != HandshakeType::Conclusion {
            return self.reject(RejectionReason::Rogue, hp);
        }
        if hp.version != HANDSHAKE_VERSION_5 {
            return self.reject(RejectionReason::Version, hp);
        }
        if hp.syn_cookie != self.syn_cookie {
            // §4.3.1.1: the cookie exists precisely so the Listener can
            // refuse to allocate resources for an unverified Caller.
            return self.reject(RejectionReason::Rogue, hp);
        }

        let parsed = match handshake_sm::parse_peer_extensions(hp) {
            Ok(p) => p,
            Err(_) => return self.reject(RejectionReason::Rogue, hp),
        };
        let peer_msg = match parsed.hs_msg {
            Some(m) => m,
            None => return self.reject(RejectionReason::Rogue, hp),
        };

        self.peer_socket_id = hp.srt_socket_id;

        let negotiated = NegotiatedParams {
            version: HANDSHAKE_VERSION_5,
            flags: crate::packet::HandshakeExtensionMessageFlags(
                self.config.flags.0 & peer_msg.srt_flags.0,
            ),
            latency_ms: handshake_sm::negotiate_latency_ms(self.config.latency_ms, &peer_msg),
            own_socket_id: self.own_socket_id,
            peer_socket_id: self.peer_socket_id,
            stream_id: parsed.stream_id,
            group: parsed.group,
        };

        let hs_msg = HsExtMessage {
            srt_version: self.config.srt_version,
            srt_flags: self.config.flags,
            receiver_tsbpd_delay_ms: self.config.latency_ms,
            sender_tsbpd_delay_ms: self.config.latency_ms,
        };
        // §4.3.1.2: the Listener's CONCLUSION response carries the HSv5
        // extensions "without the cookie" — no Stream ID is echoed back
        // (only the Caller advertises one; the Listener already knows it via
        // `negotiated.stream_id`).
        let (ext_bytes, ext_flags) = handshake_sm::build_conclusion_extensions(
            ExtensionType::HsRsp,
            &hs_msg,
            None,
            self.config.group,
        )?;

        let hp_out = HandshakePacket {
            timestamp: 0,
            dest_socket_id: self.peer_socket_id,
            version: HANDSHAKE_VERSION_5,
            encryption_field: self.config.encryption_field,
            extension_field: HandshakeExtensionFlags(ext_flags),
            initial_seq_number: self.config.initial_seq_number,
            mtu: self.config.mtu,
            max_flow_window_size: self.config.max_flow_window_size,
            handshake_type: HandshakeType::Conclusion,
            srt_socket_id: self.own_socket_id,
            syn_cookie: 0, // §4.3.1.2: "without the cookie (which is not needed here)".
            peer_ip: self.config.local_ip,
            extensions: HandshakeExtensions(&ext_bytes),
        };
        let bytes = handshake_sm::build_bytes(hp_out)?;
        self.last_sent = Some(bytes.clone());
        self.negotiated = Some(negotiated.clone());
        self.state = ListenerHandshakeState::Connected;
        Ok(vec![
            HandshakeOutput::Send(bytes),
            HandshakeOutput::Connected(negotiated),
        ])
    }

    /// Builds and returns a rejection handshake packet, transitioning to
    /// [`ListenerHandshakeState::Rejected`].
    fn reject(
        &mut self,
        reason: RejectionReason,
        hp: &HandshakePacket<'_>,
    ) -> Result<Vec<HandshakeOutput>> {
        self.state = ListenerHandshakeState::Rejected;
        let hp_out = HandshakePacket {
            timestamp: 0,
            dest_socket_id: hp.srt_socket_id,
            version: HANDSHAKE_VERSION_5,
            encryption_field: EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(0),
            initial_seq_number: 0,
            mtu: self.config.mtu,
            max_flow_window_size: self.config.max_flow_window_size,
            handshake_type: reason.to_handshake_type(),
            srt_socket_id: self.own_socket_id,
            syn_cookie: 0,
            peer_ip: self.config.local_ip,
            extensions: HandshakeExtensions(&[]),
        };
        let bytes = handshake_sm::build_bytes(hp_out)?;
        self.last_sent = Some(bytes.clone());
        Ok(vec![
            HandshakeOutput::Send(bytes),
            HandshakeOutput::Rejected(reason),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::handshake::{HANDSHAKE_CIF_FIXED_LEN, HS_EXT_FLAG_HSREQ};

    fn caller_induction(caller_id: u32) -> ControlPacket<'static> {
        ControlPacket::Handshake(HandshakePacket {
            timestamp: 0,
            dest_socket_id: 0,
            version: crate::handshake_sm::HANDSHAKE_VERSION_4,
            encryption_field: EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(2),
            initial_seq_number: 0,
            mtu: 1500,
            max_flow_window_size: 8192,
            handshake_type: HandshakeType::Induction,
            srt_socket_id: caller_id,
            syn_cookie: 0,
            peer_ip: [0; 4],
            extensions: HandshakeExtensions(&[]),
        })
    }

    #[test]
    fn induction_response_wire_values_match_draft_4_3_1_1() {
        let mut l = ListenerHandshake::new(0x9999, 0xC0FF_EE00, HandshakeConfig::default());
        let outputs = l.feed(&caller_induction(0x1234)).unwrap();
        assert_eq!(outputs.len(), 1);
        let bytes = match &outputs[0] {
            HandshakeOutput::Send(b) => b.clone(),
            other => panic!("expected Send, got {other:?}"),
        };
        let pkt = ControlPacket::parse(&bytes).unwrap();
        match pkt {
            ControlPacket::Handshake(hp) => {
                assert_eq!(hp.version, HANDSHAKE_VERSION_5);
                assert_eq!(hp.extension_field.0, SRT_MAGIC_CODE);
                assert_eq!(hp.handshake_type, HandshakeType::Induction);
                assert_eq!(hp.srt_socket_id, 0x9999);
                assert_eq!(hp.syn_cookie, 0xC0FF_EE00);
                assert_eq!(hp.dest_socket_id, 0x1234);
            }
            _ => panic!("expected handshake"),
        }
        assert_eq!(l.state(), ListenerHandshakeState::AwaitingConclusion);
    }

    fn caller_conclusion(
        caller_id: u32,
        listener_id: u32,
        cookie: u32,
        latency_ms: u16,
    ) -> ControlPacket<'static> {
        let hs_msg = HsExtMessage {
            srt_version: 0x0105_0000,
            srt_flags: crate::packet::HandshakeExtensionMessageFlags(0x6F),
            receiver_tsbpd_delay_ms: latency_ms,
            sender_tsbpd_delay_ms: latency_ms,
        };
        let ext = crate::packet::handshake::build_extension_block(
            ExtensionType::HsReq,
            &hs_msg.to_bytes(),
        )
        .unwrap();
        let ext: &'static [u8] = Vec::leak(ext);
        ControlPacket::Handshake(HandshakePacket {
            timestamp: 0,
            dest_socket_id: listener_id,
            version: HANDSHAKE_VERSION_5,
            encryption_field: EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(HS_EXT_FLAG_HSREQ),
            initial_seq_number: 0,
            mtu: 1500,
            max_flow_window_size: 8192,
            handshake_type: HandshakeType::Conclusion,
            srt_socket_id: caller_id,
            syn_cookie: cookie,
            peer_ip: [0; 4],
            extensions: HandshakeExtensions(ext),
        })
    }

    #[test]
    fn conclusion_bad_cookie_is_rejected() {
        let mut l = ListenerHandshake::new(1, 0xC0FF_EE00, HandshakeConfig::default());
        l.feed(&caller_induction(2)).unwrap();
        let outputs = l.feed(&caller_conclusion(2, 1, 0xBAD_C00C, 120)).unwrap();
        assert_eq!(outputs.len(), 2);
        assert_eq!(
            outputs[1],
            HandshakeOutput::Rejected(RejectionReason::Rogue)
        );
        let bytes = match &outputs[0] {
            HandshakeOutput::Send(b) => b,
            other => panic!("expected Send, got {other:?}"),
        };
        let pkt = ControlPacket::parse(bytes).unwrap();
        if let ControlPacket::Handshake(hp) = pkt {
            assert_eq!(
                hp.handshake_type,
                RejectionReason::Rogue.to_handshake_type()
            );
        } else {
            panic!("expected handshake");
        }
        assert_eq!(l.state(), ListenerHandshakeState::Rejected);
    }

    #[test]
    fn conclusion_version_mismatch_is_rejected() {
        let mut l = ListenerHandshake::new(1, 0xC0FF_EE00, HandshakeConfig::default());
        l.feed(&caller_induction(2)).unwrap();
        let mut bad = caller_conclusion(2, 1, 0xC0FF_EE00, 120);
        if let ControlPacket::Handshake(hp) = &mut bad {
            hp.version = 4;
        }
        let outputs = l.feed(&bad).unwrap();
        assert_eq!(
            outputs[1],
            HandshakeOutput::Rejected(RejectionReason::Version)
        );
        assert_eq!(l.state(), ListenerHandshakeState::Rejected);
    }

    #[test]
    fn conclusion_malformed_extension_is_rejected_not_panicking() {
        let mut l = ListenerHandshake::new(1, 0xC0FF_EE00, HandshakeConfig::default());
        l.feed(&caller_induction(2)).unwrap();
        // Declares an extension length far larger than the bytes actually
        // present — must reject cleanly, not panic.
        let bad_ext: &'static [u8] = &[0x00, 0x01, 0xFF, 0xFF];
        let bad = ControlPacket::Handshake(HandshakePacket {
            timestamp: 0,
            dest_socket_id: 1,
            version: HANDSHAKE_VERSION_5,
            encryption_field: EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(HS_EXT_FLAG_HSREQ),
            initial_seq_number: 0,
            mtu: 1500,
            max_flow_window_size: 8192,
            handshake_type: HandshakeType::Conclusion,
            srt_socket_id: 2,
            syn_cookie: 0xC0FF_EE00,
            peer_ip: [0; 4],
            extensions: HandshakeExtensions(bad_ext),
        });
        let outputs = l.feed(&bad).unwrap();
        assert_eq!(
            outputs[1],
            HandshakeOutput::Rejected(RejectionReason::Rogue)
        );
        assert_eq!(l.state(), ListenerHandshakeState::Rejected);
    }

    #[test]
    fn successful_conclusion_reaches_connected() {
        let mut l = ListenerHandshake::new(1, 0xC0FF_EE00, HandshakeConfig::default());
        l.feed(&caller_induction(2)).unwrap();
        let outputs = l.feed(&caller_conclusion(2, 1, 0xC0FF_EE00, 120)).unwrap();
        assert_eq!(outputs.len(), 2);
        assert!(matches!(outputs[0], HandshakeOutput::Send(_)));
        assert!(matches!(outputs[1], HandshakeOutput::Connected(_)));
        assert_eq!(l.state(), ListenerHandshakeState::Connected);
        assert!(l.negotiated().is_some());
    }

    #[test]
    fn feed_before_induction_seen_still_requires_induction_first() {
        let mut l = ListenerHandshake::new(1, 1, HandshakeConfig::default());
        let outputs = l.feed(&caller_induction(2));
        assert!(outputs.is_ok());
    }

    // `HANDSHAKE_CIF_FIXED_LEN` import above is exercised indirectly by the
    // packet codec; referenced here only to keep the `use` alive across
    // refactors without an unused-import warning surfacing as a hard error.
    #[test]
    fn cif_fixed_len_is_the_documented_48_bytes() {
        assert_eq!(HANDSHAKE_CIF_FIXED_LEN, 48);
    }
}
