//! RFC 8331 — RTP Payload for SMPTE ST 291 Ancillary Data (ST 2110-40) §2/§2.1.
//!
//! See `docs/anc_rtp_8331.md` for the full field-by-field transcription this
//! module implements (RTP-header semantics, the §2.1 payload header, and the
//! §3.1/§4 media-type/clock-rate constants) — `docs/anc_packet_291.md` for the
//! per-ANC-packet fields ([`crate::AncContent`]) this module reuses unchanged.
//!
//! This module implements only the RFC 8331 **payload** ([`AncRtpPayload`]):
//! the RTP fixed header (`V`/`P`/`X`/`CC`/`M`/`PT`/sequence number/timestamp/
//! SSRC, RFC 3550 §5.1) is the `rtp_packet` crate's [`rtp_packet::RtpPacket`]
//! — a full ANC-over-RTP packet is built by placing an [`AncRtpPayload`]'s
//! serialized bytes into `RtpPacket::payload` (see the `build_anc_rtp` /
//! `parse_anc_rtp` examples).

use alloc::vec::Vec;

use broadcast_common::bits::{BitReader, BitWriter};
use broadcast_common::{Parse, Serialize};
use rtp_packet::RtpPacket;

use crate::anc_content::{AncContent, check_field_width};
use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Named constants (no magic numbers) — RFC 8331 §2.1 / §3.1 / §4
// ---------------------------------------------------------------------------

/// Length of the RFC 8331 §2.1 payload header: `Extended Sequence Number`(16)
/// plus `Length`(16) plus `ANC_Count`(8) plus `F`(2) plus `reserved`(22),
/// i.e. 64 bits, 8 bytes.
pub const ANC_RTP_PAYLOAD_HEADER_LEN: usize = 8;

/// Maximum `ANC_Count` value — an 8-bit field; "a single ANC data RTP packet
/// payload cannot carry more than 255 ANC data packets" (§2.1).
const MAX_ANC_COUNT: usize = u8::MAX as usize;

/// Maximum `Length` value — a 16-bit field (§2.1).
const MAX_LENGTH: usize = u16::MAX as usize;

/// `F` field bit shift within payload-header byte 5 (top 2 bits; the low 6
/// bits are the top 6 bits of `reserved`, §2.1).
const F_SHIFT: u8 = 6;
/// Mask for `F`'s 2 bits once shifted down.
const F_MASK: u8 = 0b11;
/// Mask isolating the top-6-bits-of-`reserved` portion of byte 5.
const RESERVED_BYTE5_MASK: u8 = 0x3F;

/// `video/smpte291` — the RFC 8331 §3.1 media type + subtype
/// (`Type name: video`, `Subtype name: smpte291`).
pub const ANC_RTP_MEDIA_TYPE: &str = "video/smpte291";

/// Default RTP timestamp clock rate — 90 kHz, "Otherwise, a 90 kHz rate
/// SHOULD be used" (§3.1) when the ANC stream is not grouped with a specific
/// video stream at another rate. RFC 8331's own worked SDP example:
/// `a=rtpmap:112 smpte291/90000`.
pub const ANC_RTP_DEFAULT_CLOCK_RATE: u32 = 90_000;

// Field widths (bits) of the RFC 8331 §2.1 per-ANC-packet **placement**
// fields (`C`/`Line_Number`/`Horizontal_Offset`/`S`/`StreamNum`) — the
// content fields (`DID`..`Checksum_Word`) are shared, see `crate::anc_content`.
const W_C: u32 = 1;
const W_LINE_NUMBER: u32 = 11;
const W_HORIZONTAL_OFFSET: u32 = 12;
const W_S: u32 = 1;
const W_STREAM_NUM: u32 = 7;
/// Total per-ANC-packet placement bit width — `1+11+12+1+7 = 32`, i.e.
/// already 32-bit aligned on its own (word_align only has to account for the
/// variable-length content that follows).
const PLACEMENT_BITS: usize =
    (W_C + W_LINE_NUMBER + W_HORIZONTAL_OFFSET + W_S + W_STREAM_NUM) as usize;

/// Bits needed to pad `bits_so_far` up to the next 32-bit (word) boundary —
/// RFC 8331 §2.1 `word_align`: "enough '0' bits ... to complete the last
/// 32-bit word ... If ... already ... aligned with a word boundary, there is
/// no need to add any word alignment bits."
fn bits_to_word_boundary(bits_so_far: usize) -> u32 {
    let rem = bits_so_far % 32;
    if rem == 0 { 0 } else { (32 - rem) as u32 }
}

// ---------------------------------------------------------------------------
// FieldSense (`F`) — RFC 8331 §2.1, #204 label convention
// ---------------------------------------------------------------------------

/// `F` (2 bits) — field-sense signalling for the RTP timestamp in an
/// interlaced SDI raster (RFC 8331 §2.1).
///
/// `0b01` is stated by RFC 8331 to be "not valid", with receivers "SHOULD
/// ignore an ANC data packet with an F field value of 0b01 and SHOULD process
/// any other ANC data packets ... present". That is a per-packet **receiver**
/// recommendation, not a wire-validity rule for this stateless parser: per
/// this project's decode-completeness principle, parsing an `F` of `0b01`
/// **succeeds** as [`FieldSense::Invalid`] rather than being rejected —
/// leaving any SHOULD-ignore behavior to the caller (see `docs/anc_rtp_8331.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FieldSense {
    /// `0b00` — "either the video format is progressive or ... no field is
    /// specified".
    ProgressiveOrUnspecified,
    /// `0b01` — "not valid"; parses successfully (see the enum doc), the
    /// SHOULD-ignore recommendation is left to the caller.
    Invalid,
    /// `0b10` — "the timestamp refers to the first field of an interlaced
    /// video signal".
    Field1,
    /// `0b11` — "the timestamp refers to the second field of an interlaced
    /// video signal".
    Field2,
}

impl FieldSense {
    /// The spec token for this value (issue #204 label convention).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ProgressiveOrUnspecified => "progressive or unspecified",
            Self::Invalid => "invalid (0b01)",
            Self::Field1 => "field 1",
            Self::Field2 => "field 2",
        }
    }

    fn from_bits(bits: u8) -> Self {
        // `bits & F_MASK` (F_MASK == 0b11) is always in 0..=3, so 0b11 is the
        // only remaining case here — matched via `_` to avoid a panicking
        // catch-all.
        match bits & F_MASK {
            0b00 => Self::ProgressiveOrUnspecified,
            0b01 => Self::Invalid,
            0b10 => Self::Field1,
            _ => Self::Field2,
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::ProgressiveOrUnspecified => 0b00,
            Self::Invalid => 0b01,
            Self::Field1 => 0b10,
            Self::Field2 => 0b11,
        }
    }
}

broadcast_common::impl_spec_display!(FieldSense);

// ---------------------------------------------------------------------------
// RtpAncPacket — RFC 8331 §2.1 per-ANC-packet placement + shared content
// ---------------------------------------------------------------------------

/// One ANC data packet as carried in an RFC 8331 RTP payload: the five
/// RTP-specific placement fields (`C`/`Line_Number`/`Horizontal_Offset`/`S`/
/// `StreamNum`, §2.1) wrapping the transport-independent [`AncContent`]
/// (`DID`/`SDID`/`Data_Count`/`User_Data_Words`/`Checksum_Word`).
///
/// Compare `st291::AncPacket` (the ST 2038 MPEG-2 TS/PES transport), which has
/// only three placement fields (`c_not_y_channel_flag`/`line_number`/
/// `horizontal_offset`) and no `S`/`StreamNum` — the reason the two
/// transports need distinct wrapper types around the shared [`AncContent`]
/// core (issue #648).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RtpAncPacket {
    /// `C` (1 bit) — color-difference (`true`) vs luma/SD/unspecified
    /// (`false`) data channel (§2.1).
    pub c: bool,
    /// `Line_Number` (11 bits) — digital-interface line number, or one of the
    /// generic vertical-location special values `0x7FF`/`0x7FE`/`0x7FD`
    /// (§2.1).
    pub line_number: u16,
    /// `Horizontal_Offset` (12 bits) — location relative to SAV, or one of
    /// the generic horizontal-location special values `0xFFF`/`0xFFE`/
    /// `0xFFD`/`0xFFC` (§2.1).
    pub horizontal_offset: u16,
    /// `S` (Data Stream Flag, 1 bit) — whether `stream_num` carries source
    /// data-stream-number information (§2.1).
    pub s: bool,
    /// `StreamNum` (7 bits) — source data-stream number minus one (numbered
    /// interfaces), or `0`/`1` for link A/B or left/right-eye (unnumbered
    /// multi-stream interfaces); meaningful only when `s` is `true` (§2.1).
    pub stream_num: u8,
    /// The transport-independent ANC content (`DID`/`SDID`/`Data_Count`/
    /// `User_Data_Words`/`Checksum_Word`), identical to the ST 2038 transport
    /// — see [`AncContent`].
    pub content: AncContent,
}

impl RtpAncPacket {
    /// Bit width before `word_align` padding: the 32-bit placement prefix
    /// plus the content's bit width.
    fn body_bits(&self) -> usize {
        PLACEMENT_BITS + self.content.content_bit_width()
    }

    /// Serialized byte length of this record **including** its trailing
    /// `word_align` padding to the next 32-bit boundary.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        let bits = self.body_bits();
        (bits + bits_to_word_boundary(bits) as usize) / 8
    }

    /// Write this record (placement + content + `word_align`) into `w`.
    ///
    /// # Errors
    /// [`Error::FieldTooWide`] if a placement field exceeds its wire width;
    /// [`Error::InconsistentUdwLength`] if `content.user_data_words.len()`
    /// does not equal `content.data_count & 0xFF`.
    fn write_into(&self, w: &mut BitWriter<'_>) -> Result<()> {
        w.write_bool(self.c)?;
        w.write_bits(
            check_field_width("Line_Number", u64::from(self.line_number), W_LINE_NUMBER)?,
            W_LINE_NUMBER,
        )?;
        w.write_bits(
            check_field_width(
                "Horizontal_Offset",
                u64::from(self.horizontal_offset),
                W_HORIZONTAL_OFFSET,
            )?,
            W_HORIZONTAL_OFFSET,
        )?;
        w.write_bool(self.s)?;
        w.write_bits(
            check_field_width("StreamNum", u64::from(self.stream_num), W_STREAM_NUM)?,
            W_STREAM_NUM,
        )?;
        self.content.write_into(w)?;
        let pad = bits_to_word_boundary(w.bits_written());
        w.write_bits(0, pad)?;
        Ok(())
    }

    /// Read one record (placement + content + `word_align`) from `r`.
    ///
    /// # Errors
    /// [`Error::ReservedNotZero`] if the `word_align` padding is not all-zero
    /// (RFC 8331 §2.1 states its value is "0" bits); a bit-stream error if
    /// `r` runs out of bits mid-record.
    fn read_from(r: &mut BitReader<'_>) -> Result<Self> {
        let c = r.read_bool()?;
        let line_number = r.read_bits(W_LINE_NUMBER)? as u16;
        let horizontal_offset = r.read_bits(W_HORIZONTAL_OFFSET)? as u16;
        let s = r.read_bool()?;
        let stream_num = r.read_bits(W_STREAM_NUM)? as u8;
        let content = AncContent::read_from(r)?;
        let pad = bits_to_word_boundary(r.bits_read());
        if pad > 0 {
            let pad_value = r.read_bits(pad)?;
            if pad_value != 0 {
                return Err(Error::ReservedNotZero {
                    what: "word_align",
                    value: pad_value,
                });
            }
        }
        Ok(Self {
            c,
            line_number,
            horizontal_offset,
            s,
            stream_num,
            content,
        })
    }
}

// ---------------------------------------------------------------------------
// AncRtpPayload — the full RFC 8331 §2.1 payload header + ANC packet list
// ---------------------------------------------------------------------------

/// The RFC 8331 RTP payload for SMPTE ST 291 ancillary data (§2.1): the
/// payload header (`Extended Sequence Number`/`Length`/`ANC_Count`/`F`/
/// `reserved`) plus the list of [`RtpAncPacket`]s. This is the bytes placed
/// in an [`rtp_packet::RtpPacket`]'s `payload` field — the RTP fixed header
/// itself is `rtp_packet`'s responsibility (see the module doc).
///
/// `Length` and `ANC_Count` are never independent stored fields: both are
/// always recomputed from `anc_packets` on [`Serialize::serialize_into`] and
/// cross-validated against the wire value on [`Parse::parse`] (see
/// `docs/anc_rtp_8331.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AncRtpPayload {
    /// `Extended Sequence Number` (16 bits) — the high-order bits of the
    /// extended 32-bit RFC 4175 sequence number. Reconstructing the full
    /// 32-bit counter (RFC 4175's stateful sliding-window algorithm) is
    /// explicitly **out of scope** for this stateless per-packet parser (see
    /// `docs/anc_rtp_8331.md`); this is the raw 16-bit field only.
    pub extended_sequence_number: u16,
    /// `F` (2 bits) — field-sense signalling (§2.1); see [`FieldSense`].
    pub field_sense: FieldSense,
    /// The ANC data packets carried in this payload, in wire order.
    /// `ANC_Count` (§2.1) is always `anc_packets.len()`, never an independent
    /// field.
    pub anc_packets: Vec<RtpAncPacket>,
}

impl AncRtpPayload {
    /// `ANC_Count` — the value that will be written on serialize: always
    /// `anc_packets.len()` (§2.1).
    #[must_use]
    pub fn anc_count(&self) -> usize {
        self.anc_packets.len()
    }

    /// `Length` — the total octets of every [`RtpAncPacket`] (including each
    /// one's `word_align` padding), i.e. the value that will be written on
    /// serialize.
    fn body_len(&self) -> usize {
        self.anc_packets
            .iter()
            .map(RtpAncPacket::serialized_len)
            .sum()
    }

    /// Parse a full ANC-over-RTP packet: an RFC 3550 [`RtpPacket`] (the
    /// `rtp-packet` crate) whose payload is this RFC 8331 ANC payload.
    /// Convenience composition — equivalent to calling
    /// `RtpPacket::parse` then `AncRtpPayload::parse(rtp.payload)` yourself.
    ///
    /// # Errors
    /// [`Error::Rtp`] if the RTP fixed header itself is malformed; otherwise
    /// any error [`Parse::parse`] can return for [`AncRtpPayload`].
    pub fn parse_rtp_packet(bytes: &[u8]) -> Result<(RtpPacket<'_>, Self)> {
        let rtp = RtpPacket::parse(bytes).map_err(Error::Rtp)?;
        let payload = Self::parse(rtp.payload)?;
        Ok((rtp, payload))
    }
}

impl<'a> Parse<'a> for AncRtpPayload {
    type Error = Error;

    /// Parse an RFC 8331 ANC RTP payload from the bytes starting at
    /// `Extended Sequence Number` (i.e. an `rtp_packet::RtpPacket::payload`
    /// slice for a `video/smpte291` packet).
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if truncated; [`Error::ReservedNotZero`] if
    /// the 22-bit `reserved` field (or any per-packet `word_align`) is
    /// nonzero; [`Error::LengthMismatch`] if the declared `Length` does not
    /// match the bytes actually consumed while parsing `ANC_Count` packets
    /// (catches a corrupted `Length` *or* a corrupted `ANC_Count`, since
    /// either desyncs the two); a bit-stream error if a packet runs past the
    /// declared `Length`.
    fn parse(b: &'a [u8]) -> Result<Self> {
        if b.len() < ANC_RTP_PAYLOAD_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: ANC_RTP_PAYLOAD_HEADER_LEN,
                have: b.len(),
                what: "ANC RTP payload header",
            });
        }
        let extended_sequence_number = u16::from_be_bytes([b[0], b[1]]);
        let length = usize::from(u16::from_be_bytes([b[2], b[3]]));
        let anc_count = usize::from(b[4]);
        let f_bits = b[5] >> F_SHIFT;
        let reserved = (u32::from(b[5] & RESERVED_BYTE5_MASK) << 16)
            | (u32::from(b[6]) << 8)
            | u32::from(b[7]);
        if reserved != 0 {
            return Err(Error::ReservedNotZero {
                what: "reserved (RFC 8331 §2.1)",
                value: u64::from(reserved),
            });
        }
        let field_sense = FieldSense::from_bits(f_bits);

        let end = ANC_RTP_PAYLOAD_HEADER_LEN + length;
        if b.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: b.len(),
                what: "ANC RTP payload body",
            });
        }
        let body = &b[ANC_RTP_PAYLOAD_HEADER_LEN..end];

        let mut r = BitReader::new(body);
        let mut anc_packets = Vec::with_capacity(anc_count);
        for _ in 0..anc_count {
            anc_packets.push(RtpAncPacket::read_from(&mut r)?);
        }
        let consumed_bytes = r.bits_read() / 8;
        if !r.is_byte_aligned() || consumed_bytes != body.len() {
            return Err(Error::LengthMismatch {
                declared: length,
                computed: consumed_bytes,
            });
        }

        Ok(Self {
            extended_sequence_number,
            field_sense,
            anc_packets,
        })
    }
}

impl Serialize for AncRtpPayload {
    type Error = Error;

    /// Total serialized length in bytes: the 8-byte payload header plus the
    /// `Length`-counted ANC packet body.
    fn serialized_len(&self) -> usize {
        ANC_RTP_PAYLOAD_HEADER_LEN + self.body_len()
    }

    /// Serialize back to bytes, recomputing `Length` and `ANC_Count` from
    /// `anc_packets`.
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if `buf` is too small; [`Error::FieldTooWide`]
    /// if `anc_packets.len()` exceeds 255 or the recomputed `Length` exceeds
    /// 65535; [`Error::FieldTooWide`] / [`Error::InconsistentUdwLength`] from
    /// any [`RtpAncPacket`].
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "ANC RTP payload serialize output",
            });
        }
        let anc_count = self.anc_count();
        if anc_count > MAX_ANC_COUNT {
            return Err(Error::FieldTooWide {
                what: "ANC_Count",
                value: anc_count as u32,
                bits: 8,
            });
        }
        let body_len = self.body_len();
        if body_len > MAX_LENGTH {
            return Err(Error::FieldTooWide {
                what: "Length",
                value: body_len as u32,
                bits: 16,
            });
        }

        buf[0..2].copy_from_slice(&self.extended_sequence_number.to_be_bytes());
        buf[2..4].copy_from_slice(&(body_len as u16).to_be_bytes());
        buf[4] = anc_count as u8;
        buf[5] = self.field_sense.to_bits() << F_SHIFT; // reserved bits: 0
        buf[6] = 0;
        buf[7] = 0;

        let mut pos = ANC_RTP_PAYLOAD_HEADER_LEN;
        for pkt in &self.anc_packets {
            let pkt_len = pkt.serialized_len();
            let mut w = BitWriter::new(&mut buf[pos..pos + pkt_len]);
            pkt.write_into(&mut w)?;
            pos += pkt_len;
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn sample_payload() -> AncRtpPayload {
        AncRtpPayload {
            extended_sequence_number: 0x0001,
            field_sense: FieldSense::ProgressiveOrUnspecified,
            anc_packets: vec![
                RtpAncPacket {
                    c: false,
                    line_number: 9,
                    horizontal_offset: 0,
                    s: false,
                    stream_num: 0,
                    content: AncContent {
                        did: 0x161,
                        sdid: 0x101,
                        data_count: 0x002,
                        user_data_words: vec![0x2CF, 0x101],
                        checksum: 0x233,
                    },
                },
                RtpAncPacket {
                    c: true,
                    line_number: 10,
                    horizontal_offset: 0x10,
                    s: false,
                    stream_num: 0,
                    content: AncContent {
                        did: 0x241,
                        sdid: 0x102,
                        data_count: 0x003,
                        user_data_words: vec![0x111, 0x222, 0x333],
                        checksum: 0x1AB,
                    },
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let p = sample_payload();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        let reparsed = AncRtpPayload::parse(&out).unwrap();
        assert_eq!(reparsed, p);
    }

    #[test]
    fn anc_count_and_length_recomputed_on_serialize() {
        let p = sample_payload();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        assert_eq!(out[4], 2, "ANC_Count derived from anc_packets.len()");
        let declared_length = u16::from_be_bytes([out[2], out[3]]) as usize;
        assert_eq!(declared_length, out.len() - ANC_RTP_PAYLOAD_HEADER_LEN);
    }

    #[test]
    fn empty_payload_has_zero_count_and_length() {
        let p = AncRtpPayload {
            extended_sequence_number: 0,
            field_sense: FieldSense::ProgressiveOrUnspecified,
            anc_packets: vec![],
        };
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        assert_eq!(out.len(), ANC_RTP_PAYLOAD_HEADER_LEN);
        assert_eq!(out[4], 0);
        assert_eq!(&out[2..4], &[0, 0]);
        assert_eq!(AncRtpPayload::parse(&out).unwrap(), p);
    }

    #[test]
    fn field_mutation_changes_bytes() {
        let a = sample_payload();
        let mut b = a.clone();
        b.anc_packets[0].content.user_data_words[0] = 0x000;
        let mut oa = vec![0u8; a.serialized_len()];
        let mut ob = vec![0u8; b.serialized_len()];
        a.serialize_into(&mut oa).unwrap();
        b.serialize_into(&mut ob).unwrap();
        assert_ne!(oa, ob, "changing a UDW must change the wire bytes");

        let mut c = a.clone();
        c.anc_packets[1].line_number = 11;
        let mut oc = vec![0u8; c.serialized_len()];
        c.serialize_into(&mut oc).unwrap();
        assert_ne!(oa, oc, "changing line_number must change the wire bytes");

        let mut d = a.clone();
        d.anc_packets[0].stream_num = 5;
        d.anc_packets[0].s = true;
        let mut od = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut od).unwrap();
        assert_ne!(oa, od, "changing S/StreamNum must change the wire bytes");
    }

    #[test]
    fn rejects_corrupted_length_too_small() {
        let p = sample_payload();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        // Shrink the declared Length so the body slice truncates a packet.
        let good_length = u16::from_be_bytes([out[2], out[3]]);
        let bad_length = good_length - 4;
        out[2..4].copy_from_slice(&bad_length.to_be_bytes());
        assert!(AncRtpPayload::parse(&out).is_err());
    }

    #[test]
    fn rejects_corrupted_length_leaves_leftover_bytes() {
        // Grow Length beyond the actual encoded packets but keep it within
        // the buffer (by appending trailing zero bytes) -- must be rejected
        // as a Length/content mismatch, not silently accepted.
        let p = sample_payload();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        let good_length = u16::from_be_bytes([out[2], out[3]]);
        let bad_length = good_length + 4;
        out[2..4].copy_from_slice(&bad_length.to_be_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        assert!(matches!(
            AncRtpPayload::parse(&out),
            Err(Error::LengthMismatch { .. })
        ));
    }

    #[test]
    fn rejects_corrupted_anc_count() {
        let p = sample_payload();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        out[4] = 3; // claims a 3rd packet that isn't there
        assert!(AncRtpPayload::parse(&out).is_err());
    }

    #[test]
    fn rejects_nonzero_reserved() {
        let p = sample_payload();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        out[7] = 0x01; // low byte of the 22-bit reserved field
        assert!(matches!(
            AncRtpPayload::parse(&out),
            Err(Error::ReservedNotZero { .. })
        ));
    }

    #[test]
    fn rejects_nonzero_word_align_padding() {
        let p = sample_payload();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        // The last byte of the payload is entirely word_align padding for
        // this fixture (5 UDWs across the two packets keeps a nonzero pad).
        let last = out.len() - 1;
        out[last] |= 0x01;
        assert!(matches!(
            AncRtpPayload::parse(&out),
            Err(Error::ReservedNotZero {
                what: "word_align",
                ..
            })
        ));
    }

    #[test]
    fn field_sense_all_four_values_round_trip() {
        for (bits, expect) in [
            (0b00u8, FieldSense::ProgressiveOrUnspecified),
            (0b01, FieldSense::Invalid),
            (0b10, FieldSense::Field1),
            (0b11, FieldSense::Field2),
        ] {
            let mut p = sample_payload();
            p.field_sense = FieldSense::from_bits(bits);
            assert_eq!(p.field_sense, expect);
            let mut out = vec![0u8; p.serialized_len()];
            p.serialize_into(&mut out).unwrap();
            assert_eq!(out[5] >> F_SHIFT, bits);
            // F=0b01 ("invalid") must still parse successfully, not reject.
            let reparsed = AncRtpPayload::parse(&out).unwrap();
            assert_eq!(reparsed.field_sense, expect);
        }
    }

    #[test]
    fn field_sense_labels() {
        assert_eq!(
            FieldSense::ProgressiveOrUnspecified.name(),
            "progressive or unspecified"
        );
        assert_eq!(FieldSense::Invalid.name(), "invalid (0b01)");
        assert_eq!(FieldSense::Field1.name(), "field 1");
        assert_eq!(FieldSense::Field2.name(), "field 2");
        assert_eq!(FieldSense::Invalid.to_string(), "invalid (0b01)");
    }

    #[test]
    fn rejects_field_too_wide_stream_num() {
        let mut p = sample_payload();
        p.anc_packets[0].stream_num = 0x7F + 1; // 7-bit field max is 0x7F
        let mut out = vec![0u8; p.serialized_len()];
        assert!(matches!(
            p.serialize_into(&mut out),
            Err(Error::FieldTooWide {
                what: "StreamNum",
                ..
            })
        ));
    }

    #[test]
    fn parse_rtp_packet_composition() {
        let anc_payload = sample_payload();
        let mut anc_bytes = vec![0u8; anc_payload.serialized_len()];
        anc_payload.serialize_into(&mut anc_bytes).unwrap();

        let rtp = RtpPacket {
            marker: true,
            payload_type: 112, // RFC 8331 §4 worked example
            sequence_number: 1,
            timestamp: ANC_RTP_DEFAULT_CLOCK_RATE,
            ssrc: 0xCAFE_BABE,
            csrc: Vec::new(),
            extension: None,
            padding: None,
            payload: &anc_bytes,
        };
        let mut rtp_bytes = vec![0u8; rtp.serialized_len()];
        rtp.serialize_into(&mut rtp_bytes).unwrap();

        let (parsed_rtp, parsed_anc) = AncRtpPayload::parse_rtp_packet(&rtp_bytes).unwrap();
        assert!(parsed_rtp.marker);
        assert_eq!(parsed_rtp.payload_type, 112);
        assert_eq!(parsed_anc, anc_payload);
    }

    #[test]
    fn parse_rtp_packet_rejects_bad_rtp_header() {
        // Corrupt the RTP version bits (byte 0 top 2 bits) so the
        // rtp_packet layer itself rejects it before we ever reach the ANC
        // payload.
        let mut bytes = vec![0u8; rtp_packet::FIXED_HEADER_LEN + ANC_RTP_PAYLOAD_HEADER_LEN];
        bytes[0] = 0x00; // version = 0, invalid (RFC 3550 requires 2)
        assert!(matches!(
            AncRtpPayload::parse_rtp_packet(&bytes),
            Err(Error::Rtp(_))
        ));
    }

    #[test]
    fn rejects_too_many_anc_packets() {
        let one = &sample_payload().anc_packets[0];
        let mut p = AncRtpPayload {
            extended_sequence_number: 0,
            field_sense: FieldSense::ProgressiveOrUnspecified,
            anc_packets: Vec::new(),
        };
        for _ in 0..=MAX_ANC_COUNT {
            p.anc_packets.push(one.clone());
        }
        let mut out = vec![0u8; p.serialized_len()];
        assert!(matches!(
            p.serialize_into(&mut out),
            Err(Error::FieldTooWide {
                what: "ANC_Count",
                ..
            })
        ));
    }
}
