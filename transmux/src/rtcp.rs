//! RTCP — RTP Control Protocol (RFC 3550 §6).
//!
//! Companion to the [`rtp`](crate::rtp) module: typed, symmetric
//! [`Parse`]/[`Serialize`] for the RTCP control packets so a real RTP session is
//! fully supported. RTCP carries **no media** — this is not a hub
//! `Package`/`Unpackage` spoke, just the wire codec for the control channel.
//!
//! See `transmux/docs/rtp/rtcp.md` for the layout transcription.
//!
//! # Wire formats
//!
//! - **Common header** (§6.1): `V(2)=2 | P(1) | RC/SC(5) | PT(8) | length(16)`,
//!   where `length` is the packet size in 32-bit words **minus one**.
//! - **SR** — Sender Report (§6.4.1, PT 200): 20-byte sender info
//!   (SSRC, NTP MSW/LSW, RTP timestamp, packet count, octet count) then
//!   `RC` × [`ReportBlock`].
//! - **RR** — Receiver Report (§6.4.2, PT 201): reporter SSRC then
//!   `RC` × [`ReportBlock`] (no sender info).
//! - **[`ReportBlock`]** (§6.4.1, 24 bytes): `SSRC_n`, fraction lost,
//!   cumulative lost (24-bit **signed**), extended highest sequence,
//!   interarrival jitter, LSR, DLSR.
//! - **SDES** — Source Description (§6.5, PT 202): `SC` chunks of
//!   `SSRC/CSRC` + a list of `[type(8), length(8), text]` items, terminated by
//!   a type-0 item and padded to a 32-bit boundary.
//! - **BYE** (§6.6, PT 203): `SC` × `SSRC/CSRC` + an optional reason string.
//! - **APP** (§6.7, PT 204): subtype (in the RC field), SSRC, 4-byte ASCII
//!   name, application-dependent data (32-bit aligned).
//! - **[`CompoundPacket`]** (§6.1): a sequence of RTCP packets that **must**
//!   begin with an SR or RR.
//!
//! # Reserved-bit / version policy
//!
//! The version field is validated (must be 2). The padding (`P`) bit is parsed
//! and preserved but this codec emits unpadded packets (`P=0`); padding bytes on
//! the wire are consumed per the length field. `no_std` + `alloc`.

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Named constants (no magic numbers — RFC 3550 §6)
// ---------------------------------------------------------------------------

/// RTCP protocol version — always 2 (RFC 3550 §6.4.1).
const RTCP_VERSION: u8 = 2;
/// Byte 0 of the common header with `V=2 P=0` and a zero count field.
const RTCP_BYTE0_V2: u8 = RTCP_VERSION << 6;
/// Common-header length in bytes (`V/P/count | PT | length(16)`).
const RTCP_HEADER_LEN: usize = 4;
/// Padding-bit mask within byte 0 (`P` — RFC 3550 §6.4.1).
const RTCP_PADDING_MASK: u8 = 0x20;
/// Report-count / source-count mask within byte 0 (low 5 bits).
const RTCP_COUNT_MASK: u8 = 0x1F;
/// One 32-bit word, in bytes — the unit of the header `length` field.
const WORD_LEN: usize = 4;

/// Packet type: Sender Report (RFC 3550 §6.4.1).
pub const PT_SENDER_REPORT: u8 = 200;
/// Packet type: Receiver Report (RFC 3550 §6.4.2).
pub const PT_RECEIVER_REPORT: u8 = 201;
/// Packet type: Source Description (RFC 3550 §6.5).
pub const PT_SOURCE_DESCRIPTION: u8 = 202;
/// Packet type: Goodbye (RFC 3550 §6.6).
pub const PT_BYE: u8 = 203;
/// Packet type: Application-defined (RFC 3550 §6.7).
pub const PT_APP: u8 = 204;

/// Length of a single [`ReportBlock`] on the wire (RFC 3550 §6.4.1).
pub const REPORT_BLOCK_LEN: usize = 24;
/// Length of the SR sender-info block (RFC 3550 §6.4.1), excluding the SSRC.
const SR_SENDER_INFO_LEN: usize = 20;
/// Length of the APP `name` field — 4 ASCII characters (RFC 3550 §6.7).
pub const APP_NAME_LEN: usize = 4;
/// Maximum count encodable in the 5-bit `RC`/`SC` field.
const MAX_COUNT: usize = RTCP_COUNT_MASK as usize;

// ---------------------------------------------------------------------------
// Big-endian read helpers (bounds-checked)
// ---------------------------------------------------------------------------

/// Read a big-endian `u32` at `off`, or `BufferTooShort`.
fn be_u32(bytes: &[u8], off: usize, what: &'static str) -> Result<u32> {
    bytes
        .get(off..off + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(Error::BufferTooShort {
            need: off + 4,
            have: bytes.len(),
            what,
        })
}

// ---------------------------------------------------------------------------
// RtcpPacketType — the PT byte, typed
// ---------------------------------------------------------------------------

/// The RTCP packet type carried in the common header `PT` byte (RFC 3550 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RtcpPacketType {
    /// Sender Report (PT 200).
    SenderReport,
    /// Receiver Report (PT 201).
    ReceiverReport,
    /// Source Description (PT 202).
    SourceDescription,
    /// Goodbye (PT 203).
    Bye,
    /// Application-defined (PT 204).
    App,
    /// A packet type outside the RFC 3550 §6 core set.
    Unknown(u8),
}

impl RtcpPacketType {
    /// Decode the wire `PT` byte.
    pub fn from_pt(pt: u8) -> Self {
        match pt {
            PT_SENDER_REPORT => RtcpPacketType::SenderReport,
            PT_RECEIVER_REPORT => RtcpPacketType::ReceiverReport,
            PT_SOURCE_DESCRIPTION => RtcpPacketType::SourceDescription,
            PT_BYE => RtcpPacketType::Bye,
            PT_APP => RtcpPacketType::App,
            other => RtcpPacketType::Unknown(other),
        }
    }

    /// The wire `PT` byte for this packet type.
    pub fn pt(&self) -> u8 {
        match self {
            RtcpPacketType::SenderReport => PT_SENDER_REPORT,
            RtcpPacketType::ReceiverReport => PT_RECEIVER_REPORT,
            RtcpPacketType::SourceDescription => PT_SOURCE_DESCRIPTION,
            RtcpPacketType::Bye => PT_BYE,
            RtcpPacketType::App => PT_APP,
            RtcpPacketType::Unknown(pt) => *pt,
        }
    }

    /// Spec token for this packet type.
    pub fn name(&self) -> &'static str {
        match self {
            RtcpPacketType::SenderReport => "SR",
            RtcpPacketType::ReceiverReport => "RR",
            RtcpPacketType::SourceDescription => "SDES",
            RtcpPacketType::Bye => "BYE",
            RtcpPacketType::App => "APP",
            RtcpPacketType::Unknown(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(RtcpPacketType, Unknown);

// ---------------------------------------------------------------------------
// Common header (RFC 3550 §6.1 / §6.4.1)
// ---------------------------------------------------------------------------

/// The 4-byte RTCP common header shared by every packet type (RFC 3550 §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommonHeader {
    /// The `P` (padding) bit.
    pub padding: bool,
    /// The `RC`/`SC` 5-bit count (report count for SR/RR, source count for
    /// SDES/BYE, subtype for APP).
    pub count: u8,
    /// The `PT` (packet type) byte.
    pub packet_type: u8,
    /// The `length` field: packet size in 32-bit words **minus one**.
    pub length: u16,
}

impl CommonHeader {
    /// Build a header from decoded fields (`V=2`, `P=0`).
    fn new(count: u8, packet_type: u8, length_words_minus_one: u16) -> Self {
        Self {
            padding: false,
            count: count & RTCP_COUNT_MASK,
            packet_type,
            length: length_words_minus_one,
        }
    }

    /// Parse the common header, validating the version field.
    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < RTCP_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: RTCP_HEADER_LEN,
                have: bytes.len(),
                what: "RTCP common header",
            });
        }
        let version = bytes[0] >> 6;
        if version != RTCP_VERSION {
            return Err(Error::InvalidValue {
                field: "rtcp_version",
                value: version as u64,
                reason: "must be 2",
            });
        }
        Ok(Self {
            padding: bytes[0] & RTCP_PADDING_MASK != 0,
            count: bytes[0] & RTCP_COUNT_MASK,
            packet_type: bytes[1],
            length: u16::from_be_bytes([bytes[2], bytes[3]]),
        })
    }

    /// The total on-the-wire byte length of the packet this header describes:
    /// `(length + 1) * 4`.
    fn total_len(&self) -> usize {
        (self.length as usize + 1) * WORD_LEN
    }

    /// Write the 4-byte header (`V=2`, given `P`/count/PT/length).
    fn write(&self, buf: &mut [u8]) {
        buf[0] = RTCP_BYTE0_V2
            | (if self.padding { RTCP_PADDING_MASK } else { 0 })
            | (self.count & RTCP_COUNT_MASK);
        buf[1] = self.packet_type;
        buf[2..4].copy_from_slice(&self.length.to_be_bytes());
    }
}

/// Compute the header `length` field (32-bit words − 1) for a body whose total
/// serialized length (header included) is `total_len` bytes. `total_len` is a
/// multiple of 4 for every RTCP packet this codec emits.
fn length_words_minus_one(total_len: usize) -> u16 {
    (total_len / WORD_LEN - 1) as u16
}

// ---------------------------------------------------------------------------
// ReportBlock (RFC 3550 §6.4.1)
// ---------------------------------------------------------------------------

/// A reception report block (RFC 3550 §6.4.1, 24 bytes). Carried by both SR and
/// RR, one per reported source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ReportBlock {
    /// SSRC of the source this block reports on.
    pub ssrc: u32,
    /// Fraction of packets lost since the previous report (8.8 fixed-point num).
    pub fraction_lost: u8,
    /// Cumulative number of packets lost — a 24-bit **signed** value.
    pub cumulative_lost: i32,
    /// Extended highest sequence number received.
    pub ext_highest_seq: u32,
    /// Interarrival jitter estimate.
    pub jitter: u32,
    /// Last SR timestamp (middle 32 bits of the sender's NTP time), or 0.
    pub lsr: u32,
    /// Delay since last SR, in units of 1/65536 s, or 0.
    pub dlsr: u32,
}

impl ReportBlock {
    /// Sign-extend a 24-bit `cumulative_lost` field to `i32`.
    fn decode_cumulative_lost(raw: u32) -> i32 {
        // raw is a 24-bit two's-complement value; extend the sign bit (bit 23).
        const SIGN_BIT: u32 = 1 << 23;
        if raw & SIGN_BIT != 0 {
            (raw | 0xFF00_0000) as i32
        } else {
            raw as i32
        }
    }

    /// Encode a signed `cumulative_lost` back to its 24-bit field.
    fn encode_cumulative_lost(&self) -> u32 {
        (self.cumulative_lost as u32) & 0x00FF_FFFF
    }
}

impl<'a> Parse<'a> for ReportBlock {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < REPORT_BLOCK_LEN {
            return Err(Error::BufferTooShort {
                need: REPORT_BLOCK_LEN,
                have: bytes.len(),
                what: "RTCP report block",
            });
        }
        let ssrc = be_u32(bytes, 0, "report block ssrc")?;
        let fraction_lost = bytes[4];
        let cumulative_raw = u32::from_be_bytes([0, bytes[5], bytes[6], bytes[7]]);
        Ok(ReportBlock {
            ssrc,
            fraction_lost,
            cumulative_lost: ReportBlock::decode_cumulative_lost(cumulative_raw),
            ext_highest_seq: be_u32(bytes, 8, "report block ext seq")?,
            jitter: be_u32(bytes, 12, "report block jitter")?,
            lsr: be_u32(bytes, 16, "report block lsr")?,
            dlsr: be_u32(bytes, 20, "report block dlsr")?,
        })
    }
}

impl Serialize for ReportBlock {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        REPORT_BLOCK_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < REPORT_BLOCK_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: REPORT_BLOCK_LEN,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.ssrc.to_be_bytes());
        buf[4] = self.fraction_lost;
        let cum = self.encode_cumulative_lost().to_be_bytes();
        buf[5..8].copy_from_slice(&cum[1..4]);
        buf[8..12].copy_from_slice(&self.ext_highest_seq.to_be_bytes());
        buf[12..16].copy_from_slice(&self.jitter.to_be_bytes());
        buf[16..20].copy_from_slice(&self.lsr.to_be_bytes());
        buf[20..24].copy_from_slice(&self.dlsr.to_be_bytes());
        Ok(REPORT_BLOCK_LEN)
    }
}

/// Parse `count` back-to-back report blocks from `bytes`.
fn parse_report_blocks(bytes: &[u8], count: usize) -> Result<Vec<ReportBlock>> {
    let mut blocks = Vec::with_capacity(count);
    let mut off = 0;
    for _ in 0..count {
        let end = off + REPORT_BLOCK_LEN;
        if end > bytes.len() {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "RTCP report blocks",
            });
        }
        blocks.push(ReportBlock::parse(&bytes[off..end])?);
        off = end;
    }
    Ok(blocks)
}

/// Validate a report-block list fits the 5-bit `RC` count field.
fn check_report_count(blocks: &[ReportBlock]) -> Result<u8> {
    if blocks.len() > MAX_COUNT {
        return Err(Error::InvalidValue {
            field: "rtcp_report_count",
            value: blocks.len() as u64,
            reason: "exceeds 5-bit RC field",
        });
    }
    Ok(blocks.len() as u8)
}

// ---------------------------------------------------------------------------
// SenderReport (RFC 3550 §6.4.1, PT 200)
// ---------------------------------------------------------------------------

/// RTCP Sender Report (RFC 3550 §6.4.1, PT 200).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SenderReport {
    /// SSRC of the sender originating this report.
    pub ssrc: u32,
    /// NTP timestamp, most significant word (integer seconds).
    pub ntp_msw: u32,
    /// NTP timestamp, least significant word (fractional seconds).
    pub ntp_lsw: u32,
    /// RTP timestamp corresponding to the NTP wall-clock time.
    pub rtp_timestamp: u32,
    /// Sender's cumulative packet count.
    pub packet_count: u32,
    /// Sender's cumulative octet count.
    pub octet_count: u32,
    /// Reception report blocks (`RC` of them).
    pub report_blocks: Vec<ReportBlock>,
}

impl<'a> Parse<'a> for SenderReport {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let hdr = CommonHeader::parse(bytes)?;
        if hdr.packet_type != PT_SENDER_REPORT {
            return Err(Error::InvalidValue {
                field: "rtcp_pt",
                value: hdr.packet_type as u64,
                reason: "expected SR (200)",
            });
        }
        let total = hdr.total_len();
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "RTCP SR",
            });
        }
        let body = &bytes[RTCP_HEADER_LEN..total];
        if body.len() < WORD_LEN + SR_SENDER_INFO_LEN {
            return Err(Error::BufferTooShort {
                need: WORD_LEN + SR_SENDER_INFO_LEN,
                have: body.len(),
                what: "RTCP SR sender info",
            });
        }
        let ssrc = be_u32(body, 0, "SR ssrc")?;
        let ntp_msw = be_u32(body, 4, "SR ntp msw")?;
        let ntp_lsw = be_u32(body, 8, "SR ntp lsw")?;
        let rtp_timestamp = be_u32(body, 12, "SR rtp ts")?;
        let packet_count = be_u32(body, 16, "SR packet count")?;
        let octet_count = be_u32(body, 20, "SR octet count")?;
        let blocks_off = WORD_LEN + SR_SENDER_INFO_LEN;
        let report_blocks = parse_report_blocks(&body[blocks_off..], hdr.count as usize)?;
        Ok(SenderReport {
            ssrc,
            ntp_msw,
            ntp_lsw,
            rtp_timestamp,
            packet_count,
            octet_count,
            report_blocks,
        })
    }
}

impl Serialize for SenderReport {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        RTCP_HEADER_LEN
            + WORD_LEN
            + SR_SENDER_INFO_LEN
            + self.report_blocks.len() * REPORT_BLOCK_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let rc = check_report_count(&self.report_blocks)?;
        let hdr = CommonHeader::new(rc, PT_SENDER_REPORT, length_words_minus_one(len));
        hdr.write(&mut buf[0..RTCP_HEADER_LEN]);
        let mut off = RTCP_HEADER_LEN;
        buf[off..off + 4].copy_from_slice(&self.ssrc.to_be_bytes());
        buf[off + 4..off + 8].copy_from_slice(&self.ntp_msw.to_be_bytes());
        buf[off + 8..off + 12].copy_from_slice(&self.ntp_lsw.to_be_bytes());
        buf[off + 12..off + 16].copy_from_slice(&self.rtp_timestamp.to_be_bytes());
        buf[off + 16..off + 20].copy_from_slice(&self.packet_count.to_be_bytes());
        buf[off + 20..off + 24].copy_from_slice(&self.octet_count.to_be_bytes());
        off += WORD_LEN + SR_SENDER_INFO_LEN;
        for block in &self.report_blocks {
            block.serialize_into(&mut buf[off..off + REPORT_BLOCK_LEN])?;
            off += REPORT_BLOCK_LEN;
        }
        Ok(len)
    }
}

// ---------------------------------------------------------------------------
// ReceiverReport (RFC 3550 §6.4.2, PT 201)
// ---------------------------------------------------------------------------

/// RTCP Receiver Report (RFC 3550 §6.4.2, PT 201).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ReceiverReport {
    /// SSRC of the packet sender originating this report.
    pub ssrc: u32,
    /// Reception report blocks (`RC` of them).
    pub report_blocks: Vec<ReportBlock>,
}

impl<'a> Parse<'a> for ReceiverReport {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let hdr = CommonHeader::parse(bytes)?;
        if hdr.packet_type != PT_RECEIVER_REPORT {
            return Err(Error::InvalidValue {
                field: "rtcp_pt",
                value: hdr.packet_type as u64,
                reason: "expected RR (201)",
            });
        }
        let total = hdr.total_len();
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "RTCP RR",
            });
        }
        let body = &bytes[RTCP_HEADER_LEN..total];
        let ssrc = be_u32(body, 0, "RR ssrc")?;
        let report_blocks = parse_report_blocks(&body[WORD_LEN..], hdr.count as usize)?;
        Ok(ReceiverReport {
            ssrc,
            report_blocks,
        })
    }
}

impl Serialize for ReceiverReport {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        RTCP_HEADER_LEN + WORD_LEN + self.report_blocks.len() * REPORT_BLOCK_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let rc = check_report_count(&self.report_blocks)?;
        let hdr = CommonHeader::new(rc, PT_RECEIVER_REPORT, length_words_minus_one(len));
        hdr.write(&mut buf[0..RTCP_HEADER_LEN]);
        let mut off = RTCP_HEADER_LEN;
        buf[off..off + 4].copy_from_slice(&self.ssrc.to_be_bytes());
        off += WORD_LEN;
        for block in &self.report_blocks {
            block.serialize_into(&mut buf[off..off + REPORT_BLOCK_LEN])?;
            off += REPORT_BLOCK_LEN;
        }
        Ok(len)
    }
}

// ---------------------------------------------------------------------------
// SourceDescription (RFC 3550 §6.5, PT 202)
// ---------------------------------------------------------------------------

/// SDES item type (RFC 3550 §6.5). Byte-valued; type 0 is the item terminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SdesItemType {
    /// Canonical end-point identifier (CNAME = 1).
    CName,
    /// User name (NAME = 2).
    Name,
    /// Electronic mail address (EMAIL = 3).
    Email,
    /// Phone number (PHONE = 4).
    Phone,
    /// Geographic location (LOC = 5).
    Loc,
    /// Application / tool name+version (TOOL = 6).
    Tool,
    /// Notice / status (NOTE = 7).
    Note,
    /// Private extension (PRIV = 8).
    Priv,
    /// A type outside the RFC 3550 §6.5 set (never the 0 terminator).
    Unknown(u8),
}

/// SDES item type value: CNAME (RFC 3550 §6.5.1).
pub const SDES_CNAME: u8 = 1;
/// SDES item type value: NAME (RFC 3550 §6.5.2).
pub const SDES_NAME: u8 = 2;
/// SDES item type value: EMAIL (RFC 3550 §6.5.3).
pub const SDES_EMAIL: u8 = 3;
/// SDES item type value: PHONE (RFC 3550 §6.5.4).
pub const SDES_PHONE: u8 = 4;
/// SDES item type value: LOC (RFC 3550 §6.5.5).
pub const SDES_LOC: u8 = 5;
/// SDES item type value: TOOL (RFC 3550 §6.5.6).
pub const SDES_TOOL: u8 = 6;
/// SDES item type value: NOTE (RFC 3550 §6.5.7).
pub const SDES_NOTE: u8 = 7;
/// SDES item type value: PRIV (RFC 3550 §6.5.8).
pub const SDES_PRIV: u8 = 8;
/// SDES chunk item-list terminator (RFC 3550 §6.5).
const SDES_TERMINATOR: u8 = 0;

impl SdesItemType {
    /// Decode the wire item-type byte.
    pub fn from_type(t: u8) -> Self {
        match t {
            SDES_CNAME => SdesItemType::CName,
            SDES_NAME => SdesItemType::Name,
            SDES_EMAIL => SdesItemType::Email,
            SDES_PHONE => SdesItemType::Phone,
            SDES_LOC => SdesItemType::Loc,
            SDES_TOOL => SdesItemType::Tool,
            SDES_NOTE => SdesItemType::Note,
            SDES_PRIV => SdesItemType::Priv,
            other => SdesItemType::Unknown(other),
        }
    }

    /// The wire item-type byte.
    pub fn item_type(&self) -> u8 {
        match self {
            SdesItemType::CName => SDES_CNAME,
            SdesItemType::Name => SDES_NAME,
            SdesItemType::Email => SDES_EMAIL,
            SdesItemType::Phone => SDES_PHONE,
            SdesItemType::Loc => SDES_LOC,
            SdesItemType::Tool => SDES_TOOL,
            SdesItemType::Note => SDES_NOTE,
            SdesItemType::Priv => SDES_PRIV,
            SdesItemType::Unknown(t) => *t,
        }
    }

    /// Spec token for this item type.
    pub fn name(&self) -> &'static str {
        match self {
            SdesItemType::CName => "CNAME",
            SdesItemType::Name => "NAME",
            SdesItemType::Email => "EMAIL",
            SdesItemType::Phone => "PHONE",
            SdesItemType::Loc => "LOC",
            SdesItemType::Tool => "TOOL",
            SdesItemType::Note => "NOTE",
            SdesItemType::Priv => "PRIV",
            SdesItemType::Unknown(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(SdesItemType, Unknown);

/// A single SDES item: a typed, length-prefixed text field (RFC 3550 §6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdesItem {
    /// The item type.
    pub item_type: SdesItemType,
    /// The item text (up to 255 bytes; UTF-8 per §6.5).
    pub text: String,
}

/// An SDES chunk: an SSRC/CSRC plus its list of items (RFC 3550 §6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdesChunk {
    /// The SSRC or CSRC this chunk describes.
    pub source: u32,
    /// The chunk's items (in wire order), before the type-0 terminator.
    pub items: Vec<SdesItem>,
}

impl SdesChunk {
    /// On-the-wire byte length of this chunk **before** 32-bit padding:
    /// 4 (source) + Σ(2 + text.len()) + 1 (terminator).
    fn unpadded_len(&self) -> usize {
        WORD_LEN + self.items.iter().map(|it| 2 + it.text.len()).sum::<usize>() + 1
    }

    /// Padded (32-bit-aligned) length of this chunk on the wire.
    fn padded_len(&self) -> usize {
        self.unpadded_len().div_ceil(WORD_LEN) * WORD_LEN
    }
}

/// RTCP Source Description (RFC 3550 §6.5, PT 202).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SourceDescription {
    /// The chunks (`SC` of them), one per described source.
    pub chunks: Vec<SdesChunk>,
}

impl<'a> Parse<'a> for SourceDescription {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let hdr = CommonHeader::parse(bytes)?;
        if hdr.packet_type != PT_SOURCE_DESCRIPTION {
            return Err(Error::InvalidValue {
                field: "rtcp_pt",
                value: hdr.packet_type as u64,
                reason: "expected SDES (202)",
            });
        }
        let total = hdr.total_len();
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "RTCP SDES",
            });
        }
        let body = &bytes[RTCP_HEADER_LEN..total];
        let mut chunks = Vec::with_capacity(hdr.count as usize);
        let mut off = 0;
        for _ in 0..hdr.count {
            let (chunk, consumed) = parse_sdes_chunk(&body[off..])?;
            chunks.push(chunk);
            off += consumed;
        }
        Ok(SourceDescription { chunks })
    }
}

/// Parse one SDES chunk starting at `bytes[0]`; return it and bytes consumed
/// (including the type-0 terminator and any 32-bit padding).
fn parse_sdes_chunk(bytes: &[u8]) -> Result<(SdesChunk, usize)> {
    let source = be_u32(bytes, 0, "SDES chunk source")?;
    let mut off = WORD_LEN;
    let mut items = Vec::new();
    loop {
        let t = *bytes.get(off).ok_or(Error::BufferTooShort {
            need: off + 1,
            have: bytes.len(),
            what: "SDES item type",
        })?;
        off += 1;
        if t == SDES_TERMINATOR {
            break;
        }
        let len = *bytes.get(off).ok_or(Error::BufferTooShort {
            need: off + 1,
            have: bytes.len(),
            what: "SDES item length",
        })? as usize;
        off += 1;
        let end = off + len;
        let text_bytes = bytes.get(off..end).ok_or(Error::BufferTooShort {
            need: end,
            have: bytes.len(),
            what: "SDES item text",
        })?;
        let text = String::from_utf8(text_bytes.to_vec()).map_err(|_| Error::InvalidValue {
            field: "sdes_item_text",
            value: 0,
            reason: "not valid UTF-8",
        })?;
        items.push(SdesItem {
            item_type: SdesItemType::from_type(t),
            text,
        });
        off = end;
    }
    // Advance past the type-0 terminator to the next 32-bit boundary.
    let padded = off.div_ceil(WORD_LEN) * WORD_LEN;
    if padded > bytes.len() {
        return Err(Error::BufferTooShort {
            need: padded,
            have: bytes.len(),
            what: "SDES chunk padding",
        });
    }
    Ok((SdesChunk { source, items }, padded))
}

impl Serialize for SourceDescription {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        RTCP_HEADER_LEN + self.chunks.iter().map(SdesChunk::padded_len).sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.chunks.len() > MAX_COUNT {
            return Err(Error::InvalidValue {
                field: "rtcp_source_count",
                value: self.chunks.len() as u64,
                reason: "exceeds 5-bit SC field",
            });
        }
        let hdr = CommonHeader::new(
            self.chunks.len() as u8,
            PT_SOURCE_DESCRIPTION,
            length_words_minus_one(len),
        );
        hdr.write(&mut buf[0..RTCP_HEADER_LEN]);
        let mut off = RTCP_HEADER_LEN;
        for chunk in &self.chunks {
            let padded = chunk.padded_len();
            // Zero the whole chunk region first so padding bytes are 0.
            for b in buf[off..off + padded].iter_mut() {
                *b = 0;
            }
            buf[off..off + 4].copy_from_slice(&chunk.source.to_be_bytes());
            let mut io = off + WORD_LEN;
            for item in &chunk.items {
                if item.text.len() > u8::MAX as usize {
                    return Err(Error::InvalidValue {
                        field: "sdes_item_len",
                        value: item.text.len() as u64,
                        reason: "exceeds 8-bit SDES item length",
                    });
                }
                buf[io] = item.item_type.item_type();
                buf[io + 1] = item.text.len() as u8;
                buf[io + 2..io + 2 + item.text.len()].copy_from_slice(item.text.as_bytes());
                io += 2 + item.text.len();
            }
            // buf[io] terminator (already zeroed); remaining padding zeroed.
            off += padded;
        }
        Ok(len)
    }
}

// ---------------------------------------------------------------------------
// Bye (RFC 3550 §6.6, PT 203)
// ---------------------------------------------------------------------------

/// RTCP Goodbye (RFC 3550 §6.6, PT 203).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Bye {
    /// The SSRC/CSRC sources leaving (`SC` of them).
    pub sources: Vec<u32>,
    /// Optional textual reason for leaving.
    pub reason: Option<String>,
}

impl Bye {
    /// Unpadded byte length of the reason field (length octet + text), if any.
    fn reason_unpadded_len(&self) -> usize {
        match &self.reason {
            Some(r) => 1 + r.len(),
            None => 0,
        }
    }
}

impl<'a> Parse<'a> for Bye {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let hdr = CommonHeader::parse(bytes)?;
        if hdr.packet_type != PT_BYE {
            return Err(Error::InvalidValue {
                field: "rtcp_pt",
                value: hdr.packet_type as u64,
                reason: "expected BYE (203)",
            });
        }
        let total = hdr.total_len();
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "RTCP BYE",
            });
        }
        let body = &bytes[RTCP_HEADER_LEN..total];
        let sc = hdr.count as usize;
        if body.len() < sc * WORD_LEN {
            return Err(Error::BufferTooShort {
                need: sc * WORD_LEN,
                have: body.len(),
                what: "RTCP BYE sources",
            });
        }
        let mut sources = Vec::with_capacity(sc);
        let mut off = 0;
        for _ in 0..sc {
            sources.push(be_u32(body, off, "BYE source")?);
            off += WORD_LEN;
        }
        // Optional reason: a length octet + text, if any bytes remain.
        let reason = if off < body.len() {
            let len = body[off] as usize;
            off += 1;
            let end = off + len;
            if end > body.len() {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: body.len(),
                    what: "RTCP BYE reason text",
                });
            }
            let text =
                String::from_utf8(body[off..end].to_vec()).map_err(|_| Error::InvalidValue {
                    field: "bye_reason",
                    value: 0,
                    reason: "not valid UTF-8",
                })?;
            Some(text)
        } else {
            None
        };
        Ok(Bye { sources, reason })
    }
}

impl Serialize for Bye {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let raw = RTCP_HEADER_LEN + self.sources.len() * WORD_LEN + self.reason_unpadded_len();
        raw.div_ceil(WORD_LEN) * WORD_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.sources.len() > MAX_COUNT {
            return Err(Error::InvalidValue {
                field: "rtcp_source_count",
                value: self.sources.len() as u64,
                reason: "exceeds 5-bit SC field",
            });
        }
        if let Some(r) = &self.reason {
            if r.len() > u8::MAX as usize {
                return Err(Error::InvalidValue {
                    field: "bye_reason_len",
                    value: r.len() as u64,
                    reason: "exceeds 8-bit reason length",
                });
            }
        }
        // Zero the whole region so trailing padding bytes are 0.
        for b in buf[..len].iter_mut() {
            *b = 0;
        }
        let hdr = CommonHeader::new(
            self.sources.len() as u8,
            PT_BYE,
            length_words_minus_one(len),
        );
        hdr.write(&mut buf[0..RTCP_HEADER_LEN]);
        let mut off = RTCP_HEADER_LEN;
        for src in &self.sources {
            buf[off..off + 4].copy_from_slice(&src.to_be_bytes());
            off += WORD_LEN;
        }
        if let Some(r) = &self.reason {
            buf[off] = r.len() as u8;
            off += 1;
            buf[off..off + r.len()].copy_from_slice(r.as_bytes());
        }
        Ok(len)
    }
}

// ---------------------------------------------------------------------------
// App (RFC 3550 §6.7, PT 204)
// ---------------------------------------------------------------------------

/// RTCP Application-defined packet (RFC 3550 §6.7, PT 204).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct App {
    /// Application subtype (carried in the header's `RC` field, 5 bits).
    pub subtype: u8,
    /// SSRC/CSRC of the source.
    pub ssrc: u32,
    /// The 4-byte ASCII application name.
    pub name: [u8; APP_NAME_LEN],
    /// Application-dependent data (must be a multiple of 4 bytes on the wire).
    pub data: Vec<u8>,
}

impl<'a> Parse<'a> for App {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let hdr = CommonHeader::parse(bytes)?;
        if hdr.packet_type != PT_APP {
            return Err(Error::InvalidValue {
                field: "rtcp_pt",
                value: hdr.packet_type as u64,
                reason: "expected APP (204)",
            });
        }
        let total = hdr.total_len();
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "RTCP APP",
            });
        }
        let body = &bytes[RTCP_HEADER_LEN..total];
        if body.len() < WORD_LEN + APP_NAME_LEN {
            return Err(Error::BufferTooShort {
                need: WORD_LEN + APP_NAME_LEN,
                have: body.len(),
                what: "RTCP APP ssrc+name",
            });
        }
        let ssrc = be_u32(body, 0, "APP ssrc")?;
        let mut name = [0u8; APP_NAME_LEN];
        name.copy_from_slice(&body[WORD_LEN..WORD_LEN + APP_NAME_LEN]);
        let data = body[WORD_LEN + APP_NAME_LEN..].to_vec();
        Ok(App {
            subtype: hdr.count,
            ssrc,
            name,
            data,
        })
    }
}

impl Serialize for App {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        RTCP_HEADER_LEN + WORD_LEN + APP_NAME_LEN + self.data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if len % WORD_LEN != 0 {
            return Err(Error::InvalidValue {
                field: "app_data_len",
                value: self.data.len() as u64,
                reason: "APP data must be 32-bit aligned",
            });
        }
        if self.subtype > RTCP_COUNT_MASK {
            return Err(Error::InvalidValue {
                field: "app_subtype",
                value: self.subtype as u64,
                reason: "exceeds 5-bit subtype field",
            });
        }
        let hdr = CommonHeader::new(self.subtype, PT_APP, length_words_minus_one(len));
        hdr.write(&mut buf[0..RTCP_HEADER_LEN]);
        let mut off = RTCP_HEADER_LEN;
        buf[off..off + 4].copy_from_slice(&self.ssrc.to_be_bytes());
        off += WORD_LEN;
        buf[off..off + APP_NAME_LEN].copy_from_slice(&self.name);
        off += APP_NAME_LEN;
        buf[off..off + self.data.len()].copy_from_slice(&self.data);
        Ok(len)
    }
}

// ---------------------------------------------------------------------------
// RtcpPacket — the dispatch enum
// ---------------------------------------------------------------------------

/// Any single RTCP packet, dispatched by its common-header `PT` byte
/// (RFC 3550 §6).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RtcpPacket {
    /// A Sender Report (PT 200).
    SenderReport(SenderReport),
    /// A Receiver Report (PT 201).
    ReceiverReport(ReceiverReport),
    /// A Source Description (PT 202).
    SourceDescription(SourceDescription),
    /// A Goodbye (PT 203).
    Bye(Bye),
    /// An Application-defined packet (PT 204).
    App(App),
}

impl RtcpPacket {
    /// The packet type of this packet.
    pub fn packet_type(&self) -> RtcpPacketType {
        match self {
            RtcpPacket::SenderReport(_) => RtcpPacketType::SenderReport,
            RtcpPacket::ReceiverReport(_) => RtcpPacketType::ReceiverReport,
            RtcpPacket::SourceDescription(_) => RtcpPacketType::SourceDescription,
            RtcpPacket::Bye(_) => RtcpPacketType::Bye,
            RtcpPacket::App(_) => RtcpPacketType::App,
        }
    }

    /// Spec token for this packet (`SR`/`RR`/`SDES`/`BYE`/`APP`).
    pub fn name(&self) -> &'static str {
        self.packet_type().name()
    }

    /// Whether this packet is a report (SR or RR) — the only valid *first*
    /// packet of a compound packet (RFC 3550 §6.1).
    fn is_report(&self) -> bool {
        matches!(
            self,
            RtcpPacket::SenderReport(_) | RtcpPacket::ReceiverReport(_)
        )
    }
}

broadcast_common::impl_spec_display!(RtcpPacket);

impl<'a> Parse<'a> for RtcpPacket {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let hdr = CommonHeader::parse(bytes)?;
        Ok(match RtcpPacketType::from_pt(hdr.packet_type) {
            RtcpPacketType::SenderReport => RtcpPacket::SenderReport(SenderReport::parse(bytes)?),
            RtcpPacketType::ReceiverReport => {
                RtcpPacket::ReceiverReport(ReceiverReport::parse(bytes)?)
            }
            RtcpPacketType::SourceDescription => {
                RtcpPacket::SourceDescription(SourceDescription::parse(bytes)?)
            }
            RtcpPacketType::Bye => RtcpPacket::Bye(Bye::parse(bytes)?),
            RtcpPacketType::App => RtcpPacket::App(App::parse(bytes)?),
            RtcpPacketType::Unknown(pt) => {
                return Err(Error::InvalidValue {
                    field: "rtcp_pt",
                    value: pt as u64,
                    reason: "not an RFC 3550 §6 core packet type",
                })
            }
        })
    }
}

impl Serialize for RtcpPacket {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            RtcpPacket::SenderReport(p) => p.serialized_len(),
            RtcpPacket::ReceiverReport(p) => p.serialized_len(),
            RtcpPacket::SourceDescription(p) => p.serialized_len(),
            RtcpPacket::Bye(p) => p.serialized_len(),
            RtcpPacket::App(p) => p.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            RtcpPacket::SenderReport(p) => p.serialize_into(buf),
            RtcpPacket::ReceiverReport(p) => p.serialize_into(buf),
            RtcpPacket::SourceDescription(p) => p.serialize_into(buf),
            RtcpPacket::Bye(p) => p.serialize_into(buf),
            RtcpPacket::App(p) => p.serialize_into(buf),
        }
    }
}

// ---------------------------------------------------------------------------
// CompoundPacket (RFC 3550 §6.1)
// ---------------------------------------------------------------------------

/// A compound RTCP packet (RFC 3550 §6.1): a sequence of RTCP packets sent in a
/// single lower-layer datagram. The first packet **must** be a report (SR or
/// RR); this is validated on both parse and serialize.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompoundPacket {
    /// The constituent packets, in wire order (first is SR/RR).
    pub packets: Vec<RtcpPacket>,
}

impl CompoundPacket {
    /// Build a compound packet, validating the §6.1 first-packet rule.
    pub fn new(packets: Vec<RtcpPacket>) -> Result<Self> {
        let cp = CompoundPacket { packets };
        cp.check_leading_report()?;
        Ok(cp)
    }

    /// Enforce RFC 3550 §6.1: a compound packet must start with SR or RR.
    fn check_leading_report(&self) -> Result<()> {
        match self.packets.first() {
            Some(p) if p.is_report() => Ok(()),
            Some(_) => Err(Error::InvalidValue {
                field: "rtcp_compound",
                value: self.packets[0].packet_type().pt() as u64,
                reason: "compound packet must begin with SR or RR (RFC 3550 §6.1)",
            }),
            None => Err(Error::InvalidInput("empty RTCP compound packet")),
        }
    }
}

impl<'a> Parse<'a> for CompoundPacket {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut packets = Vec::new();
        let mut off = 0;
        while off < bytes.len() {
            let hdr = CommonHeader::parse(&bytes[off..])?;
            let total = hdr.total_len();
            let end = off + total;
            if end > bytes.len() {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: bytes.len(),
                    what: "RTCP compound sub-packet",
                });
            }
            packets.push(RtcpPacket::parse(&bytes[off..end])?);
            off = end;
        }
        let cp = CompoundPacket { packets };
        cp.check_leading_report()?;
        Ok(cp)
    }
}

impl Serialize for CompoundPacket {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.packets.iter().map(RtcpPacket::serialized_len).sum()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.check_leading_report()?;
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let mut off = 0;
        for pkt in &self.packets {
            let n = pkt.serialize_into(&mut buf[off..])?;
            off += n;
        }
        Ok(off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn sample_block(ssrc: u32, jitter: u32, cumulative: i32) -> ReportBlock {
        ReportBlock {
            ssrc,
            fraction_lost: 12,
            cumulative_lost: cumulative,
            ext_highest_seq: 0x0001_2345,
            jitter,
            lsr: 0xAABB_CCDD,
            dlsr: 0x0000_1000,
        }
    }

    fn sample_sr() -> SenderReport {
        SenderReport {
            ssrc: 0x1122_3344,
            ntp_msw: 0xE0E1_E2E3,
            ntp_lsw: 0x1020_3040,
            rtp_timestamp: 0x0009_0000,
            packet_count: 4321,
            octet_count: 999_999,
            report_blocks: vec![
                sample_block(0xAAAA_AAAA, 500, 17),
                sample_block(0xBBBB_BBBB, 750, -3),
            ],
        }
    }

    #[test]
    fn report_block_round_trip() {
        let b = sample_block(0xDEAD_BEEF, 4242, -5);
        let bytes = b.to_bytes();
        assert_eq!(bytes.len(), REPORT_BLOCK_LEN);
        let parsed = ReportBlock::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn report_block_negative_cumulative_lost() {
        // Negative cumulative lost must survive the 24-bit signed field.
        for v in [-1_i32, -2, -100, -0x80_0000, 0, 5, 0x7F_FFFF] {
            let b = sample_block(1, 0, v);
            let parsed = ReportBlock::parse(&b.to_bytes()).unwrap();
            assert_eq!(parsed.cumulative_lost, v, "cumulative_lost {v} round-trip");
        }
    }

    #[test]
    fn sr_round_trip_and_header_layout() {
        let sr = sample_sr();
        let bytes = sr.to_bytes();
        // V=2 in top 2 bits, RC=2 in low 5 bits.
        assert_eq!(bytes[0] >> 6, 2);
        assert_eq!(bytes[0] & 0x1F, 2);
        // PT byte == 200.
        assert_eq!(bytes[1], PT_SENDER_REPORT);
        // length field == total_words − 1.
        let total_words = bytes.len() / 4;
        assert_eq!(
            u16::from_be_bytes([bytes[2], bytes[3]]) as usize,
            total_words - 1
        );
        let parsed = SenderReport::parse(&bytes).unwrap();
        assert_eq!(parsed, sr);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn sr_two_report_blocks_boundary() {
        let sr = sample_sr();
        assert_eq!(sr.report_blocks.len(), 2);
        let bytes = sr.to_bytes();
        // 4 (hdr) + 24 (sender info incl ssrc) + 2*24 = 76 bytes = 19 words.
        assert_eq!(bytes.len(), 4 + 24 + 2 * REPORT_BLOCK_LEN);
        assert_eq!(bytes.len() % 4, 0);
        let parsed = SenderReport::parse(&bytes).unwrap();
        assert_eq!(parsed.report_blocks.len(), 2);
        assert_eq!(
            u16::from_be_bytes([bytes[2], bytes[3]]) as usize,
            bytes.len() / 4 - 1
        );
    }

    #[test]
    fn rr_round_trip() {
        let rr = ReceiverReport {
            ssrc: 0x0102_0304,
            report_blocks: vec![sample_block(0xCAFE_BABE, 33, -7)],
        };
        let bytes = rr.to_bytes();
        assert_eq!(bytes[1], PT_RECEIVER_REPORT);
        let parsed = ReceiverReport::parse(&bytes).unwrap();
        assert_eq!(parsed, rr);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn sdes_round_trip_cname_tool() {
        let sdes = SourceDescription {
            chunks: vec![SdesChunk {
                source: 0x1234_5678,
                items: vec![
                    SdesItem {
                        item_type: SdesItemType::CName,
                        text: "alice@example.com".to_string(),
                    },
                    SdesItem {
                        item_type: SdesItemType::Tool,
                        text: "transmux/1.0".to_string(),
                    },
                ],
            }],
        };
        let bytes = sdes.to_bytes();
        assert_eq!(bytes[1], PT_SOURCE_DESCRIPTION);
        assert_eq!(bytes.len() % 4, 0);
        let parsed = SourceDescription::parse(&bytes).unwrap();
        assert_eq!(parsed, sdes);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn bye_round_trip_with_reason() {
        let bye = Bye {
            sources: vec![0x1111_1111, 0x2222_2222],
            reason: Some("teardown".to_string()),
        };
        let bytes = bye.to_bytes();
        assert_eq!(bytes[1], PT_BYE);
        assert_eq!(bytes[0] & 0x1F, 2); // SC = 2
        assert_eq!(bytes.len() % 4, 0);
        let parsed = Bye::parse(&bytes).unwrap();
        assert_eq!(parsed, bye);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn bye_round_trip_no_reason() {
        let bye = Bye {
            sources: vec![0xABCD_0000],
            reason: None,
        };
        let parsed = Bye::parse(&bye.to_bytes()).unwrap();
        assert_eq!(parsed, bye);
    }

    #[test]
    fn app_round_trip() {
        let app = App {
            subtype: 3,
            ssrc: 0x9988_7766,
            name: *b"TMUX",
            data: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04],
        };
        let bytes = app.to_bytes();
        assert_eq!(bytes[1], PT_APP);
        assert_eq!(bytes[0] & 0x1F, 3); // subtype in RC field
        let parsed = App::parse(&bytes).unwrap();
        assert_eq!(parsed, app);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn sr_mutation_bites_packet_count() {
        let sr = sample_sr();
        let mut bytes = sr.to_bytes();
        let orig = SenderReport::parse(&bytes).unwrap();
        // packet_count lives at offset 4(hdr)+16 = 20.
        let pc_off = RTCP_HEADER_LEN + 16;
        bytes[pc_off] ^= 0xFF;
        let mutated = SenderReport::parse(&bytes).unwrap();
        assert_ne!(mutated.packet_count, orig.packet_count);
        // The mutated value re-serializes to the mutated bytes.
        assert_eq!(mutated.to_bytes(), bytes);
    }

    #[test]
    fn report_block_mutation_bites_jitter() {
        let mut sr = sample_sr();
        sr.report_blocks[0].jitter = 500;
        let before = sr.to_bytes();
        sr.report_blocks[0].jitter = 999;
        let after = sr.to_bytes();
        assert_ne!(before, after);
        let parsed = SenderReport::parse(&after).unwrap();
        assert_eq!(parsed.report_blocks[0].jitter, 999);
    }

    #[test]
    fn compound_sr_sdes_round_trip() {
        let sdes = SourceDescription {
            chunks: vec![SdesChunk {
                source: 0x1122_3344,
                items: vec![SdesItem {
                    item_type: SdesItemType::CName,
                    text: "cn".to_string(),
                }],
            }],
        };
        let cp = CompoundPacket::new(vec![
            RtcpPacket::SenderReport(sample_sr()),
            RtcpPacket::SourceDescription(sdes),
        ])
        .unwrap();
        let bytes = cp.to_bytes();
        let parsed = CompoundPacket::parse(&bytes).unwrap();
        assert_eq!(parsed.packets.len(), 2);
        assert_eq!(parsed, cp);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn compound_must_start_with_report() {
        // A BYE-first compound is rejected on construction.
        let err = CompoundPacket::new(vec![RtcpPacket::Bye(Bye {
            sources: vec![1],
            reason: None,
        })]);
        assert!(err.is_err());
        // And on parse: hand-build a BYE packet and try to parse as compound.
        let bye = Bye {
            sources: vec![1],
            reason: None,
        };
        let bytes = bye.to_bytes();
        assert!(CompoundPacket::parse(&bytes).is_err());
    }

    #[test]
    fn any_packet_dispatch() {
        let bytes = sample_sr().to_bytes();
        let any = RtcpPacket::parse(&bytes).unwrap();
        assert_eq!(any.packet_type(), RtcpPacketType::SenderReport);
        assert_eq!(any.name(), "SR");
        assert_eq!(any.to_bytes(), bytes);
    }

    #[test]
    fn packet_type_display() {
        assert_eq!(RtcpPacketType::SenderReport.to_string(), "SR");
        assert_eq!(RtcpPacketType::Unknown(207).to_string(), "reserved(0xCF)");
        assert_eq!(SdesItemType::CName.to_string(), "CNAME");
    }

    #[test]
    fn rejects_bad_version() {
        let mut bytes = sample_sr().to_bytes();
        bytes[0] = 0x40; // V=1
        assert!(SenderReport::parse(&bytes).is_err());
    }
}
