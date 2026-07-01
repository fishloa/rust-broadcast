//! Annex B ↔ length-prefixed NAL conversion — ITU-T H.264 Annex B / ISO/IEC 14496-15 §5.3.4.
//!
//! MPEG-2 TS carries H.264/HEVC as an **Annex B** byte stream: NAL units are
//! separated by start-code prefixes (`00 00 01`, optionally preceded by extra
//! `00` zero bytes forming the 4-byte `00 00 00 01`). ISOBMFF/CMAF `mdat`
//! instead carries each NAL **length-prefixed** by a fixed-size (here 4-byte)
//! big-endian length (`AVCDecoderConfigurationRecord.lengthSizeMinusOne = 3`).
//!
//! The remux pipeline rewrites each incoming Annex B access unit into the
//! length-prefixed form for the `mdat`. Trailing `zero_byte`s that pad an Annex B
//! NAL (never part of the RBSP — the final RBSP byte always carries the
//! `rbsp_stop_one_bit`, so it is non-zero) are dropped, making the length form
//! canonical and the length→Annex B→length round-trip byte-identical.

use alloc::vec::Vec;

use crate::error::{Error, Result};

/// Length in bytes of the fixed NAL length prefix used in `mdat` (4 → `lengthSizeMinusOne = 3`).
pub const NAL_LENGTH_SIZE: usize = 4;

/// Iterate the NAL units of an Annex B byte stream.
///
/// Each yielded slice is one NAL unit with its start-code prefix removed and any
/// trailing `zero_byte` padding stripped. Data before the first start code (if
/// any) is ignored.
pub fn iter_annexb_nals(annexb: &[u8]) -> AnnexBNalIter<'_> {
    AnnexBNalIter {
        data: annexb,
        code_positions: start_code_positions(annexb),
        idx: 0,
    }
}

/// Positions of every start code's first `00` (of the trailing `00 00 01`).
fn start_code_positions(data: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    let n = data.len();
    let mut p = 0usize;
    while p + 3 <= n {
        if data[p] == 0 && data[p + 1] == 0 && data[p + 2] == 1 {
            positions.push(p);
            p += 3;
        } else {
            p += 1;
        }
    }
    positions
}

/// Iterator over Annex B NAL units (see [`iter_annexb_nals`]).
pub struct AnnexBNalIter<'a> {
    data: &'a [u8],
    code_positions: Vec<usize>,
    idx: usize,
}

impl<'a> Iterator for AnnexBNalIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.idx >= self.code_positions.len() {
            return None;
        }
        // NAL body starts just after this `00 00 01`.
        let start = self.code_positions[self.idx] + 3;
        // ...and ends at the next start code's first `00` (so an extra leading
        // `00` of a 4-byte code lands in the trailing bytes and is stripped),
        // or at end of buffer for the last NAL.
        let end = self
            .code_positions
            .get(self.idx + 1)
            .copied()
            .unwrap_or(self.data.len());
        self.idx += 1;
        let mut slice = &self.data[start..end];
        // Strip trailing zero_byte padding (never part of the RBSP).
        while let Some((&0, rest)) = slice.split_last() {
            slice = rest;
        }
        // Skip degenerate empty NALs (e.g. consecutive start codes).
        if slice.is_empty() {
            return self.next();
        }
        Some(slice)
    }
}

/// Convert an Annex B access unit into length-prefixed form (4-byte big-endian
/// length before each NAL), suitable for a CMAF `mdat`.
pub fn annexb_to_length_prefixed(annexb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(annexb.len());
    for nal in iter_annexb_nals(annexb) {
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

/// Iterate the NAL units of a length-prefixed (4-byte) buffer.
///
/// Returns an error if a declared length runs past the end of the buffer.
pub fn iter_length_prefixed_nals(lp: &[u8]) -> Result<Vec<&[u8]>> {
    let mut nals = Vec::new();
    let mut off = 0usize;
    while off < lp.len() {
        if off + NAL_LENGTH_SIZE > lp.len() {
            return Err(Error::BufferTooShort {
                need: off + NAL_LENGTH_SIZE,
                have: lp.len(),
                what: "NAL length prefix",
            });
        }
        let len = u32::from_be_bytes([lp[off], lp[off + 1], lp[off + 2], lp[off + 3]]) as usize;
        let start = off + NAL_LENGTH_SIZE;
        let end = start + len;
        if end > lp.len() {
            return Err(Error::BufferTooShort {
                need: end,
                have: lp.len(),
                what: "NAL payload",
            });
        }
        nals.push(&lp[start..end]);
        off = end;
    }
    Ok(nals)
}

/// Convert a length-prefixed (4-byte) buffer back into an Annex B byte stream,
/// emitting a 4-byte start code (`00 00 00 01`) before each NAL.
pub fn length_prefixed_to_annexb(lp: &[u8]) -> Result<Vec<u8>> {
    let nals = iter_length_prefixed_nals(lp)?;
    let mut out = Vec::with_capacity(lp.len());
    for nal in nals {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_mixed_start_codes_and_strips_trailing_zeros() {
        // 4-byte SC, NAL "A" (0x67 SPS-ish), 3-byte SC, NAL "B" with an embedded
        // emulation triplet 00 00 03 that must be preserved, plus trailing zeros.
        let annexb = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42,
            0x00, // NAL A = [67 42] (trailing 00 stripped)
            0x00, 0x00, 0x01, 0x65, 0x00, 0x00, 0x03, 0x88, 0x00,
            0x00, // NAL B = [65 00 00 03 88] (trailing 00 00 stripped)
        ];
        let nals: Vec<&[u8]> = iter_annexb_nals(&annexb).collect();
        assert_eq!(nals.len(), 2);
        assert_eq!(nals[0], &[0x67, 0x42]);
        assert_eq!(nals[1], &[0x65, 0x00, 0x00, 0x03, 0x88]);
    }

    #[test]
    fn annexb_to_length_prefixed_bijection() {
        let annexb = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x99,
        ];
        let lp = annexb_to_length_prefixed(&annexb);
        // [0,0,0,2, 67,42, 0,0,0,2, 65,88,99]? second NAL is 65 88 99 = len 3
        assert_eq!(&lp[0..4], &2u32.to_be_bytes());
        assert_eq!(&lp[4..6], &[0x67, 0x42]);
        assert_eq!(&lp[6..10], &3u32.to_be_bytes());
        assert_eq!(&lp[10..13], &[0x65, 0x88, 0x99]);

        // length → annexb → length is byte-identical (canonical form).
        let back = length_prefixed_to_annexb(&lp).unwrap();
        let lp2 = annexb_to_length_prefixed(&back);
        assert_eq!(lp, lp2, "length↔annexb round-trip must be canonical");
    }

    #[test]
    fn length_prefixed_rejects_overrun() {
        // Declares a 99-byte NAL in a 6-byte buffer.
        let lp = [0x00, 0x00, 0x00, 0x63, 0xAA, 0xBB];
        assert!(iter_length_prefixed_nals(&lp).is_err());
    }
}
