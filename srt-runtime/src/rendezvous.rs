//! Rendezvous handshake engine — `draft-sharabayko-srt-01` §4.3.2 (Rendezvous
//! Handshake), curated at `specs/rules/srt-rendezvous.md`. Line cites below
//! (`LNNNN`) are the source draft's line numbers, exactly as recorded in that
//! curation.
//!
//! [`RendezvousHandshake`] is symmetric: both peers run the *same* engine
//! (unlike [`crate::caller::CallerHandshake`] / [`crate::listener::ListenerHandshake`],
//! which run different code for different roles). Which of the two logical
//! roles — **Initiator** or **Responder** — a given instance ends up playing
//! is decided at runtime by the **cookie contest** (§4.3.2, L2107-2135):
//! each side supplies its own 32-bit cookie to [`RendezvousHandshake::new`]
//! (this crate never reads a clock or a socket address — see
//! [`crate::handshake_sm::derive_cookie`] for a ready-made, non-standardized
//! derivation helper, exactly as for [`crate::listener::ListenerHandshake`]),
//! and the greater cookie value wins ("becomes Initiator", L2133-2135).
//!
//! # State machine
//!
//! States are named exactly as the draft's Parallel Handshake Flow diagram
//! (§4.3.2.2, L2280, quoted verbatim): **Waving → Attention → Initiated →
//! Connected** (plus `Idle` before [`RendezvousHandshake::start`] is called —
//! not spec-named, mirrors [`crate::caller::CallerHandshakeState::Idle`] — and
//! `Rejected`/`TimedOut` terminal states, also not spec-named). Every
//! transition below is the Initiator table (L2301-2334) or Responder table
//! (L2336-2382), or a missing-packet recovery rule (L2383-2432).
//!
//! ## Serial vs Parallel flow: one engine, not two
//!
//! The draft narrates two "flows" (§4.3.2.1 Serial, L2140-2269; §4.3.2.2
//! Parallel, L2270-2432) that differ only in *message interleaving*, not in
//! any new transition rule: the Parallel flow's Initiator/Responder tables are
//! complete state × received-message tables, driven purely by message
//! content — so they already cover the Serial flow's crossing case. That case
//! is: a peer still in `Waving` (having sent its own WAVEAHAND but never
//! having received the other side's) receives a **CONCLUSION** directly
//! instead of a WAVEAHAND (§4.3.2.1 step 3, L2203-2219 — the draft calls the
//! resulting state "fine"). Tracing the draft's own worked example confirms
//! the action taken is identical to applying the Parallel Attention row to
//! that CONCLUSION: an Initiator receiving an extension-less CONCLUSION here
//! behaves exactly like the Initiator-Attention-row "no extensions" case
//! (L2312-2318); a Responder receiving a CONCLUSION+HSREQ here behaves
//! exactly like the Responder-Attention-row HSREQ case (L2360-2364) — because
//! by this point in the exchange the peer's role-appropriate first CONCLUSION
//! already carries whatever extension its role dictates (Initiator: HSREQ
//! immediately, L2286-2287; Responder: none until it has seen HSREQ,
//! L2357-2360/L2287-2288). This implementation therefore has **no separate
//! "Fine" state** — a received CONCLUSION while still in `Waving` dispatches
//! straight into the same Attention-row logic used when genuinely in
//! `Attention`, rather than inventing a fourth, behaviourally-divergent state
//! the tables do not define.
//!
//! ## Resolved ambiguities (not explicit in the curated rules)
//!
//! - **Cookie collision → rejection.** The draft says only "the connection
//!   will not be made until new, unique cookies are generated" (L2119-2124),
//!   describing an out-of-band retry, not a wire action. This engine surfaces
//!   it as [`RejectionReason::RdvCookie`] (Table 7 code `1009`, "rendezvous
//!   cookie collision" — an exact fit) rather than blocking or retrying
//!   internally; a driver that wants the "regenerate and retry" behaviour
//!   builds a fresh [`RendezvousHandshake`] with a new cookie.
//! - **CONCLUSION `SYN Cookie` field.** The draft specifies WAVEAHAND's SYN
//!   Cookie (L2156-2165) but not what later CONCLUSION/AGREEMENT messages
//!   carry in that field. Every message this engine sends after WAVEAHAND
//!   continues to carry *this side's own* cookie (never an echo of the peer's,
//!   unlike the Caller-Listener flood-protection cookie) — consistent with
//!   the cookie's role here being mutual identification/contest, not a
//!   flood-protection echo-token.
//! - **No Stream ID / Group Membership exchange.** Neither extension is
//!   mentioned anywhere in §4.3.2; unlike
//!   [`crate::handshake_sm::NegotiatedParams`] from the Caller-Listener flow,
//!   this engine never sends them and always reports `stream_id: None`,
//!   `group: None` — flagged rather than fabricated.
//! - **Latency/flags reconciliation.** §4.3.2 does not restate the
//!   greater-latency / AND-flags rule from §4.3.1.2, but it is a property of
//!   the Handshake Extension Message itself (§3.2.1.1), not of the flow that
//!   carried it, so the same shared `handshake_sm` reconciliation helper used
//!   by the Caller-Listener flow is reused unchanged.
//! - **Duplicate WAVEAHAND while already `Attention`.** Not covered by either
//!   table (which only fire on the *first* WAVEAHAND). Treated as a benign
//!   duplicate: resend the last message, no state change.
//! - **Recovery rule 3 (data packet promotes a stuck Responder, L2413-2422)**
//!   is exposed as [`RendezvousHandshake::on_recovery_trigger`], since this
//!   engine's `feed` takes a [`ControlPacket`] (matching
//!   [`crate::caller::CallerHandshake`] / [`crate::listener::ListenerHandshake`]),
//!   not the data-plane [`crate::packet::SrtPacket`]; a driver that receives a
//!   `SrtPacket::Data` (or any Control packet normally only sent between
//!   connected parties — [`RendezvousHandshake::feed`] already treats any
//!   *inbound* non-Handshake `ControlPacket` this way for exactly this
//!   reason) calls it directly.
//!
//! Explicit non-goals, unchanged from the crate root: ARQ/loss, TSBPD
//! delivery, congestion control, AES key-wrap/unwrap crypto, a `tokio` socket
//! adapter, and the Version-4 legacy Rendezvous path (L2101-2105, out of
//! scope of the draft excerpt this crate implements against).

use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::handshake_sm::{
    self, HANDSHAKE_VERSION_5, HandshakeConfig, HandshakeOutput, NegotiatedParams, RejectionReason,
};
use crate::packet::{
    ControlPacket, EncryptionField, ExtensionType, HandshakeExtensionFlags,
    HandshakeExtensionMessageFlags, HandshakeExtensions, HandshakePacket, HandshakeType,
    HsExtMessage,
};

/// Rendezvous handshake lifecycle state (`draft-sharabayko-srt-01` §4.3.2).
/// State names are exactly the Parallel Handshake Flow diagram (§4.3.2.2,
/// L2280): `Waving -> Attention -> Initiated -> Connected`. See the module
/// doc "Serial vs Parallel flow" note for why there is no separate state for
/// the Serial flow's "fine".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RendezvousHandshakeState {
    /// No handshake message sent yet. Not spec-named — mirrors
    /// [`crate::caller::CallerHandshakeState::Idle`].
    Idle,
    /// L2100/L2153/L2280 "waving"/"Waving": both parties' initial state.
    Waving,
    /// L2172/L2280 "attention"/"Attention": a WAVEAHAND was received, the
    /// cookie contest is resolved, awaiting the peer's CONCLUSION.
    Attention,
    /// L2234/L2280 "initiated"/"Initiated": role-appropriate extension
    /// content has been seen at least once; awaiting the peer's confirmation.
    Initiated,
    /// L2224/L2251/L2280 "connected"/"Connected": the handshake completed;
    /// [`NegotiatedParams`] are available.
    Connected,
    /// The handshake was rejected (locally, or by an explicit peer Table 7
    /// code). Not spec-named.
    Rejected,
    /// No response arrived after the configured retry budget. Not spec-named.
    TimedOut,
}

impl RendezvousHandshakeState {
    /// A short label for this state.
    pub fn name(&self) -> &'static str {
        match self {
            RendezvousHandshakeState::Idle => "Idle",
            RendezvousHandshakeState::Waving => "Waving",
            RendezvousHandshakeState::Attention => "Attention",
            RendezvousHandshakeState::Initiated => "Initiated",
            RendezvousHandshakeState::Connected => "Connected",
            RendezvousHandshakeState::Rejected => "Rejected",
            RendezvousHandshakeState::TimedOut => "TimedOut",
        }
    }
}

impl core::fmt::Display for RendezvousHandshakeState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// The role a [`RendezvousHandshake`] plays, resolved by the cookie contest
/// (`draft-sharabayko-srt-01` §4.3.2, L2107-2135): "When one party's cookie
/// value is greater than its peer's, it wins the cookie contest and becomes
/// Initiator (the other party becomes the Responder)."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum RendezvousRole {
    /// Wins the cookie contest (the greater cookie value, L2133-2135). MUST
    /// attach the HSREQ extension (L2286-2287).
    Initiator,
    /// Loses the cookie contest. MUST attach the HSRSP extension
    /// (L2287-2288).
    Responder,
}

impl RendezvousRole {
    /// A short label for this role.
    pub fn name(&self) -> &'static str {
        match self {
            RendezvousRole::Initiator => "Initiator",
            RendezvousRole::Responder => "Responder",
        }
    }
}

broadcast_common::impl_spec_display!(RendezvousRole);

/// The Handshake Extension content found on one received CONCLUSION, keyed by
/// which side sends which (§4.3.2.2, L2282-2288). Internal to this module —
/// [`crate::handshake_sm::parse_peer_extensions`] does not distinguish
/// HSREQ from HSRSP, which the Rendezvous tables need to.
enum PeerHsExt {
    /// No Handshake Extension block present.
    None,
    /// An `HSREQ` block was present (only an Initiator sends this).
    HsReq(HsExtMessage),
    /// An `HSRSP` block was present (only a Responder sends this).
    HsRsp(HsExtMessage),
}

/// Walks `hp`'s extension blocks looking for an HSREQ or HSRSP Handshake
/// Extension Message. Returns [`Error::InvalidField`] on any malformed block
/// or malformed HSREQ/HSRSP contents — never panics on untrusted input.
fn parse_peer_hs_ext(hp: &HandshakePacket<'_>) -> Result<PeerHsExt> {
    let mut found = PeerHsExt::None;
    for block in hp.extensions.iter() {
        let block = block.map_err(|_| Error::InvalidField {
            what: "rendezvous handshake extensions",
            reason: "malformed extension block",
        })?;
        match block.ext_type {
            ExtensionType::HsReq => {
                let msg = block.as_hs_ext_message().map_err(|_| Error::InvalidField {
                    what: "HSREQ extension message",
                    reason: "malformed contents",
                })?;
                found = PeerHsExt::HsReq(msg);
            }
            ExtensionType::HsRsp => {
                let msg = block.as_hs_ext_message().map_err(|_| Error::InvalidField {
                    what: "HSRSP extension message",
                    reason: "malformed contents",
                })?;
                found = PeerHsExt::HsRsp(msg);
            }
            _ => {}
        }
    }
    Ok(found)
}

/// A driveable, symmetric SRT Rendezvous handshake
/// (`draft-sharabayko-srt-01` §4.3.2). See the module docs for the state
/// machine and the design decisions not explicit in the curated rules.
#[derive(Debug)]
pub struct RendezvousHandshake {
    own_socket_id: u32,
    own_cookie: u32,
    config: HandshakeConfig,
    state: RendezvousHandshakeState,
    role: Option<RendezvousRole>,
    peer_socket_id: u32,
    peer_cookie: u32,
    peer_hs_msg: Option<HsExtMessage>,
    last_sent: Option<Vec<u8>>,
    ticks_since_send: u32,
    retries: u32,
    negotiated: Option<NegotiatedParams>,
}

impl RendezvousHandshake {
    /// Creates a fresh Rendezvous handshake in
    /// [`RendezvousHandshakeState::Idle`], with the 32-bit cookie this side
    /// will offer in the cookie contest (§4.3.2, L2107-2135). This crate never
    /// reads a clock or a socket address — see
    /// [`crate::handshake_sm::derive_cookie`] for a ready-made,
    /// non-standardized derivation helper.
    pub fn new(own_socket_id: u32, own_cookie: u32, config: HandshakeConfig) -> Self {
        RendezvousHandshake {
            own_socket_id,
            own_cookie,
            config,
            state: RendezvousHandshakeState::Idle,
            role: None,
            peer_socket_id: 0,
            peer_cookie: 0,
            peer_hs_msg: None,
            last_sent: None,
            ticks_since_send: 0,
            retries: 0,
            negotiated: None,
        }
    }

    /// The current state.
    pub fn state(&self) -> RendezvousHandshakeState {
        self.state
    }

    /// The role resolved by the cookie contest, once past
    /// [`RendezvousHandshakeState::Waving`].
    pub fn role(&self) -> Option<RendezvousRole> {
        self.role
    }

    /// The negotiated parameters, once [`RendezvousHandshakeState::Connected`].
    pub fn negotiated(&self) -> Option<&NegotiatedParams> {
        self.negotiated.as_ref()
    }

    /// Builds the initial WAVEAHAND handshake (§4.3.2, L2100/L2153-2170:
    /// Version 5, this side's own cookie, no extensions) and transitions to
    /// [`RendezvousHandshakeState::Waving`].
    ///
    /// # Errors
    /// [`Error::HandshakeOutOfSequence`] if called more than once.
    pub fn start(&mut self) -> Result<Vec<u8>> {
        if self.state != RendezvousHandshakeState::Idle {
            return Err(Error::HandshakeOutOfSequence {
                state: self.state.name(),
                reason: "start() called after the handshake already began",
            });
        }
        let hp = HandshakePacket {
            timestamp: 0,
            dest_socket_id: 0, // Unknown yet — mirrors CallerHandshake::start's INDUCTION.
            version: HANDSHAKE_VERSION_5,
            encryption_field: self.config.encryption_field,
            extension_field: HandshakeExtensionFlags(0),
            initial_seq_number: self.config.initial_seq_number,
            mtu: self.config.mtu,
            max_flow_window_size: self.config.max_flow_window_size,
            handshake_type: HandshakeType::Wavehand,
            srt_socket_id: self.own_socket_id,
            syn_cookie: self.own_cookie,
            peer_ip: self.config.local_ip,
            extensions: HandshakeExtensions(&[]),
        };
        let bytes = handshake_sm::build_bytes(hp)?;
        self.last_sent = Some(bytes.clone());
        self.ticks_since_send = 0;
        self.state = RendezvousHandshakeState::Waving;
        Ok(bytes)
    }

    /// Feeds an inbound control packet.
    ///
    /// A non-Handshake `ControlPacket` fed while this side is a Responder
    /// stuck in [`RendezvousHandshakeState::Initiated`] is treated as
    /// missing-packet recovery rule 3 (L2413-2422: "any control packet
    /// normally only sent between connected parties") rather than an error —
    /// see [`Self::on_recovery_trigger`] for the data-packet analogue.
    ///
    /// # Errors
    /// [`Error::UnexpectedControlPacket`] for a non-Handshake packet outside
    /// that one recovery case; [`Error::HandshakeOutOfSequence`] if fed before
    /// [`Self::start`] or after a terminal state (a driver bug, not a peer
    /// protocol failure).
    pub fn feed(&mut self, packet: &ControlPacket<'_>) -> Result<Vec<HandshakeOutput>> {
        let hp = match packet {
            ControlPacket::Handshake(hp) => hp,
            other => {
                if self.state == RendezvousHandshakeState::Initiated
                    && self.role == Some(RendezvousRole::Responder)
                {
                    return self.enter_connected();
                }
                return Err(Error::UnexpectedControlPacket {
                    actual: other.control_type().name(),
                });
            }
        };
        if let Some(reason) = RejectionReason::from_handshake_type(hp.handshake_type) {
            return Ok(self.reject(reason));
        }
        if hp.version != HANDSHAKE_VERSION_5 {
            // §4.3.2, L2101-2105: Version-4 legacy Rendezvous is out of scope.
            return Ok(self.reject(RejectionReason::Version));
        }
        match self.state {
            RendezvousHandshakeState::Idle => Err(Error::HandshakeOutOfSequence {
                state: self.state.name(),
                reason: "feed() called before start()",
            }),
            RendezvousHandshakeState::Waving => self.on_waving(hp),
            RendezvousHandshakeState::Attention => self.on_attention(hp),
            RendezvousHandshakeState::Initiated => self.on_initiated(hp),
            RendezvousHandshakeState::Connected => self.on_connected(hp),
            RendezvousHandshakeState::Rejected | RendezvousHandshakeState::TimedOut => {
                Err(Error::HandshakeOutOfSequence {
                    state: self.state.name(),
                    reason: "handshake already reached a terminal state",
                })
            }
        }
    }

    /// Convenience wrapper: parses `bytes` as a [`ControlPacket`] then
    /// [`Self::feed`]s it.
    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Result<Vec<HandshakeOutput>> {
        let packet = ControlPacket::parse(bytes)?;
        self.feed(&packet)
    }

    /// Missing-packet recovery rule 3 (§4.3.2.2, L2413-2422): call this when
    /// the driver receives a data packet (`SrtPacket::Data`) while this side
    /// has not yet reached [`RendezvousHandshakeState::Connected`]. A no-op
    /// (`Ok(Vec::new())`) unless this side is a Responder stuck in
    /// [`RendezvousHandshakeState::Initiated`] — the one case the draft says
    /// is "exceptionally allowed" to promote to Connected "as if it had
    /// received AGREEMENT".
    pub fn on_recovery_trigger(&mut self) -> Result<Vec<HandshakeOutput>> {
        if self.state == RendezvousHandshakeState::Initiated
            && self.role == Some(RendezvousRole::Responder)
        {
            self.enter_connected()
        } else {
            Ok(Vec::new())
        }
    }

    /// Advances retransmit timing by one caller-defined tick, mirroring
    /// [`crate::caller::CallerHandshake::tick`] /
    /// [`crate::listener::ListenerHandshake::tick`].
    pub fn tick(&mut self) -> Vec<HandshakeOutput> {
        if matches!(
            self.state,
            RendezvousHandshakeState::Idle
                | RendezvousHandshakeState::Connected
                | RendezvousHandshakeState::Rejected
                | RendezvousHandshakeState::TimedOut
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
            self.state = RendezvousHandshakeState::TimedOut;
            return vec![HandshakeOutput::TimedOut];
        }
        match self.last_sent.clone() {
            Some(bytes) => vec![HandshakeOutput::Send(bytes)],
            None => Vec::new(),
        }
    }

    /// [`RendezvousHandshakeState::Waving`]: the first message ever received
    /// from the peer — either a genuine WAVEAHAND (§4.3.2.2 Waving row,
    /// L2301-2306 Initiator / L2336-2342 Responder) or, in the Serial flow's
    /// crossing case, a CONCLUSION directly (see module docs). Either way the
    /// cookie contest is resolved here, from `hp.syn_cookie`.
    fn on_waving(&mut self, hp: &HandshakePacket<'_>) -> Result<Vec<HandshakeOutput>> {
        if !matches!(
            hp.handshake_type,
            HandshakeType::Wavehand | HandshakeType::Conclusion
        ) {
            return Ok(self.reject(RejectionReason::Rogue));
        }
        self.peer_socket_id = hp.srt_socket_id;
        let role = match self.resolve_role(hp.syn_cookie) {
            Ok(r) => r,
            Err(_) => return Ok(self.reject(RejectionReason::RdvCookie)),
        };
        self.peer_cookie = hp.syn_cookie;
        self.role = Some(role);

        if hp.handshake_type == HandshakeType::Wavehand {
            self.state = RendezvousHandshakeState::Attention;
            match role {
                // Initiator Waving row (L2301-2306): send CONCLUSION+HSREQ.
                RendezvousRole::Initiator => self.send_conclusion(Some(ExtensionType::HsReq)),
                // Responder Waving row (L2336-2342): send CONCLUSION, no
                // extensions (it has not seen the peer's HSREQ yet).
                RendezvousRole::Responder => self.send_conclusion(None),
            }
        } else {
            // Serial-flow crossing (§4.3.2.1 step 3, L2203-2219): apply the
            // same content-driven Attention-row logic (module docs).
            self.on_attention_conclusion(hp, role)
        }
    }

    /// [`RendezvousHandshakeState::Attention`]: a WAVEAHAND was already seen
    /// and processed; only a CONCLUSION (Attention row) or a duplicate
    /// WAVEAHAND (not covered by either table — treated as a benign resend,
    /// module docs) is expected here.
    fn on_attention(&mut self, hp: &HandshakePacket<'_>) -> Result<Vec<HandshakeOutput>> {
        let role = self
            .role
            .expect("role is always resolved before Attention is reached");
        if hp.handshake_type == HandshakeType::Conclusion {
            return self.on_attention_conclusion(hp, role);
        }
        if hp.handshake_type == HandshakeType::Wavehand {
            return Ok(self.resend());
        }
        Ok(self.reject(RejectionReason::Rogue))
    }

    /// The Attention row's CONCLUSION-handling logic (§4.3.2.2, L2307-2320
    /// Initiator / L2343-2364 Responder), shared between a genuine
    /// [`RendezvousHandshakeState::Attention`] and the Serial-flow crossing
    /// case fed straight from [`RendezvousHandshakeState::Waving`].
    fn on_attention_conclusion(
        &mut self,
        hp: &HandshakePacket<'_>,
        role: RendezvousRole,
    ) -> Result<Vec<HandshakeOutput>> {
        self.peer_socket_id = hp.srt_socket_id;
        let ext = match parse_peer_hs_ext(hp) {
            Ok(e) => e,
            Err(_) => return Ok(self.reject(RejectionReason::Rogue)),
        };
        match (role, ext) {
            (RendezvousRole::Initiator, PeerHsExt::None) => {
                // L2312-2318: no extensions -> Initiated, still send
                // CONCLUSION+HSREQ.
                self.state = RendezvousHandshakeState::Initiated;
                self.send_conclusion(Some(ExtensionType::HsReq))
            }
            (RendezvousRole::Initiator, PeerHsExt::HsRsp(msg)) => {
                // L2318-2320: contains HSRSP -> Connected, send AGREEMENT.
                self.peer_hs_msg = Some(msg);
                self.enter_connected()
            }
            (RendezvousRole::Responder, PeerHsExt::None) => {
                // L2357-2360: no extensions yet -> resend the empty
                // CONCLUSION, remain in Attention.
                self.state = RendezvousHandshakeState::Attention;
                self.send_conclusion(None)
            }
            (RendezvousRole::Responder, PeerHsExt::HsReq(msg)) => {
                // L2360-2364: HSREQ present -> Initiated, send
                // CONCLUSION+HSRSP.
                self.peer_hs_msg = Some(msg);
                self.state = RendezvousHandshakeState::Initiated;
                self.send_conclusion(Some(ExtensionType::HsRsp))
            }
            // A peer sending the extension kind only its own role should ever
            // send (HSREQ into an Initiator, HSRSP into a Responder) is not a
            // transition either table defines — reject rather than guess.
            _ => Ok(self.reject(RejectionReason::Rogue)),
        }
    }

    /// [`RendezvousHandshakeState::Initiated`] (§4.3.2.2, L2321-2334 Initiator
    /// / L2365-2382 Responder), including the idempotent-resend recovery
    /// rules (L2383-2422).
    fn on_initiated(&mut self, hp: &HandshakePacket<'_>) -> Result<Vec<HandshakeOutput>> {
        let role = self
            .role
            .expect("role is always resolved before Initiated is reached");
        match role {
            RendezvousRole::Initiator => {
                if hp.handshake_type != HandshakeType::Conclusion {
                    return Ok(self.reject(RejectionReason::Rogue));
                }
                let ext = match parse_peer_hs_ext(hp) {
                    Ok(e) => e,
                    Err(_) => return Ok(self.reject(RejectionReason::Rogue)),
                };
                match ext {
                    // L2325: "REMAINS IN THIS STATE" — still resend
                    // CONCLUSION+HSREQ.
                    PeerHsExt::None => self.send_conclusion(Some(ExtensionType::HsReq)),
                    // L2325-2334ish: contains HSRSP -> Connected, AGREEMENT.
                    PeerHsExt::HsRsp(msg) => {
                        self.peer_hs_msg = Some(msg);
                        self.enter_connected()
                    }
                    PeerHsExt::HsReq(_) => Ok(self.reject(RejectionReason::Rogue)),
                }
            }
            RendezvousRole::Responder => {
                if hp.handshake_type == HandshakeType::Agreement {
                    // Responder Initiated row: AGREEMENT -> respond AGREEMENT,
                    // switch to Connected.
                    return self.enter_connected();
                }
                if hp.handshake_type != HandshakeType::Conclusion {
                    return Ok(self.reject(RejectionReason::Rogue));
                }
                let ext = match parse_peer_hs_ext(hp) {
                    Ok(e) => e,
                    Err(_) => return Ok(self.reject(RejectionReason::Rogue)),
                };
                match ext {
                    // Recovery rule 2 (L2391-2395): MUST always resend HSRSP,
                    // even if this HSREQ was already seen and processed once.
                    PeerHsExt::HsReq(msg) => {
                        self.peer_hs_msg = Some(msg);
                        self.send_conclusion(Some(ExtensionType::HsRsp))
                    }
                    _ => Ok(self.reject(RejectionReason::Rogue)),
                }
            }
        }
    }

    /// [`RendezvousHandshakeState::Connected`] (Initiator row item 4,
    /// L2331-2334; Responder row item 4, L2377-2381): normally no more
    /// handshake traffic, but a repeated CONCLUSION is answered with another
    /// AGREEMENT (recovery rule 4, L2424-2432, from the other side's point of
    /// view); anything else is disregarded rather than regressing a completed
    /// handshake.
    fn on_connected(&mut self, hp: &HandshakePacket<'_>) -> Result<Vec<HandshakeOutput>> {
        if hp.handshake_type == HandshakeType::Conclusion {
            return self.send_agreement();
        }
        Ok(Vec::new())
    }

    /// The cookie contest (§4.3.2, L2107-2135). Identical cookies are a
    /// collision the draft says must not connect (L2119-2124) — surfaced by
    /// the caller as [`RejectionReason::RdvCookie`].
    fn resolve_role(&self, peer_cookie: u32) -> Result<RendezvousRole> {
        if peer_cookie == self.own_cookie {
            return Err(Error::InvalidField {
                what: "rendezvous cookie",
                reason: "identical to the peer's cookie (collision, L2119-2124)",
            });
        }
        Ok(if self.own_cookie > peer_cookie {
            RendezvousRole::Initiator
        } else {
            RendezvousRole::Responder
        })
    }

    fn reject(&mut self, reason: RejectionReason) -> Vec<HandshakeOutput> {
        self.state = RendezvousHandshakeState::Rejected;
        vec![HandshakeOutput::Rejected(reason)]
    }

    fn resend(&mut self) -> Vec<HandshakeOutput> {
        match &self.last_sent {
            Some(bytes) => vec![HandshakeOutput::Send(bytes.clone())],
            None => Vec::new(),
        }
    }

    /// Builds and sends a CONCLUSION: `ext_type` picks HSREQ/HSRSP, or `None`
    /// for the extension-less greeting (§4.3.2.2, L2286-2288: no Stream ID /
    /// Group Membership is ever attached — see module docs).
    fn send_conclusion(&mut self, ext_type: Option<ExtensionType>) -> Result<Vec<HandshakeOutput>> {
        let (ext_bytes, ext_flags): (Vec<u8>, u16) = match ext_type {
            None => (Vec::new(), 0),
            Some(t) => {
                let hs_msg = HsExtMessage {
                    srt_version: self.config.srt_version,
                    srt_flags: self.config.flags,
                    receiver_tsbpd_delay_ms: self.config.latency_ms,
                    sender_tsbpd_delay_ms: self.config.latency_ms,
                };
                handshake_sm::build_conclusion_extensions(t, &hs_msg, None, None)?
            }
        };
        let hp = HandshakePacket {
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
            syn_cookie: self.own_cookie,
            peer_ip: self.config.local_ip,
            extensions: HandshakeExtensions(&ext_bytes),
        };
        let bytes = handshake_sm::build_bytes(hp)?;
        self.last_sent = Some(bytes.clone());
        self.ticks_since_send = 0;
        self.retries = 0;
        Ok(vec![HandshakeOutput::Send(bytes)])
    }

    /// Builds and sends an AGREEMENT: no extensions (§4.3.2.1, L2224-2230).
    fn send_agreement(&mut self) -> Result<Vec<HandshakeOutput>> {
        let hp = HandshakePacket {
            timestamp: 0,
            dest_socket_id: self.peer_socket_id,
            version: HANDSHAKE_VERSION_5,
            encryption_field: EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(0),
            initial_seq_number: self.config.initial_seq_number,
            mtu: self.config.mtu,
            max_flow_window_size: self.config.max_flow_window_size,
            handshake_type: HandshakeType::Agreement,
            srt_socket_id: self.own_socket_id,
            syn_cookie: self.own_cookie,
            peer_ip: self.config.local_ip,
            extensions: HandshakeExtensions(&[]),
        };
        let bytes = handshake_sm::build_bytes(hp)?;
        self.last_sent = Some(bytes.clone());
        self.ticks_since_send = 0;
        self.retries = 0;
        Ok(vec![HandshakeOutput::Send(bytes)])
    }

    /// Reaches [`RendezvousHandshakeState::Connected`]: builds
    /// [`NegotiatedParams`] from `self.peer_hs_msg` (always captured by every
    /// caller of this method before it is called) and sends the AGREEMENT
    /// every documented transition into Connected requires.
    fn enter_connected(&mut self) -> Result<Vec<HandshakeOutput>> {
        let negotiated = self.build_negotiated();
        self.negotiated = Some(negotiated.clone());
        self.state = RendezvousHandshakeState::Connected;
        let mut out = self.send_agreement()?;
        out.push(HandshakeOutput::Connected(negotiated));
        Ok(out)
    }

    fn build_negotiated(&self) -> NegotiatedParams {
        let peer_msg = self
            .peer_hs_msg
            .expect("peer_hs_msg is always captured before any transition reaches Connected");
        NegotiatedParams {
            version: HANDSHAKE_VERSION_5,
            flags: HandshakeExtensionMessageFlags(self.config.flags.0 & peer_msg.srt_flags.0),
            latency_ms: handshake_sm::negotiate_latency_ms(self.config.latency_ms, &peer_msg),
            own_socket_id: self.own_socket_id,
            peer_socket_id: self.peer_socket_id,
            // §4.3.2 never mentions a Stream ID / Group Membership exchange —
            // module docs "Resolved ambiguities".
            stream_id: None,
            group: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::handshake::HS_EXT_FLAG_HSREQ;

    #[test]
    fn start_is_idempotent_guard() {
        let mut r = RendezvousHandshake::new(1, 500, HandshakeConfig::default());
        assert!(r.start().is_ok());
        assert!(r.start().is_err());
    }

    #[test]
    fn wavehand_wire_values_match_draft_4_3_2() {
        let mut r = RendezvousHandshake::new(0xAAAA_BBBB, 0xC0FF_EE00, HandshakeConfig::default());
        let bytes = r.start().unwrap();
        let pkt = ControlPacket::parse(&bytes).unwrap();
        match pkt {
            ControlPacket::Handshake(hp) => {
                assert_eq!(hp.version, HANDSHAKE_VERSION_5);
                assert_eq!(hp.handshake_type, HandshakeType::Wavehand);
                assert_eq!(hp.srt_socket_id, 0xAAAA_BBBB);
                assert_eq!(hp.syn_cookie, 0xC0FF_EE00);
                assert_eq!(hp.extension_field.0, 0);
            }
            _ => panic!("expected handshake"),
        }
        assert_eq!(r.state(), RendezvousHandshakeState::Waving);
    }

    fn wavehand(socket_id: u32, cookie: u32) -> ControlPacket<'static> {
        ControlPacket::Handshake(HandshakePacket {
            timestamp: 0,
            dest_socket_id: 0,
            version: HANDSHAKE_VERSION_5,
            encryption_field: EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(0),
            initial_seq_number: 0,
            mtu: 1500,
            max_flow_window_size: 8192,
            handshake_type: HandshakeType::Wavehand,
            srt_socket_id: socket_id,
            syn_cookie: cookie,
            peer_ip: [0; 4],
            extensions: HandshakeExtensions(&[]),
        })
    }

    #[test]
    fn greater_cookie_wins_initiator() {
        let mut a = RendezvousHandshake::new(1, 500, HandshakeConfig::default());
        let mut b = RendezvousHandshake::new(2, 100, HandshakeConfig::default());
        a.start().unwrap();
        b.start().unwrap();

        a.feed(&wavehand(2, 100)).unwrap();
        b.feed(&wavehand(1, 500)).unwrap();

        assert_eq!(a.role(), Some(RendezvousRole::Initiator));
        assert_eq!(b.role(), Some(RendezvousRole::Responder));
        assert_eq!(a.state(), RendezvousHandshakeState::Attention);
        assert_eq!(b.state(), RendezvousHandshakeState::Attention);
    }

    #[test]
    fn identical_cookies_are_rejected_as_a_collision() {
        let mut a = RendezvousHandshake::new(1, 0x00C0_FFEE, HandshakeConfig::default());
        a.start().unwrap();
        let outputs = a.feed(&wavehand(2, 0x00C0_FFEE)).unwrap();
        assert_eq!(
            outputs,
            vec![HandshakeOutput::Rejected(RejectionReason::RdvCookie)]
        );
        assert_eq!(a.state(), RendezvousHandshakeState::Rejected);
    }

    #[test]
    fn initiator_attention_entry_sends_hsreq() {
        let mut a = RendezvousHandshake::new(1, 500, HandshakeConfig::default());
        a.start().unwrap();
        let outputs = a.feed(&wavehand(2, 100)).unwrap();
        assert_eq!(outputs.len(), 1);
        let bytes = match &outputs[0] {
            HandshakeOutput::Send(b) => b.clone(),
            other => panic!("expected Send, got {other:?}"),
        };
        let pkt = ControlPacket::parse(&bytes).unwrap();
        match pkt {
            ControlPacket::Handshake(hp) => {
                assert_eq!(hp.handshake_type, HandshakeType::Conclusion);
                assert_eq!(hp.extension_field.0 & HS_EXT_FLAG_HSREQ, HS_EXT_FLAG_HSREQ);
                let blocks: Vec<_> = hp.extensions.iter().map(|b| b.unwrap()).collect();
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].ext_type, ExtensionType::HsReq);
            }
            _ => panic!("expected handshake"),
        }
    }

    #[test]
    fn responder_attention_entry_sends_empty_conclusion() {
        let mut b = RendezvousHandshake::new(2, 100, HandshakeConfig::default());
        b.start().unwrap();
        let outputs = b.feed(&wavehand(1, 500)).unwrap();
        assert_eq!(outputs.len(), 1);
        let bytes = match &outputs[0] {
            HandshakeOutput::Send(b) => b.clone(),
            other => panic!("expected Send, got {other:?}"),
        };
        let pkt = ControlPacket::parse(&bytes).unwrap();
        match pkt {
            ControlPacket::Handshake(hp) => {
                assert_eq!(hp.handshake_type, HandshakeType::Conclusion);
                assert_eq!(hp.extension_field.0, 0);
                assert_eq!(hp.extensions.iter().count(), 0);
            }
            _ => panic!("expected handshake"),
        }
    }

    #[test]
    fn malformed_extension_mid_flow_is_rejected_not_panicking() {
        let mut a = RendezvousHandshake::new(1, 500, HandshakeConfig::default());
        a.start().unwrap();
        a.feed(&wavehand(2, 100)).unwrap();
        assert_eq!(a.state(), RendezvousHandshakeState::Attention);

        // A CONCLUSION whose extension block declares a length far larger
        // than the bytes actually present.
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
            syn_cookie: 100,
            peer_ip: [0; 4],
            extensions: HandshakeExtensions(bad_ext),
        });
        let outputs = a.feed(&bad).unwrap();
        assert_eq!(
            outputs,
            vec![HandshakeOutput::Rejected(RejectionReason::Rogue)]
        );
        assert_eq!(a.state(), RendezvousHandshakeState::Rejected);
    }

    #[test]
    fn feed_before_start_is_out_of_sequence() {
        let mut r = RendezvousHandshake::new(1, 500, HandshakeConfig::default());
        assert!(matches!(
            r.feed(&wavehand(2, 100)),
            Err(Error::HandshakeOutOfSequence { .. })
        ));
    }

    #[test]
    fn feed_rejects_non_handshake_packets_outside_recovery_case() {
        use crate::packet::misc::KeepAlivePacket;
        let mut r = RendezvousHandshake::new(1, 500, HandshakeConfig::default());
        r.start().unwrap();
        let ka = ControlPacket::KeepAlive(KeepAlivePacket {
            timestamp: 0,
            dest_socket_id: 0,
        });
        assert!(matches!(
            r.feed(&ka),
            Err(Error::UnexpectedControlPacket { .. })
        ));
    }
}
