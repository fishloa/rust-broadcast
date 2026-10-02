//! Shared types for the sans-IO HSv5 handshake state machine —
//! `draft-sharabayko-srt-01` §4.3 (Handshake Messages) / §4.3.1
//! (Caller-Listener Handshake).
//!
//! This module holds the pieces [`crate::caller::CallerHandshake`] and
//! [`crate::listener::ListenerHandshake`] share: the negotiation input
//! ([`HandshakeConfig`]), the negotiation output ([`NegotiatedParams`]), the
//! event type both engines emit ([`HandshakeOutput`]), and the Handshake
//! Rejection Reason codes (§4.3, Table 7) as a typed [`RejectionReason`].
//!
//! This module also holds the `crypto`-feature-gated §6.1.5 Key Material
//! Exchange helpers ([`CryptoConfig`], `build_key_material_extension`,
//! `recover_sek`, `echo_key_material_as_response`, `verify_km_echo`) shared
//! by [`crate::caller::CallerHandshake`] / [`crate::listener::ListenerHandshake`]
//! — see [`CryptoConfig`]'s doc and `specs/rules/srt-crypto.md`. Congestion
//! control beyond LiveCC packet pacing is an explicit follow-up — see the
//! crate root docs.

use alloc::string::String;
use alloc::vec::Vec;
#[cfg(feature = "crypto")]
use subtle::ConstantTimeEq;
#[cfg(feature = "crypto")]
use zeroize::Zeroize;

use crate::error::{Error, Result};
#[cfg(feature = "crypto")]
use crate::packet::KeyMaterial;
use crate::packet::handshake::{HS_EXT_FLAG_CONFIG, HS_EXT_FLAG_HSREQ};
#[cfg(feature = "crypto")]
use crate::packet::{Cipher, KmAuth, KmKeyFlag, StreamEncapsulation};
use crate::packet::{
    ControlPacket, EncryptionField, GroupMembershipExtension, HandshakeExtensionMessageFlags,
    HandshakePacket, HandshakeType, HsExtMessage,
};

/// A base protocol version number of `4` — the value the Caller's INDUCTION
/// handshake MUST always carry (`draft-sharabayko-srt-01` §4.3.1.1), kept for
/// UDT compatibility.
pub const HANDSHAKE_VERSION_4: u32 = 4;
/// A base protocol version number of `5` — HSv5, used by every handshake
/// message from the Listener's INDUCTION response onward (§4.3.1.1/§4.3.1.2).
pub const HANDSHAKE_VERSION_5: u32 = 5;

/// The `Extension Field` value the Listener echoes on its INDUCTION response
/// so the Caller can recognise it as an SRT (not legacy UDT) party
/// (`draft-sharabayko-srt-01` §4.3.1.1: "SRT magic code 0x4A17").
pub const SRT_MAGIC_CODE: u16 = 0x4A17;

/// The `Extension Field` value the Caller sets on its very first INDUCTION
/// handshake (§4.3.1.1: "Extension Field: 2"). This is *not* the §3.2.1
/// Table 3 `Extension Flags` bitmask (whose `KMREQ` bit happens to share the
/// same numeric value) — it is a legacy UDT socket-type field (`UDT_DGRAM`)
/// carried over because the INDUCTION handshake is version-4-shaped for UDT
/// compatibility.
pub const INDUCTION_LEGACY_SOCKET_TYPE: u16 = 2;

/// The base wire value of the Handshake Rejection Reason codes
/// (`draft-sharabayko-srt-01` §4.3, Table 7): a rejected connection's
/// `Handshake Type` field carries `1000 + <code>`.
pub const REJECTION_CODE_BASE: u32 = 1000;

/// One past the largest `Handshake Type` value read as a rejection: libsrt's
/// request type is a signed 32-bit integer, so everything from `2^31` up is
/// the negative `URQ_*` range, never a rejection reason (see
/// [`RejectionReason::from_handshake_type`]).
pub const REJECTION_CODE_LIMIT: u32 = 0x8000_0000;

/// Handshake Rejection Reason (`draft-sharabayko-srt-01` §4.3, Table 7). Sent
/// in place of a normal `Handshake Type` value (`1000 + code`, decoded via
/// [`HandshakeType::Reserved`]) when a connection attempt is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RejectionReason {
    /// `1000`: unknown reason.
    Unknown,
    /// `1001`: system function error.
    System,
    /// `1002`: rejected by peer.
    Peer,
    /// `1003`: resource allocation problem.
    Resource,
    /// `1004`: incorrect data in handshake.
    Rogue,
    /// `1005`: listener's backlog exceeded.
    Backlog,
    /// `1006`: internal program error.
    Ipe,
    /// `1007`: socket is closing.
    Close,
    /// `1008`: peer is an older version than the agent's minimum.
    Version,
    /// `1009`: rendezvous cookie collision.
    RdvCookie,
    /// `1010`: wrong password.
    BadSecret,
    /// `1011`: password required or unexpected.
    Unsecure,
    /// `1012`: stream flag collision.
    MessageApi,
    /// `1013`: incompatible congestion-controller type.
    Congestion,
    /// `1014`: incompatible packet filter.
    Filter,
    /// `1015`: incompatible group.
    Group,
    /// A `1000 + code` value Table 7 does not define.
    Reserved(u32),
}

impl RejectionReason {
    /// Decode a Table 7 wire code (the value carried in `Handshake Type`,
    /// i.e. already `1000 + code`).
    pub fn from_bits(v: u32) -> Self {
        match v {
            1000 => RejectionReason::Unknown,
            1001 => RejectionReason::System,
            1002 => RejectionReason::Peer,
            1003 => RejectionReason::Resource,
            1004 => RejectionReason::Rogue,
            1005 => RejectionReason::Backlog,
            1006 => RejectionReason::Ipe,
            1007 => RejectionReason::Close,
            1008 => RejectionReason::Version,
            1009 => RejectionReason::RdvCookie,
            1010 => RejectionReason::BadSecret,
            1011 => RejectionReason::Unsecure,
            1012 => RejectionReason::MessageApi,
            1013 => RejectionReason::Congestion,
            1014 => RejectionReason::Filter,
            1015 => RejectionReason::Group,
            other => RejectionReason::Reserved(other),
        }
    }

    /// The wire code (`1000 + code`), as carried in `Handshake Type`.
    pub fn to_bits(self) -> u32 {
        match self {
            RejectionReason::Unknown => 1000,
            RejectionReason::System => 1001,
            RejectionReason::Peer => 1002,
            RejectionReason::Resource => 1003,
            RejectionReason::Rogue => 1004,
            RejectionReason::Backlog => 1005,
            RejectionReason::Ipe => 1006,
            RejectionReason::Close => 1007,
            RejectionReason::Version => 1008,
            RejectionReason::RdvCookie => 1009,
            RejectionReason::BadSecret => 1010,
            RejectionReason::Unsecure => 1011,
            RejectionReason::MessageApi => 1012,
            RejectionReason::Congestion => 1013,
            RejectionReason::Filter => 1014,
            RejectionReason::Group => 1015,
            RejectionReason::Reserved(v) => v,
        }
    }

    /// Spec label (Table 7).
    pub fn name(&self) -> &'static str {
        match self {
            RejectionReason::Unknown => "REJ_UNKNOWN",
            RejectionReason::System => "REJ_SYSTEM",
            RejectionReason::Peer => "REJ_PEER",
            RejectionReason::Resource => "REJ_RESOURCE",
            RejectionReason::Rogue => "REJ_ROGUE",
            RejectionReason::Backlog => "REJ_BACKLOG",
            RejectionReason::Ipe => "REJ_IPE",
            RejectionReason::Close => "REJ_CLOSE",
            RejectionReason::Version => "REJ_VERSION",
            RejectionReason::RdvCookie => "REJ_RDVCOOKIE",
            RejectionReason::BadSecret => "REJ_BADSECRET",
            RejectionReason::Unsecure => "REJ_UNSECURE",
            RejectionReason::MessageApi => "REJ_MESSAGEAPI",
            RejectionReason::Congestion => "REJ_CONGESTION",
            RejectionReason::Filter => "REJ_FILTER",
            RejectionReason::Group => "REJ_GROUP",
            RejectionReason::Reserved(_) => "reserved",
        }
    }

    /// Recover the [`RejectionReason`] a peer sent back as a `Handshake
    /// Type`, or `None` if `ht` is a normal (non-rejection) handshake type.
    ///
    /// Only `1000..`[`REJECTION_CODE_LIMIT`] is a rejection: libsrt carries the
    /// handshake request type as a *signed* 32-bit integer whose negative
    /// values (the `URQ_*` family, e.g. `CONCLUSION` = -1 = `0xFFFF_FFFF`)
    /// are handshake types, not reasons, so an undefined value such as
    /// `0xFFFF_FFFC` is not a rejection.
    pub fn from_handshake_type(ht: HandshakeType) -> Option<Self> {
        match ht {
            HandshakeType::Reserved(v)
                if (REJECTION_CODE_BASE..REJECTION_CODE_LIMIT).contains(&v) =>
            {
                Some(RejectionReason::from_bits(v))
            }
            _ => None,
        }
    }

    /// Encode as the `Handshake Type` value a rejection packet carries.
    pub fn to_handshake_type(self) -> HandshakeType {
        HandshakeType::from_bits(self.to_bits())
    }
}

broadcast_common::impl_spec_display!(RejectionReason, Reserved);

/// Opt-in §6 payload-encryption config for one side of a Caller-Listener
/// handshake (`draft-sharabayko-srt-01` §6.1.5, Key Material Exchange —
/// curated at `specs/rules/srt-crypto.md`). `None` on
/// [`HandshakeConfig::crypto`] (the default) disables the encryption path
/// entirely: no Key Material extension is sent, and a peer that sends one
/// unexpectedly is rejected ([`RejectionReason::Unsecure`]).
///
/// This crate's sans-IO core never reads OS randomness — the same design
/// choice as [`derive_cookie`]'s caller-supplied `time_bucket`/`secret`
/// inputs, or [`crate::io`]'s OS-random SYN Cookie key.
/// [`Self::salt`] and, on the initiator, [`Self::sek`] must be freshly
/// generated by the caller/driver (e.g. a `tokio` adapter with access to a
/// real CSPRNG) for **every new connection** (§6.2.1: `Salt = PRNG(128)`,
/// `SEK = PRNG(KLen)`) and never reused across connections.
///
/// # Secret hygiene (issue #1142)
/// The passphrase and SEK never appear in [`core::fmt::Debug`] output (it
/// prints only their lengths), [`PartialEq`] compares them in constant time,
/// and dropping a `CryptoConfig` (or calling [`Zeroize::zeroize`]) wipes them.
#[cfg(feature = "crypto")]
#[cfg_attr(docsrs, doc(cfg(feature = "crypto")))]
#[derive(Clone)]
pub struct CryptoConfig {
    /// The pre-shared secret (§6.1.4). Must be identical on both peers, or
    /// the responder's KEK derivation recovers the wrong SEK and the RFC
    /// 3394 wrap's integrity check fails
    /// ([`RejectionReason::BadSecret`] on the Listener side).
    pub passphrase: Vec<u8>,
    /// A fresh, cryptographically random 128-bit Salt for **this
    /// connection** (§6.2.1: `Salt = PRNG(128)`). See the struct doc for why
    /// this crate does not generate it internally.
    pub salt: [u8; crate::crypto::SALT_LEN],
    /// This side's plaintext Stream Encrypting Key. **Only meaningful on the
    /// connection initiator** (the Caller — §6.1.5: "sent by the connection
    /// initiator ... to the responder"). Must be a fresh, cryptographically
    /// random 16/24/32-byte value (§6.2.1: `SEK = PRNG(KLen)`) matching
    /// [`HandshakeConfig::encryption_field`]'s advertised cipher. Ignored on
    /// the Listener/responder side (which instead recovers the SEK by
    /// unwrapping the initiator's Key Material) — may be left empty there.
    pub sek: Vec<u8>,
}

#[cfg(feature = "crypto")]
impl core::fmt::Debug for CryptoConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CryptoConfig")
            .field(
                "passphrase",
                &format_args!("<redacted, {} bytes>", self.passphrase.len()),
            )
            .field(
                "salt",
                &format_args!("<redacted, {} bytes>", self.salt.len()),
            )
            .field("sek", &format_args!("<redacted, {} bytes>", self.sek.len()))
            .finish()
    }
}

#[cfg(feature = "crypto")]
impl PartialEq for CryptoConfig {
    /// Constant-time in the secret contents (the lengths are not secret).
    fn eq(&self, other: &Self) -> bool {
        let passphrase = self
            .passphrase
            .as_slice()
            .ct_eq(other.passphrase.as_slice());
        let salt = self.salt.as_slice().ct_eq(other.salt.as_slice());
        let sek = self.sek.as_slice().ct_eq(other.sek.as_slice());
        bool::from(passphrase & salt & sek)
    }
}

#[cfg(feature = "crypto")]
impl Zeroize for CryptoConfig {
    fn zeroize(&mut self) {
        self.passphrase.zeroize();
        self.salt.zeroize();
        self.sek.zeroize();
    }
}

#[cfg(feature = "crypto")]
impl Drop for CryptoConfig {
    fn drop(&mut self) {
        self.zeroize();
    }
}

#[cfg(feature = "crypto")]
impl zeroize::ZeroizeOnDrop for CryptoConfig {}

/// Secret key bytes — a negotiated Stream Encrypting Key.
///
/// Derefs to `&[u8]` for use with [`crate::crypto::aes_ctr_apply`], but is
/// deliberately not a plain `Vec<u8>`: [`core::fmt::Debug`] prints only the
/// length, serde never serializes it (it is skipped where it is a field),
/// [`PartialEq`] is constant-time, and it is wiped when dropped (issue #1142).
#[cfg(feature = "crypto")]
#[cfg_attr(docsrs, doc(cfg(feature = "crypto")))]
#[derive(Clone)]
pub struct SecretBytes(Vec<u8>);

#[cfg(feature = "crypto")]
impl SecretBytes {
    /// Wrap `bytes`.
    pub fn new(bytes: Vec<u8>) -> Self {
        SecretBytes(bytes)
    }
}

#[cfg(feature = "crypto")]
impl From<Vec<u8>> for SecretBytes {
    fn from(bytes: Vec<u8>) -> Self {
        SecretBytes(bytes)
    }
}

#[cfg(feature = "crypto")]
impl core::ops::Deref for SecretBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(feature = "crypto")]
impl core::fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SecretBytes(<redacted, {} bytes>)", self.0.len())
    }
}

#[cfg(feature = "crypto")]
impl PartialEq for SecretBytes {
    /// Constant-time in the contents (the length is not secret).
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.0.as_slice().ct_eq(other.0.as_slice()))
    }
}

#[cfg(feature = "crypto")]
impl Eq for SecretBytes {}

#[cfg(feature = "crypto")]
impl Zeroize for SecretBytes {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(feature = "crypto")]
impl Drop for SecretBytes {
    fn drop(&mut self) {
        self.zeroize();
    }
}

#[cfg(feature = "crypto")]
impl zeroize::ZeroizeOnDrop for SecretBytes {}

/// Local configuration for one side of a Caller-Listener handshake — the
/// values *this* engine advertises (`draft-sharabayko-srt-01` §3.2.1,
/// §3.2.1.1, §3.2.1.3, §3.2.1.4).
#[derive(Debug, Clone, PartialEq)]
pub struct HandshakeConfig {
    /// TSBPD delay in milliseconds this side requests, sent as both the
    /// Receiver and Sender TSBPD Delay of its Handshake Extension Message
    /// (§3.2.1.1, Figure 6). The negotiated value is the greater of the two
    /// parties' requests (§4.3.1.2).
    pub latency_ms: u16,
    /// Maximum Transmission Unit Size (§3.2.1).
    pub mtu: u32,
    /// Maximum Flow Window Size (§3.2.1).
    pub max_flow_window_size: u32,
    /// Initial Packet Sequence Number this side will use for its first data
    /// packet (§3.2.1). No data plane exists yet in this crate; callers of a
    /// later data-plane PR are expected to set this to a real ISN.
    pub initial_seq_number: u32,
    /// The `SRT Version` this side reports in its Handshake Extension
    /// Message (`major * 0x10000 + minor * 0x100 + patch`, §3.2.1.1).
    pub srt_version: u32,
    /// `SRT Flags` this side advertises (§3.2.1.1.1, Table 6).
    pub flags: HandshakeExtensionMessageFlags,
    /// Advertised cipher family and block size (§3.2.1, Table 2). Actual
    /// key-wrap/unwrap crypto is an explicit follow-up; this field is
    /// negotiated but not acted on in this release.
    pub encryption_field: EncryptionField,
    /// Stream ID to advertise (Caller only — §3.2.1.3); `None` sends no
    /// Stream ID extension.
    pub stream_id: Option<String>,
    /// Group Membership to advertise (§3.2.1.4); `None` sends no Group
    /// extension.
    pub group: Option<GroupMembershipExtension>,
    /// This side's own IP address, reported in every outgoing handshake
    /// packet's `Peer IP Address` field (§3.2.1: "IPv4 or IPv6 address of
    /// the packet's *sender*" — despite the field's name, each party reports
    /// its own address, not its peer's).
    pub local_ip: [u32; 4],
    /// Number of [`crate::caller::CallerHandshake::tick`] /
    /// [`crate::listener::ListenerHandshake::tick`] calls with no reply
    /// before the last-sent handshake packet is retransmitted. Timeouts are
    /// modeled as caller-driven ticks — no wall-clock lives in this crate.
    pub retransmit_after_ticks: u32,
    /// Maximum number of retransmissions before the handshake gives up
    /// ([`HandshakeOutput::TimedOut`]).
    pub max_retries: u32,
    /// Opt-in §6 payload-encryption negotiation (`crypto` feature only).
    /// `None` (the default) means this side neither offers nor requires
    /// encryption. See [`CryptoConfig`] for the caller-supplied Salt/SEK
    /// contract.
    #[cfg(feature = "crypto")]
    #[cfg_attr(docsrs, doc(cfg(feature = "crypto")))]
    pub crypto: Option<CryptoConfig>,
}

/// Default [`HandshakeConfig::max_retries`]: twelve retransmissions at the
/// adapter's 250 ms retransmit period is libsrt's 3 s connect timeout
/// (`SRTO_CONNTIMEO`) — the previous 5 gave up after 1.25 s of a 250 ms
/// retransmit period.
pub const DEFAULT_MAX_RETRIES: u32 = 12;

impl Default for HandshakeConfig {
    fn default() -> Self {
        HandshakeConfig {
            latency_ms: 120,
            mtu: 1500,
            max_flow_window_size: 8192,
            initial_seq_number: 0,
            srt_version: 0x0105_0000,
            flags: HandshakeExtensionMessageFlags(
                crate::packet::handshake::HS_MSG_FLAG_TSBPDSND
                    | crate::packet::handshake::HS_MSG_FLAG_TSBPDRCV
                    | crate::packet::handshake::HS_MSG_FLAG_CRYPT
                    | crate::packet::handshake::HS_MSG_FLAG_TLPKTDROP
                    | crate::packet::handshake::HS_MSG_FLAG_PERIODICNAK
                    | crate::packet::handshake::HS_MSG_FLAG_REXMITFLG,
            ),
            encryption_field: EncryptionField::NoEncryption,
            stream_id: None,
            group: None,
            local_ip: [0, 0, 0, 0],
            retransmit_after_ticks: 3,
            max_retries: DEFAULT_MAX_RETRIES,
            #[cfg(feature = "crypto")]
            crypto: None,
        }
    }
}

/// The outcome of a Caller-Listener handshake, once both TSBPD delays and
/// flags have been reconciled per `draft-sharabayko-srt-01` §4.3.1.2 ("The
/// value for latency is always agreed to be the greater of those reported by
/// each party").
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NegotiatedParams {
    /// The negotiated base protocol version — always `5` (HSv5) for a
    /// completed handshake in this crate.
    pub version: u32,
    /// The flags both sides advertised, ANDed together (only mutually
    /// supported capabilities are considered agreed).
    pub flags: HandshakeExtensionMessageFlags,
    /// The agreed TSBPD latency in milliseconds: the greater of this side's
    /// configured [`HandshakeConfig::latency_ms`] and the peer's reported
    /// Receiver/Sender TSBPD Delay (§4.3.1.2).
    pub latency_ms: u16,
    /// This side's own SRT Socket ID.
    pub own_socket_id: u32,
    /// The peer's SRT Socket ID.
    pub peer_socket_id: u32,
    /// The agreed Maximum Transmission Unit Size (§3.2.1): the smaller of the
    /// two parties' advertised values, so neither is sent a datagram larger
    /// than it said it could carry.
    pub mtu: u32,
    /// The agreed Maximum Flow Window Size (§3.2.1): the smaller of the two
    /// parties' advertised values — how many packets may be unacknowledged in
    /// flight (the sender's window) and how far ahead of its ack point a
    /// receiver tracks (the receiver's window).
    pub max_flow_window_size: u32,
    /// The negotiated Stream ID (§3.2.1.3), if one was advertised.
    pub stream_id: Option<String>,
    /// The negotiated Group Membership (§3.2.1.4), if one was advertised.
    pub group: Option<GroupMembershipExtension>,
    /// The negotiated Stream Encrypting Key (§6.1.5), if
    /// [`HandshakeConfig::crypto`] was set on this side and the Key Material
    /// exchange succeeded. `None` if encryption was not negotiated.
    ///
    /// A [`SecretBytes`]: redacted in `Debug`, constant-time in `==`, wiped on
    /// drop, and never serialized.
    #[cfg(feature = "crypto")]
    #[cfg_attr(docsrs, doc(cfg(feature = "crypto")))]
    #[cfg_attr(feature = "serde", serde(skip))]
    pub sek: Option<SecretBytes>,
    /// The Salt paired with [`Self::sek`] — both are required by
    /// [`crate::crypto::aes_ctr_apply`] to encrypt/decrypt a data packet's
    /// payload (§6.1.2/§6.2.2/§6.3.2).
    #[cfg(feature = "crypto")]
    #[cfg_attr(docsrs, doc(cfg(feature = "crypto")))]
    pub salt: Option<[u8; crate::crypto::SALT_LEN]>,
}

/// An event produced by [`crate::caller::CallerHandshake::feed`] /
/// [`crate::caller::CallerHandshake::start`] /
/// [`crate::listener::ListenerHandshake::feed`] / either engine's `tick`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HandshakeOutput {
    /// Send these bytes (one serialized Handshake control packet, built from
    /// the existing [`crate::packet`] codecs) to the peer.
    Send(Vec<u8>),
    /// The handshake completed; here are the negotiated parameters.
    Connected(NegotiatedParams),
    /// The handshake was rejected — either the peer sent back an explicit
    /// [`RejectionReason`], or a local validation check failed (in which
    /// case a matching `Send` rejection packet is also emitted, in the same
    /// output batch, for the Listener side).
    Rejected(RejectionReason),
    /// No reply arrived after [`HandshakeConfig::max_retries`]
    /// retransmissions.
    TimedOut,
}

/// Serializes one [`HandshakePacket`] using the existing [`ControlPacket`]
/// codec — never hand-rolled.
pub(crate) fn build_bytes(hp: HandshakePacket<'_>) -> Result<Vec<u8>> {
    let pkt = ControlPacket::Handshake(hp);
    let mut buf = alloc::vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut buf)?;
    Ok(buf)
}

/// Builds the extensions payload for a CONCLUSION-phase handshake carrying a
/// Handshake Extension Message plus optional Stream ID / Group Membership,
/// returning `(bytes, Extension Field flags)`.
pub(crate) fn build_conclusion_extensions(
    hs_ext_type: crate::packet::ExtensionType,
    hs_msg: &HsExtMessage,
    stream_id: Option<&str>,
    group: Option<GroupMembershipExtension>,
) -> Result<(Vec<u8>, u16)> {
    use crate::packet::handshake::{build_extension_block, encode_stream_id};

    let mut ext_bytes = Vec::new();
    ext_bytes.extend(build_extension_block(hs_ext_type, &hs_msg.to_bytes())?);
    let mut ext_flags = HS_EXT_FLAG_HSREQ;
    if let Some(sid) = stream_id {
        let sid_bytes = encode_stream_id(sid);
        ext_bytes.extend(build_extension_block(
            crate::packet::ExtensionType::Sid,
            &sid_bytes,
        )?);
        ext_flags |= HS_EXT_FLAG_CONFIG;
    }
    if let Some(g) = group {
        ext_bytes.extend(build_extension_block(
            crate::packet::ExtensionType::Group,
            &g.to_bytes(),
        )?);
        ext_flags |= HS_EXT_FLAG_CONFIG;
    }
    Ok((ext_bytes, ext_flags))
}

/// A parsed peer extension payload relevant to the Caller-Listener handshake:
/// the mandatory Handshake Extension Message plus any optional Stream ID /
/// Group Membership / Key Material.
#[derive(Debug, Default)]
pub(crate) struct ParsedPeerExtensions<'a> {
    pub hs_msg: Option<HsExtMessage>,
    pub stream_id: Option<String>,
    pub group: Option<GroupMembershipExtension>,
    /// The Key Material extension (§3.2.1.2/§3.2.2, `SRT_CMD_KMREQ` on the
    /// Caller's CONCLUSION / `SRT_CMD_KMRSP` on the Listener's) — decoded
    /// regardless of the `crypto` feature (wire-structure decode only,
    /// [`KeyMaterial`] has no crypto dependency); only *acted on*
    /// (KEK derive/unwrap/wrap) when `crypto` is enabled, see
    /// [`crate::caller::CallerHandshake`]/[`crate::listener::ListenerHandshake`].
    #[cfg(feature = "crypto")]
    pub km: Option<KeyMaterial<'a>>,
    /// Keeps the `'a` lifetime parameter meaningful when the `crypto`
    /// feature (and with it, [`Self::km`]) is compiled out.
    #[cfg(not(feature = "crypto"))]
    _phantom: core::marker::PhantomData<&'a ()>,
}

/// Walks a handshake packet's extension blocks, decoding the ones this crate
/// understands. Returns [`Error::InvalidField`] (mapped by the caller to a
/// [`RejectionReason::Rogue`] rejection) on any decode failure, rather than
/// panicking — extension content is untrusted peer input.
pub(crate) fn parse_peer_extensions<'a>(
    hp: &HandshakePacket<'a>,
) -> Result<ParsedPeerExtensions<'a>> {
    use crate::packet::ExtensionType;

    let mut out = ParsedPeerExtensions::default();
    for block in hp.extensions.iter() {
        let block = block.map_err(|_| Error::InvalidField {
            what: "handshake extensions",
            reason: "malformed extension block",
        })?;
        match block.ext_type {
            ExtensionType::HsReq | ExtensionType::HsRsp => {
                let msg = block.as_hs_ext_message().map_err(|_| Error::InvalidField {
                    what: "handshake extension message",
                    reason: "malformed HSREQ/HSRSP contents",
                })?;
                out.hs_msg = Some(msg);
            }
            ExtensionType::Sid => {
                let sid = block.as_stream_id().map_err(|_| Error::InvalidField {
                    what: "stream ID extension",
                    reason: "malformed or non-UTF-8 contents",
                })?;
                out.stream_id = Some(sid);
            }
            ExtensionType::Group => {
                let g = block
                    .as_group_membership()
                    .map_err(|_| Error::InvalidField {
                        what: "group membership extension",
                        reason: "malformed contents",
                    })?;
                out.group = Some(g);
            }
            #[cfg(feature = "crypto")]
            ExtensionType::KmReq | ExtensionType::KmRsp => {
                let km = block.as_key_material().map_err(|_| Error::InvalidField {
                    what: "key material extension",
                    reason: "malformed contents",
                })?;
                out.km = Some(km);
            }
            _ => {}
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// §6.1.5 Key Material Exchange — piggybacked on the CONCLUSION extensions.
// ---------------------------------------------------------------------------

/// Build the Key Material handshake extension block (`Extension Type` +
/// `Extension Length` + contents — ready to append after the mandatory
/// HSREQ block) offering `crypto.sek`, wrapped under a KEK derived from
/// `crypto.passphrase`/`crypto.salt` (`draft-sharabayko-srt-01` §6.1.5 "sent
/// by the connection initiator ... to the responder", §6.2.1's `Wrap =
/// AESkw(KEK, SEK)`). Always encodes `KK = Even` — this crate's convention
/// for the handshake-negotiated initial SEK; §6.1.6's odd/even alternation
/// only matters from the first KM Refresh onward (see [`crate::km_refresh`]).
#[cfg(feature = "crypto")]
pub(crate) fn build_key_material_extension(crypto: &CryptoConfig) -> Result<Vec<u8>> {
    let kek = crate::crypto::derive_kek(&crypto.passphrase, &crypto.salt, crypto.sek.len())?;
    let (icv, wrapped) = crate::crypto::wrap_sek(&kek, &crypto.sek)?;
    let km = KeyMaterial {
        kk: KmKeyFlag::Even,
        keki: 0,
        cipher: Cipher::AesCtr,
        auth: KmAuth::None,
        se: StreamEncapsulation::MpegTsSrt,
        salt: &crypto.salt,
        icv,
        x_sek: &wrapped,
        o_sek: None,
    };
    let mut buf = alloc::vec![0u8; km.serialized_len()];
    km.serialize_into(&mut buf)?;
    crate::packet::handshake::build_extension_block(crate::packet::ExtensionType::KmReq, &buf)
}

/// A recovered/negotiated `(SEK, Salt)` pair.
#[cfg(feature = "crypto")]
pub(crate) type RecoveredSek = (SecretBytes, [u8; crate::crypto::SALT_LEN]);

/// Recover the SEK a peer's Key Material offered, given this side's shared
/// passphrase (§6.1.5/§6.3.1: "the responder MUST know the passphrase ...
/// everything else needed is extracted from the Keying Material message").
/// Returns the recovered SEK bytes and the 16-byte Salt — both required by
/// [`crate::crypto::aes_ctr_apply`].
///
/// # Errors
/// [`Error::InvalidField`] if the Salt is not 16 bytes (the only Salt length
/// this crate's Key Material codec accepts); otherwise propagates
/// [`crate::crypto::unwrap_sek`]'s error (RFC 3394 wrap-integrity failure —
/// wrong passphrase, §6.1.5's "it does not have the SEK" case).
#[cfg(feature = "crypto")]
pub(crate) fn recover_sek(passphrase: &[u8], km: &KeyMaterial<'_>) -> Result<RecoveredSek> {
    if km.salt.len() != crate::crypto::SALT_LEN {
        return Err(Error::InvalidField {
            what: "Key Material Salt",
            reason: "must be 16 bytes (the only Salt length this crate's codec accepts)",
        });
    }
    let mut salt = [0u8; crate::crypto::SALT_LEN];
    salt.copy_from_slice(km.salt);
    let kek = crate::crypto::derive_kek(passphrase, &salt, km.x_sek.len())?;
    let sek = SecretBytes::new(crate::crypto::unwrap_sek(&kek, &km.icv, km.x_sek)?.to_vec());
    Ok((sek, salt))
}

/// Re-serialize a parsed Key Material message and wrap it as a `KmRsp`
/// extension block — the responder's confirmation echo (§6.1.5: "the
/// responder ... echoes the same KM message back to prove it derived the
/// same SEK").
#[cfg(feature = "crypto")]
pub(crate) fn echo_key_material_as_response(km: &KeyMaterial<'_>) -> Result<Vec<u8>> {
    let mut buf = alloc::vec![0u8; km.serialized_len()];
    km.serialize_into(&mut buf)?;
    crate::packet::handshake::build_extension_block(crate::packet::ExtensionType::KmRsp, &buf)
}

/// Verify a Listener's echoed Key Material matches what this Caller sent.
/// Recomputed deterministically from `crypto` (the same passphrase/salt/sek
/// always wraps to the same bytes, §6.2.1) rather than requiring the Caller
/// to keep the original wrap bytes around.
#[cfg(feature = "crypto")]
pub(crate) fn verify_km_echo(crypto: &CryptoConfig, echoed: &KeyMaterial<'_>) -> bool {
    let kek = match crate::crypto::derive_kek(&crypto.passphrase, &crypto.salt, crypto.sek.len()) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let (icv, wrapped) = match crate::crypto::wrap_sek(&kek, &crypto.sek) {
        Ok(v) => v,
        Err(_) => return false,
    };
    // The wrapped key is derived from the secret: compare it in constant time.
    let salt = echoed.salt.ct_eq(crypto.salt.as_slice());
    let icv = echoed.icv.as_slice().ct_eq(icv.as_slice());
    let wrapped = echoed.x_sek.ct_eq(wrapped.as_slice());
    bool::from(salt & icv & wrapped)
}

/// The greater-of-both-parties TSBPD latency rule (§4.3.1.2).
pub(crate) fn negotiate_latency_ms(local_latency_ms: u16, peer_msg: &HsExtMessage) -> u16 {
    local_latency_ms
        .max(peer_msg.receiver_tsbpd_delay_ms)
        .max(peer_msg.sender_tsbpd_delay_ms)
}

/// SipHash-2-4 (Aumasson & Bernstein, "SipHash: a fast short-input PRF",
/// 2012): a keyed 64-bit pseudorandom function designed for exactly this use —
/// short inputs, an unpredictable key, an attacker who must not be able to
/// forge outputs. Verified against the paper's reference test vector in this
/// module's tests.
fn siphash24(k0: u64, k1: u64, data: &[u8]) -> u64 {
    // Compression rounds per message block and finalisation rounds: the "2-4"
    // of SipHash-2-4.
    const C_ROUNDS: usize = 2;
    const D_ROUNDS: usize = 4;
    // The four state words' initialisation constants (ASCII of
    // "somepseudorandomlygeneratedbytes"), per the SipHash paper.
    const INIT_V0: u64 = 0x736f_6d65_7073_6575;
    const INIT_V1: u64 = 0x646f_7261_6e64_6f6d;
    const INIT_V2: u64 = 0x6c79_6765_6e65_7261;
    const INIT_V3: u64 = 0x7465_6462_7974_6573;
    // XORed into `v2` before the finalisation rounds.
    const FINALIZATION_XOR: u64 = 0xFF;

    let mut v0 = k0 ^ INIT_V0;
    let mut v1 = k1 ^ INIT_V1;
    let mut v2 = k0 ^ INIT_V2;
    let mut v3 = k1 ^ INIT_V3;

    fn round(v0: &mut u64, v1: &mut u64, v2: &mut u64, v3: &mut u64) {
        *v0 = v0.wrapping_add(*v1);
        *v1 = v1.rotate_left(13);
        *v1 ^= *v0;
        *v0 = v0.rotate_left(32);
        *v2 = v2.wrapping_add(*v3);
        *v3 = v3.rotate_left(16);
        *v3 ^= *v2;
        *v0 = v0.wrapping_add(*v3);
        *v3 = v3.rotate_left(21);
        *v3 ^= *v0;
        *v2 = v2.wrapping_add(*v1);
        *v1 = v1.rotate_left(17);
        *v1 ^= *v2;
        *v2 = v2.rotate_left(32);
    }

    let mut chunks = data.chunks_exact(8);
    for chunk in chunks.by_ref() {
        let mut word = [0u8; 8];
        word.copy_from_slice(chunk);
        let m = u64::from_le_bytes(word);
        v3 ^= m;
        for _ in 0..C_ROUNDS {
            round(&mut v0, &mut v1, &mut v2, &mut v3);
        }
        v0 ^= m;
    }
    // Final block: the remaining 0..=7 bytes, with the input length in the
    // top byte (modulo 256, as the reference does).
    let mut last = [0u8; 8];
    let rest = chunks.remainder();
    last[..rest.len()].copy_from_slice(rest);
    last[7] = u8::try_from(data.len() & 0xFF).unwrap_or(0);
    let m = u64::from_le_bytes(last);
    v3 ^= m;
    for _ in 0..C_ROUNDS {
        round(&mut v0, &mut v1, &mut v2, &mut v3);
    }
    v0 ^= m;
    v2 ^= FINALIZATION_XOR;
    for _ in 0..D_ROUNDS {
        round(&mut v0, &mut v1, &mut v2, &mut v3);
    }
    v0 ^ v1 ^ v2 ^ v3
}

/// Derives a SYN Cookie from caller-supplied inputs (`draft-sharabayko-srt-01`
/// §4.3.1.1: "a cookie that is crafted based on host, port and current time
/// with 1 minute accuracy"). The draft specifies the semantic inputs, not a
/// wire algorithm; this crate's core never reads a clock, so `time_bucket`
/// (e.g. UNIX time / 60) must come from the caller/driver.
///
/// The cookie is the low 32 bits of **SipHash-2-4** over `peer_key ‖
/// time_bucket`, keyed by the 128-bit `secret` (SipHash's whole key, low half
/// as `k0`): a real keyed PRF, so a peer that sees cookies cannot predict or
/// forge the cookie for another `(peer, minute)` without the secret. It is
/// still *not* a message-authentication code of the handshake packet: the
/// cookie only proves the Caller received the INDUCTION response sent to its
/// address, and its 32 bits are a flood-protection token, not a long-lived
/// credential. `secret` must come from a CSPRNG and should be replaced
/// periodically (the tokio adapter's listener draws it from the OS and
/// rotates it every minute).
pub fn derive_cookie(peer_key: u64, time_bucket: u32, secret: u128) -> u32 {
    let mut input = [0u8; 12];
    input[..8].copy_from_slice(&peer_key.to_le_bytes());
    input[8..].copy_from_slice(&time_bucket.to_le_bytes());
    let k0 = u64::try_from(secret & u128::from(u64::MAX)).unwrap_or(0);
    let k1 = u64::try_from(secret >> 64).unwrap_or(0);
    let digest = siphash24(k0, k1, &input);
    // Keep the low 32 bits; `| 1` so it is never 0, never confused with "no
    // cookie yet".
    u32::try_from(digest & u64::from(u32::MAX)).unwrap_or(0) | 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_reason_round_trips_table_7() {
        let all = [
            RejectionReason::Unknown,
            RejectionReason::System,
            RejectionReason::Peer,
            RejectionReason::Resource,
            RejectionReason::Rogue,
            RejectionReason::Backlog,
            RejectionReason::Ipe,
            RejectionReason::Close,
            RejectionReason::Version,
            RejectionReason::RdvCookie,
            RejectionReason::BadSecret,
            RejectionReason::Unsecure,
            RejectionReason::MessageApi,
            RejectionReason::Congestion,
            RejectionReason::Filter,
            RejectionReason::Group,
        ];
        for (i, r) in all.iter().enumerate() {
            assert_eq!(r.to_bits(), 1000 + i as u32);
            assert_eq!(RejectionReason::from_bits(r.to_bits()), *r);
            assert_eq!(
                RejectionReason::from_handshake_type(r.to_handshake_type()),
                Some(*r)
            );
        }
    }

    #[test]
    fn non_rejection_handshake_types_are_not_a_rejection() {
        for ht in [
            HandshakeType::Induction,
            HandshakeType::Conclusion,
            HandshakeType::Wavehand,
            HandshakeType::Agreement,
            HandshakeType::Done,
        ] {
            assert_eq!(RejectionReason::from_handshake_type(ht), None);
        }
    }

    #[test]
    fn derive_cookie_is_deterministic_and_nonzero() {
        let a = derive_cookie(0x1234_5678_9ABC_DEF0, 12345, 0xDEAD_BEEF);
        let b = derive_cookie(0x1234_5678_9ABC_DEF0, 12345, 0xDEAD_BEEF);
        assert_eq!(a, b);
        assert_ne!(a, 0);
        let c = derive_cookie(0x1234_5678_9ABC_DEF1, 12345, 0xDEAD_BEEF);
        assert_ne!(a, c);
    }

    /// r08-SRT-W14: only `1000..2^31` is a rejection. `0xFFFF_FFFC` (libsrt's
    /// negative URQ range) and `2^31` itself used to be reported as
    /// `Rejected(Reserved(..))`.
    #[test]
    fn only_the_positive_rejection_range_counts_as_a_rejection() {
        let is_rejection =
            |v: u32| RejectionReason::from_handshake_type(HandshakeType::from_bits(v)).is_some();
        assert!(!is_rejection(0xFFFF_FFFC));
        assert!(!is_rejection(0xFFFF_FFFE)); // AGREEMENT is its own type
        assert!(!is_rejection(REJECTION_CODE_LIMIT));
        assert!(!is_rejection(REJECTION_CODE_BASE - 1));
        assert!(is_rejection(REJECTION_CODE_BASE));
        assert!(is_rejection(REJECTION_CODE_LIMIT - 1));
        // A predefined reason, a reason the table does not define, and a
        // user-defined (2000+) reason all decode.
        assert_eq!(
            RejectionReason::from_handshake_type(HandshakeType::from_bits(1010)),
            Some(RejectionReason::BadSecret)
        );
        assert_eq!(
            RejectionReason::from_handshake_type(HandshakeType::from_bits(1500)),
            Some(RejectionReason::Reserved(1500))
        );
        assert_eq!(
            RejectionReason::from_handshake_type(HandshakeType::from_bits(2001)),
            Some(RejectionReason::Reserved(2001))
        );
    }

    /// r08-SRT-W7: the Caller's KMREQ carries `SE` = 2 (MPEG-TS/SRT), the
    /// value libsrt's `hcryptCtx_Rx_ParseKM` requires (`km_msg[SE] ==
    /// crypto->se`, `HCRYPT_SE_TSSRT`); `SE` = 0 is rejected as a bad KM.
    #[cfg(feature = "crypto")]
    #[test]
    fn caller_kmreq_declares_the_mpeg_ts_srt_encapsulation() {
        let crypto = CryptoConfig {
            passphrase: b"0123456789abcdef".to_vec(),
            salt: [0x33; crate::crypto::SALT_LEN],
            sek: alloc::vec![0x44; 16],
        };
        let ext = build_key_material_extension(&crypto).unwrap();
        // Extension block: type(16) length(16, words) then the KM message.
        let km = KeyMaterial::parse(&ext[4..]).unwrap();
        assert_eq!(km.se, StreamEncapsulation::MpegTsSrt);
        assert_eq!(km.se.to_bits(), 2);
        // Wire byte: word 2 of the KM is Cipher(8) Auth(8) SE(8) Resv2(8).
        assert_eq!(ext[4 + 10], 2);
    }

    /// The SipHash-2-4 reference vector from the paper's appendix: key
    /// `00 01 .. 0f`, input `00 01 .. 0e` (15 bytes), output `a129ca6149be45e5`.
    #[test]
    fn siphash24_matches_the_reference_vector() {
        let k0 = u64::from_le_bytes([0, 1, 2, 3, 4, 5, 6, 7]);
        let k1 = u64::from_le_bytes([8, 9, 10, 11, 12, 13, 14, 15]);
        let input: Vec<u8> = (0u8..15).collect();
        assert_eq!(siphash24(k0, k1, &input), 0xa129_ca61_49be_45e5);
        // And the empty input (a second reference value from the same table).
        assert_eq!(siphash24(k0, k1, &[]), 0x726f_db47_dd0e_0e31);
    }

    /// The cookie depends on every input, including the secret.
    #[test]
    fn derive_cookie_depends_on_secret_peer_and_time() {
        let base = derive_cookie(1, 2, 3);
        assert_ne!(base, derive_cookie(1, 2, 4), "secret");
        assert_ne!(base, derive_cookie(9, 2, 3), "peer");
        assert_ne!(base, derive_cookie(1, 9, 3), "time bucket");
        assert_eq!(base & 1, 1);
        // Pinned against an independent SipHash-2-4 implementation (Python,
        // checked against the same reference vector): fixes the input layout
        // (`peer_key` LE ‖ `time_bucket` LE) and the key split (low half k0).
        assert_eq!(
            derive_cookie(0x1234_5678_9ABC_DEF0, 12345, 0xDEAD_BEEF),
            0x5905_9ee9
        );
        assert_eq!(
            derive_cookie(
                0x1234_5678_9ABC_DEF0,
                12345,
                0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210
            ),
            0xec46_1e0b
        );
        // Both halves of the key matter.
        assert_ne!(
            derive_cookie(1, 2, 7),
            derive_cookie(1, 2, 7 | (1u128 << 64)),
            "the high key half must influence the cookie"
        );
    }

    // ---- #1142: secrets stay out of Debug / serde, are compared in constant
    // time and wiped on drop ------------------------------------------------

    /// A non-repeating pseudo-random run, so a needle cut from it cannot occur
    /// by chance in unrelated numbers.
    #[cfg(feature = "crypto")]
    fn distinctive(seed: u32, len: usize) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(12_345);
        (0..len)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                u8::try_from(x >> 24).unwrap()
            })
            .collect()
    }

    /// Every spelling a leak of the first six bytes of `bytes` could take.
    #[cfg(feature = "crypto")]
    fn needles(bytes: &[u8]) -> Vec<String> {
        let head = &bytes[..6];
        let dec = |sep: &str| {
            head.iter()
                .map(|b| alloc::format!("{b}"))
                .collect::<Vec<_>>()
                .join(sep)
        };
        alloc::vec![
            dec(", "),
            dec(","),
            head.iter()
                .map(|b| alloc::format!("{b:x}"))
                .collect::<Vec<_>>()
                .join(", "),
            head.iter().map(|b| alloc::format!("{b:02x}")).collect(),
        ]
    }

    #[cfg(feature = "crypto")]
    fn secret_config() -> CryptoConfig {
        CryptoConfig {
            passphrase: distinctive(1, 15),
            salt: distinctive(2, crate::crypto::SALT_LEN).try_into().unwrap(),
            sek: distinctive(3, 16),
        }
    }

    /// `{:?}` and `{:#?}` of a config (alone or inside a `HandshakeConfig`)
    /// carry no passphrase, salt or key bytes — only their lengths.
    #[cfg(feature = "crypto")]
    #[test]
    fn crypto_config_debug_never_shows_secret_bytes() {
        let config = HandshakeConfig {
            crypto: Some(secret_config()),
            ..HandshakeConfig::default()
        };
        for dump in [
            alloc::format!("{:?}", secret_config()),
            alloc::format!("{:#?}", secret_config()),
            alloc::format!("{:?}", config),
            alloc::format!("{:#?}", config),
            alloc::format!("{:x?}", config),
        ] {
            let secret = secret_config();
            for (name, bytes) in [
                ("passphrase", &secret.passphrase[..]),
                ("salt", &secret.salt[..]),
                ("sek", &secret.sek[..]),
            ] {
                for needle in needles(bytes) {
                    assert!(
                        !dump.contains(&needle),
                        "{name} leaked as {needle:?}: {dump}"
                    );
                }
            }
            assert!(dump.contains("redacted"), "{dump}");
        }
        assert!(alloc::format!("{:?}", secret_config()).contains("15 bytes"));
    }

    /// A negotiated key is redacted in `Debug` — also when it rides inside a
    /// [`HandshakeOutput`] — and is not serialized.
    #[cfg(feature = "crypto")]
    #[test]
    fn negotiated_sek_is_redacted_in_debug_and_absent_from_serde() {
        let sek_bytes = distinctive(9, 16);
        let params = NegotiatedParams {
            version: HANDSHAKE_VERSION_5,
            flags: HandshakeExtensionMessageFlags(0),
            latency_ms: 120,
            own_socket_id: 1,
            peer_socket_id: 2,
            mtu: 1500,
            max_flow_window_size: 8192,
            stream_id: None,
            group: None,
            sek: Some(SecretBytes::new(sek_bytes.clone())),
            salt: Some([0x11; crate::crypto::SALT_LEN]),
        };
        for dump in [
            alloc::format!("{params:?}"),
            alloc::format!("{:#?}", HandshakeOutput::Connected(params.clone())),
            alloc::format!("{:x?}", HandshakeOutput::Connected(params.clone())),
        ] {
            for needle in needles(&sek_bytes) {
                assert!(!dump.contains(&needle), "SEK leaked as {needle:?}: {dump}");
            }
            assert!(dump.contains("redacted"), "{dump}");
        }
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string(&params).expect("serialize");
            assert!(!json.contains("sek"), "the SEK was serialized: {json}");
            for needle in needles(&sek_bytes) {
                assert!(!json.contains(&needle), "SEK leaked as {needle:?}: {json}");
            }
        }
    }

    /// Secret comparison and wiping: `==` distinguishes contents (and
    /// lengths) correctly through the constant-time path, and `zeroize`
    /// clears the key and the passphrase.
    #[cfg(feature = "crypto")]
    #[test]
    fn secrets_compare_correctly_and_wipe_on_zeroize() {
        let a = SecretBytes::new(alloc::vec![1, 2, 3, 4]);
        assert!(a == SecretBytes::new(alloc::vec![1, 2, 3, 4]));
        assert!(a != SecretBytes::new(alloc::vec![1, 2, 3, 5]));
        assert!(a != SecretBytes::new(alloc::vec![1, 2, 3]));
        assert_eq!(&*a, &[1u8, 2, 3, 4][..]);

        let mut sek = SecretBytes::new(alloc::vec![9; 16]);
        sek.zeroize();
        assert!(sek.is_empty(), "zeroize must clear the key");

        let mut config = secret_config();
        assert!(config == secret_config());
        let mut other = secret_config();
        other.sek[0] ^= 1;
        assert!(config != other);
        config.zeroize();
        assert!(config.passphrase.is_empty() && config.sek.is_empty());
        assert_eq!(config.salt, [0u8; crate::crypto::SALT_LEN]);
    }

    /// Compile-time guard: both secret holders wipe themselves on drop.
    #[cfg(feature = "crypto")]
    #[test]
    fn secret_holders_are_zeroize_on_drop() {
        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<CryptoConfig>();
        assert_zeroize_on_drop::<SecretBytes>();
    }
}
