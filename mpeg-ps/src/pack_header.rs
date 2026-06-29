//! Pack Header — ISO/IEC 13818-1 §2.5.3.3, Table 2-39.
//!
//! The pack header opens every pack in an MPEG-1/2 Program Stream.
//! It carries the 42-bit System Clock Reference (SCR), the
//! `program_mux_rate`, and optional stuffing bytes.

use crate::error::{Error, Result};
use crate::scr::{self, Scr};
use broadcast_common::{Parse, Serialize};

/// `pack_start_code` — `0x000001BA`.
pub const PACK_START_CODE: u32 = 0x0000_01BA;

/// Fixed overhead of the pack header: start_code(4) + SCR field(6) +
/// mux_rate+reserved+stuffing(4) = 14 bytes.
const FIXED_LEN: usize = 14;

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

impl<'a> PackHeader<'a> {
    /// Offset to the byte immediately after this pack header (i.e. where PES data starts).
    #[must_use]
    pub fn header_len(&self) -> usize {
        self.serialized_len()
    }
}

impl<'a> Parse<'a> for PackHeader<'a> {
    type Error = Error;

    fn parse(b: &'a [u8]) -> Result<Self> {
        if b.len() < FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN,
                have: b.len(),
                what: "pack_header",
            });
        }

        if u32::from_be_bytes([b[0], b[1], b[2], b[3]]) != PACK_START_CODE {
            return Err(Error::BadPackStartCode(u32::from_be_bytes([
                b[0], b[1], b[2], b[3],
            ])));
        }

        let scr = scr::read_scr_field(&b[4..10], "SCR")?;

        // Bytes 10..13 (Table 2-39):
        // byte 10: '01'(2) | mux[21:16](6)
        // byte 11: mux[15:8](8)
        // byte 12: mux[7:0](8) — includes the 2 marker bits at bits[1:0]
        // byte 13: reserved(5, bits[7:3]) | stuffing(3, bits[2:0])
        //
        // The 22-bit mux_rate spans the bottom 22 bits of the 3 bytes.
        // Top 2 bits of byte10 are '01' prefix (like SCR).

        // Validate marker bits at byte12[1:0]
        if b[12] & 0x03 != 0x03 {
            return Err(Error::BadMarker("program_mux_rate markers"));
        }

        let program_mux_rate =
            ((u32::from(b[10] & 0x3F) << 16) | (u32::from(b[11]) << 8) | u32::from(b[12]))
                & 0x3F_FFFF;

        if program_mux_rate == 0 {
            return Err(Error::ZeroMuxRate);
        }

        let reserved = (b[13] >> 3) & 0x1F;
        let stuffing_length = b[13] & 0x07;

        let total = FIXED_LEN + stuffing_length as usize;
        if b.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: b.len(),
                what: "pack_header stuffing bytes",
            });
        }

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
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "pack_header serialize output",
            });
        }

        // pack_start_code
        buf[0..4].copy_from_slice(&PACK_START_CODE.to_be_bytes());

        // SCR field (6 bytes)
        buf[4..10].copy_from_slice(&scr::write_scr_field(self.scr));

        // program_mux_rate: 22 bits across 3 bytes
        let mux = self.program_mux_rate & 0x3F_FFFF;
        // byte 10: '01' prefix + mux[21:16]
        buf[10] = 0x40 | ((mux >> 16) & 0x3F) as u8;
        // byte 11: mux[15:8]
        buf[11] = ((mux >> 8) & 0xFF) as u8;
        // byte 12: mux[7:0] (incl. markers at bits[1:0])
        buf[12] = (mux & 0xFF) as u8;
        // byte 13: reserved(5) + stuffing(3)
        buf[13] = (self.reserved & 0x1F) << 3 | (self.stuffing_length & 0x07);

        // stuffing bytes
        buf[FIXED_LEN..len].fill(0xFF);

        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn pack_header_round_trip_fixture_pattern() {
        // Match the fixture pattern: SCR=0, mux_rate=0x03363B, reserved=0x1F, stuffing=0
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
        assert_eq!(h.program_mux_rate, 0x03363B);
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
    fn pack_header_round_trip_with_stuffing() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x40, 0x00, 0x43,
            0x03, // mux=0x43 (LSBs=markers), reserved=0, stuffing=3
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
        b[10..14].copy_from_slice(&[0x40, 0x00, 0x43, 0x00]); // mux=0x43

        let h = PackHeader::parse(&b).unwrap();
        assert_eq!(h.scr, scr);
        assert_eq!(h.program_mux_rate, 0x43);

        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], &b[..]);
    }
}
