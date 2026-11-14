//! The ANC data PES packet — SMPTE ST 2038:2021 §4.2 (Table 2, pp. 5–6).
//!
//! An [`AncDataPacket`] is a PES packet (`stream_id == 0xBD`,
//! `private_stream_1`, PTS present, `PES_header_data_length == 0x05`) whose
//! payload is a tightly bit-packed sequence of ST 291-1 ANC packets followed by
//! a run of `0xFF` stuffing bytes. Each [`AncPacket`] entry carries one ST 291-1
//! ANC data packet (DID/SDID/data_count/user_data_words/checksum) with its
//! HD/SD placement (`c_not_y_channel_flag`, `line_number`, `horizontal_offset`).
//!
//! ## Bit packing
//! The per-ANC-packet fields are **not byte-aligned** — they are a contiguous
//! MSB-first bit stream: `'000000'`(6) + `c_not_y_channel_flag`(1) +
//! `line_number`(11) + `horizontal_offset`(12) + `DID`(10) + `SDID`(10) +
//! `data_count`(10) + (`data_count & 0xFF`)×`user_data_word`(10) +
//! `checksum_word`(10), then `'1'` bits padding up to the next byte boundary
//! (§4.2, Table 2). [`broadcast_common::bits`] provides the MSB-first reader/writer.
//!
//! ## `data_count` loop counter (§4.2.1) — the easy-to-misimplement point
//! `DID`, `SDID`, `data_count`, every `user_data_word` and `checksum_word` are
//! **10-bit** values on the wire (the upper two bits are the ST 291-1
//! even/odd-parity bits). For the `user_data_word` loop counter **only the
//! lower 8 bits of `data_count`** (`data_count & 0xFF`) are used. The full
//! 10-bit values are stored verbatim as `u16`; ST 291-1 parity/checksum
//! derivation is **not** computed or validated here (ST 2038 defers it to
//! ST 291-1, which is vendored as SMPTE ST 291-1:2011: §6.1/6.2/6.4 DID/SDID/DBN
//! parity, §6.7 checksum word; the derivation is simply not implemented yet).
//!
//! ## Shared content (issue #648)
//! The `DID`/`SDID`/`data_count`/`user_data_words`/`checksum` bit sequence is
//! identical to RFC 8331's RTP carriage of the same ST 291-1 content (see
//! `private/specs-verified/smpte/st291/anc_rtp_8331.md`); [`AncPacket`]'s `read_from`/`write_into` delegate
//! that portion to the always-compiled [`crate::anc_content::AncContent`], so
//! the bit-packing logic is implemented exactly once. [`AncPacket`]'s public
//! field layout is unchanged by this — see `anc_content`'s module doc for why.

use alloc::vec::Vec;
use broadcast_common::len::{fit_u16, need_out};

use broadcast_common::bits::{BitReader, BitWriter};

use crate::anc_content::{AncContent, check_field_width};
use crate::error::{Error, Result};

/// `packet_start_code_prefix` — `0x000001` (Table 2).
pub const PACKET_START_CODE_PREFIX: [u8; 3] = [0x00, 0x00, 0x01];

/// `stream_id` — `0xBD` (`private_stream_1`), Table 2.
pub const ANC_STREAM_ID: u8 = 0xBD;

/// `PES_header_data_length` — `0x05` (exactly a 5-byte PTS), Table 2.
pub const ANC_PES_HEADER_DATA_LENGTH: u8 = 0x05;

/// `stuffing_byte` value — `0xFF` (`'1111 1111'`), §4.2.1.
pub const STUFFING_BYTE: u8 = 0xFF;

/// 33-bit PTS mask.
#[cfg(test)]
const PTS_MASK: u64 = (1 << 33) - 1;

// Byte 6 (Table 2): `'10'`(2, bits 7:6) marker + `PES_scrambling_control`
// (2, bits 5:4, fixed `'00'`) + `PES_priority`(1, bit 3, bslbf) +
// `data_alignment_indicator`(1, bit 2, fixed `'1'`) + `copyright`(1, bit 1,
// bslbf) + `original_or_copy`(1, bit 0, bslbf) (S291-W4, #1116).
/// `data_alignment_indicator` bit value (bit 2 of byte 6; ST 2038 Table 2:
/// fixed `'1'`).
const BYTE6_DATA_ALIGNMENT_BIT: u8 = 0b0000_0100;
// Field widths (bits) of the per-ANC-packet **placement** (Table 2); the
// content field widths (DID/SDID/data_count/user_data_word/checksum) live in
// `crate::anc_content` since they are shared with the RTP transport.
const W_LEADING_ZEROS: u32 = 6;
const W_C_NOT_Y: u32 = 1;
const W_LINE_NUMBER: u32 = 11;
const W_HORIZONTAL_OFFSET: u32 = 12;

/// Number of leading bytes before `PES_packet_length`'s value applies:
/// start_code(3) + stream_id(1) + PES_packet_length(2).
const PES_PREFIX_LEN: usize = 6;
/// Fixed bytes of the PES optional header for ST 2038: 2 flag bytes +
/// `PES_header_data_length` + 5-byte PTS.
const PES_OPTIONAL_HEADER_LEN: usize = 3 + 5;
/// Full PES header length (everything before the ANC payload).
const PES_HEADER_LEN: usize = PES_PREFIX_LEN + PES_OPTIONAL_HEADER_LEN;

/// One ST 291-1 ANC data packet plus its ST 2038 placement (Table 2 inner
/// loop).
///
/// The `DID`/`SDID`/`User_Data_Words` are the values stored on the wire
/// (10-bit, `DID`/`SDID` including their parity bits). `Data_Count` and the
/// `Checksum_Word` are **derived at serialize time** from those values —
/// they are functions of them (ST 291-1:2011 §6.5/§6.7,
/// `private/specs-verified/smpte/st291/anc_packet_291.md`), so editing a
/// user data word can never leave a stale count or checksum on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AncPacket {
    /// `c_not_y_channel_flag` — `true` = color-difference channel (HD); for SD
    /// shall be `false` (§4.2.1).
    pub c_not_y_channel_flag: bool,
    /// `line_number` (11 bits) — raster line (ITU-R BT.1700/BT.1120-9).
    pub line_number: u16,
    /// `horizontal_offset` (12 bits) — ANC location relative to SAV.
    pub horizontal_offset: u16,
    /// `DID` (10-bit raw, incl. ST 291-1 parity bits).
    pub did: u16,
    /// `SDID` (10-bit raw).
    pub sdid: u16,
    /// `user_data_word`s (each 10-bit raw). Their count is the `Data_Count`
    /// value written for this packet.
    pub user_data_words: Vec<u16>,
}

impl AncPacket {
    /// Build a packet from raw (unparitied) 8-bit payload values, filling in
    /// every field's parity bits — the producer-side counterpart to
    /// [`AncContent::build`], with the ST 2038 placement alongside it.
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if `udw8s.len()` exceeds `u8::MAX`
    /// (`Data_Count`'s 8-bit value field) or a placement field exceeds its
    /// wire width.
    pub fn build(
        c_not_y_channel_flag: bool,
        line_number: u16,
        horizontal_offset: u16,
        did8: u8,
        sdid8: u8,
        udw8s: &[u8],
    ) -> Result<Self> {
        check_field_width("line_number", u64::from(line_number), W_LINE_NUMBER)?;
        check_field_width(
            "horizontal_offset",
            u64::from(horizontal_offset),
            W_HORIZONTAL_OFFSET,
        )?;
        AncContent::udw_count_fields(udw8s.len())?;
        Ok(Self {
            c_not_y_channel_flag,
            line_number,
            horizontal_offset,
            did: AncContent::with_parity(did8),
            sdid: AncContent::with_parity(sdid8),
            user_data_words: udw8s.iter().map(|&b| AncContent::with_parity(b)).collect(),
        })
    }

    /// The `User_Data_Words` loop count this packet writes: its number of
    /// user data words (`Data_Count`, §4.2.1).
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if there are more than 255 words
    /// (`Data_Count`'s 8-bit value field).
    pub fn udw_loop_count(&self) -> Result<usize> {
        AncContent::udw_count_fields(self.user_data_words.len())
    }

    /// The `Data_Count` word (10-bit, with parity) for this packet's fields.
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if there are more than 255 user data words.
    pub fn data_count(&self) -> Result<u16> {
        let count = broadcast_common::len::fit_u8(self.user_data_words.len(), "Data_Count")?;
        Ok(AncContent::with_parity(count))
    }

    /// The `Checksum_Word` (10-bit) this packet writes over its current
    /// fields.
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if there are more than 255 user data words.
    pub fn checksum(&self) -> Result<u16> {
        Ok(AncContent::checksum_for(
            self.did,
            self.sdid,
            self.data_count()?,
            &self.user_data_words,
        ))
    }

    /// This packet's content as the shared [`AncContent`] with `data_count`
    /// and `checksum` derived from its fields.
    fn content(&self) -> Result<AncContent> {
        let data_count = self.data_count()?;
        Ok(AncContent {
            did: self.did,
            sdid: self.sdid,
            data_count,
            checksum: AncContent::checksum_for(
                self.did,
                self.sdid,
                data_count,
                &self.user_data_words,
            ),
            user_data_words: self.user_data_words.clone(),
        })
    }

    /// Run every ST 291-1 check on this packet's content
    /// ([`AncContent::validate`]): parity of `DID`/`SDID`/`Data_Count`,
    /// `Data_Count` vs the stored UDW count, and the `Checksum_Word`.
    /// Parsing never validates; this is the explicit opt-in.
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if the packet carries more than 255 user
    /// data words (`Data_Count`'s 8-bit value field).
    pub fn validate(&self) -> Result<crate::ValidationReport> {
        Ok(self.content()?.validate())
    }

    /// Bit length of this record's payload, **excluding** the trailing `'1'`
    /// byte-alignment padding: the fixed 30-bit placement prefix + the shared
    /// [`AncContent`] content bit width.
    fn body_bits(&self) -> usize {
        let placement =
            (W_LEADING_ZEROS + W_C_NOT_Y + W_LINE_NUMBER + W_HORIZONTAL_OFFSET) as usize;
        placement + AncContent::content_bit_width_for(self.user_data_words.len())
    }

    /// Serialized byte length of this record (body bits rounded up to a byte
    /// boundary, the pad being `'1'` bits).
    fn serialized_byte_len(&self) -> usize {
        self.body_bits().div_ceil(8)
    }

    /// Write this record's bits into `w`, MSB-first, then pad to the byte
    /// boundary with `'1'` bits.
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if there are more than 255 user data words
    /// (`Data_Count`'s 8-bit value field).
    fn write_into(&self, w: &mut BitWriter<'_>) -> Result<()> {
        w.write_bits(0, W_LEADING_ZEROS)?; // '000000'
        w.write_bits(u64::from(self.c_not_y_channel_flag), W_C_NOT_Y)?;
        w.write_bits(
            check_field_width("line_number", u64::from(self.line_number), W_LINE_NUMBER)?,
            W_LINE_NUMBER,
        )?;
        w.write_bits(
            check_field_width(
                "horizontal_offset",
                u64::from(self.horizontal_offset),
                W_HORIZONTAL_OFFSET,
            )?,
            W_HORIZONTAL_OFFSET,
        )?;
        // DID/SDID/data_count/user_data_words/checksum (§4.2.1) — shared with
        // the RTP transport, see `crate::anc_content`.
        self.content()?.write_into(w)?;
        // Pad to byte boundary with '1' bits (note: '1', not '0' — §4.2).
        while !w.is_byte_aligned() {
            w.write_bits(1, 1)?;
        }
        Ok(())
    }

    /// Read one record starting at the byte boundary `r` is currently on.
    ///
    /// # Errors
    /// [`Error::ReservedBits`] if the leading `'000000'` field (Table 2) is
    /// nonzero, or the trailing byte-alignment padding is not all `'1'` bits
    /// (S291-W3, #1116) — either would otherwise parse `Ok` and reserialize
    /// to different bytes, since the writer always emits the canonical
    /// values.
    fn read_from(r: &mut BitReader<'_>) -> Result<Self> {
        // '000000' leading bits (Table 2) are fixed, like every other fixed
        // pattern this crate validates.
        let leading = r.read_bits(W_LEADING_ZEROS)?;
        if leading != 0 {
            return Err(Error::ReservedBits(broadcast_common::error::ReservedBits {
                field: "ANC record leading '000000' bits",
                expected: 0,
                found: leading,
            }));
        }
        let c_not_y_channel_flag = r.read_bool()?;
        let line_number = r.read_bits(W_LINE_NUMBER)? as u16;
        let horizontal_offset = r.read_bits(W_HORIZONTAL_OFFSET)? as u16;
        let content = AncContent::read_from(r)?;
        // ST 291-1 derives both from the other fields (§6.5/§6.7): a packet
        // whose stored `Data_Count`/`Checksum_Word` disagree with its own
        // user data words would reserialize to different bytes.
        if content.data_count != content.derived_data_count()? {
            return Err(Error::DataCountMismatch {
                stored: content.data_count,
                derived: content.derived_data_count()?,
            });
        }
        if content.checksum != content.compute_checksum() {
            return Err(Error::ChecksumMismatch {
                stored: content.checksum,
                computed: content.compute_checksum(),
            });
        }
        // The '1' padding up to the byte boundary (§4.2) is fixed too — note
        // '1', not '0' (contrast the RFC 8331 `word_align`, which is '0').
        let pad = (8 - r.bits_read() % 8) % 8;
        if pad > 0 {
            let pad_value = r.read_bits(pad as u32)?;
            let all_ones = (1u64 << pad) - 1;
            if pad_value != all_ones {
                return Err(Error::ReservedBits(broadcast_common::error::ReservedBits {
                    field: "ANC record byte-alignment '1' padding",
                    expected: all_ones,
                    found: pad_value,
                }));
            }
        }
        Ok(Self {
            c_not_y_channel_flag,
            line_number,
            horizontal_offset,
            did: content.did,
            sdid: content.sdid,
            user_data_words: content.user_data_words,
        })
    }
}

/// A parsed ANC data PES packet (Table 2): the fixed PES header (stream_id
/// `0xBD`, PTS) + the list of [`AncPacket`]s + a count of trailing `0xFF`
/// stuffing bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AncDataPacket {
    /// `PES_priority` (bslbf, not fixed by ST 2038).
    pub pes_priority: bool,
    /// `copyright` (bslbf).
    pub copyright: bool,
    /// `original_or_copy` (bslbf).
    pub original_or_copy: bool,
    /// 33-bit `PTS` (90 kHz units).
    pub pts: u64,
    /// The ANC packets carried (one PES packet carries one line's worth, §4.2).
    pub anc_packets: Vec<AncPacket>,
    /// Number of trailing `stuffing_byte` (`0xFF`) values after the ANC loop.
    pub stuffing_bytes: usize,
}

impl AncDataPacket {
    /// Total serialized payload length (ANC records + stuffing), excluding the
    /// PES header.
    fn payload_len(&self) -> usize {
        let anc: usize = self
            .anc_packets
            .iter()
            .map(AncPacket::serialized_byte_len)
            .sum();
        anc + self.stuffing_bytes
    }

    /// `PES_packet_length` value = bytes after the length field (optional
    /// header + payload).
    fn pes_packet_length(&self) -> usize {
        PES_OPTIONAL_HEADER_LEN + self.payload_len()
    }

    /// This packet's header as the shared [`mpeg_pes::PesHeader`] (ST 2038
    /// Table 2 pins every other field of it: `PES_scrambling_control` `'00'`,
    /// `data_alignment_indicator` `'1'`, PTS-only, no other optional fields).
    fn pes_header(&self) -> mpeg_pes::PesHeader<'static> {
        let mut header = mpeg_pes::PesHeader::new();
        header.data_alignment_indicator = true;
        header.pts = Some(mpeg_pes::Pts(self.pts));
        header.pes_priority = self.pes_priority;
        header.original_or_copy = self.original_or_copy;
        header
    }

    /// The whole PES optional header — the two flag bytes,
    /// `PES_header_data_length` and its fields — from the shared codec.
    fn optional_header(&self) -> Result<Vec<u8>> {
        broadcast_common::len::fit_bits(self.pts, 33, "PTS")?;
        let header = self.pes_header();
        let mut buf = alloc::vec![0u8; PES_OPTIONAL_HEADER_LEN];
        buf[0..2].copy_from_slice(&header.flag_bytes());
        buf[2] = ANC_PES_HEADER_DATA_LENGTH;
        header.serialize_optional_fields(&mut buf[3..])?;
        Ok(buf)
    }

    /// Reject a header field ST 2038 Table 2 pins to one value.
    ///
    /// # Errors
    /// [`Error::ReservedBits`] naming the field that differed.
    fn check_pinned_header(h: &mpeg_pes::PesHeader<'_>) -> Result<()> {
        if h.scrambling_control != 0 {
            return Err(Error::ReservedBits(broadcast_common::error::ReservedBits {
                field: "PES_scrambling_control",
                expected: 0,
                found: u64::from(h.scrambling_control),
            }));
        }
        if !h.data_alignment_indicator {
            return Err(Error::ReservedBits(broadcast_common::error::ReservedBits {
                field: "data_alignment_indicator",
                expected: u64::from(BYTE6_DATA_ALIGNMENT_BIT & 1),
                found: 0,
            }));
        }
        if h.dts.is_some()
            || h.escr.is_some()
            || h.es_rate.is_some()
            || h.dsm_trick_mode.is_some()
            || h.additional_copy_info.is_some()
            || h.pes_crc.is_some()
            || h.pes_extension.is_some()
            || !h.header_stuffing.is_empty()
        {
            return Err(Error::ReservedBits(broadcast_common::error::ReservedBits {
                field: "ST 2038 optional PES header fields beyond the PTS",
                expected: 0,
                found: 1,
            }));
        }
        Ok(())
    }

    /// Parse an ANC data PES packet from the bytes starting at its
    /// `packet_start_code_prefix`.
    ///
    /// The fixed PES header is parsed by the shared [`mpeg_pes::PesPacket`]
    /// codec; the fields ST 2038 Table 2 pins are then validated.
    ///
    /// # Errors
    /// [`Error::Pes`] for a malformed PES header, [`Error::BadStreamId`] for
    /// a `stream_id` other than `0xBD`, [`Error::ReservedBits`] for a header
    /// field ST 2038 pins to another value, or a bit-stream error walking the
    /// ANC payload.
    pub fn parse(b: &[u8]) -> Result<Self> {
        let packet = mpeg_pes::PesPacket::parse(b)?;
        if packet.stream_id.0 != ANC_STREAM_ID {
            return Err(Error::BadStreamId(packet.stream_id.0));
        }
        let header =
            packet
                .header
                .ok_or(Error::ReservedBits(broadcast_common::error::ReservedBits {
                    field: "ANC PES optional header",
                    expected: 1,
                    found: 0,
                }))?;
        Self::check_pinned_header(&header)?;
        let pts = header
            .pts
            .ok_or(Error::ReservedBits(broadcast_common::error::ReservedBits {
                field: "ANC PES PTS",
                expected: 1,
                found: 0,
            }))?;
        let payload = packet.payload;
        let pes_packet_length = usize::from(packet.pes_packet_length);
        let pes_length_end = PES_PREFIX_LEN.saturating_add(pes_packet_length);
        if pes_length_end > b.len() {
            return Err(Error::PesLengthOverflow {
                len: pes_packet_length,
                available: b.len().saturating_sub(PES_PREFIX_LEN),
            });
        }

        let mut anc_packets = Vec::new();
        let mut pos = 0usize;
        while pos < payload.len() {
            // A run of 0xFF starting here is the trailing stuffing region.
            // ST 2038 has no per-record length, so we treat a 0xFF byte at a
            // record boundary as the start of stuffing (a real ANC record's
            // first byte is '000000' + c_not_y_channel_flag + line_number MSBs,
            // never 0xFF, since the top 6 bits are zero).
            if payload[pos] == STUFFING_BYTE {
                break;
            }
            let mut r = BitReader::new(&payload[pos..]);
            let rec = AncPacket::read_from(&mut r)?;
            let consumed = r.bits_read() / 8;
            pos += consumed;
            anc_packets.push(rec);
        }
        // Everything left must be stuffing.
        let stuffing_bytes = payload.len() - pos;
        for &byte in &payload[pos..] {
            if byte != STUFFING_BYTE {
                return Err(Error::ReservedBits(broadcast_common::error::ReservedBits {
                    field: "stuffing_byte",
                    expected: u64::from(STUFFING_BYTE),
                    found: u64::from(byte),
                }));
            }
        }

        Ok(Self {
            pes_priority: header.pes_priority,
            copyright: header.copyright,
            original_or_copy: header.original_or_copy,
            pts: pts.0,
            anc_packets,
            stuffing_bytes,
        })
    }

    /// Serialized length in bytes (PES header + ANC payload + stuffing).
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        PES_HEADER_LEN + self.payload_len()
    }

    /// Serialize back to bytes, recomputing `PES_packet_length` and bit-packing
    /// every ANC record from its typed fields.
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if `buf` is too small;
    /// [`Error::PesLengthTooLarge`] if `PES_packet_length` overflows 16 bits;
    /// [`Error::FieldOverflow`] if any field exceeds its wire width (including
    /// more than 255 user data words on one ANC packet).
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let pes_len = self.pes_packet_length();
        if pes_len > usize::from(u16::MAX) {
            return Err(Error::PesLengthTooLarge(pes_len));
        }

        buf[0..3].copy_from_slice(&PACKET_START_CODE_PREFIX);
        buf[3] = ANC_STREAM_ID;
        buf[4..6].copy_from_slice(&fit_u16(pes_len, "PES_packet_length")?.to_be_bytes());
        buf[PES_PREFIX_LEN..PES_HEADER_LEN].copy_from_slice(&self.optional_header()?);

        // Payload: ANC records (each byte-aligned), then 0xFF stuffing.
        let mut pos = PES_HEADER_LEN;
        for rec in &self.anc_packets {
            let rec_len = rec.serialized_byte_len();
            let mut w = BitWriter::new(&mut buf[pos..pos + rec_len]);
            rec.write_into(&mut w)?;
            pos += rec_len;
        }
        for byte in buf.iter_mut().skip(pos).take(self.stuffing_bytes) {
            *byte = STUFFING_BYTE;
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample_2packet() -> AncDataPacket {
        AncDataPacket {
            pes_priority: false,
            copyright: false,
            original_or_copy: false,
            pts: 0x1_2345_6789,
            anc_packets: vec![
                AncPacket {
                    c_not_y_channel_flag: false,
                    line_number: 9,
                    horizontal_offset: 0,
                    did: 0x161,
                    sdid: 0x101,
                    user_data_words: vec![0x2CF, 0x101],
                },
                AncPacket {
                    c_not_y_channel_flag: true,
                    line_number: 0x2A,
                    horizontal_offset: 0x10,
                    did: 0x241,
                    sdid: 0x102,
                    user_data_words: vec![0x111, 0x222, 0x333],
                },
            ],
            stuffing_bytes: 3,
        }
    }

    /// The PTS now travels through the shared `mpeg_pes` PES codec
    /// (#2762): every representable value survives the whole packet.
    #[test]
    fn pts_round_trip() {
        for ts in [0u64, 1, 90_000, 0x1_2345_6789, PTS_MASK] {
            let mut p = sample_2packet();
            p.pts = ts;
            let mut out = vec![0u8; p.serialized_len()];
            p.serialize_into(&mut out).unwrap();
            assert_eq!(AncDataPacket::parse(&out).unwrap().pts, ts, "ts={ts:#x}");
        }
    }

    /// A PTS of 2^33 and above is rejected by st291, never masked to a
    /// different (wrapped) timestamp; [st291's own 33-bit check is
    /// [#1116](https://github.com/fishloa/rust-broadcast/issues/1116) and it
    /// runs ahead of the shared codec, which masks].
    #[test]
    fn oversized_pts_is_rejected_not_masked() {
        let mut p = sample_2packet();
        p.pts = PTS_MASK + 1;
        let mut out = vec![0u8; p.serialized_len()];
        assert!(matches!(
            p.serialize_into(&mut out),
            Err(Error::FieldOverflow(
                broadcast_common::error::FieldOverflow { field: "PTS", .. }
            ))
        ));
    }

    #[test]
    fn max_pts_still_serializes_and_round_trips() {
        let mut p = sample_2packet();
        p.pts = PTS_MASK;
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        assert_eq!(AncDataPacket::parse(&out).unwrap().pts, PTS_MASK);
    }

    #[test]
    fn round_trip_two_packets() {
        let p = sample_2packet();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        let reparsed = AncDataPacket::parse(&out).unwrap();
        assert_eq!(reparsed, p);
    }

    #[test]
    fn hand_computed_wire_bytes() {
        // One ANC packet, 2 UDWs, no stuffing — hand-pack the bit stream.
        // Fields:
        //   '000000'(6) c_not_y=0(1) line_number=9(11) h_off=0(12)
        //   DID=0x161(10) SDID=0x101(10) data_count=0x102(10, the parity-coded
        //   count of the 2 words below) UDW0=0x2CF(10) UDW1=0x101(10)
        //   checksum=0x134(10, the §6.7 sum of the fields above)
        //   then '1' pad to byte boundary.
        let p = AncDataPacket {
            pes_priority: false,
            copyright: false,
            original_or_copy: false,
            pts: 0,
            anc_packets: vec![AncPacket {
                c_not_y_channel_flag: false,
                line_number: 9,
                horizontal_offset: 0,
                did: 0x161,
                sdid: 0x101,
                user_data_words: vec![0x2CF, 0x101],
            }],
            stuffing_bytes: 0,
        };

        // Build the expected ANC record bit-stream independently.
        // bits: 000000 | 0 | 00000001001 | 000000000000 |
        //       0101100001 | 0100000001 | 0100000010 |
        //       1011001111 | 0100000001 | 0100110100 | pad '1'...
        let mut expect_bits: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        let mut push = |val: u64, n: u32| {
            for i in (0..n).rev() {
                expect_bits.push(((val >> i) & 1) as u8);
            }
        };
        push(0, 6);
        push(0, 1); // c_not_y
        push(9, 11);
        push(0, 12);
        push(0x161, 10);
        push(0x101, 10);
        push(0x102, 10);
        push(0x2CF, 10);
        push(0x101, 10);
        push(0x134, 10);
        while !expect_bits.len().is_multiple_of(8) {
            expect_bits.push(1); // '1' padding
        }
        let mut expect_payload = alloc::vec::Vec::new();
        for chunk in expect_bits.chunks(8) {
            let mut byte = 0u8;
            for &bit in chunk {
                byte = (byte << 1) | bit;
            }
            expect_payload.push(byte);
        }

        let out = {
            let mut o = vec![0u8; p.serialized_len()];
            p.serialize_into(&mut o).unwrap();
            o
        };
        // PES header is 14 bytes; payload follows.
        assert_eq!(
            &out[PES_HEADER_LEN..],
            &expect_payload[..],
            "ANC bit-packing"
        );

        // Also verify the full PES header fixed bytes.
        assert_eq!(&out[0..4], &[0x00, 0x00, 0x01, 0xBD]);
        // PES_packet_length = optional_header(8) + payload.
        let pes_len = (8 + expect_payload.len()) as u16;
        assert_eq!(&out[4..6], &pes_len.to_be_bytes());
        assert_eq!(out[6], 0x84); // '10' 00 0 1 0 0 → 1000_0100
        assert_eq!(out[7], 0x80); // PTS_DTS_flags '10' then zeros
        assert_eq!(out[8], 0x05); // PES_header_data_length

        // Reparse → equal.
        assert_eq!(AncDataPacket::parse(&out).unwrap(), p);
    }

    #[test]
    fn field_mutation_changes_bytes() {
        let a = sample_2packet();
        let mut b = a.clone();
        b.anc_packets[0].user_data_words[0] = 0x000; // was 0x2CF
        let mut oa = vec![0u8; a.serialized_len()];
        let mut ob = vec![0u8; b.serialized_len()];
        a.serialize_into(&mut oa).unwrap();
        b.serialize_into(&mut ob).unwrap();
        assert_ne!(oa, ob, "changing a UDW must change the wire bytes");

        let mut c = a.clone();
        c.anc_packets[1].line_number = 0x2B; // was 0x2A
        let mut oc = vec![0u8; c.serialized_len()];
        c.serialize_into(&mut oc).unwrap();
        assert_ne!(oa, oc, "changing line_number must change the wire bytes");

        // DID must be reflected in the wire bytes.
        let mut d = a.clone();
        d.anc_packets[0].did = 0x001; // was 0x161
        let mut od = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut od).unwrap();
        assert_ne!(oa, od, "changing DID must change the wire bytes");

        // A changed user data word changes the derived checksum on the wire.
        let mut e = a.clone();
        e.anc_packets[0].user_data_words[1] = 0x111; // was 0x101
        let mut oe = vec![0u8; e.serialized_len()];
        e.serialize_into(&mut oe).unwrap();
        assert_ne!(oa, oe, "changing a UDW must change the wire bytes");
        let reparsed = AncDataPacket::parse(&oe).unwrap();
        assert!(reparsed.anc_packets[0].validate().unwrap().is_valid());
    }

    /// `Data_Count` is derived from the user data words (#2762), so a packet
    /// can no longer declare one count and carry another. A packet whose wire
    /// `Data_Count` disagrees with its words is rejected on parse rather than
    /// silently rewritten.
    #[test]
    fn rejects_a_stored_data_count_that_contradicts_the_user_data_words() {
        let p = sample_2packet();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        let mut bits = bits_of_first_record(&out);
        // First record: placement 30 bits, then DID(10) SDID(10) Data_Count(10).
        let data_count_at = 30 + 10 + 10;
        bits[data_count_at..data_count_at + 10].fill(0);
        bits[data_count_at + 8..data_count_at + 10].fill(1);
        let corrupted = pack_bits(&bits);
        let mut bytes = out[..PES_HEADER_LEN].to_vec();
        bytes.extend_from_slice(&corrupted);
        bytes.extend_from_slice(&out[PES_HEADER_LEN + corrupted.len()..]);
        assert!(matches!(
            AncDataPacket::parse(&bytes),
            Err(Error::DataCountMismatch { .. })
        ));
    }

    /// Parser must reject PES_scrambling_control != '00' (ST 2038 Table 2).
    #[test]
    fn rejects_nonzero_scrambling_control() {
        let p = sample_2packet();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        // bits[5:4] of byte 6 = PES_scrambling_control; set to '01'.
        out[6] = (out[6] & !0x30) | 0x10;
        assert!(
            matches!(AncDataPacket::parse(&out), Err(Error::ReservedBits(_))),
            "expected ReservedBits for non-zero scrambling_control"
        );
    }

    /// Parser must reject data_alignment_indicator != '1' (ST 2038 Table 2).
    #[test]
    fn rejects_zero_data_alignment_indicator() {
        let p = sample_2packet();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        // bit[2] of byte 6 = data_alignment_indicator; clear it.
        out[6] &= !0x04;
        assert!(
            matches!(AncDataPacket::parse(&out), Err(Error::ReservedBits(_))),
            "expected ReservedBits for data_alignment_indicator=0"
        );
    }

    #[test]
    fn data_count_upper_bits_use_low_8_for_loop() {
        // data_count = 0x302: upper two bits (b9,b8) set, low 8 bits = 0x02.
        // The UDW loop must run 2 times and round-trip, preserving the full
        // 10-bit data_count = 0x302.
        let p = AncDataPacket {
            pes_priority: false,
            copyright: false,
            original_or_copy: false,
            pts: 42,
            anc_packets: vec![AncPacket {
                c_not_y_channel_flag: true,
                line_number: 20,
                horizontal_offset: 100,
                did: 0x241,
                sdid: 0x102,
                user_data_words: vec![0x123, 0x3FF],
            }],
            stuffing_bytes: 0,
        };
        assert_eq!(p.anc_packets[0].udw_loop_count(), Ok(2));
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        let rp = AncDataPacket::parse(&out).unwrap();
        assert_eq!(rp, p);
        assert_eq!(
            rp.anc_packets[0].data_count(),
            Ok(AncContent::with_parity(2))
        );
        assert_eq!(rp.anc_packets[0].user_data_words.len(), 2);
    }

    #[test]
    fn rejects_bad_stream_id() {
        let p = sample_2packet();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        out[3] = 0xE0;
        assert!(matches!(
            AncDataPacket::parse(&out),
            Err(Error::BadStreamId(0xE0))
        ));
    }

    #[test]
    fn rejects_bad_start_code() {
        let mut out = vec![0u8; sample_2packet().serialized_len()];
        sample_2packet().serialize_into(&mut out).unwrap();
        out[2] = 0x02;
        let result = AncDataPacket::parse(&out);
        assert!(matches!(result, Err(Error::Pes(_))), "got {result:?}");
    }

    #[test]
    fn no_stuffing_round_trips() {
        let mut p = sample_2packet();
        p.stuffing_bytes = 0;
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        assert_eq!(AncDataPacket::parse(&out).unwrap(), p);
    }

    /// A single ANC packet (2 UDWs), no stuffing, for the S291-W3 fixed-bit
    /// regression tests below: the exact same layout as
    /// `hand_computed_wire_bytes`, so the payload offsets are known.
    fn single_packet_bytes() -> Vec<u8> {
        let p = AncDataPacket {
            pes_priority: false,
            copyright: false,
            original_or_copy: false,
            pts: 0,
            anc_packets: vec![AncPacket {
                c_not_y_channel_flag: false,
                line_number: 9,
                horizontal_offset: 0,
                did: 0x161,
                sdid: 0x101,
                user_data_words: vec![0x2CF, 0x101],
            }],
            stuffing_bytes: 0,
        };
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        // 30 (placement) + 60 (content) = 90 bits = 12 payload bytes (6 bits
        // of '1' padding at the end); sanity-check that assumption holds.
        assert_eq!(out.len(), PES_HEADER_LEN + 12);
        out
    }

    /// S291-W3 (audit issue #1116): the leading `'000000'` bits (ST 2038
    /// Table 2) were skipped, not validated, so a corrupt packet with a
    /// nonzero leading bit parsed `Ok` and reserialized to different bytes
    /// (round-trip asymmetry). Observed pre-fix: `AncDataPacket::parse`
    /// returned `Ok`, and `parsed.to_bytes() != corrupted_bytes` (the
    /// serializer always writes the canonical `'000000'`).
    #[test]
    fn rejects_nonzero_leading_bits() {
        let mut bytes = single_packet_bytes();
        bytes[PES_HEADER_LEN] |= 0x80; // top bit of the '000000' field
        assert!(matches!(
            AncDataPacket::parse(&bytes),
            Err(Error::ReservedBits(_))
        ));
    }

    /// S291-W3 (audit issue #1116): the trailing `'1'`-bit byte-alignment
    /// padding (ST 2038 §4.2) was skipped via `align_to_byte()`, not
    /// validated, so a corrupt packet with a `'0'` padding bit parsed `Ok`
    /// with the same round-trip asymmetry as the leading bits.
    #[test]
    fn rejects_non_one_padding_bits() {
        let mut bytes = single_packet_bytes();
        let last = bytes.len() - 1;
        bytes[last] &= 0xFE; // clear the last bit: part of the '1' padding
        assert!(matches!(
            AncDataPacket::parse(&bytes),
            Err(Error::ReservedBits(_))
        ));
    }

    /// S291-W3 (audit issue #1116): byte 7's six flag bits after
    /// `PTS_DTS_flags` (`ESCR_flag`/`ES_rate_flag`/`DSM_trick_mode_flag`/
    /// `additional_copy_info_flag`/`PES_CRC_flag`/`PES_extension_flag`, all
    /// fixed `'0'` per Table 2) were entirely ignored.
    #[test]
    fn rejects_nonzero_byte7_flags() {
        let mut bytes = single_packet_bytes();
        bytes[7] |= 0x01; // PES_extension_flag, fixed '0'
        let result = AncDataPacket::parse(&bytes);
        assert!(matches!(result, Err(Error::Pes(_))), "got {result:?}");
    }

    /// The payload bits of the first ANC record in a serialized packet, as
    /// one bit per element.
    fn bits_of_first_record(packet: &[u8]) -> alloc::vec::Vec<u8> {
        packet[PES_HEADER_LEN..]
            .iter()
            .flat_map(|byte| (0..8).rev().map(move |bit| (byte >> bit) & 1))
            .collect()
    }

    /// Pack bits back into bytes, MSB first.
    fn pack_bits(bits: &[u8]) -> alloc::vec::Vec<u8> {
        bits.chunks(8)
            .map(|chunk| {
                chunk.iter().fold(0u8, |byte, &bit| (byte << 1) | (bit & 1)) << (8 - chunk.len())
            })
            .collect()
    }
}
