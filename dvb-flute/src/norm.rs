//! NORM — NACK-Oriented Reliable Multicast common header + message types
//! (RFC 5740 §4).
//!
//! NORM uses its **own** common message header (not the LCT header), but borrows
//! the same HET/HEL header-extension convention ([`crate::ext`]) and the
//! FEC-Payload-ID concept. This module models the 8-byte common header, the
//! `type` registry, the shared sender word, and the fixed-header portions of
//! every defined message type. Variable trailing regions whose length the spec
//! infers from the datagram length (FEC Payload IDs, node lists, NACK content,
//! payloads) are exposed as opaque byte slices for the caller to interpret with
//! knowledge of the FEC scheme.
//!
//! ⚠ FEC Payload ID layouts are FEC-scheme dependent (RFC 5740 §4.2.1 only
//! reproduces one example, `fec_id` = 129); they are opaque here. NORM_REPORT
//! has no defined wire format (RFC 5740 §4.4.1) — exposed as opaque content.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::ext::{self, HeaderExtension, WORD};

/// NORM protocol version (RFC 5740 = 1).
pub const NORM_VERSION: u8 = 1;
/// Size of the NORM common message header in bytes.
pub const COMMON_HEADER_LEN: usize = 8;
/// Size of the shared sender word (instance_id/grtt/backoff/gsize) in bytes.
pub const SENDER_WORD_LEN: usize = 4;

/// Reserved NormNodeId: invalid / none.
pub const NORM_NODE_NONE: u32 = 0x0000_0000;
/// Reserved NormNodeId: wildcard / any.
pub const NORM_NODE_ANY: u32 = 0xFFFF_FFFF;

/// HET for NORM EXT_AUTH (variable-length) — RFC 5740 §8.5.
pub const HET_EXT_AUTH: u8 = 1;
/// HET for NORM EXT_CC (variable-length, hel = 3) — RFC 5740 §4.2.3.
pub const HET_EXT_CC: u8 = 3;
/// HET for NORM EXT_FTI (variable-length) — RFC 5740 §4.2.1.
pub const HET_EXT_FTI: u8 = 64;
/// HET for NORM EXT_RATE (fixed-length) — RFC 5740 §4.2.3.
pub const HET_EXT_RATE: u8 = 128;

/// NORM message `type` (RFC 5740 §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum NormMessageType {
    /// NORM_INFO (1).
    Info,
    /// NORM_DATA (2).
    Data,
    /// NORM_CMD (3).
    Cmd,
    /// NORM_NACK (4).
    Nack,
    /// NORM_ACK (5).
    Ack,
    /// NORM_REPORT (6).
    Report,
    /// Any other (unassigned) 4-bit type value.
    Other(u8),
}

impl NormMessageType {
    /// Decode a 4-bit type value.
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => NormMessageType::Info,
            2 => NormMessageType::Data,
            3 => NormMessageType::Cmd,
            4 => NormMessageType::Nack,
            5 => NormMessageType::Ack,
            6 => NormMessageType::Report,
            other => NormMessageType::Other(other),
        }
    }

    /// The 4-bit wire value.
    pub fn to_u8(self) -> u8 {
        match self {
            NormMessageType::Info => 1,
            NormMessageType::Data => 2,
            NormMessageType::Cmd => 3,
            NormMessageType::Nack => 4,
            NormMessageType::Ack => 5,
            NormMessageType::Report => 6,
            NormMessageType::Other(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            NormMessageType::Info => "NORM_INFO",
            NormMessageType::Data => "NORM_DATA",
            NormMessageType::Cmd => "NORM_CMD",
            NormMessageType::Nack => "NORM_NACK",
            NormMessageType::Ack => "NORM_ACK",
            NormMessageType::Report => "NORM_REPORT",
            NormMessageType::Other(_) => "reserved",
        }
    }
}

dvb_common::impl_spec_display!(NormMessageType, Other);

/// The NORM common message header (RFC 5740 §4.1, Figure 1): 8 bytes carrying
/// `version | type | hdr_len | sequence | source_id`.
///
/// `hdr_len` is **not** stored: it is recomputed on serialize from the typed
/// message body so the round-trip is field-driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NormCommonHeader {
    /// Protocol version (`version`, 4 bits). RFC 5740 = [`NORM_VERSION`] (1).
    pub version: u8,
    /// Message type (`type`, 4 bits).
    pub message_type: NormMessageType,
    /// Sequence number (16 bits).
    pub sequence: u16,
    /// Originator's NormNodeId (`source_id`, 32 bits).
    pub source_id: u32,
}

impl NormCommonHeader {
    /// Parse the 8-byte common header. Returns the header and the `hdr_len`
    /// value (in 32-bit words) read from the wire.
    pub fn parse(data: &[u8]) -> Result<(Self, u8)> {
        if data.len() < COMMON_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: COMMON_HEADER_LEN,
                have: data.len(),
                what: "NORM common header",
            });
        }
        let version = data[0] >> 4;
        let message_type = NormMessageType::from_u8(data[0] & 0x0F);
        let hdr_len = data[1];
        let sequence = u16::from_be_bytes([data[2], data[3]]);
        let source_id = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        Ok((
            NormCommonHeader {
                version,
                message_type,
                sequence,
                source_id,
            },
            hdr_len,
        ))
    }

    /// Serialize the common header into `out`, writing the supplied `hdr_len`
    /// (caller computes it from the message body). Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8], hdr_len: u8) -> Result<usize> {
        if out.len() < COMMON_HEADER_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: COMMON_HEADER_LEN,
                have: out.len(),
            });
        }
        if self.version > 0x0F {
            return Err(Error::FieldTooWide {
                what: "version",
                value: self.version as u64,
                bits: 4,
            });
        }
        let ty = self.message_type.to_u8();
        if ty > 0x0F {
            return Err(Error::FieldTooWide {
                what: "type",
                value: ty as u64,
                bits: 4,
            });
        }
        out[0] = (self.version << 4) | (ty & 0x0F);
        out[1] = hdr_len;
        out[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        out[4..8].copy_from_slice(&self.source_id.to_be_bytes());
        Ok(COMMON_HEADER_LEN)
    }
}

/// The shared sender word carried by NORM_DATA / NORM_INFO / NORM_CMD
/// (RFC 5740 §4.2): `instance_id(16) | grtt(8) | backoff(4) | gsize(4)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SenderWord {
    /// Sender's current participation instance.
    pub instance_id: u16,
    /// Quantized group RTT estimate.
    pub grtt: u8,
    /// NACK backoff factor (4 bits).
    pub backoff: u8,
    /// Quantized group-size estimate (4 bits).
    pub gsize: u8,
}

impl SenderWord {
    /// Parse the 4-byte sender word.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < SENDER_WORD_LEN {
            return Err(Error::BufferTooShort {
                need: SENDER_WORD_LEN,
                have: data.len(),
                what: "NORM sender word",
            });
        }
        Ok(SenderWord {
            instance_id: u16::from_be_bytes([data[0], data[1]]),
            grtt: data[2],
            backoff: data[3] >> 4,
            gsize: data[3] & 0x0F,
        })
    }

    /// Serialize the 4-byte sender word into `out`.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        if out.len() < SENDER_WORD_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: SENDER_WORD_LEN,
                have: out.len(),
            });
        }
        if self.backoff > 0x0F {
            return Err(Error::FieldTooWide {
                what: "backoff",
                value: self.backoff as u64,
                bits: 4,
            });
        }
        if self.gsize > 0x0F {
            return Err(Error::FieldTooWide {
                what: "gsize",
                value: self.gsize as u64,
                bits: 4,
            });
        }
        out[0..2].copy_from_slice(&self.instance_id.to_be_bytes());
        out[2] = self.grtt;
        out[3] = (self.backoff << 4) | (self.gsize & 0x0F);
        Ok(SENDER_WORD_LEN)
    }
}

/// NORM_INFO fixed header beyond the common header + sender word (RFC 5740
/// §4.2.2, Figure 8): `flags | fec_id | object_transport_id`.
///
/// NORM_INFO is the **atomic** out-of-band context message for one object.
/// Unlike NORM_DATA it carries **no** `fec_payload_id` field — the fixed part
/// of the header is exactly 4 words (common 8 + sender 4 + flags-word 4 = 16
/// bytes = hdr_len 4 when there are no extensions). The payload is the
/// application-defined info content (≤ NormSegmentSize).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NormInfo<'a> {
    /// Common header (message_type = `NormMessageType::Info`).
    pub common: NormCommonHeader,
    /// Shared sender word.
    pub sender: SenderWord,
    /// Object flags (NORM_FLAG_*). Same set as NORM_DATA.
    pub flags: u8,
    /// FEC Encoding ID.
    pub fec_id: u8,
    /// NormTransportId of the object this INFO is associated with.
    pub object_transport_id: u16,
    /// Header-extension chain (e.g. EXT_FTI per §4.2.1 Figure 6).
    pub extensions: Vec<HeaderExtension<'a>>,
    /// Application-defined content (≤ NormSegmentSize). NOT part of `hdr_len`.
    pub payload: &'a [u8],
}

/// Fixed header size of NORM_INFO before header extensions:
/// common(8) + sender(4) + flags-word(4) = 16 bytes.
pub const NORM_INFO_FIXED_LEN: usize = COMMON_HEADER_LEN + SENDER_WORD_LEN + WORD;

impl<'a> NormInfo<'a> {
    /// Total header bytes (common + sender + flags-word + extensions).
    fn header_bytes(&self) -> usize {
        NORM_INFO_FIXED_LEN + ext::chain_len(&self.extensions)
    }

    /// Total serialized length in bytes.
    pub fn serialized_len(&self) -> usize {
        self.header_bytes() + self.payload.len()
    }

    /// Parse a NORM_INFO message.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let (common, hdr_len) = NormCommonHeader::parse(data)?;
        let sender = SenderWord::parse(&data[COMMON_HEADER_LEN..])?;
        let off = COMMON_HEADER_LEN + SENDER_WORD_LEN;
        if data.len() < off + WORD {
            return Err(Error::BufferTooShort {
                need: off + WORD,
                have: data.len(),
                what: "NORM_INFO flags word",
            });
        }
        let flags = data[off];
        let fec_id = data[off + 1];
        let object_transport_id = u16::from_be_bytes([data[off + 2], data[off + 3]]);

        // hdr_len bounds the header (incl. extensions); payload starts after.
        let header_end = hdr_len as usize * WORD;
        if header_end < NORM_INFO_FIXED_LEN {
            return Err(Error::InconsistentLength {
                length: hdr_len,
                reason: "hdr_len smaller than the NORM_INFO fixed header",
            });
        }
        if data.len() < header_end {
            return Err(Error::BufferTooShort {
                need: header_end,
                have: data.len(),
                what: "NORM_INFO header (per hdr_len)",
            });
        }
        let extensions = ext::parse_chain(&data[NORM_INFO_FIXED_LEN..header_end])?;
        let payload = &data[header_end..];

        Ok(NormInfo {
            common,
            sender,
            flags,
            fec_id,
            object_transport_id,
            extensions,
            payload,
        })
    }

    /// Serialize into `out`, recomputing `hdr_len`. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        let header_bytes = self.header_bytes();
        if header_bytes % WORD != 0 {
            return Err(Error::InvalidField {
                what: "hdr_len",
                reason: "NORM_INFO header length is not a multiple of 4 bytes",
            });
        }
        let words = header_bytes / WORD;
        if words > u8::MAX as usize {
            return Err(Error::FieldTooWide {
                what: "hdr_len",
                value: words as u64,
                bits: 8,
            });
        }
        let mut off = self.common.serialize_into(out, words as u8)?;
        off += self.sender.serialize_into(&mut out[off..])?;
        out[off] = self.flags;
        out[off + 1] = self.fec_id;
        out[off + 2..off + 4].copy_from_slice(&self.object_transport_id.to_be_bytes());
        off += WORD;
        off += ext::serialize_chain(&self.extensions, &mut out[off..])?;
        out[off..off + self.payload.len()].copy_from_slice(self.payload);
        off += self.payload.len();
        Ok(off)
    }
}

// NORM_DATA `flags` bits (RFC 5740 §4.2.1).
/// Message is a repair transmission.
pub const NORM_FLAG_REPAIR: u8 = 0x01;
/// Repair segment meeting a specific erasure.
pub const NORM_FLAG_EXPLICIT: u8 = 0x02;
/// NORM_INFO is available for this object.
pub const NORM_FLAG_INFO: u8 = 0x04;
/// No repair will be supplied (one-shot best-effort).
pub const NORM_FLAG_UNRELIABLE: u8 = 0x08;
/// Object is file-based.
pub const NORM_FLAG_FILE: u8 = 0x10;
/// Object is a NORM_OBJECT_STREAM (enables the payload_* fields).
pub const NORM_FLAG_STREAM: u8 = 0x20;

/// NORM_DATA fixed header beyond the common header + sender word (RFC 5740
/// §4.2.1, Figure 4): `flags | fec_id | object_transport_id | fec_payload_id`.
///
/// The `fec_payload_id` is opaque (size per `fec_id`). STREAM-only
/// `payload_len`/`payload_msg_start`/`payload_offset` fields and the payload
/// data are part of `payload` (they do not contribute to `hdr_len`); the caller
/// interprets them when `NORM_FLAG_STREAM` is set.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NormData<'a> {
    /// Common header.
    pub common: NormCommonHeader,
    /// Shared sender word.
    pub sender: SenderWord,
    /// Object flags (NORM_FLAG_*).
    pub flags: u8,
    /// FEC Encoding ID (implies `fec_payload_id` size/format).
    pub fec_id: u8,
    /// Monotonic NormTransportId of the object.
    pub object_transport_id: u16,
    /// FEC coding-block + symbol identifier (opaque, size per `fec_id`).
    pub fec_payload_id: &'a [u8],
    /// Header-extension chain.
    pub extensions: Vec<HeaderExtension<'a>>,
    /// Source/parity content (and, for STREAM, the leading payload_* fields).
    pub payload: &'a [u8],
}

impl<'a> NormData<'a> {
    /// `hdr_len` (32-bit words) = common(2) + sender(1) + the 4-byte
    /// flags/fec_id/object_transport_id word + fec_payload_id + extensions.
    fn header_bytes(&self) -> usize {
        COMMON_HEADER_LEN
            + SENDER_WORD_LEN
            + WORD
            + self.fec_payload_id.len()
            + ext::chain_len(&self.extensions)
    }

    /// Total serialized length in bytes.
    pub fn serialized_len(&self) -> usize {
        self.header_bytes() + self.payload.len()
    }

    /// Parse a NORM_DATA. `fec_payload_id_len` is the FEC-scheme-defined size of
    /// the FEC Payload ID in bytes.
    pub fn parse(data: &'a [u8], fec_payload_id_len: usize) -> Result<Self> {
        let (common, hdr_len) = NormCommonHeader::parse(data)?;
        let sender = SenderWord::parse(&data[COMMON_HEADER_LEN..])?;
        let mut off = COMMON_HEADER_LEN + SENDER_WORD_LEN;
        if data.len() < off + WORD {
            return Err(Error::BufferTooShort {
                need: off + WORD,
                have: data.len(),
                what: "NORM_DATA flags word",
            });
        }
        let flags = data[off];
        let fec_id = data[off + 1];
        let object_transport_id = u16::from_be_bytes([data[off + 2], data[off + 3]]);
        off += WORD;

        if data.len() < off + fec_payload_id_len {
            return Err(Error::BufferTooShort {
                need: off + fec_payload_id_len,
                have: data.len(),
                what: "NORM_DATA fec_payload_id",
            });
        }
        let fec_payload_id = &data[off..off + fec_payload_id_len];
        off += fec_payload_id_len;

        // hdr_len bounds the header (incl. extensions). Everything from off..
        // header_end is the extension chain; the rest is payload.
        let header_end = hdr_len as usize * WORD;
        if header_end < off {
            return Err(Error::InconsistentLength {
                length: hdr_len,
                reason: "hdr_len smaller than the fixed NORM_DATA header + fec_payload_id",
            });
        }
        if data.len() < header_end {
            return Err(Error::BufferTooShort {
                need: header_end,
                have: data.len(),
                what: "NORM_DATA header (per hdr_len)",
            });
        }
        let extensions = ext::parse_chain(&data[off..header_end])?;
        let payload = &data[header_end..];

        Ok(NormData {
            common,
            sender,
            flags,
            fec_id,
            object_transport_id,
            fec_payload_id,
            extensions,
            payload,
        })
    }

    /// Serialize into `out`, recomputing `hdr_len`. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        let header_bytes = self.header_bytes();
        if header_bytes % WORD != 0 {
            return Err(Error::InvalidField {
                what: "hdr_len",
                reason: "NORM_DATA header length is not a multiple of 4 bytes",
            });
        }
        let words = header_bytes / WORD;
        if words > u8::MAX as usize {
            return Err(Error::FieldTooWide {
                what: "hdr_len",
                value: words as u64,
                bits: 8,
            });
        }
        let mut off = self.common.serialize_into(out, words as u8)?;
        off += self.sender.serialize_into(&mut out[off..])?;
        out[off] = self.flags;
        out[off + 1] = self.fec_id;
        out[off + 2..off + 4].copy_from_slice(&self.object_transport_id.to_be_bytes());
        off += WORD;
        out[off..off + self.fec_payload_id.len()].copy_from_slice(self.fec_payload_id);
        off += self.fec_payload_id.len();
        off += ext::serialize_chain(&self.extensions, &mut out[off..])?;
        out[off..off + self.payload.len()].copy_from_slice(self.payload);
        off += self.payload.len();
        Ok(off)
    }
}

/// NORM_CMD sub-type (RFC 5740 §4.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum NormCmdType {
    /// NORM_CMD(FLUSH) (1).
    Flush,
    /// NORM_CMD(EOT) (2).
    Eot,
    /// NORM_CMD(SQUELCH) (3).
    Squelch,
    /// NORM_CMD(CC) (4).
    Cc,
    /// NORM_CMD(REPAIR_ADV) (5).
    RepairAdv,
    /// NORM_CMD(ACK_REQ) (6).
    AckReq,
    /// NORM_CMD(APPLICATION) (7).
    Application,
    /// Any other sub-type value.
    Other(u8),
}

impl NormCmdType {
    /// Decode a sub-type byte.
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => NormCmdType::Flush,
            2 => NormCmdType::Eot,
            3 => NormCmdType::Squelch,
            4 => NormCmdType::Cc,
            5 => NormCmdType::RepairAdv,
            6 => NormCmdType::AckReq,
            7 => NormCmdType::Application,
            other => NormCmdType::Other(other),
        }
    }

    /// The sub-type wire byte.
    pub fn to_u8(self) -> u8 {
        match self {
            NormCmdType::Flush => 1,
            NormCmdType::Eot => 2,
            NormCmdType::Squelch => 3,
            NormCmdType::Cc => 4,
            NormCmdType::RepairAdv => 5,
            NormCmdType::AckReq => 6,
            NormCmdType::Application => 7,
            NormCmdType::Other(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            NormCmdType::Flush => "NORM_CMD(FLUSH)",
            NormCmdType::Eot => "NORM_CMD(EOT)",
            NormCmdType::Squelch => "NORM_CMD(SQUELCH)",
            NormCmdType::Cc => "NORM_CMD(CC)",
            NormCmdType::RepairAdv => "NORM_CMD(REPAIR_ADV)",
            NormCmdType::AckReq => "NORM_CMD(ACK_REQ)",
            NormCmdType::Application => "NORM_CMD(APPLICATION)",
            NormCmdType::Other(_) => "reserved",
        }
    }
}

dvb_common::impl_spec_display!(NormCmdType, Other);

/// NORM ack_type (RFC 5740 §4.2.3, shared by NORM_CMD(ACK_REQ) and NORM_ACK).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum NormAckType {
    /// NORM_ACK(CC) (1).
    Cc,
    /// NORM_ACK(FLUSH) (2).
    Flush,
    /// Reserved for future NORM use (3..=15).
    Reserved(u8),
    /// Application discretion (16..=255).
    Application(u8),
}

impl NormAckType {
    /// Decode an ack_type byte.
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => NormAckType::Cc,
            2 => NormAckType::Flush,
            3..=15 => NormAckType::Reserved(v),
            _ => NormAckType::Application(v),
        }
    }

    /// The wire byte.
    pub fn to_u8(self) -> u8 {
        match self {
            NormAckType::Cc => 1,
            NormAckType::Flush => 2,
            NormAckType::Reserved(v) | NormAckType::Application(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            NormAckType::Cc => "NORM_ACK(CC)",
            NormAckType::Flush => "NORM_ACK(FLUSH)",
            NormAckType::Reserved(_) => "reserved",
            NormAckType::Application(_) => "application",
        }
    }
}

dvb_common::impl_spec_display!(NormAckType, Reserved, Application);

/// A NORM_CMD message (RFC 5740 §4.2.3): common header + sender word + an 8-bit
/// `sub-type` selecting the body, then the sub-type-specific content (kept
/// opaque, plus the extension chain).
///
/// The fixed per-sub-type layouts (FLUSH/EOT/SQUELCH/CC/REPAIR_ADV/ACK_REQ/
/// APPLICATION) live in `content`; the `sub_type` discriminant tells the caller
/// how to interpret it. This keeps the variable, length-inferred regions
/// (node lists, fec_payload_id, app content) opaque as the spec requires.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NormCmd<'a> {
    /// Common header.
    pub common: NormCommonHeader,
    /// Shared sender word.
    pub sender: SenderWord,
    /// The command sub-type.
    pub sub_type: NormCmdType,
    /// The 3 bytes that share the sub-type's first word (sub-type-specific:
    /// e.g. fec_id+object_transport_id for FLUSH, reserved for EOT/APPLICATION,
    /// reserved+cc_sequence for CC).
    pub head: [u8; 3],
    /// Header-extension chain (e.g. EXT_RATE in CC, EXT_CC in REPAIR_ADV).
    pub extensions: Vec<HeaderExtension<'a>>,
    /// Remaining sub-type-specific content (fec_payload_id, node lists, etc.),
    /// opaque to this layer.
    pub content: &'a [u8],
}

impl<'a> NormCmd<'a> {
    fn header_bytes(&self) -> usize {
        // common + sender + the sub-type word + extensions.
        COMMON_HEADER_LEN + SENDER_WORD_LEN + WORD + ext::chain_len(&self.extensions)
    }

    /// Total serialized length in bytes.
    pub fn serialized_len(&self) -> usize {
        self.header_bytes() + self.content.len()
    }

    /// Parse a NORM_CMD.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let (common, hdr_len) = NormCommonHeader::parse(data)?;
        let sender = SenderWord::parse(&data[COMMON_HEADER_LEN..])?;
        let mut off = COMMON_HEADER_LEN + SENDER_WORD_LEN;
        if data.len() < off + WORD {
            return Err(Error::BufferTooShort {
                need: off + WORD,
                have: data.len(),
                what: "NORM_CMD sub-type word",
            });
        }
        let sub_type = NormCmdType::from_u8(data[off]);
        let head = [data[off + 1], data[off + 2], data[off + 3]];
        off += WORD;

        let header_end = hdr_len as usize * WORD;
        if header_end < off {
            return Err(Error::InconsistentLength {
                length: hdr_len,
                reason: "hdr_len smaller than the NORM_CMD fixed header",
            });
        }
        if data.len() < header_end {
            return Err(Error::BufferTooShort {
                need: header_end,
                have: data.len(),
                what: "NORM_CMD header (per hdr_len)",
            });
        }
        let extensions = ext::parse_chain(&data[off..header_end])?;
        let content = &data[header_end..];
        Ok(NormCmd {
            common,
            sender,
            sub_type,
            head,
            extensions,
            content,
        })
    }

    /// Serialize into `out`, recomputing `hdr_len`. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        let words = self.header_bytes() / WORD;
        if words > u8::MAX as usize {
            return Err(Error::FieldTooWide {
                what: "hdr_len",
                value: words as u64,
                bits: 8,
            });
        }
        let mut off = self.common.serialize_into(out, words as u8)?;
        off += self.sender.serialize_into(&mut out[off..])?;
        out[off] = self.sub_type.to_u8();
        out[off + 1..off + 4].copy_from_slice(&self.head);
        off += WORD;
        off += ext::serialize_chain(&self.extensions, &mut out[off..])?;
        out[off..off + self.content.len()].copy_from_slice(self.content);
        off += self.content.len();
        Ok(off)
    }
}

/// A NORM feedback message — NORM_NACK (type 4) or NORM_ACK (type 5)
/// (RFC 5740 §4.3): common header, `server_id`, `instance_id`, a 16-bit field
/// that is `reserved` for NACK / `ack_type|ack_id` for ACK, then
/// `grtt_response_sec`/`grtt_response_usec`, extensions, and opaque payload.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NormFeedback<'a> {
    /// Common header (type = NORM_NACK or NORM_ACK).
    pub common: NormCommonHeader,
    /// Destination sender NormNodeId.
    pub server_id: u32,
    /// Sender's current instance_id.
    pub instance_id: u16,
    /// The 16-bit word after instance_id: reserved (NACK) or ack_type|ack_id
    /// (ACK). Stored as raw bytes; interpret per `common.message_type`.
    pub ack_or_reserved: u16,
    /// Adjusted NORM_CMD(CC) send_time seconds (0 = none yet).
    pub grtt_response_sec: u32,
    /// Adjusted send_time microseconds.
    pub grtt_response_usec: u32,
    /// Header-extension chain (e.g. EXT_CC).
    pub extensions: Vec<HeaderExtension<'a>>,
    /// nack_payload (NACK) or ack_payload (ACK), opaque here.
    pub payload: &'a [u8],
}

/// Fixed-header byte size of a NORM feedback message before extensions:
/// common(8) + server_id(4) + instance_id+ack/reserved(4) + 2×grtt(8) = 24.
pub const FEEDBACK_FIXED_LEN: usize = COMMON_HEADER_LEN + 4 + 4 + 8;

impl<'a> NormFeedback<'a> {
    /// The 8-bit `ack_type` (NORM_ACK): the high byte of `ack_or_reserved`.
    pub fn ack_type(&self) -> NormAckType {
        NormAckType::from_u8((self.ack_or_reserved >> 8) as u8)
    }
    /// The 8-bit `ack_id` (NORM_ACK): the low byte of `ack_or_reserved`.
    pub fn ack_id(&self) -> u8 {
        self.ack_or_reserved as u8
    }

    fn header_bytes(&self) -> usize {
        FEEDBACK_FIXED_LEN + ext::chain_len(&self.extensions)
    }

    /// Total serialized length in bytes.
    pub fn serialized_len(&self) -> usize {
        self.header_bytes() + self.payload.len()
    }

    /// Parse a NORM_NACK / NORM_ACK message.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let (common, hdr_len) = NormCommonHeader::parse(data)?;
        if data.len() < FEEDBACK_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FEEDBACK_FIXED_LEN,
                have: data.len(),
                what: "NORM feedback fixed header",
            });
        }
        let server_id = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        let instance_id = u16::from_be_bytes([data[12], data[13]]);
        let ack_or_reserved = u16::from_be_bytes([data[14], data[15]]);
        let grtt_response_sec = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let grtt_response_usec = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);

        let header_end = hdr_len as usize * WORD;
        if header_end < FEEDBACK_FIXED_LEN {
            return Err(Error::InconsistentLength {
                length: hdr_len,
                reason: "hdr_len smaller than the NORM feedback fixed header",
            });
        }
        if data.len() < header_end {
            return Err(Error::BufferTooShort {
                need: header_end,
                have: data.len(),
                what: "NORM feedback header (per hdr_len)",
            });
        }
        let extensions = ext::parse_chain(&data[FEEDBACK_FIXED_LEN..header_end])?;
        let payload = &data[header_end..];
        Ok(NormFeedback {
            common,
            server_id,
            instance_id,
            ack_or_reserved,
            grtt_response_sec,
            grtt_response_usec,
            extensions,
            payload,
        })
    }

    /// Serialize into `out`, recomputing `hdr_len`. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        let words = self.header_bytes() / WORD;
        if words > u8::MAX as usize {
            return Err(Error::FieldTooWide {
                what: "hdr_len",
                value: words as u64,
                bits: 8,
            });
        }
        let mut off = self.common.serialize_into(out, words as u8)?;
        out[off..off + 4].copy_from_slice(&self.server_id.to_be_bytes());
        out[off + 4..off + 6].copy_from_slice(&self.instance_id.to_be_bytes());
        out[off + 6..off + 8].copy_from_slice(&self.ack_or_reserved.to_be_bytes());
        out[off + 8..off + 12].copy_from_slice(&self.grtt_response_sec.to_be_bytes());
        out[off + 12..off + 16].copy_from_slice(&self.grtt_response_usec.to_be_bytes());
        off += 16;
        off += ext::serialize_chain(&self.extensions, &mut out[off..])?;
        out[off..off + self.payload.len()].copy_from_slice(self.payload);
        off += self.payload.len();
        Ok(off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn common(ty: NormMessageType) -> NormCommonHeader {
        NormCommonHeader {
            version: NORM_VERSION,
            message_type: ty,
            sequence: 0x1234,
            source_id: 0xCAFEBABE,
        }
    }

    fn sender() -> SenderWord {
        SenderWord {
            instance_id: 0x00FF,
            grtt: 0x40,
            backoff: 0x0A,
            gsize: 0x05,
        }
    }

    #[test]
    fn message_type_round_trip() {
        for v in 0u8..=6 {
            assert_eq!(NormMessageType::from_u8(v).to_u8(), v);
        }
        assert_eq!(NormMessageType::Data.to_string(), "NORM_DATA");
        assert_eq!(NormMessageType::Other(9).to_string(), "reserved(0x09)");
    }

    #[test]
    fn common_header_exact_bytes() {
        let c = common(NormMessageType::Data);
        let mut out = [0u8; COMMON_HEADER_LEN];
        c.serialize_into(&mut out, 7).unwrap();
        // version=1, type=2 -> 0x12; hdr_len=7; seq=0x1234; source=0xCAFEBABE.
        assert_eq!(out, [0x12, 0x07, 0x12, 0x34, 0xCA, 0xFE, 0xBA, 0xBE]);
        let (re, hl) = NormCommonHeader::parse(&out).unwrap();
        assert_eq!(re, c);
        assert_eq!(hl, 7);
    }

    #[test]
    fn sender_word_exact_bytes() {
        let s = sender();
        let mut out = [0u8; SENDER_WORD_LEN];
        s.serialize_into(&mut out).unwrap();
        // instance=0x00FF, grtt=0x40, backoff=0xA gsize=0x5 -> 0xA5.
        assert_eq!(out, [0x00, 0xFF, 0x40, 0xA5]);
        assert_eq!(SenderWord::parse(&out).unwrap(), s);
    }

    #[test]
    fn norm_data_round_trip_with_fec_payload_id() {
        // fec_id=129 → 8-byte fec_payload_id.
        let fpid = [0x00u8, 0x00, 0x00, 0x01, 0x00, 0x05, 0x00, 0x02];
        let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let d = NormData {
            common: common(NormMessageType::Data),
            sender: sender(),
            flags: NORM_FLAG_FILE,
            fec_id: 129,
            object_transport_id: 0x0007,
            fec_payload_id: &fpid,
            extensions: vec![],
            payload: &payload,
        };
        // hdr_len = (8 + 4 + 4 + 8)/4 = 6 words.
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(n, d.serialized_len());
        assert_eq!(out[1], 6, "hdr_len");
        let re = NormData::parse(&out, 8).unwrap();
        assert_eq!(re, d);
    }

    #[test]
    fn norm_data_with_ext_fti() {
        // EXT_FTI HET=64, hel=4 → 16-byte extension.
        let fti = [
            0x40u8, 0x04, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, // het|hel|object_size
            0x00, 0x01, 0x02, 0x00, 0x04, 0x00, 0x00, 0x10, // scheme-specific
        ];
        let (ext_fti, used) = HeaderExtension::parse(&fti).unwrap();
        assert_eq!(used, 16);
        let fpid = [0u8; 8];
        let d = NormData {
            common: common(NormMessageType::Data),
            sender: sender(),
            flags: 0,
            fec_id: 129,
            object_transport_id: 1,
            fec_payload_id: &fpid,
            extensions: vec![ext_fti],
            payload: &[],
        };
        // hdr_len = (8 + 4 + 4 + 8 + 16)/4 = 10.
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out[1], 10);
        let re = NormData::parse(&out, 8).unwrap();
        assert_eq!(re, d);
        assert_eq!(re.extensions.len(), 1);
        assert_eq!(re.extensions[0].het, HET_EXT_FTI);
    }

    #[test]
    fn norm_cmd_eot_round_trip() {
        let c = NormCmd {
            common: common(NormMessageType::Cmd),
            sender: sender(),
            sub_type: NormCmdType::Eot,
            head: [0, 0, 0], // reserved
            extensions: vec![],
            content: &[],
        };
        let mut out = vec![0u8; c.serialized_len()];
        c.serialize_into(&mut out).unwrap();
        // hdr_len = (8+4+4)/4 = 4.
        assert_eq!(out[1], 4);
        // sub-type byte sits right after sender word (offset 12).
        assert_eq!(out[12], 2);
        let re = NormCmd::parse(&out).unwrap();
        assert_eq!(re, c);
        assert_eq!(re.sub_type, NormCmdType::Eot);
    }

    #[test]
    fn norm_cmd_flush_with_content() {
        // FLUSH: head = fec_id | object_transport_id(16); content = fec_payload_id.
        let c = NormCmd {
            common: common(NormMessageType::Cmd),
            sender: sender(),
            sub_type: NormCmdType::Flush,
            head: [129, 0x00, 0x07], // fec_id=129, object_transport_id=7
            extensions: vec![],
            content: &[0, 0, 0, 1, 0, 5, 0, 2],
        };
        let mut out = vec![0u8; c.serialized_len()];
        c.serialize_into(&mut out).unwrap();
        let re = NormCmd::parse(&out).unwrap();
        assert_eq!(re, c);
    }

    #[test]
    fn norm_nack_round_trip() {
        let f = NormFeedback {
            common: common(NormMessageType::Nack),
            server_id: 0x11223344,
            instance_id: 0x00FF,
            ack_or_reserved: 0, // reserved for NACK
            grtt_response_sec: 0x55667788,
            grtt_response_usec: 0x99AABBCC,
            extensions: vec![],
            payload: &[0x01, 0x02, 0x00, 0x04],
        };
        // hdr_len = 24/4 = 6.
        let mut out = vec![0u8; f.serialized_len()];
        f.serialize_into(&mut out).unwrap();
        assert_eq!(out[1], 6);
        let re = NormFeedback::parse(&out).unwrap();
        assert_eq!(re, f);
    }

    #[test]
    fn norm_ack_ack_type_id() {
        let f = NormFeedback {
            common: common(NormMessageType::Ack),
            server_id: 1,
            instance_id: 2,
            ack_or_reserved: (2 << 8) | 0x07, // ack_type=FLUSH(2), ack_id=7
            grtt_response_sec: 0,
            grtt_response_usec: 0,
            extensions: vec![],
            payload: &[],
        };
        assert_eq!(f.ack_type(), NormAckType::Flush);
        assert_eq!(f.ack_id(), 7);
        let mut out = vec![0u8; f.serialized_len()];
        f.serialize_into(&mut out).unwrap();
        let re = NormFeedback::parse(&out).unwrap();
        assert_eq!(re, f);
    }

    #[test]
    fn ack_type_ranges() {
        assert_eq!(NormAckType::from_u8(1), NormAckType::Cc);
        assert_eq!(NormAckType::from_u8(2), NormAckType::Flush);
        assert_eq!(NormAckType::from_u8(10), NormAckType::Reserved(10));
        assert_eq!(NormAckType::from_u8(200), NormAckType::Application(200));
        assert_eq!(NormAckType::Reserved(10).to_string(), "reserved(0x0A)");
    }

    /// NORM_INFO round-trip: construct from typed fields, serialize, parse back,
    /// verify byte-exact. The NormInfo header has NO fec_payload_id —
    /// `hdr_len` base is 4 words (16 bytes).
    #[test]
    fn norm_info_round_trip() {
        let payload = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02, 0x03u8];
        let info = NormInfo {
            common: common(NormMessageType::Info),
            sender: sender(),
            flags: NORM_FLAG_FILE,
            fec_id: 129,
            object_transport_id: 0x0042,
            extensions: vec![],
            payload: &payload,
        };
        // hdr_len = (8 + 4 + 4) / 4 = 4 words; no fec_payload_id.
        assert_eq!(info.header_bytes(), NORM_INFO_FIXED_LEN);
        let total = info.serialized_len();
        assert_eq!(total, NORM_INFO_FIXED_LEN + payload.len());

        let mut out = vec![0u8; total];
        let n = info.serialize_into(&mut out).unwrap();
        assert_eq!(n, total);

        // Byte-exact checks: version|type=0x11, hdr_len=4.
        assert_eq!(out[0], 0x11, "version=1 type=1");
        assert_eq!(out[1], 4, "hdr_len = 4 words");
        // flags-word: flags | fec_id | oti.
        assert_eq!(out[12], NORM_FLAG_FILE);
        assert_eq!(out[13], 129);
        assert_eq!(u16::from_be_bytes([out[14], out[15]]), 0x0042);
        // Payload follows immediately (no fec_payload_id).
        assert_eq!(&out[16..], &payload);

        let re = NormInfo::parse(&out).unwrap();
        assert_eq!(re, info);
    }

    /// NORM_INFO round-trip with a mutated field confirms the value is encoded
    /// and recovered (not silently dropped).
    #[test]
    fn norm_info_mutated_field_changes_wire() {
        let payload = [0u8; 4];
        let make = |oti: u16| {
            let i = NormInfo {
                common: common(NormMessageType::Info),
                sender: sender(),
                flags: 0,
                fec_id: 0,
                object_transport_id: oti,
                extensions: vec![],
                payload: &payload,
            };
            let mut out = vec![0u8; i.serialized_len()];
            i.serialize_into(&mut out).unwrap();
            out
        };
        let a = make(0x0001);
        let b = make(0x0002);
        assert_ne!(a, b);
        // OTI is at bytes 14..16 in the header.
        assert_eq!(u16::from_be_bytes([a[14], a[15]]), 0x0001);
        assert_eq!(u16::from_be_bytes([b[14], b[15]]), 0x0002);
    }

    /// NORM_INFO with an EXT_FTI extension: hdr_len grows by the extension words.
    #[test]
    fn norm_info_with_ext_fti() {
        // EXT_FTI: het=64, hel=4 → 16 bytes.
        let fti = [
            0x40u8, 0x04, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x01, 0x02, 0x00, 0x04, 0x00,
            0x00, 0x10,
        ];
        let (ext_fti, _) = HeaderExtension::parse(&fti).unwrap();
        let payload = [0xAB, 0xCDu8];
        let info = NormInfo {
            common: common(NormMessageType::Info),
            sender: sender(),
            flags: NORM_FLAG_INFO,
            fec_id: 129,
            object_transport_id: 3,
            extensions: vec![ext_fti],
            payload: &payload,
        };
        // hdr_len = (8 + 4 + 4 + 16) / 4 = 8 words.
        assert_eq!(info.header_bytes(), 32);
        let mut out = vec![0u8; info.serialized_len()];
        info.serialize_into(&mut out).unwrap();
        assert_eq!(out[1], 8, "hdr_len with EXT_FTI");
        let re = NormInfo::parse(&out).unwrap();
        assert_eq!(re, info);
        assert_eq!(re.extensions.len(), 1);
        assert_eq!(re.extensions[0].het, HET_EXT_FTI);
    }
}
