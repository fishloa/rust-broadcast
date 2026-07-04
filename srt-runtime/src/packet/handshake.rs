//! Handshake control packet — `draft-sharabayko-srt-01` §3.2.1, Figure 5, and
//! its extension messages: Handshake Extension Message (§3.2.1.1 / Figure 6,
//! flags in §3.2.1.1.1 / Table 6), Key Material Extension (§3.2.1.2, content
//! is a [`super::KeyMaterial`] — §3.2.2), Stream ID Extension (§3.2.1.3 /
//! Figure 7), and Group Membership Extension (§3.2.1.4 / Figures 8-9).
//!
//! ```text
//! word0..0   Version (32)
//! word1      Encryption Field (16) | Extension Field (16)
//! word2      Initial Packet Sequence Number (32)
//! word3      Maximum Transmission Unit Size (32)
//! word4      Maximum Flow Window Size (32)
//! word5      Handshake Type (32)
//! word6      SRT Socket ID (32)
//! word7      SYN Cookie (32)
//! word8..11  Peer IP Address (128)
//! ── repeated ──
//!            Extension Type (16) | Extension Length (16, in 4-byte blocks)
//!            Extension Contents (Extension Length * 4 bytes)
//! ```
//!
//! This module covers *structure* only: parsing the fixed CIF fields and
//! walking/decoding extension blocks. The handshake *exchange* (caller /
//! listener / rendezvous state machine, §4.3) is an explicit follow-up.

use alloc::string::String;
use alloc::vec::Vec;

use super::key_material::KeyMaterial;
use super::{Error, Result, be16, be32, put_be16, put_be32};

/// Length in bytes of the fixed Handshake CIF core (Version through Peer IP
/// Address, §3.2.1 Figure 5) — 12 header words.
pub const HANDSHAKE_CIF_FIXED_LEN: usize = 48;

/// `Encryption Field` wire values (§3.2.1, Table 2).
pub const ENCRYPTION_FIELD_NONE: u16 = 0;
/// AES-128.
pub const ENCRYPTION_FIELD_AES_128: u16 = 2;
/// AES-192.
pub const ENCRYPTION_FIELD_AES_192: u16 = 3;
/// AES-256.
pub const ENCRYPTION_FIELD_AES_256: u16 = 4;

/// `Encryption Field`: block cipher family and key size advertised in the
/// handshake (`draft-sharabayko-srt-01` §3.2.1, Table 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EncryptionField {
    /// `0`: no encryption advertised.
    NoEncryption,
    /// `2`: AES-128 (the default).
    Aes128,
    /// `3`: AES-192.
    Aes192,
    /// `4`: AES-256.
    Aes256,
    /// A value Table 2 does not define (includes `1`).
    Reserved(u16),
}

impl EncryptionField {
    /// Decode the 16-bit `Encryption Field`.
    pub fn from_bits(v: u16) -> Self {
        match v {
            ENCRYPTION_FIELD_NONE => EncryptionField::NoEncryption,
            ENCRYPTION_FIELD_AES_128 => EncryptionField::Aes128,
            ENCRYPTION_FIELD_AES_192 => EncryptionField::Aes192,
            ENCRYPTION_FIELD_AES_256 => EncryptionField::Aes256,
            other => EncryptionField::Reserved(other),
        }
    }

    /// The wire value.
    pub fn to_bits(self) -> u16 {
        match self {
            EncryptionField::NoEncryption => ENCRYPTION_FIELD_NONE,
            EncryptionField::Aes128 => ENCRYPTION_FIELD_AES_128,
            EncryptionField::Aes192 => ENCRYPTION_FIELD_AES_192,
            EncryptionField::Aes256 => ENCRYPTION_FIELD_AES_256,
            EncryptionField::Reserved(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            EncryptionField::NoEncryption => "no encryption advertised",
            EncryptionField::Aes128 => "AES-128",
            EncryptionField::Aes192 => "AES-192",
            EncryptionField::Aes256 => "AES-256",
            EncryptionField::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(EncryptionField, Reserved);

/// `Handshake Type` wire values (§3.2.1, Table 4).
pub const HANDSHAKE_TYPE_DONE: u32 = 0xFFFF_FFFD;
/// AGREEMENT.
pub const HANDSHAKE_TYPE_AGREEMENT: u32 = 0xFFFF_FFFE;
/// CONCLUSION.
pub const HANDSHAKE_TYPE_CONCLUSION: u32 = 0xFFFF_FFFF;
/// WAVEHAND.
pub const HANDSHAKE_TYPE_WAVEHAND: u32 = 0x0000_0000;
/// INDUCTION.
pub const HANDSHAKE_TYPE_INDUCTION: u32 = 0x0000_0001;

/// `Handshake Type` (`draft-sharabayko-srt-01` §3.2.1, Table 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HandshakeType {
    /// `0xFFFFFFFD`.
    Done,
    /// `0xFFFFFFFE`.
    Agreement,
    /// `0xFFFFFFFF`.
    Conclusion,
    /// `0x00000000`.
    Wavehand,
    /// `0x00000001`.
    Induction,
    /// A value Table 4 does not define (real SRT implementations also use
    /// this range for `REJ_*` rejection-reason codes, not standardised here).
    Reserved(u32),
}

impl HandshakeType {
    /// Decode the 32-bit `Handshake Type`.
    pub fn from_bits(v: u32) -> Self {
        match v {
            HANDSHAKE_TYPE_DONE => HandshakeType::Done,
            HANDSHAKE_TYPE_AGREEMENT => HandshakeType::Agreement,
            HANDSHAKE_TYPE_CONCLUSION => HandshakeType::Conclusion,
            HANDSHAKE_TYPE_WAVEHAND => HandshakeType::Wavehand,
            HANDSHAKE_TYPE_INDUCTION => HandshakeType::Induction,
            other => HandshakeType::Reserved(other),
        }
    }

    /// The wire value.
    pub fn to_bits(self) -> u32 {
        match self {
            HandshakeType::Done => HANDSHAKE_TYPE_DONE,
            HandshakeType::Agreement => HANDSHAKE_TYPE_AGREEMENT,
            HandshakeType::Conclusion => HANDSHAKE_TYPE_CONCLUSION,
            HandshakeType::Wavehand => HANDSHAKE_TYPE_WAVEHAND,
            HandshakeType::Induction => HANDSHAKE_TYPE_INDUCTION,
            HandshakeType::Reserved(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            HandshakeType::Done => "DONE",
            HandshakeType::Agreement => "AGREEMENT",
            HandshakeType::Conclusion => "CONCLUSION",
            HandshakeType::Wavehand => "WAVEHAND",
            HandshakeType::Induction => "INDUCTION",
            HandshakeType::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(HandshakeType, Reserved);

/// `Extension Field` bitmask values (§3.2.1, Table 3). Only meaningful on a
/// CONCLUSION handshake; on an INDUCTION response this 16-bit field is
/// instead echoed back opaquely by the Listener.
pub const HS_EXT_FLAG_HSREQ: u16 = 0x0001;
/// KMREQ.
pub const HS_EXT_FLAG_KMREQ: u16 = 0x0002;
/// CONFIG.
pub const HS_EXT_FLAG_CONFIG: u16 = 0x0004;

/// The `Extension Field` (§3.2.1, Table 3) — a 16-bit bitmask on a CONCLUSION
/// handshake, or an opaque echoed value on an INDUCTION response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HandshakeExtensionFlags(pub u16);

impl HandshakeExtensionFlags {
    /// `HSREQ` bit set.
    pub fn hsreq(self) -> bool {
        self.0 & HS_EXT_FLAG_HSREQ != 0
    }
    /// `KMREQ` bit set.
    pub fn kmreq(self) -> bool {
        self.0 & HS_EXT_FLAG_KMREQ != 0
    }
    /// `CONFIG` bit set.
    pub fn config(self) -> bool {
        self.0 & HS_EXT_FLAG_CONFIG != 0
    }
}

/// `Extension Type` wire values (§3.2.1, Table 5).
pub const EXT_TYPE_HSREQ: u16 = 1;
/// `SRT_CMD_HSRSP`.
pub const EXT_TYPE_HSRSP: u16 = 2;
/// `SRT_CMD_KMREQ` — also used as the control-packet `Subtype` for a Key
/// Material message delivered as a User-Defined control packet (§3.2.2).
pub const EXT_TYPE_KMREQ: u16 = 3;
/// `SRT_CMD_KMRSP`.
pub const EXT_TYPE_KMRSP: u16 = 4;
/// `SRT_CMD_SID`.
pub const EXT_TYPE_SID: u16 = 5;
/// `SRT_CMD_CONGESTION`.
pub const EXT_TYPE_CONGESTION: u16 = 6;
/// `SRT_CMD_FILTER`.
pub const EXT_TYPE_FILTER: u16 = 7;
/// `SRT_CMD_GROUP`.
pub const EXT_TYPE_GROUP: u16 = 8;

/// Handshake Extension `Extension Type` (`draft-sharabayko-srt-01` §3.2.1,
/// Table 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ExtensionType {
    /// `1`: `SRT_CMD_HSREQ` — Handshake Extension request (§3.2.1.1).
    HsReq,
    /// `2`: `SRT_CMD_HSRSP` — Handshake Extension response (§3.2.1.1).
    HsRsp,
    /// `3`: `SRT_CMD_KMREQ` — Key Material request (§3.2.1.2).
    KmReq,
    /// `4`: `SRT_CMD_KMRSP` — Key Material response (§3.2.1.2).
    KmRsp,
    /// `5`: `SRT_CMD_SID` — Stream ID (§3.2.1.3).
    Sid,
    /// `6`: `SRT_CMD_CONGESTION`.
    Congestion,
    /// `7`: `SRT_CMD_FILTER`.
    Filter,
    /// `8`: `SRT_CMD_GROUP` — Group Membership (§3.2.1.4).
    Group,
    /// A value Table 5 does not define.
    Reserved(u16),
}

impl ExtensionType {
    /// Decode the 16-bit `Extension Type`.
    pub fn from_bits(v: u16) -> Self {
        match v {
            EXT_TYPE_HSREQ => ExtensionType::HsReq,
            EXT_TYPE_HSRSP => ExtensionType::HsRsp,
            EXT_TYPE_KMREQ => ExtensionType::KmReq,
            EXT_TYPE_KMRSP => ExtensionType::KmRsp,
            EXT_TYPE_SID => ExtensionType::Sid,
            EXT_TYPE_CONGESTION => ExtensionType::Congestion,
            EXT_TYPE_FILTER => ExtensionType::Filter,
            EXT_TYPE_GROUP => ExtensionType::Group,
            other => ExtensionType::Reserved(other),
        }
    }

    /// The wire value.
    pub fn to_bits(self) -> u16 {
        match self {
            ExtensionType::HsReq => EXT_TYPE_HSREQ,
            ExtensionType::HsRsp => EXT_TYPE_HSRSP,
            ExtensionType::KmReq => EXT_TYPE_KMREQ,
            ExtensionType::KmRsp => EXT_TYPE_KMRSP,
            ExtensionType::Sid => EXT_TYPE_SID,
            ExtensionType::Congestion => EXT_TYPE_CONGESTION,
            ExtensionType::Filter => EXT_TYPE_FILTER,
            ExtensionType::Group => EXT_TYPE_GROUP,
            ExtensionType::Reserved(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            ExtensionType::HsReq => "SRT_CMD_HSREQ",
            ExtensionType::HsRsp => "SRT_CMD_HSRSP",
            ExtensionType::KmReq => "SRT_CMD_KMREQ",
            ExtensionType::KmRsp => "SRT_CMD_KMRSP",
            ExtensionType::Sid => "SRT_CMD_SID",
            ExtensionType::Congestion => "SRT_CMD_CONGESTION",
            ExtensionType::Filter => "SRT_CMD_FILTER",
            ExtensionType::Group => "SRT_CMD_GROUP",
            ExtensionType::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(ExtensionType, Reserved);

/// `SRT Flags` bitmask values of the Handshake Extension Message (§3.2.1.1.1,
/// Table 6).
pub const HS_MSG_FLAG_TSBPDSND: u32 = 0x0000_0001;
/// TSBPDRCV.
pub const HS_MSG_FLAG_TSBPDRCV: u32 = 0x0000_0002;
/// CRYPT.
pub const HS_MSG_FLAG_CRYPT: u32 = 0x0000_0004;
/// TLPKTDROP.
pub const HS_MSG_FLAG_TLPKTDROP: u32 = 0x0000_0008;
/// PERIODICNAK.
pub const HS_MSG_FLAG_PERIODICNAK: u32 = 0x0000_0010;
/// REXMITFLG.
pub const HS_MSG_FLAG_REXMITFLG: u32 = 0x0000_0020;
/// STREAM.
pub const HS_MSG_FLAG_STREAM: u32 = 0x0000_0040;
/// PACKET_FILTER.
pub const HS_MSG_FLAG_PACKET_FILTER: u32 = 0x0000_0080;

/// `SRT Flags` of a Handshake Extension Message (§3.2.1.1.1, Table 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HandshakeExtensionMessageFlags(pub u32);

impl HandshakeExtensionMessageFlags {
    /// TSBPD used for sending.
    pub fn tsbpdsnd(self) -> bool {
        self.0 & HS_MSG_FLAG_TSBPDSND != 0
    }
    /// TSBPD used for receiving.
    pub fn tsbpdrcv(self) -> bool {
        self.0 & HS_MSG_FLAG_TSBPDRCV != 0
    }
    /// Legacy flag: peer understands the data packet `KK` field. MUST be set.
    pub fn crypt(self) -> bool {
        self.0 & HS_MSG_FLAG_CRYPT != 0
    }
    /// Too-late packet drop will be used.
    pub fn tlpktdrop(self) -> bool {
        self.0 & HS_MSG_FLAG_TLPKTDROP != 0
    }
    /// Peer will send periodic NAK packets.
    pub fn periodicnak(self) -> bool {
        self.0 & HS_MSG_FLAG_PERIODICNAK != 0
    }
    /// Legacy flag: peer understands the data packet `R` field. MUST be set.
    pub fn rexmitflg(self) -> bool {
        self.0 & HS_MSG_FLAG_REXMITFLG != 0
    }
    /// Buffer mode (`true`) vs message mode (`false`).
    pub fn stream(self) -> bool {
        self.0 & HS_MSG_FLAG_STREAM != 0
    }
    /// Peer supports packet filter.
    pub fn packet_filter(self) -> bool {
        self.0 & HS_MSG_FLAG_PACKET_FILTER != 0
    }
}

/// Handshake Extension Message contents (§3.2.1.1, Figure 6) — the payload of
/// an [`ExtensionType::HsReq`] / [`ExtensionType::HsRsp`] block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HsExtMessage {
    /// SRT library version: `major * 0x10000 + minor * 0x100 + patch`.
    pub srt_version: u32,
    /// SRT configuration flags.
    pub srt_flags: HandshakeExtensionMessageFlags,
    /// Receiver TSBPD Delay, in milliseconds.
    pub receiver_tsbpd_delay_ms: u16,
    /// Sender TSBPD Delay, in milliseconds.
    pub sender_tsbpd_delay_ms: u16,
}

/// Wire length of a Handshake Extension Message (Figure 6).
pub const HS_EXT_MESSAGE_LEN: usize = 12;

impl HsExtMessage {
    /// Parse a Handshake Extension Message from an extension block's
    /// contents (must be exactly 12 bytes).
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != HS_EXT_MESSAGE_LEN {
            return Err(Error::BufferTooShort {
                need: HS_EXT_MESSAGE_LEN,
                have: bytes.len(),
                what: "handshake extension message",
            });
        }
        Ok(HsExtMessage {
            srt_version: be32(bytes, 0),
            srt_flags: HandshakeExtensionMessageFlags(be32(bytes, 4)),
            receiver_tsbpd_delay_ms: be16(bytes, 8),
            sender_tsbpd_delay_ms: be16(bytes, 10),
        })
    }

    /// Serialize into a fresh 12-byte buffer.
    pub fn to_bytes(&self) -> [u8; HS_EXT_MESSAGE_LEN] {
        let mut buf = [0u8; HS_EXT_MESSAGE_LEN];
        put_be32(&mut buf, 0, self.srt_version);
        put_be32(&mut buf, 4, self.srt_flags.0);
        put_be16(&mut buf, 8, self.receiver_tsbpd_delay_ms);
        put_be16(&mut buf, 10, self.sender_tsbpd_delay_ms);
        buf
    }
}

/// `Type` (Group Type) wire values of the Group Membership Extension
/// (§3.2.1.4).
pub const GROUP_TYPE_UNDEFINED: u8 = 0;
/// Broadcast.
pub const GROUP_TYPE_BROADCAST: u8 = 1;
/// Main/backup.
pub const GROUP_TYPE_MAIN_BACKUP: u8 = 2;
/// Balancing.
pub const GROUP_TYPE_BALANCING: u8 = 3;
/// Multicast (reserved for future use).
pub const GROUP_TYPE_MULTICAST: u8 = 4;

/// Group Membership Extension `Type` (`draft-sharabayko-srt-01` §3.2.1.4,
/// `SRT_GTYPE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum GroupType {
    /// `0`: undefined group type.
    Undefined,
    /// `1`: broadcast group type.
    Broadcast,
    /// `2`: main/backup group type.
    MainBackup,
    /// `3`: balancing group type.
    Balancing,
    /// `4`: multicast group type (reserved for future use).
    Multicast,
    /// A value not defined above.
    Reserved(u8),
}

impl GroupType {
    /// Decode the 8-bit `Type`.
    pub fn from_bits(v: u8) -> Self {
        match v {
            GROUP_TYPE_UNDEFINED => GroupType::Undefined,
            GROUP_TYPE_BROADCAST => GroupType::Broadcast,
            GROUP_TYPE_MAIN_BACKUP => GroupType::MainBackup,
            GROUP_TYPE_BALANCING => GroupType::Balancing,
            GROUP_TYPE_MULTICAST => GroupType::Multicast,
            other => GroupType::Reserved(other),
        }
    }

    /// The wire value.
    pub fn to_bits(self) -> u8 {
        match self {
            GroupType::Undefined => GROUP_TYPE_UNDEFINED,
            GroupType::Broadcast => GROUP_TYPE_BROADCAST,
            GroupType::MainBackup => GROUP_TYPE_MAIN_BACKUP,
            GroupType::Balancing => GROUP_TYPE_BALANCING,
            GroupType::Multicast => GROUP_TYPE_MULTICAST,
            GroupType::Reserved(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            GroupType::Undefined => "undefined",
            GroupType::Broadcast => "broadcast",
            GroupType::MainBackup => "main/backup",
            GroupType::Balancing => "balancing",
            GroupType::Multicast => "multicast",
            GroupType::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(GroupType, Reserved);

/// Group Membership Extension `Flags` (§3.2.1.4, Figure 9): only the `M` bit
/// is defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GroupFlags(pub u8);

impl GroupFlags {
    const M_BIT: u8 = 0x01;

    /// `M`: synchronize on message numbers (`true`) vs sequence numbers
    /// (`false`).
    pub fn message_number_sync(self) -> bool {
        self.0 & Self::M_BIT != 0
    }
}

/// Group Membership Extension (§3.2.1.4, Figure 8) — the payload of an
/// [`ExtensionType::Group`] block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GroupMembershipExtension {
    /// Group ID.
    pub group_id: u32,
    /// Group Type.
    pub group_type: GroupType,
    /// Flags.
    pub flags: GroupFlags,
    /// Link priority (main/backup) or otherwise reserved.
    pub weight: u16,
}

/// Wire length of a Group Membership Extension (Figure 8).
pub const GROUP_MEMBERSHIP_EXT_LEN: usize = 8;

impl GroupMembershipExtension {
    /// Parse from an extension block's contents (must be exactly 8 bytes).
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != GROUP_MEMBERSHIP_EXT_LEN {
            return Err(Error::BufferTooShort {
                need: GROUP_MEMBERSHIP_EXT_LEN,
                have: bytes.len(),
                what: "group membership extension",
            });
        }
        let group_id = be32(bytes, 0);
        let word1 = be32(bytes, 4);
        Ok(GroupMembershipExtension {
            group_id,
            group_type: GroupType::from_bits((word1 >> 24) as u8),
            flags: GroupFlags((word1 >> 16) as u8),
            weight: (word1 & 0xFFFF) as u16,
        })
    }

    /// Serialize into a fresh 8-byte buffer.
    pub fn to_bytes(&self) -> [u8; GROUP_MEMBERSHIP_EXT_LEN] {
        let mut buf = [0u8; GROUP_MEMBERSHIP_EXT_LEN];
        let word1 = (u32::from(self.group_type.to_bits()) << 24)
            | (u32::from(self.flags.0) << 16)
            | u32::from(self.weight);
        put_be32(&mut buf, 0, self.group_id);
        put_be32(&mut buf, 4, word1);
        buf
    }
}

/// One decoded Handshake Extension block (§3.2.1): `Extension Type` /
/// `Extension Length` / `Extension Contents`. Data-carrying (borrows the raw
/// contents) — decode the contents with [`Self::as_hs_ext_message`],
/// [`Self::as_key_material`], [`Self::as_stream_id`], or
/// [`Self::as_group_membership`] as appropriate for [`Self::ext_type`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HandshakeExtensionBlock<'a> {
    /// The extension's type.
    pub ext_type: ExtensionType,
    /// The raw contents (`Extension Length * 4` bytes).
    pub contents: &'a [u8],
}

impl<'a> HandshakeExtensionBlock<'a> {
    /// Decode [`Self::contents`] as a Handshake Extension Message (§3.2.1.1)
    /// — valid for [`ExtensionType::HsReq`] / [`ExtensionType::HsRsp`].
    pub fn as_hs_ext_message(&self) -> Result<HsExtMessage> {
        HsExtMessage::parse(self.contents)
    }

    /// Decode [`Self::contents`] as a Key Material message (§3.2.2) — valid
    /// for [`ExtensionType::KmReq`] / [`ExtensionType::KmRsp`].
    pub fn as_key_material(&self) -> Result<KeyMaterial<'a>> {
        KeyMaterial::parse(self.contents)
    }

    /// Decode [`Self::contents`] as the Stream ID extension (§3.2.1.3) —
    /// valid for [`ExtensionType::Sid`].
    ///
    /// Per §3.2.1.3 "The content is stored as 32-bit little endian words":
    /// each 4-byte word of `contents` is byte-reversed before concatenation,
    /// trailing NUL padding is trimmed, and the result is validated as UTF-8.
    pub fn as_stream_id(&self) -> Result<String> {
        if self.contents.len() % 4 != 0 {
            return Err(Error::BufferTooShort {
                need: self.contents.len().div_ceil(4) * 4,
                have: self.contents.len(),
                what: "stream ID extension (not a whole number of 4-byte words)",
            });
        }
        let mut bytes = Vec::with_capacity(self.contents.len());
        for word in self.contents.chunks_exact(4) {
            bytes.extend(word.iter().rev());
        }
        while bytes.last() == Some(&0u8) {
            bytes.pop();
        }
        String::from_utf8(bytes).map_err(|_| Error::InvalidStreamIdUtf8)
    }

    /// Decode [`Self::contents`] as a Group Membership Extension (§3.2.1.4) —
    /// valid for [`ExtensionType::Group`].
    pub fn as_group_membership(&self) -> Result<GroupMembershipExtension> {
        GroupMembershipExtension::parse(self.contents)
    }
}

/// Encode the Stream ID extension's 32-bit-little-endian-word storage
/// (§3.2.1.3) from a plain UTF-8 stream id string, padding with `0x00` up to
/// the next 4-byte boundary. Inverse of [`HandshakeExtensionBlock::as_stream_id`].
pub fn encode_stream_id(id: &str) -> Vec<u8> {
    let mut bytes = Vec::from(id.as_bytes());
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    let mut out = Vec::with_capacity(bytes.len());
    for word in bytes.chunks_exact(4) {
        out.extend(word.iter().rev());
    }
    out
}

/// Build one Handshake Extension block's raw bytes (`Extension Type` +
/// `Extension Length` + `Extension Contents`, §3.2.1) — append the result of
/// repeated calls to chain multiple blocks.
///
/// # Errors
/// [`Error::FieldTooWide`] if `contents` is not a whole number of 4-byte
/// words, or has more than `0xFFFF` such words.
pub fn build_extension_block(ext_type: ExtensionType, contents: &[u8]) -> Result<Vec<u8>> {
    if contents.len() % 4 != 0 {
        return Err(Error::InvalidField {
            what: "Extension Contents",
            reason: "length must be a whole number of 4-byte words",
        });
    }
    let words = contents.len() / 4;
    let words_u16 = u16::try_from(words).map_err(|_| Error::FieldTooWide {
        what: "Extension Length",
        value: words as u64,
        bits: 16,
    })?;
    let mut out = Vec::with_capacity(4 + contents.len());
    out.extend_from_slice(&ext_type.to_bits().to_be_bytes());
    out.extend_from_slice(&words_u16.to_be_bytes());
    out.extend_from_slice(contents);
    Ok(out)
}

/// A borrowed, lazily-walked list of Handshake Extension blocks — the same
/// convention `dvb-si` uses for descriptor loops. Iterate with [`Self::iter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HandshakeExtensions<'a>(pub &'a [u8]);

impl<'a> HandshakeExtensions<'a> {
    /// Walk the extension blocks.
    pub fn iter(&self) -> HandshakeExtensionIter<'a> {
        HandshakeExtensionIter { rest: self.0 }
    }
}

/// Iterator over [`HandshakeExtensions`]. See [`HandshakeExtensions::iter`].
#[derive(Debug, Clone)]
pub struct HandshakeExtensionIter<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for HandshakeExtensionIter<'a> {
    type Item = Result<HandshakeExtensionBlock<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        if self.rest.len() < 4 {
            self.rest = &[];
            return Some(Err(Error::BufferTooShort {
                need: 4,
                have: self.rest.len(),
                what: "handshake extension block header",
            }));
        }
        let ext_type_bits = be16(self.rest, 0);
        let ext_len_words = be16(self.rest, 2);
        let ext_len_bytes = usize::from(ext_len_words) * 4;
        if self.rest.len() < 4 + ext_len_bytes {
            let remaining = self.rest.len() - 4;
            self.rest = &[];
            return Some(Err(Error::ExtensionOverrun {
                declared: ext_len_words,
                remaining,
            }));
        }
        let contents = &self.rest[4..4 + ext_len_bytes];
        self.rest = &self.rest[4 + ext_len_bytes..];
        Some(Ok(HandshakeExtensionBlock {
            ext_type: ExtensionType::from_bits(ext_type_bits),
            contents,
        }))
    }
}

/// Handshake control packet (§3.2.1, Figure 5).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HandshakePacket<'a> {
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// A base protocol version number (`4` or `5`; `>5` reserved).
    pub version: u32,
    /// Block cipher family and key size (Table 2).
    pub encryption_field: EncryptionField,
    /// Message-specific extension flags/echo (Table 3, or opaque on
    /// INDUCTION).
    pub extension_field: HandshakeExtensionFlags,
    /// The sequence number of the very first data packet to be sent.
    pub initial_seq_number: u32,
    /// Typically `1500` (Ethernet MTU) or less.
    pub mtu: u32,
    /// Maximum number of data packets allowed in flight.
    pub max_flow_window_size: u32,
    /// The handshake packet type (Table 4).
    pub handshake_type: HandshakeType,
    /// The ID of the source SRT socket issuing this handshake packet.
    pub srt_socket_id: u32,
    /// Randomized value for processing the handshake.
    pub syn_cookie: u32,
    /// IPv4 or IPv6 address of the packet's sender, as 4 wire words (IPv4:
    /// only the first word is non-zero).
    pub peer_ip: [u32; 4],
    /// The trailing Handshake Extension blocks (possibly empty).
    pub extensions: HandshakeExtensions<'a>,
}

impl<'a> HandshakePacket<'a> {
    pub(crate) fn parse_cif(timestamp: u32, dest_socket_id: u32, cif: &'a [u8]) -> Result<Self> {
        if cif.len() < HANDSHAKE_CIF_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: HANDSHAKE_CIF_FIXED_LEN,
                have: cif.len(),
                what: "handshake CIF",
            });
        }
        let version = be32(cif, 0);
        let word1 = be32(cif, 4);
        let encryption_field = EncryptionField::from_bits((word1 >> 16) as u16);
        let extension_field = HandshakeExtensionFlags((word1 & 0xFFFF) as u16);
        let initial_seq_number = be32(cif, 8);
        let mtu = be32(cif, 12);
        let max_flow_window_size = be32(cif, 16);
        let handshake_type = HandshakeType::from_bits(be32(cif, 20));
        let srt_socket_id = be32(cif, 24);
        let syn_cookie = be32(cif, 28);
        let peer_ip = [be32(cif, 32), be32(cif, 36), be32(cif, 40), be32(cif, 44)];
        let extensions = HandshakeExtensions(&cif[HANDSHAKE_CIF_FIXED_LEN..]);
        Ok(HandshakePacket {
            timestamp,
            dest_socket_id,
            version,
            encryption_field,
            extension_field,
            initial_seq_number,
            mtu,
            max_flow_window_size,
            handshake_type,
            srt_socket_id,
            syn_cookie,
            peer_ip,
            extensions,
        })
    }

    pub(crate) fn cif_len(&self) -> usize {
        HANDSHAKE_CIF_FIXED_LEN + self.extensions.0.len()
    }

    pub(crate) fn write_cif(&self, buf: &mut [u8]) -> Result<()> {
        let word1 =
            (u32::from(self.encryption_field.to_bits()) << 16) | u32::from(self.extension_field.0);
        put_be32(buf, 0, self.version);
        put_be32(buf, 4, word1);
        put_be32(buf, 8, self.initial_seq_number);
        put_be32(buf, 12, self.mtu);
        put_be32(buf, 16, self.max_flow_window_size);
        put_be32(buf, 20, self.handshake_type.to_bits());
        put_be32(buf, 24, self.srt_socket_id);
        put_be32(buf, 28, self.syn_cookie);
        put_be32(buf, 32, self.peer_ip[0]);
        put_be32(buf, 36, self.peer_ip[1]);
        put_be32(buf, 40, self.peer_ip[2]);
        put_be32(buf, 44, self.peer_ip[3]);
        buf[HANDSHAKE_CIF_FIXED_LEN..].copy_from_slice(self.extensions.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::control::ControlPacket;
    use super::*;

    fn sample_no_ext() -> HandshakePacket<'static> {
        HandshakePacket {
            timestamp: 111,
            dest_socket_id: 222,
            version: 5,
            encryption_field: EncryptionField::Aes128,
            extension_field: HandshakeExtensionFlags(0),
            initial_seq_number: 1000,
            mtu: 1500,
            max_flow_window_size: 8192,
            handshake_type: HandshakeType::Induction,
            srt_socket_id: 0xABCD_EF01,
            syn_cookie: 0x1234_5678,
            peer_ip: [0x0A00_0001, 0, 0, 0],
            extensions: HandshakeExtensions(&[]),
        }
    }

    #[test]
    fn round_trips_hand_computed_bytes_no_extensions() {
        let pkt = ControlPacket::Handshake(sample_no_ext());
        let mut buf = [0u8; 16 + HANDSHAKE_CIF_FIXED_LEN];
        let n = pkt.serialize_into(&mut buf).unwrap();
        assert_eq!(n, buf.len());
        assert_eq!(&buf[0..4], &0x8000_0000u32.to_be_bytes()); // control type 0
        assert_eq!(&buf[16..20], &5u32.to_be_bytes()); // Version
        // Extension Field is 0 (no extensions on this sample).
        let expected_word1 = u32::from(ENCRYPTION_FIELD_AES_128) << 16;
        assert_eq!(&buf[20..24], &expected_word1.to_be_bytes());
        assert_eq!(&buf[36..40], &HANDSHAKE_TYPE_INDUCTION.to_be_bytes());
        assert_eq!(ControlPacket::parse(&buf).unwrap(), pkt);
    }

    #[test]
    fn round_trips_with_hsreq_and_sid_extensions() {
        let hs_msg = HsExtMessage {
            srt_version: 0x0105_0000,
            srt_flags: HandshakeExtensionMessageFlags(
                HS_MSG_FLAG_TSBPDSND | HS_MSG_FLAG_TSBPDRCV | HS_MSG_FLAG_CRYPT,
            ),
            receiver_tsbpd_delay_ms: 120,
            sender_tsbpd_delay_ms: 120,
        };
        let hsreq_block = build_extension_block(ExtensionType::HsReq, &hs_msg.to_bytes()).unwrap();

        let sid_contents = encode_stream_id("live/stream1");
        let sid_block = build_extension_block(ExtensionType::Sid, &sid_contents).unwrap();

        let mut ext_bytes = Vec::new();
        ext_bytes.extend_from_slice(&hsreq_block);
        ext_bytes.extend_from_slice(&sid_block);

        let mut hp = sample_no_ext();
        hp.handshake_type = HandshakeType::Conclusion;
        hp.extension_field = HandshakeExtensionFlags(HS_EXT_FLAG_HSREQ);
        hp.extensions = HandshakeExtensions(&ext_bytes);

        let pkt = ControlPacket::Handshake(hp.clone());
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).unwrap();
        let parsed = ControlPacket::parse(&buf).unwrap();
        assert_eq!(parsed, pkt);

        if let ControlPacket::Handshake(h) = parsed {
            let blocks: Vec<_> = h.extensions.iter().map(|b| b.unwrap()).collect();
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0].ext_type, ExtensionType::HsReq);
            assert_eq!(blocks[0].as_hs_ext_message().unwrap(), hs_msg);
            assert_eq!(blocks[1].ext_type, ExtensionType::Sid);
            assert_eq!(blocks[1].as_stream_id().unwrap(), "live/stream1");
        } else {
            panic!("expected handshake");
        }
    }

    #[test]
    fn stream_id_padding_is_trimmed() {
        // "abc" is not a multiple of 4 bytes; encode_stream_id pads with a
        // NUL, and as_stream_id must trim it back off.
        let contents = encode_stream_id("abc");
        assert_eq!(contents.len(), 4);
        let block = HandshakeExtensionBlock {
            ext_type: ExtensionType::Sid,
            contents: &contents,
        };
        assert_eq!(block.as_stream_id().unwrap(), "abc");
    }

    #[test]
    fn group_membership_extension_round_trips() {
        let g = GroupMembershipExtension {
            group_id: 42,
            group_type: GroupType::MainBackup,
            flags: GroupFlags(0x01),
            weight: 7,
        };
        let bytes = g.to_bytes();
        assert_eq!(GroupMembershipExtension::parse(&bytes).unwrap(), g);
        assert!(g.flags.message_number_sync());
    }

    #[test]
    fn extension_overrun_does_not_panic() {
        // Declares 0xFFFF words (huge) but supplies none.
        let bytes = [0x00, 0x05, 0xFF, 0xFF];
        let exts = HandshakeExtensions(&bytes);
        let mut it = exts.iter();
        assert!(matches!(
            it.next(),
            Some(Err(Error::ExtensionOverrun { .. }))
        ));
        assert!(it.next().is_none());
    }

    #[test]
    fn all_encryption_fields_and_types_round_trip() {
        for e in [
            EncryptionField::NoEncryption,
            EncryptionField::Aes128,
            EncryptionField::Aes192,
            EncryptionField::Aes256,
        ] {
            assert_eq!(EncryptionField::from_bits(e.to_bits()), e);
        }
        for t in [
            HandshakeType::Done,
            HandshakeType::Agreement,
            HandshakeType::Conclusion,
            HandshakeType::Wavehand,
            HandshakeType::Induction,
        ] {
            assert_eq!(HandshakeType::from_bits(t.to_bits()), t);
        }
    }
}
