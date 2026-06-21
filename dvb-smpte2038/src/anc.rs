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
//! (§4.2, Table 2). [`dvb_common::bits`] provides the MSB-first reader/writer.
//!
//! ## `data_count` loop counter (§4.2.1) — the easy-to-misimplement point
//! `DID`, `SDID`, `data_count`, every `user_data_word` and `checksum_word` are
//! **10-bit** values on the wire (the upper two bits are the ST 291-1
//! even/odd-parity bits). For the `user_data_word` loop counter **only the
//! lower 8 bits of `data_count`** (`data_count & 0xFF`) are used. The full
//! 10-bit values are stored verbatim as `u16`; ST 291-1 parity/checksum
//! derivation is **not** computed or validated here (ST 2038 defers it to
//! ST 291-1, which is not vendored).

use alloc::vec::Vec;

use dvb_common::bits::{BitReader, BitWriter};

use crate::error::{Error, Result};

/// `packet_start_code_prefix` — `0x000001` (Table 2).
pub const PACKET_START_CODE_PREFIX: [u8; 3] = [0x00, 0x00, 0x01];

/// `stream_id` — `0xBD` (`private_stream_1`), Table 2.
pub const ANC_STREAM_ID: u8 = 0xBD;

/// `PES_header_data_length` — `0x05` (exactly a 5-byte PTS), Table 2.
pub const ANC_PES_HEADER_DATA_LENGTH: u8 = 0x05;

/// `PTS_DTS_flags` — `'10'` (PTS only), Table 2.
const PTS_DTS_FLAGS_PTS_ONLY: u8 = 0b10;

/// 4-bit PTS marker prefix in a PTS-only field — `'0010'`, Table 2.
const PTS_PREFIX: u8 = 0b0010;

/// `stuffing_byte` value — `0xFF` (`'1111 1111'`), §4.2.1.
pub const STUFFING_BYTE: u8 = 0xFF;

/// 33-bit PTS mask.
const PTS_MASK: u64 = (1 << 33) - 1;

// Field widths (bits) of the per-ANC-packet record (Table 2).
const W_LEADING_ZEROS: u32 = 6;
const W_C_NOT_Y: u32 = 1;
const W_LINE_NUMBER: u32 = 11;
const W_HORIZONTAL_OFFSET: u32 = 12;
const W_DID: u32 = 10;
const W_SDID: u32 = 10;
const W_DATA_COUNT: u32 = 10;
const W_USER_DATA_WORD: u32 = 10;
const W_CHECKSUM: u32 = 10;

/// Number of leading bytes before `PES_packet_length`'s value applies:
/// start_code(3) + stream_id(1) + PES_packet_length(2).
const PES_PREFIX_LEN: usize = 6;
/// Fixed bytes of the PES optional header for ST 2038: 2 flag bytes +
/// `PES_header_data_length` + 5-byte PTS.
const PES_OPTIONAL_HEADER_LEN: usize = 3 + 5;
/// Full PES header length (everything before the ANC payload).
const PES_HEADER_LEN: usize = PES_PREFIX_LEN + PES_OPTIONAL_HEADER_LEN;

/// One ST 291-1 ANC data packet plus its ST 2038 placement (Table 2 inner
/// loop). The 10-bit `did`/`sdid`/`data_count`/`user_data_words`/`checksum`
/// are stored as raw `u16` (ST 291-1 parity/checksum not validated here).
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
    /// `data_count` (10-bit raw). The loop counter uses only `& 0xFF`.
    pub data_count: u16,
    /// `user_data_word`s (each 10-bit raw). Length is `data_count & 0xFF`.
    pub user_data_words: Vec<u16>,
    /// `checksum_word` (10-bit raw, ST 291-1 checksum — not validated here).
    pub checksum: u16,
}

impl AncPacket {
    /// The `user_data_word` loop count actually used on the wire: the **low 8
    /// bits** of `data_count` (§4.2.1), independent of the stored Vec length.
    #[must_use]
    pub fn udw_loop_count(&self) -> usize {
        usize::from(self.data_count & 0xFF)
    }

    /// Bit length of this record's payload, **excluding** the trailing `'1'`
    /// byte-alignment padding: the fixed 70-bit prefix + 10 bits per
    /// `user_data_word` (counted as `data_count & 0xFF`) + the 10-bit checksum.
    fn body_bits(&self) -> usize {
        let fixed = (W_LEADING_ZEROS
            + W_C_NOT_Y
            + W_LINE_NUMBER
            + W_HORIZONTAL_OFFSET
            + W_DID
            + W_SDID
            + W_DATA_COUNT
            + W_CHECKSUM) as usize;
        fixed + self.udw_loop_count() * W_USER_DATA_WORD as usize
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
    /// Returns [`Error::InconsistentUdwLength`] if `user_data_words.len()` does
    /// not equal `data_count & 0xFF`; a mismatch would silently zero-fill missing
    /// words, making serialize→parse non-identity.
    fn write_into(&self, w: &mut BitWriter<'_>) -> Result<()> {
        fn check(what: &'static str, value: u16, bits: u32) -> Result<u64> {
            if u32::from(value) >= (1u32 << bits) {
                return Err(Error::FieldTooWide {
                    what,
                    value: u32::from(value),
                    bits,
                });
            }
            Ok(u64::from(value))
        }

        // Validate coherence: user_data_words.len() must equal data_count & 0xFF
        // (§4.2.1).  A mismatch would produce extra zero words on re-parse.
        let need = self.udw_loop_count();
        let have = self.user_data_words.len();
        if have != need {
            return Err(Error::InconsistentUdwLength { have, need });
        }

        w.write_bits(0, W_LEADING_ZEROS)?; // '000000'
        w.write_bits(u64::from(self.c_not_y_channel_flag), W_C_NOT_Y)?;
        w.write_bits(
            check("line_number", self.line_number, W_LINE_NUMBER)?,
            W_LINE_NUMBER,
        )?;
        w.write_bits(
            check(
                "horizontal_offset",
                self.horizontal_offset,
                W_HORIZONTAL_OFFSET,
            )?,
            W_HORIZONTAL_OFFSET,
        )?;
        w.write_bits(check("DID", self.did, W_DID)?, W_DID)?;
        w.write_bits(check("SDID", self.sdid, W_SDID)?, W_SDID)?;
        w.write_bits(
            check("data_count", self.data_count, W_DATA_COUNT)?,
            W_DATA_COUNT,
        )?;
        // §4.2.1: loop runs exactly `data_count & 0xFF` times; coherence
        // validated above so user_data_words.len() == need here.
        for udw in &self.user_data_words {
            w.write_bits(
                check("user_data_word", *udw, W_USER_DATA_WORD)?,
                W_USER_DATA_WORD,
            )?;
        }
        w.write_bits(
            check("checksum_word", self.checksum, W_CHECKSUM)?,
            W_CHECKSUM,
        )?;
        // Pad to byte boundary with '1' bits (note: '1', not '0' — §4.2).
        while !w.is_byte_aligned() {
            w.write_bits(1, 1)?;
        }
        Ok(())
    }

    /// Read one record starting at the byte boundary `r` is currently on.
    fn read_from(r: &mut BitReader<'_>) -> Result<Self> {
        // '000000' leading bits are ignored (reserved); we do not enforce them
        // zero — ST 2038 fixes them but a tolerant reader skips.
        r.skip_bits(W_LEADING_ZEROS as usize)?;
        let c_not_y_channel_flag = r.read_bool()?;
        let line_number = r.read_bits(W_LINE_NUMBER)? as u16;
        let horizontal_offset = r.read_bits(W_HORIZONTAL_OFFSET)? as u16;
        let did = r.read_bits(W_DID)? as u16;
        let sdid = r.read_bits(W_SDID)? as u16;
        let data_count = r.read_bits(W_DATA_COUNT)? as u16;
        let n = usize::from(data_count & 0xFF);
        let mut user_data_words = Vec::with_capacity(n);
        for _ in 0..n {
            user_data_words.push(r.read_bits(W_USER_DATA_WORD)? as u16);
        }
        let checksum = r.read_bits(W_CHECKSUM)? as u16;
        // Consume the '1' padding up to the byte boundary.
        r.align_to_byte();
        Ok(Self {
            c_not_y_channel_flag,
            line_number,
            horizontal_offset,
            did,
            sdid,
            data_count,
            user_data_words,
            checksum,
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

    /// Parse an ANC data PES packet from the bytes starting at its
    /// `packet_start_code_prefix`.
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] / [`Error::BadStartCode`] /
    /// [`Error::BadStreamId`] / [`Error::BadPtsDtsFlags`] /
    /// [`Error::BadHeaderDataLength`] / [`Error::BadFixedBits`] on malformed
    /// input, or a bit-stream error walking the ANC payload.
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < PES_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: PES_HEADER_LEN,
                have: b.len(),
                what: "ANC PES header",
            });
        }
        if b[0..3] != PACKET_START_CODE_PREFIX {
            return Err(Error::BadStartCode(
                (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]),
            ));
        }
        if b[3] != ANC_STREAM_ID {
            return Err(Error::BadStreamId(b[3]));
        }
        let pes_packet_length = usize::from(u16::from_be_bytes([b[4], b[5]]));

        // Byte 6: '10'(2) + PES_scrambling_control(2)='00' + PES_priority(1) +
        // data_alignment_indicator(1)='1' + copyright(1) + original_or_copy(1).
        // ST 2038 Table 2: scrambling shall be '00', alignment shall be '1'.
        // Mask: bits[7:6]='10' (0x80), bits[5:4]=scrambling (0x30),
        //       bit[2]=data_alignment_indicator (0x04).
        // Expected fixed pattern: bits[7:6]=0b10, bits[5:4]=0b00, bit[2]=1
        //   → (f1 & 0xB4) == 0x84 checks '10'(marker) + '00'(scrambling) + '1'(align).
        let f1 = b[6];
        if (f1 & 0xB4) != 0x84 {
            if (f1 >> 6) != 0b10 {
                return Err(Error::BadFixedBits("PES '10' marker"));
            }
            if (f1 & 0x30) != 0x00 {
                return Err(Error::BadFixedBits(
                    "PES_scrambling_control shall be '00' (ST 2038 Table 2)",
                ));
            }
            // data_alignment_indicator bit[2] must be '1'.
            return Err(Error::BadFixedBits(
                "data_alignment_indicator shall be '1' (ST 2038 Table 2)",
            ));
        }
        let pes_priority = f1 & 0x08 != 0;
        let copyright = f1 & 0x02 != 0;
        let original_or_copy = f1 & 0x01 != 0;

        // Byte 7: PTS_DTS_flags(2) + 6 zero flags.
        let f2 = b[7];
        let pts_dts_flags = (f2 >> 6) & 0x03;
        if pts_dts_flags != PTS_DTS_FLAGS_PTS_ONLY {
            return Err(Error::BadPtsDtsFlags(pts_dts_flags));
        }

        // Byte 8: PES_header_data_length == 0x05.
        if b[8] != ANC_PES_HEADER_DATA_LENGTH {
            return Err(Error::BadHeaderDataLength(b[8]));
        }

        // Bytes 9..14: PTS field, prefix '0010', three marker bits.
        let pts = read_pts(&b[9..14])?;

        // Payload bounded by PES_packet_length.
        let payload_start = PES_HEADER_LEN;
        let payload_end = PES_PREFIX_LEN + pes_packet_length;
        if payload_end < payload_start {
            return Err(Error::PesLengthOverflow {
                len: pes_packet_length,
                available: b.len().saturating_sub(PES_PREFIX_LEN),
            });
        }
        if b.len() < payload_end {
            return Err(Error::PesLengthOverflow {
                len: pes_packet_length,
                available: b.len().saturating_sub(PES_PREFIX_LEN),
            });
        }
        let payload = &b[payload_start..payload_end];

        // Walk ANC records until the remaining bytes are all stuffing (0xFF).
        // Each record is byte-aligned (its '1' padding ends on a byte boundary).
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
                return Err(Error::BadFixedBits("stuffing_byte (expected 0xFF)"));
            }
        }

        Ok(Self {
            pes_priority,
            copyright,
            original_or_copy,
            pts,
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
    /// [`Error::FieldTooWide`] if any field exceeds its wire width;
    /// [`Error::InconsistentUdwLength`] if any [`AncPacket`]'s
    /// `user_data_words.len()` does not equal `data_count & 0xFF`.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "ANC PES serialize output",
            });
        }
        let pes_len = self.pes_packet_length();
        if pes_len > usize::from(u16::MAX) {
            return Err(Error::PesLengthTooLarge(pes_len));
        }

        buf[0..3].copy_from_slice(&PACKET_START_CODE_PREFIX);
        buf[3] = ANC_STREAM_ID;
        buf[4..6].copy_from_slice(&(pes_len as u16).to_be_bytes());

        // Byte 6: '10' + scrambling '00' + priority + data_alignment '1' +
        // copyright + original_or_copy.
        buf[6] = 0x80
            | (u8::from(self.pes_priority) << 3)
            | 0x04 // data_alignment_indicator = '1' (Table 2)
            | (u8::from(self.copyright) << 1)
            | u8::from(self.original_or_copy);
        // Byte 7: PTS_DTS_flags '10', all other flags 0.
        buf[7] = PTS_DTS_FLAGS_PTS_ONLY << 6;
        // Byte 8: PES_header_data_length.
        buf[8] = ANC_PES_HEADER_DATA_LENGTH;
        // Bytes 9..14: PTS field.
        buf[9..14].copy_from_slice(&write_pts(self.pts));

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

/// Decode the 5-byte ST 2038 PTS field (prefix `'0010'`, three `'1'` markers).
fn read_pts(b: &[u8]) -> Result<u64> {
    if b.len() < 5 {
        return Err(Error::BufferTooShort {
            need: 5,
            have: b.len(),
            what: "PTS field",
        });
    }
    if (b[0] >> 4) != PTS_PREFIX {
        return Err(Error::BadFixedBits("PTS prefix '0010'"));
    }
    if b[0] & 0x01 == 0 || b[2] & 0x01 == 0 || b[4] & 0x01 == 0 {
        return Err(Error::BadFixedBits("PTS marker_bit"));
    }
    let hi = u64::from((b[0] >> 1) & 0x07); // [32:30]
    let mid = (u64::from(b[1]) << 7) | u64::from(b[2] >> 1); // [29:15]
    let lo = (u64::from(b[3]) << 7) | u64::from(b[4] >> 1); // [14:0]
    Ok((hi << 30) | (mid << 15) | lo)
}

/// Encode a 33-bit PTS into the 5-byte ST 2038 PTS field (prefix `'0010'`).
fn write_pts(pts: u64) -> [u8; 5] {
    let ts = pts & PTS_MASK;
    [
        (PTS_PREFIX << 4) | ((((ts >> 30) & 0x07) as u8) << 1) | 0x01,
        ((ts >> 22) & 0xFF) as u8,
        ((((ts >> 15) & 0x7F) as u8) << 1) | 0x01,
        ((ts >> 7) & 0xFF) as u8,
        (((ts & 0x7F) as u8) << 1) | 0x01,
    ]
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
                    data_count: 0x102, // low 8 bits = 0x02 → 2 UDWs
                    user_data_words: vec![0x2CF, 0x101],
                    checksum: 0x233,
                },
                AncPacket {
                    c_not_y_channel_flag: true,
                    line_number: 0x2A,
                    horizontal_offset: 0x10,
                    did: 0x241,
                    sdid: 0x102,
                    data_count: 0x103, // 3 UDWs
                    user_data_words: vec![0x111, 0x222, 0x333],
                    checksum: 0x1AB,
                },
            ],
            stuffing_bytes: 3,
        }
    }

    #[test]
    fn pts_round_trip() {
        for ts in [0u64, 1, 90_000, 0x1_2345_6789, PTS_MASK] {
            assert_eq!(read_pts(&write_pts(ts)).unwrap(), ts, "ts={ts:#x}");
        }
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
        //   DID=0x161(10) SDID=0x101(10) data_count=0x002(10)
        //   UDW0=0x2CF(10) UDW1=0x101(10) checksum=0x233(10)
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
                data_count: 0x002,
                user_data_words: vec![0x2CF, 0x101],
                checksum: 0x233,
            }],
            stuffing_bytes: 0,
        };

        // Build the expected ANC record bit-stream independently.
        // bits: 000000 | 0 | 00000001001 | 000000000000 |
        //       0101100001 | 0100000001 | 0000000010 |
        //       1011001111 | 0100000001 | 1000110011 | pad '1'...
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
        push(0x002, 10);
        push(0x2CF, 10);
        push(0x101, 10);
        push(0x233, 10);
        while expect_bits.len() % 8 != 0 {
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

        // checksum must be reflected in the wire bytes.
        let mut e = a.clone();
        e.anc_packets[0].checksum = 0x001; // was 0x233
        let mut oe = vec![0u8; e.serialized_len()];
        e.serialize_into(&mut oe).unwrap();
        assert_ne!(oa, oe, "changing checksum must change the wire bytes");
    }

    /// Regression: serialize_into must return Err when user_data_words.len()
    /// is shorter than data_count & 0xFF (the old code silently zero-filled,
    /// making serialize→parse non-identity).
    #[test]
    fn serialize_rejects_inconsistent_udw_length() {
        let p = AncDataPacket {
            pes_priority: false,
            copyright: false,
            original_or_copy: false,
            pts: 0,
            anc_packets: vec![AncPacket {
                c_not_y_channel_flag: false,
                line_number: 1,
                horizontal_offset: 0,
                did: 0x161,
                sdid: 0x101,
                data_count: 0x003,            // expects 3 UDWs
                user_data_words: vec![0x100], // only 1 — mismatch!
                checksum: 0x001,
            }],
            stuffing_bytes: 0,
        };
        // serialized_len uses udw_loop_count() (3), so the buffer is sized for 3.
        let mut buf = vec![0u8; p.serialized_len()];
        assert!(
            matches!(
                p.serialize_into(&mut buf),
                Err(Error::InconsistentUdwLength { have: 1, need: 3 })
            ),
            "expected InconsistentUdwLength {{ have: 1, need: 3 }}"
        );
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
            matches!(AncDataPacket::parse(&out), Err(Error::BadFixedBits(_))),
            "expected BadFixedBits for non-zero scrambling_control"
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
            matches!(AncDataPacket::parse(&out), Err(Error::BadFixedBits(_))),
            "expected BadFixedBits for data_alignment_indicator=0"
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
                data_count: 0x302, // & 0xFF == 2
                user_data_words: vec![0x123, 0x3FF],
                checksum: 0x199,
            }],
            stuffing_bytes: 0,
        };
        assert_eq!(p.anc_packets[0].udw_loop_count(), 2);
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        let rp = AncDataPacket::parse(&out).unwrap();
        assert_eq!(rp, p);
        assert_eq!(rp.anc_packets[0].data_count, 0x302); // full 10-bit preserved
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
        assert!(matches!(
            AncDataPacket::parse(&out),
            Err(Error::BadStartCode(0x000002))
        ));
    }

    #[test]
    fn no_stuffing_round_trips() {
        let mut p = sample_2packet();
        p.stuffing_bytes = 0;
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        assert_eq!(AncDataPacket::parse(&out).unwrap(), p);
    }
}
