//! Pack Header — ISO/IEC 13818-1 §2.5.3.3, Table 2-39.
//!
//! The pack header opens every pack in an MPEG-1/2 Program Stream.
//! It carries the 42-bit System Clock Reference (SCR), the
//! `program_mux_rate`, and optional stuffing bytes.

use crate::error::{Error, Result};
use crate::scr::{self, Scr};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// `pack_start_code` — `0x000001BA`.
pub const PACK_START_CODE: u32 = 0x0000_01BA;

/// Fixed overhead of the pack header: start_code(4) + SCR field(6) +
/// mux_rate+reserved+stuffing(4) = 14 bytes.
const FIXED_LEN: usize = 14;

/// `program_mux_rate` width in bits (Table 2-39, ISO/IEC 13818-1 §2.5.3.3).
const PROGRAM_MUX_RATE_BITS: u32 = 22;
/// `reserved` field width in bits (Table 2-39, byte 13 bits[7:3]).
const RESERVED_BITS: u32 = 5;
/// Largest `pack_stuffing_length` (3-bit field, Table 2-39).
const STUFFING_LENGTH_MAX: u8 = 7;

/// Byte offsets in the pack header (ISO/IEC 13818-1 §2.5.3.3, Table 2-39).
const START_CODE_OFFSET: usize = 0;
const SCR_OFFSET: usize = START_CODE_OFFSET + core::mem::size_of::<u32>();
/// Byte width of the 6-byte SCR field.
const SCR_FIELD_LEN: usize = 6;
const MUX_RATE_OFFSET: usize = SCR_OFFSET + SCR_FIELD_LEN;
/// Byte offset of the `program_mux_rate` low bits, which share byte 12 with the
/// two marker bits.
const MUX_RATE_LO_OFFSET: usize = MUX_RATE_OFFSET + 2;
/// Byte offset of the `reserved` + `pack_stuffing_length` byte.
const RESERVED_STUFFING_OFFSET: usize = MUX_RATE_LO_OFFSET + 1;
/// Bit shifts of the 22-bit `program_mux_rate` across bytes 10..12.
const MUX_RATE_HI_SHIFT: u32 = 14;
const MUX_RATE_MID_SHIFT: u32 = 6;
const MUX_RATE_LO_SHIFT: u32 = 2;
/// Mask of the `program_mux_rate` low bits in byte 12.
const MUX_RATE_LO_MASK: u32 = 0x3F;
/// Mask of the two marker bits at the bottom of byte 12.
const MARKER_BITS_MASK: u8 = 0x03;
/// Bit shift of the `reserved` field within byte 13.
const RESERVED_SHIFT: u32 = 3;
/// Mask of the `reserved` field within byte 13.
const RESERVED_MASK: u8 = 0x1F;

/// Read the big-endian `u32` at `off`.
fn read_u32_at(b: &[u8], off: usize) -> u32 {
    let mut raw = [0_u8; core::mem::size_of::<u32>()];
    raw.copy_from_slice(&b[off..off + core::mem::size_of::<u32>()]);
    u32::from_be_bytes(raw)
}

/// A parsed pack header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PackHeader<'a> {
    /// System Clock Reference (42-bit; 33-bit base + 9-bit extension, 27 MHz).
    pub scr: Scr,
    /// Rate at which the P-STD receives the stream during this pack,
    /// in units of 50 bytes/s. Must be non-zero.
    pub program_mux_rate: u32,
    /// The `pack_stuffing_length` field value (≤ 7).
    pub stuffing_length: u8,
    /// Stuffing bytes (`0xFF`), zero to seven bytes.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub stuffing: &'a [u8],
    /// Reserved bits (5 bits, preserved for round-trip).
    pub reserved: u8,
}

impl PackHeader<'_> {
    /// Offset to the byte immediately after this pack header (i.e. where PES data starts).
    #[must_use]
    pub fn header_len(&self) -> usize {
        self.serialized_len()
    }
}

impl<'a> Parse<'a> for PackHeader<'a> {
    type Error = Error;

    fn parse(b: &'a [u8]) -> Result<Self> {
        need_in(b, FIXED_LEN, "pack_header")?;

        if read_u32_at(b, START_CODE_OFFSET) != PACK_START_CODE {
            return Err(Error::BadPackStartCode(read_u32_at(b, START_CODE_OFFSET)));
        }

        let scr = scr::read_scr_field(&b[SCR_OFFSET..MUX_RATE_OFFSET], "SCR")?;

        // Bytes 10..13 (Table 2-39, ISO/IEC 13818-1 §2.5.3.3): unlike the SCR
        // field, there is no `'01'` prefix before `program_mux_rate` — the
        // field is the TOP 22 bits of bytes 10..12, followed by the 2 marker
        // bits in byte 12 bits[1:0]:
        // byte 10: mux[21:14](8)
        // byte 11: mux[13:6](8)
        // byte 12: mux[5:0](6, bits[7:2]) | marker_bit(2, bits[1:0])
        // byte 13: reserved(5, bits[7:3]) | stuffing(3, bits[2:0])

        if b[MUX_RATE_LO_OFFSET] & MARKER_BITS_MASK != MARKER_BITS_MASK {
            return Err(Error::BadMarker("program_mux_rate markers"));
        }

        let program_mux_rate = (u32::from(b[MUX_RATE_OFFSET]) << MUX_RATE_HI_SHIFT)
            | (u32::from(b[MUX_RATE_OFFSET + 1]) << MUX_RATE_MID_SHIFT)
            | (u32::from(b[MUX_RATE_LO_OFFSET]) >> MUX_RATE_LO_SHIFT);

        if program_mux_rate == 0 {
            return Err(Error::ZeroMuxRate);
        }

        let reserved = (b[RESERVED_STUFFING_OFFSET] >> RESERVED_SHIFT) & RESERVED_MASK;
        let stuffing_length = b[RESERVED_STUFFING_OFFSET] & STUFFING_LENGTH_MAX;

        let total = FIXED_LEN + stuffing_length as usize;
        need_in(b, total, "pack_header stuffing bytes")?;

        Ok(PackHeader {
            scr,
            program_mux_rate,
            stuffing_length,
            stuffing: &b[FIXED_LEN..total],
            reserved,
        })
    }
}

impl Serialize for PackHeader<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        FIXED_LEN + self.stuffing_length as usize
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        // Range-check every narrowed field BEFORE sizing/writing: a silent
        // mask would make `serialized_len()` (unmasked stuffing_length)
        // disagree with the bytes actually framed (#1129).
        if self.stuffing_length > STUFFING_LENGTH_MAX {
            return Err(Error::StuffingLengthTooLarge(self.stuffing_length));
        }
        // `parse` refuses a zero `program_mux_rate` (Table 2-39 semantics,
        // ISO/IEC 13818-1 §2.5.3.4), so writing one would emit a header that
        // cannot be read back (#2744).
        if self.program_mux_rate == 0 {
            return Err(Error::ZeroMuxRate);
        }
        let mux = broadcast_common::len::fit_bits(
            u64::from(self.program_mux_rate),
            PROGRAM_MUX_RATE_BITS,
            "pack_header.program_mux_rate",
        )? as u32;
        let reserved = broadcast_common::len::fit_bits(
            u64::from(self.reserved),
            RESERVED_BITS,
            "pack_header.reserved",
        )? as u8;
        let len = self.serialized_len();
        need_out(buf, len)?;

        // pack_start_code
        buf[START_CODE_OFFSET..SCR_OFFSET].copy_from_slice(&PACK_START_CODE.to_be_bytes());

        // SCR field (6 bytes)
        buf[SCR_OFFSET..MUX_RATE_OFFSET].copy_from_slice(&scr::encode_scr_field(self.scr)?);

        // program_mux_rate: 22 bits across bytes 10..12, no '01' prefix
        // (Table 2-39; see the parse-side comment above).
        // byte 10: mux[21:14]
        buf[MUX_RATE_OFFSET] = (mux >> MUX_RATE_HI_SHIFT) as u8;
        // byte 11: mux[13:6]
        buf[MUX_RATE_OFFSET + 1] = (mux >> MUX_RATE_MID_SHIFT) as u8;
        // byte 12: mux[5:0] + marker bits (always '11') at bits[1:0]
        buf[MUX_RATE_LO_OFFSET] =
            (((mux & MUX_RATE_LO_MASK) as u8) << MUX_RATE_LO_SHIFT) | MARKER_BITS_MASK;
        // byte 13: reserved(5) + stuffing(3)
        buf[RESERVED_STUFFING_OFFSET] = reserved << RESERVED_SHIFT | self.stuffing_length;

        // stuffing bytes: `stuffing_byte` is spec-fixed `0xFF` (Table 2-39),
        // but parse does not reject a non-conformant value (it stores
        // whatever bytes are present in `self.stuffing`) — write them back
        // verbatim rather than always `0xFF`, so a hand-built or
        // non-strictly-conformant-but-parseable `PackHeader` still
        // round-trips byte-identically (#1119 W8). `stuffing_length` and
        // `stuffing` are independently-settable public fields; validate they
        // agree before indexing by one and copying a slice sized by the
        // other.
        if self.stuffing.len() != self.stuffing_length as usize {
            return Err(Error::StuffingLengthMismatch {
                declared: self.stuffing_length,
                actual: self.stuffing.len(),
            });
        }
        buf[FIXED_LEN..len].copy_from_slice(self.stuffing);

        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// #2269: a too-small output buffer is an output-buffer error, not a
    /// parse-side input-truncation error.
    #[test]
    fn serialize_into_too_small_output_reports_output_buffer_error() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x36, 0x3B, 0xF8,
        ];
        let h = PackHeader::parse(&bytes).unwrap();
        let mut out = vec![0u8; h.serialized_len() - 1];
        assert!(matches!(
            h.serialize_into(&mut out),
            Err(Error::OutputBufferTooSmall(_))
        ));
    }

    #[test]
    fn pack_header_round_trip_fixture_pattern() {
        // Byte-for-byte the real fixture's first pack header (fixtures/ps/
        // ffmpeg-mpeg2-ps.mpg, bytes 0..14): SCR=0, reserved=0x1F, stuffing=0.
        // mux = (0x43<<14)|(0x36<<6)|(0x3B>>2) = 0x10CD8E (Table 2-39, no '01'
        // prefix before program_mux_rate — see the parse-side comment), which
        // matches this same fixture's system_header.rate_bound (0x10CD8E,
        // cross-checked in `real_fixture_walk` / see mpeg-ps/tests/it/fixture_ps.rs).
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, // SCR=0
            0x43, 0x36, 0x3B, 0xF8, // mux + reserved + stuffing
        ];
        let h = PackHeader::parse(&bytes).unwrap();
        assert_eq!(
            h.scr,
            Scr {
                base: 0,
                extension: 0,
            }
        );
        assert_eq!(h.program_mux_rate, 0x10CD8E);
        assert_eq!(h.stuffing_length, 0);
        assert_eq!(h.reserved, 0x1F);
        assert!(h.stuffing.is_empty());

        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], &bytes[..], "round-trip mismatch");

        // Biting mutation: change mux_rate → output differs
        let h_mut = PackHeader {
            program_mux_rate: 0x12345,
            ..h.clone()
        };
        let mut out2 = vec![0u8; h_mut.serialized_len()];
        h_mut.serialize_into(&mut out2).unwrap();
        assert_ne!(&out[..], &out2[..]);
    }

    #[test]
    fn serialize_rejects_out_of_range_fields() {
        let ok = PackHeader {
            scr: Scr {
                base: 0,
                extension: 0,
            },
            program_mux_rate: 1,
            stuffing_length: 0,
            stuffing: &[],
            reserved: 0x1F,
        };
        let mut out = vec![0u8; 64];
        assert!(ok.serialize_into(&mut out).is_ok());

        let long = [0xFFu8; 9];
        let bad = PackHeader {
            stuffing_length: 9,
            stuffing: &long,
            ..ok.clone()
        };
        assert_eq!(
            bad.serialize_into(&mut out),
            Err(Error::StuffingLengthTooLarge(9))
        );
        let bad = PackHeader {
            program_mux_rate: 0x40_0000,
            ..ok.clone()
        };
        assert!(matches!(
            bad.serialize_into(&mut out),
            Err(Error::FieldOverflow(_))
        ));
        let bad = PackHeader {
            reserved: 0x20,
            ..ok.clone()
        };
        assert!(matches!(
            bad.serialize_into(&mut out),
            Err(Error::FieldOverflow(_))
        ));
    }

    #[test]
    fn pack_header_round_trip_with_stuffing() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x00, 0x01,
            0x0F, // mux=0x43: byte10=mux[21:14], byte11=mux[13:6], byte12=mux[5:0]<<2|markers
            0x03, // reserved=0, stuffing=3
            0xFF, 0xFF, 0xFF,
        ];
        let h = PackHeader::parse(&bytes).unwrap();
        assert_eq!(h.program_mux_rate, 0x43);
        assert_eq!(h.stuffing_length, 3);
        assert_eq!(h.stuffing, &[0xFF, 0xFF, 0xFF]);
        assert_eq!(h.reserved, 0);

        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], &bytes[..]);

        // Re-parse equals original
        let h2 = PackHeader::parse(&out).unwrap();
        assert_eq!(h, h2);

        // Biting mutation: change reserved → output differs
        let h_mut = PackHeader {
            reserved: 0x0A,
            ..h.clone()
        };
        let mut out2 = vec![0u8; h_mut.serialized_len()];
        h_mut.serialize_into(&mut out2).unwrap();
        assert_ne!(&out[..], &out2[..]);
    }

    #[test]
    fn pack_header_nonzero_scr() {
        let scr = Scr {
            base: 0x12345678,
            extension: 0x0AA,
        };
        let scr_enc = scr::write_scr_field(scr);
        let mut b = vec![0u8; 14];
        b[0..4].copy_from_slice(&PACK_START_CODE.to_be_bytes());
        b[4..10].copy_from_slice(&scr_enc);
        b[10..14].copy_from_slice(&[0x00, 0x01, 0x0F, 0x00]); // mux=0x43

        let h = PackHeader::parse(&b).unwrap();
        assert_eq!(h.scr, scr);
        assert_eq!(h.program_mux_rate, 0x43);

        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], &b[..]);
    }

    /// W8 (#1119): serialize used to always write stuffing bytes as `0xFF`
    /// regardless of `self.stuffing`, even though `parse` already captured
    /// the actual bytes present. Non-`0xFF` stuffing (non-conformant to the
    /// spec's fixed `stuffing_byte`, but not rejected by `parse` either)
    /// must still round-trip byte-identically.
    #[test]
    fn non_0xff_stuffing_round_trips_byte_identically() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x00, 0x01, 0x0F,
            0x03, // reserved=0, stuffing_length=3
            0xAB, 0xCD, 0xEF, // non-0xFF stuffing
        ];
        let h = PackHeader::parse(&bytes).unwrap();
        assert_eq!(h.stuffing, &[0xAB, 0xCD, 0xEF]);

        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], &bytes[..]);
    }

    /// W8 (#1119): `stuffing_length` and `stuffing` are independently
    /// settable public fields; serialize must reject a mismatch rather than
    /// panic on the length-mismatched `copy_from_slice`.
    #[test]
    fn serialize_rejects_stuffing_length_mismatch() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x00, 0x01, 0x0F, 0x03,
            0xFF, 0xFF, 0xFF,
        ];
        let mut h = PackHeader::parse(&bytes).unwrap();
        h.stuffing = &[0xFF, 0xFF]; // 2 bytes, but stuffing_length still says 3

        let mut out = vec![0u8; h.serialized_len()];
        let err = h.serialize_into(&mut out).unwrap_err();
        assert!(matches!(
            err,
            Error::StuffingLengthMismatch {
                declared: 3,
                actual: 2
            }
        ));
    }
}
