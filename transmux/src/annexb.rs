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

/// The NAL length-prefix sizes ISO/IEC 14496-15 §5.3.3 defines:
/// `lengthSizeMinusOne` 0/1/3 → 1/2/4-byte lengths. The value 2 (3-byte
/// lengths) is reserved, so 3 is absent.
pub const VALID_NAL_LENGTH_SIZES: [usize; 3] = [1, 2, NAL_LENGTH_SIZE];

/// The `lengthSizeMinusOne` an `avcC`/`hvcC` must declare for the length size
/// this crate's IR uses ([`NAL_LENGTH_SIZE`] = 4 → 3; ISO/IEC 14496-15 §5.3.3).
/// A demuxer that normalises a sample's NAL prefixes with
/// [`normalise_nal_length_size`] must set this on the config record it emits,
/// or the init segment will describe lengths the samples do not have.
pub const NAL_LENGTH_SIZE_MINUS_ONE: u8 = (NAL_LENGTH_SIZE - 1) as u8;

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
///
/// Shared by [`iter_annexb_nals`] and the PS demuxer (which carried a verbatim
/// copy; audit r04-O1). Other scans in the crate answer different questions and
/// stay separate: `au` finds the first NAL start (shared with the PS demuxer
/// through `au::first_nal_start`) and scans incrementally from a resume offset
/// in the streaming splitter, and `mpeg_legacy::find_start_code` looks for one
/// specific `00 00 01 <code>` MPEG-2 start code.
pub(crate) fn start_code_positions(data: &[u8]) -> Vec<usize> {
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
        // A loop, not recursion: a run of consecutive start codes (all empty
        // NALs) would otherwise recurse once per start code and overflow the
        // stack — e.g. a hostile H.264 PES with `PES_packet_length = 0` fed
        // through `annexb_to_length_prefixed` (r04-W4).
        loop {
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
                continue;
            }
            return Some(slice);
        }
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
    iter_length_prefixed_nals_with(lp, NAL_LENGTH_SIZE)
}

/// Iterate the NAL units of a length-prefixed buffer whose prefix size is
/// `length_size` bytes (ISO/IEC 14496-15 §5.3.3: `lengthSizeMinusOne + 1`).
///
/// `lengthSizeMinusOne` is a 2-bit field, but only 0, 1 and 3 are defined —
/// 1-, 2- and 4-byte lengths; the value 2 (a 3-byte length) is reserved, so a
/// `length_size` of 3 is rejected rather than interpreted (the transcription at
/// `docs/codec/avcC-hvcC-14496-15.md` states `0/1/3 → 1/2/4-byte
/// NALUnitLength`).
///
/// Returns an error if `length_size` is not 1, 2 or 4, or if a declared length
/// runs past the end of the buffer.
pub fn iter_length_prefixed_nals_with(lp: &[u8], length_size: usize) -> Result<Vec<&[u8]>> {
    if !VALID_NAL_LENGTH_SIZES.contains(&length_size) {
        return Err(Error::InvalidValue {
            field: "NAL length size",
            value: length_size as u64,
            reason: "must be 1, 2 or 4 (lengthSizeMinusOne 0/1/3; 2 is reserved)",
        });
    }
    let mut nals = Vec::new();
    let mut off = 0usize;
    while off < lp.len() {
        let prefix_end = off.checked_add(length_size).ok_or(Error::BufferTooShort {
            need: usize::MAX,
            have: lp.len(),
            what: "NAL length prefix",
        })?;
        if prefix_end > lp.len() {
            return Err(Error::BufferTooShort {
                need: prefix_end,
                have: lp.len(),
                what: "NAL length prefix",
            });
        }
        let mut len: usize = 0;
        for &byte in &lp[off..prefix_end] {
            len = (len << 8) | usize::from(byte);
        }
        let start = prefix_end;
        // As in [`iter_length_prefixed_nals`]: the length is attacker-controlled,
        // so check the addition rather than assuming 64-bit `usize`.
        let end = start.checked_add(len).ok_or(Error::BufferTooShort {
            need: usize::MAX,
            have: lp.len(),
            what: "NAL length prefix overflows the buffer offset",
        })?;
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

/// Rewrite a length-prefixed access unit whose NAL length prefixes are
/// `length_size` bytes into the crate's canonical 4-byte-prefixed form
/// ([`NAL_LENGTH_SIZE`]).
///
/// A pass-through demuxer (WebM `CodecPrivate`, FLV AVC sequence header) reads
/// the `lengthSizeMinusOne` its source `avcC`/`hvcC` declares, which is 1-, 2-
/// or 4-byte (§5.3.3) — but the rest of the pipeline
/// ([`iter_length_prefixed_nals`], `nal::access_unit_is_keyframe`, `rtp`) is
/// fixed at 4 bytes, so a 2-byte-length source parses every sample as garbage.
///
/// The size-4 case is not a blind pass-through: the framing is still walked
/// ([`iter_length_prefixed_nals_with`]), so a sample whose declared lengths
/// overrun the buffer is an error for every length size — the caller can then
/// trust that a `Ok` result is a well-formed access unit. The bytes are
/// re-emitted (a copy, as the return type requires); no length size is
/// special-cased away.
///
/// `length_size` must be 1, 2 or 4 — 3 (a 3-byte length) is reserved by
/// §5.3.3 and is rejected.
pub fn normalise_nal_length_size(au: &[u8], length_size: usize) -> Result<Vec<u8>> {
    let nals = iter_length_prefixed_nals_with(au, length_size)?;
    if length_size == NAL_LENGTH_SIZE {
        // Already canonical, but the framing walk above has validated it.
        return Ok(au.to_vec());
    }
    let total: usize = nals.iter().map(|n| NAL_LENGTH_SIZE + n.len()).sum();
    let mut out = Vec::with_capacity(total);
    for nal in nals {
        let len = u32::try_from(nal.len())
            .map_err(|_| Error::InvalidInput("NAL unit longer than u32::MAX"))?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(nal);
    }
    Ok(out)
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

    /// A maximal (`0xFFFFFFFF`) declared NAL length must be rejected cleanly,
    /// never panic. This is the input that wraps `start + len` on a 32-bit
    /// `usize` — where the wrapped `end` slips under the `end > lp.len()` guard
    /// and panics on `&lp[start..end]` — hence the `checked_add` in
    /// [`iter_length_prefixed_nals`]. Reached from
    /// `cenc_encrypt::nal_subsamples` on caller-supplied sample data, so the
    /// input is untrusted. On a 64-bit target the addition cannot wrap and this
    /// exercises the plain overrun guard; the behaviour asserted (an error, no
    /// panic) is the same on both.
    #[test]
    fn length_prefixed_rejects_maximal_declared_length() {
        let lp = [0xFF, 0xFF, 0xFF, 0xFF, 0xAA, 0xBB];
        let err = iter_length_prefixed_nals(&lp).expect_err("must not be accepted");
        assert!(matches!(err, Error::BufferTooShort { .. }), "{err:?}");
    }

    /// The same maximal length at a nonzero offset (after one well-formed NAL),
    /// so `start` is not 0 when the addition is checked.
    #[test]
    fn length_prefixed_rejects_maximal_declared_length_mid_buffer() {
        let lp = [
            0x00, 0x00, 0x00, 0x01, 0x67, // one 1-byte NAL
            0xFF, 0xFF, 0xFF, 0xFF, 0xAA, // then a maximal length prefix
        ];
        assert!(iter_length_prefixed_nals(&lp).is_err());
    }

    /// r04-W4: a long run of back-to-back start codes (all empty NALs) must be
    /// skipped iteratively. Unfixed, `next()` recursed once per empty NAL, so a
    /// hostile Annex B PES drained via `annexb_to_length_prefixed` could
    /// overflow the stack (uncatchable) rather than return.
    #[test]
    fn run_of_empty_nals_does_not_recurse() {
        let mut data = Vec::new();
        for _ in 0..200_000 {
            data.extend_from_slice(&[0x00, 0x00, 0x01]);
        }
        data.extend_from_slice(&[0x00, 0x00, 0x01, 0x65, 0x88]);
        assert_eq!(iter_annexb_nals(&data).count(), 1);
        assert_eq!(annexb_to_length_prefixed(&data).len(), 4 + 2);
    }

    /// r04-W43: a 2-byte-length access unit is rewritten to 4-byte prefixes;
    /// the NAL bodies and their order are preserved exactly. A 2-byte-length
    /// prefix of `0x0003` followed by `AA BB CC`, then `0x0002` + `DD EE`.
    #[test]
    fn normalise_two_byte_lengths_to_four() {
        let au = [0x00, 0x03, 0xAA, 0xBB, 0xCC, 0x00, 0x02, 0xDD, 0xEE];
        let out = normalise_nal_length_size(&au, 2).expect("2-byte AU normalises");
        assert_eq!(
            out,
            [
                0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC, // NAL 1
                0x00, 0x00, 0x00, 0x02, 0xDD, 0xEE, // NAL 2
            ]
        );
        // And the result is what the fixed-4-byte reader sees.
        assert_eq!(iter_length_prefixed_nals(&out).unwrap().len(), 2);
    }

    /// A 3-byte length-prefix source is rejected: ISO/IEC 14496-15 §5.3.3
    /// defines `lengthSizeMinusOne` 0/1/3 (1/2/4-byte) and reserves the value 2,
    /// so a 3-byte length has no defined meaning and must not be guessed at.
    #[test]
    fn normalise_three_byte_lengths_is_rejected() {
        let au = [0x00, 0x00, 0x04, 0xAA, 0xBB, 0xCC, 0xDD];
        assert!(matches!(
            normalise_nal_length_size(&au, 3),
            Err(Error::InvalidValue {
                field: "NAL length size",
                value: 3,
                ..
            })
        ));
    }

    /// A 1-byte length-prefix source (`lengthSizeMinusOne = 0`).
    #[test]
    fn normalise_one_byte_lengths_to_four() {
        let au = [0x02, 0xAA, 0xBB];
        let out = normalise_nal_length_size(&au, 1).expect("1-byte AU normalises");
        assert_eq!(out, [0x00, 0x00, 0x00, 0x02, 0xAA, 0xBB]);
    }

    /// A 4-byte source is returned byte-identical, but its framing is still
    /// validated: a declared length past the buffer is an error even at the
    /// canonical size, so an `Ok` always means a well-formed access unit.
    #[test]
    fn normalise_four_byte_validates_framing() {
        let well_formed = [0x00, 0x00, 0x00, 0x02, 0xAA, 0xBB];
        assert_eq!(
            normalise_nal_length_size(&well_formed, 4).unwrap(),
            well_formed
        );
        // Declared length 0xFFFF with only 3 bytes present.
        assert!(matches!(
            normalise_nal_length_size(&[0x00, 0x00, 0xFF, 0xFF, 0xAA], 4),
            Err(Error::BufferTooShort { .. })
        ));
        // A truncated prefix (fewer than 4 bytes at the tail).
        assert!(matches!(
            normalise_nal_length_size(&[0x00, 0x00, 0x00, 0x01, 0xAA, 0x00, 0x00], 4),
            Err(Error::BufferTooShort { .. })
        ));
    }

    /// Hostile input: an out-of-range length size, a truncated prefix, and a
    /// declared length past the buffer are each `Err`, never a panic.
    #[test]
    fn normalise_rejects_hostile_input() {
        assert!(normalise_nal_length_size(&[0x00], 0).is_err());
        assert!(normalise_nal_length_size(&[0x00], 5).is_err());
        // Truncated 2-byte prefix (a lone byte).
        assert!(normalise_nal_length_size(&[0x00], 2).is_err());
        // Declared length 0xFFFF far past the buffer.
        assert!(normalise_nal_length_size(&[0xFF, 0xFF, 0xAA], 2).is_err());
        // A zero-length NAL is legal framing (an empty NAL), not an error.
        let out = normalise_nal_length_size(&[0x00, 0x00], 2).unwrap();
        assert_eq!(out, [0x00, 0x00, 0x00, 0x00]);
    }

    /// `iter_length_prefixed_nals_with` accepts only 1, 2 and 4 (3 is reserved
    /// by §5.3.3).
    #[test]
    fn iter_length_prefixed_with_rejects_bad_size() {
        assert!(iter_length_prefixed_nals_with(&[0x00], 0).is_err());
        assert!(iter_length_prefixed_nals_with(&[0x00], 3).is_err());
        assert!(iter_length_prefixed_nals_with(&[0x00], 5).is_err());
        // The three defined sizes are all accepted.
        for size in VALID_NAL_LENGTH_SIZES {
            assert!(iter_length_prefixed_nals_with(&[], size).is_ok());
        }
    }
}
