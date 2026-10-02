//! RIST compound RTCP packet builders — VSF TR-06-1:2020 §5.2.1.
//!
//! RIST mandates that RTCP compound packets follow the RFC 3550 §6.1 structure
//! (SR or RR first, then SDES) with RIST-specific extensions appended:
//! retransmission NACKs and RTT Echo messages.
//!
//! - [`RistSenderCompound`] — sender-side compound: [`ReportPart::Sr`] (or
//!   an empty [`ReportPart::Rr`] per TR-06-1 §5.2.3) + SDES(CNAME) +
//!   optional RTT Echo.
//! - [`RistReceiverCompound`] — receiver-side compound: [`ReportPart::Rr`] +
//!   SDES(CNAME) + optional Generic/Range NACKs + optional RTT Echo.
//!
//! Both types implement [`Parse`]/[`Serialize`] for byte-exact round-trip.
//! The leading report is a typed [`ReportPart`]; the RTCP padding bit (P,
//! RFC 3550 §6.4.1) is honoured — on parse the padding count (last byte of
//! the sub-packet) is validated and the padding region preserved, on
//! serialize it is written back byte-identically; and sub-packets this crate
//! does not model are preserved verbatim as [`UnknownPacket`] instead of
//! being dropped (RFC 5506 forwarding of unrecognized RTCP packets).

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};
use rtcp_packet::{
    PT_RECEIVER_REPORT, PT_SENDER_REPORT, PT_SOURCE_DESCRIPTION, ReceiverReport, SdesChunk,
    SdesItem, SdesItemType, SenderReport, SourceDescription,
};

use crate::error::{Error, Result};
use crate::nack::{GenericNack, RangeNack};
use crate::rtt_echo::RttEcho;
use crate::{
    FMT_GENERIC_NACK, PT_RTPFB, RTCP_COUNT_MASK, SUBTYPE_RANGE_NACK, SUBTYPE_RTT_ECHO_REQUEST,
    SUBTYPE_RTT_ECHO_RESPONSE,
};

// ---------------------------------------------------------------------------
// Wire constants
// ---------------------------------------------------------------------------

/// Common-header length in bytes.
const RTCP_HEADER_LEN: usize = 4;
/// One 32-bit word, in bytes.
const WORD_LEN: usize = 4;
/// PT for RTCP APP (RFC 3550 §6.7).
const PT_APP: u8 = 204;
/// RTCP protocol version — always 2 (RFC 3550 §6.4.1).
const RTCP_VERSION: u8 = 2;
/// Padding-bit mask within byte 0 (`P` — RFC 3550 §6.4.1): the bit right
/// after the 2-bit version field.
const RTCP_PADDING_MASK: u8 = 0x20;
/// Byte offset of the packet-type field within the common header.
const PT_OFFSET: usize = 1;

// ---------------------------------------------------------------------------
// Common-header helpers (shared by every sub-packet in a compound)
// ---------------------------------------------------------------------------

/// Read the total wire length (bytes) of the RTCP sub-packet at the front of
/// `bytes`, from its 4-byte common header `length` field (RFC 3550 §6.1):
/// `(length + 1) * 4`.
fn peek_total_len(bytes: &[u8]) -> Result<usize> {
    if bytes.len() < RTCP_HEADER_LEN {
        return Err(Error::BufferTooShort {
            need: RTCP_HEADER_LEN,
            have: bytes.len(),
        });
    }
    let length_field = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    let total = length_field
        .checked_add(1)
        .and_then(|words| words.checked_mul(WORD_LEN))
        .ok_or(Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
        })?;
    // `bytes` is the *rest of the compound* when walking, so a sub-packet is
    // valid as long as the compound has at least its minimum content (the
    // 4-byte common header); the exact fit is enforced per-type afterwards
    // (typed parsers require their own full length, and the walk's final
    // `TrailingData`/`MissingCname` checks reject malformed compounds whose
    // lengths do not add up).
    if bytes.len() < total {
        return Err(Error::BufferTooShort {
            need: total,
            have: bytes.len(),
        });
    }
    let version = bytes[0] >> 6;
    if version != RTCP_VERSION {
        return Err(Error::InvalidVersion(version));
    }
    Ok(total)
}

/// Decode and validate the common header of the sub-packet at the front of
/// `bytes` (RFC 3550 §6.4.1).
///
/// Returns `(padding count, packet type, total sub-packet length)`. When the
/// P bit is set the last byte of the sub-packet gives the padding count: the
/// number of bytes to strip from the end of the body. A padding count of 0
/// (a set P bit must carry at least one padding byte) or one larger than the
/// body (everything after the common header) is rejected with
/// [`Error::InvalidPaddingCount`].
fn header_of(bytes: &[u8]) -> Result<(usize, u8, usize)> {
    let total = peek_total_len(bytes)?;
    let packet_type = bytes[PT_OFFSET];
    let padding = if bytes[0] & RTCP_PADDING_MASK != 0 {
        let body = total - RTCP_HEADER_LEN;
        let count = bytes[total - 1] as usize;
        if count == 0 || count > body {
            return Err(Error::InvalidPaddingCount { count, body });
        }
        count
    } else {
        0
    };
    Ok((padding, packet_type, total))
}

/// The parsed view of a sub-packet's common header, with the wire offsets
/// the padding region lives at.
#[derive(Debug, Clone, Copy)]
struct Head {
    /// The 5-bit count/FMT/subtype value from the low bits of byte 0.
    count: u8,
    /// The wire `PT` byte.
    packet_type: u8,
    /// Total sub-packet length in bytes (including padding).
    total: usize,
    /// Padding count in bytes (0 when the P bit was clear); includes the
    /// trailing count byte.
    padding: usize,
}

impl Head {
    /// Parse the header of the sub-packet at the front of `bytes`.
    fn parse(bytes: &[u8]) -> Result<Self> {
        let (padding, packet_type, total) = header_of(bytes)?;
        Ok(Head {
            count: bytes[0] & RTCP_COUNT_MASK,
            packet_type,
            total,
            padding,
        })
    }

    /// End of the unpadded body (start of the padding region, or the count
    /// byte when there is none).
    fn unpadded_end(&self) -> usize {
        self.total - self.padding
    }
}

/// Write a 4-byte common header for a sub-packet of `total` bytes, with
/// `byte0_low` in the low 5 bits of byte 0. When `padding > 0` the P bit is
/// set and the padding-count byte is written as the packet's last byte; the
/// padding region *before* it is left for the caller to copy in.
fn write_header(
    buf: &mut [u8],
    total: usize,
    byte0_low: u8,
    packet_type: u8,
    padding: usize,
) -> Result<usize> {
    if buf.len() < total {
        return Err(Error::OutputBufferTooSmall {
            need: total,
            have: buf.len(),
        });
    }
    let length_words =
        broadcast_common::len::fit_u16(total / WORD_LEN - 1, "rtcp common-header length field")?;
    let p_bit = if padding > 0 { RTCP_PADDING_MASK } else { 0 };
    buf[0] = (RTCP_VERSION << 6) | p_bit | byte0_low;
    buf[PT_OFFSET] = packet_type;
    buf[2..4].copy_from_slice(&length_words.to_be_bytes());
    if padding > 0 {
        buf[total - 1] = broadcast_common::len::fit_u8(padding, "rtcp padding count")?;
    }
    Ok(RTCP_HEADER_LEN)
}

/// Parse one sub-packet body (already isolated from any padding). Works for
/// any sub-packet type, whether its `Parse::Error` is `rist_runtime::Error`
/// (the RIST-specific types) or `rtcp_packet::Error` (the underlying
/// SR/RR/SDES types) — both convert into [`Error`] via `?`.
fn parse_body<'a, T>(bytes: &'a [u8]) -> Result<T>
where
    T: Parse<'a>,
    Error: From<T::Error>,
{
    T::parse(bytes).map_err(Error::from)
}

// ---------------------------------------------------------------------------
// ReportPart — the SR/RR packet that opens a compound (RFC 3550 §6.1)
// ---------------------------------------------------------------------------

/// The Sender Report / Receiver Report packet that opens a RIST compound
/// packet (TR-06-1 §5.2.1, RFC 3550 §6.1: SR or RR must come first).
///
/// A sender compound may carry either variant (SR, or an empty RR per
/// TR-06-1 §5.2.3); a receiver compound always carries [`ReportPart::Rr`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReportPart {
    /// Sender Report (RFC 3550 §6.4.1, PT 200).
    Sr(SenderReport),
    /// Receiver Report (RFC 3550 §6.4.2, PT 201) — empty (RC = 0) or with
    /// report blocks.
    Rr(ReceiverReport),
}

impl ReportPart {
    /// SSRC of the packet sender originating this report.
    pub fn ssrc(&self) -> u32 {
        match self {
            ReportPart::Sr(sr) => sr.ssrc,
            ReportPart::Rr(rr) => rr.ssrc,
        }
    }

    /// Spec token.
    pub fn name(&self) -> &'static str {
        match self {
            ReportPart::Sr(_) => "SR",
            ReportPart::Rr(_) => "RR",
        }
    }

    /// The wire `PT` byte this report serializes to.
    fn packet_type(&self) -> u8 {
        match self {
            ReportPart::Sr(_) => PT_SENDER_REPORT,
            ReportPart::Rr(_) => PT_RECEIVER_REPORT,
        }
    }

    /// Length of the report sub-packet's unpadded body, including the common
    /// header.
    fn unpadded_len(&self) -> usize {
        match self {
            ReportPart::Sr(sr) => sr.serialized_len(),
            ReportPart::Rr(rr) => rr.serialized_len(),
        }
    }

    /// Total wire length of the report sub-packet, including the preserved
    /// padding region (`padding` excludes the trailing count byte).
    fn wire_len(&self, padding: &[u8]) -> usize {
        self.unpadded_len() + RistPadding::new(padding).wire_len()
    }

    /// Parse the report from the sub-packet at the front of `bytes`
    /// (`head.packet_type` selects SR or RR), returning it together with its
    /// preserved padding region (empty when the P bit was clear).
    ///
    /// rtcp-packet's report parsers accept any sub-packet at least as long
    /// as the report's own content, so the region between the parsed value
    /// and the padding count byte is this crate's to validate: RFC 3550
    /// §6.4.1 fixes the padding bytes to zero (only the trailing count byte
    /// is meaningful), so a non-zero fill there is rejected rather than
    /// silently reproduced.
    fn parse(bytes: &[u8], head: Head) -> Result<(Self, Vec<u8>)> {
        let unpadded_end = head.unpadded_end();
        // RFC 3550 §6.4.1: the P-bit count covers the padding *and itself*,
        // so the count byte is part of the padding, not of the report body.
        // rtcp-packet's parsers require the full claimed sub-packet length,
        // so they see the whole sub-packet; the preserved region below is
        // then everything the count accounted for past the parsed value.
        let parse_end = if head.padding != 0 {
            &bytes[..head.total]
        } else {
            &bytes[..unpadded_end]
        };
        match head.packet_type {
            PT_SENDER_REPORT => {
                let sr = parse_body::<SenderReport>(parse_end)?;
                let tail = if head.padding == 0 {
                    Vec::new()
                } else {
                    bytes[sr.serialized_len()..head.total - 1].to_vec()
                };
                if head.padding != 0 && tail.iter().any(|&b| b != 0) {
                    return Err(Error::InvalidPaddingCount {
                        count: head.padding,
                        body: unpadded_end - RTCP_HEADER_LEN,
                    });
                }
                Ok((ReportPart::Sr(sr), tail))
            }
            PT_RECEIVER_REPORT => {
                let rr = parse_body::<ReceiverReport>(parse_end)?;
                let tail = if head.padding == 0 {
                    Vec::new()
                } else {
                    bytes[rr.serialized_len()..head.total - 1].to_vec()
                };
                if head.padding != 0 && tail.iter().any(|&b| b != 0) {
                    return Err(Error::InvalidPaddingCount {
                        count: head.padding,
                        body: unpadded_end - RTCP_HEADER_LEN,
                    });
                }
                Ok((ReportPart::Rr(rr), tail))
            }
            other => Err(Error::UnexpectedPacketType(other)),
        }
    }

    /// Serialize the report followed by the preserved `padding` region.
    ///
    /// rtcp-packet's report serializers write their own common header, so
    /// the value is serialized into a temporary of its exact length and the
    /// body is copied into `buf` after our [`write_header`] header; the
    /// padding region (zero-fill, guaranteed by [`ReportPart::parse`]) and
    /// its count byte follow. Handing the serializers a mid-buffer slice
    /// instead would fail their `buf.len()`-vs-own-length checks, and
    /// rtcp-packet's SR additionally writes its fixed sender-info block by
    /// position, reaching 4 bytes past the 28 its `serialized_len()` admits
    /// when `RC = 0`.
    fn serialize_with_padding(&self, padding: &[u8], buf: &mut [u8]) -> Result<usize> {
        let rist_padding = RistPadding::new(padding);
        let total = self.wire_len(padding);
        let count = rist_padding.count().unwrap_or(0);
        // Serialize the value standalone (with its own common header — see
        // the fn doc for why), into the *front* of `buf`, then rewrite the
        // first four bytes in place so the value's header becomes ours
        // (which it already equals when the value carries no real report
        // blocks: rtcp-packet's RC-less header is byte-identical to the
        // header `write_header` writes for a padding-free RR/SR).
        match self {
            ReportPart::Sr(sr) => {
                sr.serialize_into(buf).map_err(Error::Rtcp)?;
            }
            ReportPart::Rr(rr) => {
                rr.serialize_into(buf).map_err(Error::Rtcp)?;
            }
        };
        let value_end = self.unpadded_len();
        // Rewrite the common header for the padded wire form (P bit, real
        // count, sub-packet length field including the count byte).
        let rc = match self {
            ReportPart::Sr(sr) => sr.report_blocks.len(),
            ReportPart::Rr(rr) => rr.report_blocks.len(),
        };
        let byte0_low = broadcast_common::len::fit_u8(rc, "rtcp report count")?;
        write_header(buf, total, byte0_low, self.packet_type(), count)?;
        rist_padding.write_into(&mut buf[value_end..])?;
        Ok(total)
    }
}

broadcast_common::impl_spec_display!(ReportPart);

// ---------------------------------------------------------------------------
// RistPadding — the preserved padding region of one sub-packet
// ---------------------------------------------------------------------------

/// The preserved padding region of one sub-packet: everything the P bit
/// accounts for except the trailing count byte, which RFC 3550 §6.4.1 puts
/// at the very end and which this type re-derives on serialize.
///
/// Note the deliberate wire asymmetry with RTT Echo's own opaque padding
/// (TR-06-1 §5.2.6, arbitrary content, no count byte): this is the generic
/// RTCP P-bit padding, which *does* carry the count byte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RistPadding {
    bytes: Vec<u8>,
}

impl RistPadding {
    /// Wrap a preserved padding region (excluding the count byte). An empty
    /// region means the P bit was clear.
    pub fn new(bytes: &[u8]) -> Self {
        RistPadding {
            bytes: bytes.to_vec(),
        }
    }

    /// No padding (P bit clear).
    fn empty() -> Self {
        RistPadding { bytes: Vec::new() }
    }

    /// The preserved padding bytes (without the count byte).
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Is the region empty (P bit clear on serialize)?
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The wire padding count (region plus the count byte), or `None` when
    /// there is no padding.
    fn count(&self) -> Option<usize> {
        if self.bytes.is_empty() {
            None
        } else {
            Some(self.bytes.len() + 1)
        }
    }

    /// Total wire bytes this padding occupies (region plus count byte).
    fn wire_len(&self) -> usize {
        self.count().unwrap_or(0)
    }

    /// Write the preserved region (the count byte itself is written by
    /// [`write_header`] as the packet's last byte).
    fn write_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < self.bytes.len() {
            return Err(Error::OutputBufferTooSmall {
                need: self.bytes.len(),
                have: buf.len(),
            });
        }
        buf[..self.bytes.len()].copy_from_slice(&self.bytes);
        Ok(self.bytes.len())
    }

    /// Recover the region from the sub-packet at the front of `bytes`
    /// (everything between the unpadded body end and the count byte).
    fn extract(bytes: &[u8], head: Head, unpadded_body_end: usize) -> Self {
        if head.padding == 0 {
            RistPadding::empty()
        } else {
            RistPadding::new(&bytes[unpadded_body_end..head.total - 1])
        }
    }
}

// ---------------------------------------------------------------------------
// SDES(CNAME) helpers
// ---------------------------------------------------------------------------

/// Build an SDES packet containing a single chunk with one CNAME item.
fn build_sdes(ssrc: u32, cname: &str) -> SourceDescription {
    SourceDescription {
        chunks: alloc::vec![SdesChunk {
            source: ssrc,
            items: alloc::vec![SdesItem {
                item_type: SdesItemType::CName,
                text: String::from(cname),
            }],
        }],
    }
}

/// Extract the CNAME text from a parsed SDES packet — every RIST compound
/// packet carries exactly one (TR-06-1:2020 §5.2.1).
fn extract_cname(sdes: &SourceDescription) -> Result<String> {
    sdes.chunks
        .iter()
        .flat_map(|chunk| chunk.items.iter())
        .find(|item| item.item_type == SdesItemType::CName)
        .map(|item| item.text.clone())
        .ok_or(Error::MissingCname)
}

// ---------------------------------------------------------------------------
// TrailingSlot — a non-report sub-packet preserved at its wire position
// ---------------------------------------------------------------------------

/// The common-header count value for the SDES(CNAME) the compound rebuilds:
/// exactly one source chunk.
const SDES_SOURCE_COUNT: u8 = 1;

/// A non-report sub-packet: its value and the position (in modelled-sub-packet
/// order, starting at 0 right after the report) it occupied on the wire, so
/// serialize reproduces the original interleaving of modelled and unknown
/// packets. `None` means "hand-built, not parsed": appended after the parsed
/// ones in natural order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPacket {
    /// The wire `PT` byte from the common header.
    pub packet_type: u8,
    /// The 5-bit count/FMT/subtype value from the low bits of header byte 0.
    pub count: u8,
    /// Everything between the common header and any padding, verbatim.
    pub payload: Vec<u8>,
    /// The preserved padding region *excluding* the trailing count byte
    /// (which is re-derived from the region's length on serialize).
    pub padding: Vec<u8>,
    /// Wire position among the compound's trailing sub-packets.
    pub position: Option<usize>,
}

impl UnknownPacket {
    /// Spec token — an unmodelled packet type has no spec name here.
    pub fn name(&self) -> &'static str {
        "reserved"
    }

    /// Wrap the raw wire bytes of the sub-packet at `position`, splitting off
    /// the padding region named by `head` (everything after the unpadded body
    /// except the trailing count byte, which is re-derived on serialize).
    fn from_bytes(bytes: &[u8], head: Head, position: usize) -> Self {
        let payload_end = head.unpadded_end();
        UnknownPacket {
            packet_type: head.packet_type,
            count: head.count,
            payload: bytes[RTCP_HEADER_LEN..payload_end].to_vec(),
            padding: RistPadding::extract(bytes, head, payload_end).bytes,
            position: Some(position),
        }
    }

    /// Total wire length of this unknown sub-packet: common header + payload
    /// + the preserved padding region + the count byte.
    fn wire_len(&self) -> usize {
        RTCP_HEADER_LEN
            + self.payload.len()
            + self.padding.len()
            + usize::from(!self.padding.is_empty())
    }

    /// Write header (P bit + count byte), payload, then preserved padding.
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.wire_len();
        let count = self.padding.len() + usize::from(!self.padding.is_empty());
        let off = write_header(buf, total, self.count, self.packet_type, count)?;
        let body_end = total - count;
        buf[off..body_end].copy_from_slice(&self.payload);
        if count > 0 {
            buf[body_end..total - 1].copy_from_slice(&self.padding);
        }
        Ok(total)
    }
}

/// A trailing sub-packet slot: the typed value (or preserved unknown) at its
/// wire position (index after the report; `None` = hand-built, appended),
/// plus the preserved padding region when the modelled type cannot carry the
/// padding inside its own serialization (the `Unknown` variant keeps its
/// padding internally instead).
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrailingSlot {
    position: Option<usize>,
    value: TrailingValue,
    padding: Vec<u8>,
}

impl TrailingSlot {
    fn padding_len(&self) -> usize {
        self.padding.len() + usize::from(!self.padding.is_empty())
    }
}

/// The value of a trailing sub-packet of a receiver compound.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TrailingValue {
    Sdes(SourceDescription),
    GenericNack(GenericNack),
    RangeNack(RangeNack),
    RttEcho(RttEcho),
    Unknown(UnknownPacket),
}

impl TrailingValue {
    /// Length of this value's own serialization (common header included).
    /// Padding preserved outside the value (see [`TrailingSlot::padding`])
    /// is not counted here.
    fn body_len(&self) -> usize {
        match self {
            TrailingValue::Sdes(sdes) => sdes.serialized_len(),
            TrailingValue::GenericNack(nack) => nack.serialized_len(),
            TrailingValue::RangeNack(rn) => rn.serialized_len(),
            TrailingValue::RttEcho(echo) => echo.serialized_len(),
            TrailingValue::Unknown(unk) => RTCP_HEADER_LEN + unk.payload.len(),
        }
    }

    fn packet_type(&self) -> u8 {
        match self {
            TrailingValue::Sdes(_) => PT_SOURCE_DESCRIPTION,
            TrailingValue::GenericNack(_) => PT_RTPFB,
            TrailingValue::RangeNack(_) | TrailingValue::RttEcho(_) => PT_APP,
            TrailingValue::Unknown(unk) => unk.packet_type,
        }
    }

    fn byte0_low(&self) -> u8 {
        match self {
            TrailingValue::Sdes(_) => SDES_SOURCE_COUNT,
            TrailingValue::GenericNack(_) => FMT_GENERIC_NACK,
            TrailingValue::RangeNack(_) => SUBTYPE_RANGE_NACK,
            TrailingValue::RttEcho(echo) => echo.kind.subtype(),
            TrailingValue::Unknown(unk) => unk.count,
        }
    }
}

impl TrailingSlot {
    /// Total wire length: value body + preserved padding region + count byte.
    /// The `Unknown` variant carries its padding internally, so its own wire
    /// length already includes the padding region and count byte.
    fn wire_len(&self) -> usize {
        if let TrailingValue::Unknown(unk) = &self.value {
            debug_assert!(self.padding.is_empty());
            return unk.wire_len();
        }
        self.value.body_len() + self.padding.len() + usize::from(!self.padding.is_empty())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if let TrailingValue::Unknown(unk) = &self.value {
            debug_assert!(self.padding.is_empty());
            return unk.serialize_into(buf);
        }
        let total = self.wire_len();
        let count = self.padding_len();
        let byte0_low = self.value.byte0_low();
        let packet_type = self.value.packet_type();
        // Hand the typed serializer the tail slice starting at the slot:
        // every value writes its own common header at the slice start, and
        // its buffer check (`buf.len() >= serialized_len()`) only holds for
        // such a slice, not for a body-bounded sub-slice (the header is part
        // of the value's own length). It writes `P = 0` and its own length
        // word, so `write_header` afterwards restores the real common header
        // (P bit + padding count byte for the padding case).
        let written = match &self.value {
            TrailingValue::Sdes(sdes) => sdes.serialize_into(buf).map_err(Error::Rtcp)?,
            TrailingValue::GenericNack(nack) => nack.serialize_into(buf)?,
            TrailingValue::RangeNack(rn) => rn.serialize_into(buf)?,
            TrailingValue::RttEcho(echo) => echo.serialize_into(buf)?,
            TrailingValue::Unknown(unk) => {
                return Err(Error::UnexpectedPacketType(unk.packet_type));
            }
        };
        write_header(buf, total, byte0_low, packet_type, count)?;
        if count > 0 {
            // `written` is the unpadded body end; the preserved padding
            // region (minus the count byte) sits between it and the last
            // byte, which `write_header` already wrote.
            buf[written..total - 1].copy_from_slice(&self.padding);
        }
        Ok(total)
    }

    /// Sort key: parsed slots by their recorded wire position, hand-built
    /// slots (`None`) appended after them.
    fn sort_key(&self) -> usize {
        self.position.unwrap_or(usize::MAX)
    }
}

/// The classified parts of a compound packet: the mandatory leading report
/// with its preserved padding, and every following sub-packet as a slot at
/// its wire position.
struct CompoundParts {
    report: ReportPart,
    report_padding: Vec<u8>,
    trailing: Vec<TrailingSlot>,
}

/// Classify the next sub-packet at the front of `rest`, consuming the number
/// of bytes its own common header claims, and preserve its padding region.
///
/// When `receiver` is true the receiver-side RIST types (Generic NACK,
/// Range/RTT-Echo APP subtypes) are decoded, and an unexpected FMT/subtype
/// for those PTs is an error. Any other PT — including an SR/RR appearing
/// after the first position — is preserved verbatim as [`UnknownPacket`] so
/// the compound still round-trips.
/// Is this (PT, FMT/subtype) decoded into a modelled type? `receiver` gates
/// the RIST-side types (NACKs, RTT Echo); the SR/RR PTs are handled as the
/// compound's leading report, not here.
fn modelled(packet_type: u8, receiver: bool, count: u8) -> bool {
    match packet_type {
        PT_SOURCE_DESCRIPTION => true,
        PT_RTPFB => receiver && count == FMT_GENERIC_NACK,
        // RTT Echo (subtypes 2/3) is bidirectional — TR-06-1 §5.2.6 has the
        // sender embed a Request in its SR compound and the receiver answer
        // with a Response in its RR compound — so it is modelled in both
        // directions. Range NACK (subtype 0) is receiver-only.
        PT_APP => {
            matches!(count, SUBTYPE_RTT_ECHO_REQUEST | SUBTYPE_RTT_ECHO_RESPONSE)
                || (receiver && count == SUBTYPE_RANGE_NACK)
        }
        _ => false,
    }
}

fn next_sub_packet(
    rest: &[u8],
    receiver: bool,
    position: usize,
) -> Result<Option<(TrailingSlot, usize)>> {
    if rest.is_empty() {
        return Ok(None);
    }
    let head = Head::parse(rest)?;
    // Not a modelled type at this position: preserve the sub-packet verbatim,
    // padding and all (RIST-W6: unknown sub-packets round-trip unchanged).
    if !modelled(head.packet_type, receiver, head.count) {
        return Ok(Some((
            TrailingSlot {
                position: Some(position),
                value: TrailingValue::Unknown(UnknownPacket::from_bytes(rest, head, position)),
                padding: Vec::new(),
            },
            head.total,
        )));
    }
    if head.packet_type == PT_APP && head.count != SUBTYPE_RANGE_NACK {
        // RTT Echo: unlike every other modelled type, its padding is an
        // opaque field of its own message (TR-06-1 §5.2.6), not RFC 3550
        // padding, and `RttEcho::parse` reads the length from the buffer
        // behind the sub-packet. Pass the same tail `RistReceiverCompound`
        // passed before the slot rework: `rest` un-truncated.
        let echo = parse_body::<RttEcho>(rest)?;
        return Ok(Some((
            TrailingSlot {
                position: Some(position),
                value: TrailingValue::RttEcho(echo),
                padding: Vec::new(),
            },
            head.total,
        )));
    }

    let full_sub = &rest[..head.total];
    // Parse body: the sub-packet minus any P-bit padding, so a modelled
    // parser never sees padding bytes. RTT Echo is the exception — its
    // padding is its own opaque TR-06-1 §5.2.6 field, not RFC 3550 padding.
    let body = &full_sub[..head.unpadded_end()];
    let value = match head.packet_type {
        PT_SOURCE_DESCRIPTION => TrailingValue::Sdes(parse_body::<SourceDescription>(body)?),
        PT_RTPFB => {
            if head.count != FMT_GENERIC_NACK {
                return Err(Error::InvalidFmt {
                    expected: FMT_GENERIC_NACK,
                    got: head.count,
                });
            }
            TrailingValue::GenericNack(parse_body::<GenericNack>(body)?)
        }
        PT_APP if head.count == SUBTYPE_RANGE_NACK => {
            TrailingValue::RangeNack(parse_body::<RangeNack>(body)?)
        }
        _ => TrailingValue::RttEcho(parse_body::<RttEcho>(full_sub)?),
    };
    // P-bit padding of a modelled sub-packet is preserved in the slot and
    // written back verbatim (P bit + count byte) on serialize, so a padded
    // sub-packet round-trips byte-identically (RIST-W5).
    let padding_bytes = RistPadding::extract(full_sub, head, value.body_len()).bytes;
    Ok(Some((
        TrailingSlot {
            position: Some(position),
            value,
            padding: padding_bytes,
        },
        head.total,
    )))
}
/// Walk a whole compound packet: the first sub-packet must be an SR or RR
/// (RFC 3550 §6.1), and a duplicate RTT Echo is rejected (TR-06-1:2020
/// §5.2.6 permits at most one per compound packet).
fn walk(bytes: &[u8], receiver: bool) -> Result<CompoundParts> {
    if bytes.len() < RTCP_HEADER_LEN {
        return Err(Error::BufferTooShort {
            need: RTCP_HEADER_LEN,
            have: bytes.len(),
        });
    }
    let head = Head::parse(bytes)?;
    let (report, report_padding) = ReportPart::parse(bytes, head)?;

    let mut trailing = Vec::new();
    let mut rtt_echo_seen = false;
    let mut sdes_seen = false;
    let mut off = head.total;
    while let Some((mut slot, total)) = next_sub_packet(&bytes[off..], receiver, trailing.len())? {
        if let TrailingValue::RttEcho(_) = slot.value {
            if rtt_echo_seen {
                return Err(Error::DuplicateRttEcho);
            }
            rtt_echo_seen = true;
        }
        if let TrailingValue::Sdes(_) = slot.value {
            if sdes_seen {
                // A second SDES is not modelled; preserve it verbatim.
                slot.value = TrailingValue::Unknown(unknown_from_slot(&slot)?);
                slot.padding = Vec::new();
            }
            sdes_seen = true;
        }
        trailing.push(slot);
        off += total;
    }
    Ok(CompoundParts {
        report,
        report_padding,
        trailing,
    })
}

/// The report's wire length (body plus preserved padding region).
fn report_wire_len(report: &ReportPart, report_padding: &[u8]) -> usize {
    report.wire_len(report_padding)
}

/// Preserve a trailing slot verbatim as an [`UnknownPacket`] at its recorded
/// position: for already-unknown values this is a clone; for modelled values
/// (duplicates, or values carrying padding the typed serializer cannot
/// reproduce) the slot is re-serialized into its exact wire bytes first.
fn unknown_from_slot(slot: &TrailingSlot) -> Result<UnknownPacket> {
    if let TrailingValue::Unknown(unk) = &slot.value {
        return Ok(unk.clone());
    }
    let mut buffer = alloc::vec![0u8; slot.wire_len()];
    slot.serialize_into(&mut buffer)?;
    let head = Head::parse(&buffer)?;
    let payload_end = head.unpadded_end();
    Ok(UnknownPacket {
        packet_type: head.packet_type,
        count: head.count,
        payload: buffer[RTCP_HEADER_LEN..payload_end].to_vec(),
        padding: buffer[payload_end..head.total - 1].to_vec(),
        position: slot.position,
    })
}

// ---------------------------------------------------------------------------
// RistSenderCompound
// ---------------------------------------------------------------------------

/// Build a RIST sender compound RTCP packet (TR-06-1 §5.2.1).
///
/// Structure: [`ReportPart`] (SR, or an empty RR per TR-06-1 §5.2.3) +
/// SDES(CNAME) + optional RTT Echo, followed by any sub-packets this crate
/// does not model (preserved verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RistSenderCompound {
    /// The Sender Report (or empty Receiver Report) opening the compound.
    pub report: ReportPart,
    /// Preserved padding region of the report sub-packet (P bit set on the
    /// wire), excluding the trailing count byte, which is re-derived on
    /// serialize. Empty when the P bit was clear.
    pub report_padding: Vec<u8>,
    /// The CNAME string for the SDES chunk.
    pub cname: String,
    /// Optional RTT Echo Request or Response.
    pub rtt_echo: Option<RttEcho>,
    /// Sub-packets this crate does not model, preserved verbatim in wire
    /// order.
    pub unknown: Vec<UnknownPacket>,
}

impl RistSenderCompound {
    /// The trailing sub-packets in wire order: the SDES(CNAME) rebuilt from
    /// `cname`, the optional RTT Echo, and every preserved unknown packet.
    /// Unknowns keep the wire positions they were parsed at; the modelled
    /// slots fill the remaining positions in canonical order (SDES first).
    fn trailing(&self) -> Vec<TrailingSlot> {
        let taken: Vec<usize> = self.unknown.iter().filter_map(|unk| unk.position).collect();
        let mut free = (0..).filter(|p| !taken.contains(p));
        let mut slots = alloc::vec![TrailingSlot {
            position: free.next(),
            value: TrailingValue::Sdes(build_sdes(self.report.ssrc(), &self.cname)),
            padding: Vec::new(),
        }];
        if let Some(ref echo) = self.rtt_echo {
            slots.push(TrailingSlot {
                position: free.next(),
                value: TrailingValue::RttEcho(echo.clone()),
                padding: Vec::new(),
            });
        }
        for unk in &self.unknown {
            slots.push(TrailingSlot {
                position: unk.position,
                value: TrailingValue::Unknown(unk.clone()),
                padding: Vec::new(),
            });
        }
        slots.sort_by_key(TrailingSlot::sort_key);
        slots
    }
}

impl Serialize for RistSenderCompound {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        report_wire_len(&self.report, &self.report_padding)
            + self
                .trailing()
                .iter()
                .map(TrailingSlot::wire_len)
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let mut off = 0;
        off += self
            .report
            .serialize_with_padding(&self.report_padding, &mut buf[off..])?;
        for slot in self.trailing() {
            off += slot.serialize_into(&mut buf[off..])?;
        }

        Ok(off)
    }
}

impl<'a> Parse<'a> for RistSenderCompound {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let parts = walk(bytes, false)?;
        let mut cname: Option<String> = None;
        let mut rtt_echo = None;
        let mut unknown = Vec::new();

        for slot in &parts.trailing {
            match &slot.value {
                TrailingValue::Sdes(sdes) => {
                    if cname.is_none() && slot.padding.is_empty() {
                        cname = Some(extract_cname(sdes)?);
                    } else {
                        unknown.push(unknown_from_slot(slot)?);
                    }
                }
                TrailingValue::RttEcho(echo) => {
                    if rtt_echo.is_none() {
                        rtt_echo = Some(echo.clone());
                    } else {
                        unknown.push(unknown_from_slot(slot)?);
                    }
                }
                _ => unknown.push(unknown_from_slot(slot)?),
            }
        }

        Ok(RistSenderCompound {
            report: parts.report,
            report_padding: parts.report_padding,
            cname: cname.ok_or(Error::MissingCname)?,
            rtt_echo,
            unknown,
        })
    }
}

// ---------------------------------------------------------------------------
// RistReceiverCompound
// ---------------------------------------------------------------------------

/// Build a RIST receiver compound RTCP packet (TR-06-1 §5.2.1).
///
/// Structure: [`ReportPart`] (RR with 0 or 1 report blocks) + SDES(CNAME) +
/// optional Generic/Range NACKs + optional RTT Echo. Sub-packets the crate
/// does not model — and any NACK/SDES/RTT-Echo carrying P-bit padding, which
/// is only preserved verbatim — are kept in `unknown` at their wire positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RistReceiverCompound {
    /// The Receiver Report opening the compound.
    pub report: ReportPart,
    /// Preserved padding region of the report sub-packet (P bit set on the
    /// wire), excluding the trailing count byte, which is re-derived on
    /// serialize. Empty when the P bit was clear.
    pub report_padding: Vec<u8>,
    /// The CNAME string for the SDES chunk.
    pub cname: String,
    /// Optional Generic NACKs (RFC 4585, PT 205).
    pub nacks: Vec<GenericNack>,
    /// Optional Range NACKs (RIST APP, PT 204).
    pub range_nacks: Vec<RangeNack>,
    /// Optional RTT Echo Request or Response.
    pub rtt_echo: Option<RttEcho>,
    /// Sub-packets this crate does not model (or modelled sub-packets
    /// carrying padding/duplicates), preserved verbatim at their wire
    /// positions.
    pub unknown: Vec<UnknownPacket>,
}

impl RistReceiverCompound {
    /// The trailing sub-packets in wire order: the SDES(CNAME) rebuilt from
    /// `cname`, the NACK/RTT-Echo slots, and every preserved unknown, each
    /// sorted by its recorded wire position.
    fn trailing(&self) -> Vec<TrailingSlot> {
        // Positions after the report: parsed unknowns keep the wire positions
        // they were walked at; the modelled slots (SDES, NACKs, RTT Echo)
        // fill the remaining positions in canonical order (SDES first, then
        // Generic NACKs, Range NACKs, RTT Echo).
        let taken: Vec<usize> = self.unknown.iter().filter_map(|unk| unk.position).collect();
        let mut free = (0..).filter(|p| !taken.contains(p));
        let mut slots = Vec::new();
        slots.push(TrailingSlot {
            position: free.next(),
            value: TrailingValue::Sdes(build_sdes(self.report.ssrc(), &self.cname)),
            padding: Vec::new(),
        });
        for nack in &self.nacks {
            slots.push(TrailingSlot {
                position: free.next(),
                value: TrailingValue::GenericNack(nack.clone()),
                padding: Vec::new(),
            });
        }
        for rn in &self.range_nacks {
            slots.push(TrailingSlot {
                position: free.next(),
                value: TrailingValue::RangeNack(rn.clone()),
                padding: Vec::new(),
            });
        }
        if let Some(ref echo) = self.rtt_echo {
            slots.push(TrailingSlot {
                position: free.next(),
                value: TrailingValue::RttEcho(echo.clone()),
                padding: Vec::new(),
            });
        }
        for unk in &self.unknown {
            slots.push(TrailingSlot {
                position: unk.position,
                value: TrailingValue::Unknown(unk.clone()),
                padding: Vec::new(),
            });
        }
        slots.sort_by_key(TrailingSlot::sort_key);
        slots
    }
}

impl Serialize for RistReceiverCompound {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        report_wire_len(&self.report, &self.report_padding)
            + self
                .trailing()
                .iter()
                .map(TrailingSlot::wire_len)
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let mut off = 0;
        off += self
            .report
            .serialize_with_padding(&self.report_padding, &mut buf[off..])?;
        for slot in self.trailing() {
            off += slot.serialize_into(&mut buf[off..])?;
        }
        Ok(off)
    }
}

impl<'a> Parse<'a> for RistReceiverCompound {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let parts = walk(bytes, true)?;
        let mut cname: Option<String> = None;
        let mut nacks = Vec::new();
        let mut range_nacks = Vec::new();
        let mut rtt_echo = None;
        let mut unknown = Vec::new();

        for slot in &parts.trailing {
            match &slot.value {
                TrailingValue::Sdes(sdes) => {
                    if cname.is_none() && slot.padding.is_empty() {
                        cname = Some(extract_cname(sdes)?);
                    } else {
                        unknown.push(unknown_from_slot(slot)?);
                    }
                }
                TrailingValue::GenericNack(nack) => {
                    if slot.padding.is_empty() {
                        nacks.push(nack.clone());
                    } else {
                        unknown.push(unknown_from_slot(slot)?);
                    }
                }
                TrailingValue::RangeNack(rn) => {
                    if slot.padding.is_empty() {
                        range_nacks.push(rn.clone());
                    } else {
                        unknown.push(unknown_from_slot(slot)?);
                    }
                }
                TrailingValue::RttEcho(echo) => {
                    if rtt_echo.is_some() {
                        unknown.push(unknown_from_slot(slot)?);
                    } else {
                        rtt_echo = Some(echo.clone());
                    }
                }
                TrailingValue::Unknown(_) => unknown.push(unknown_from_slot(slot)?),
            }
        }

        Ok(RistReceiverCompound {
            report: parts.report,
            report_padding: parts.report_padding,
            cname: cname.ok_or(Error::MissingCname)?,
            nacks,
            range_nacks,
            rtt_echo,
            unknown,
        })
    }
}
