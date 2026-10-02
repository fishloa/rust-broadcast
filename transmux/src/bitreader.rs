//! Exp-Golomb bit reader over an RBSP byte stream.
//!
//! RBSP (Raw Byte Sequence Payload) requires **emulation prevention byte removal**
//! before parsing: every `00 00 03` triplet in the NAL unit byte stream is replaced
//! by `00 00` (the `03` is discarded).  This is done once when constructing the
//! reader via [`BitReader::with_unescape`].
//!
//! Supports `ue(v)` (unsigned) and `se(v)` (signed) Exp-Golomb coding per
//! ITU-T H.264 §9.1 and H.265 §9.2.2.

use crate::error::{Error, Result};
use alloc::vec::Vec;

/// Unescape a NAL unit byte stream into an RBSP: remove every `00 00 03` → `00 00`.
fn unescape(nal: &[u8]) -> Vec<u8> {
    let n = nal.len();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        if i + 2 < n && nal[i] == 0 && nal[i + 1] == 0 && nal[i + 2] == 3 {
            out.push(0);
            out.push(0);
            i += 3;
        } else {
            out.push(nal[i]);
            i += 1;
        }
    }
    out
}

/// Read `n` (<= 64) bits MSB-first from `data` at `*bit_pos`, advancing it.
///
/// The one cursor-style reader shared by the per-module header decoders
/// (`aac_asc`, `ac3`, `dts`), which previously each carried a copy that built a
/// fresh `broadcast_common::bits::BitReader` and re-skipped to `*bit_pos` after
/// duplicating the bounds check (audit r04-O1). Extraction delegates to
/// `broadcast_common::bits::BitReader` (shared with `dvb-t2mi`/`rdd29`/`st291`)
/// so a bit-order/overrun fix there reaches every reader. `None` when `n > 64`
/// or fewer than `n` bits remain; `*bit_pos` is then left unchanged.
pub(crate) fn read_bits_at(data: &[u8], bit_pos: &mut usize, n: usize) -> Option<u64> {
    let n_u32 = u32::try_from(n).ok().filter(|&n| n <= u64::BITS)?;
    let mut br = broadcast_common::bits::BitReader::new(data);
    br.skip_bits(*bit_pos).ok()?;
    let val = br.read_bits(n_u32).ok()?;
    *bit_pos += n;
    Some(val)
}

/// Bytes needed to hold `n` more bits starting at `bit_pos` (saturating, for
/// the `need` field of a `BufferTooShort`).
pub(crate) fn bytes_needed(bit_pos: usize, n: usize) -> usize {
    bit_pos.saturating_add(n).div_ceil(8)
}

/// Bit-level reader over an RBSP buffer (after emulation-prevention byte removal).
///
/// Reads bits from left to right within each byte (MSB-first, big-endian
/// bit numbering — ITU-T H.264 §7.2).
///
/// The struct owns the RBSP `Vec<u8>` and tracks the current bit position.
pub struct BitReader {
    data: Vec<u8>,
    bit_pos: usize,
}

impl BitReader {
    /// Create a new reader from already-unescaped RBSP bytes (no lifetime
    /// entanglement — the data is copied into the reader).
    pub fn from_rbsp(data: &[u8], what: &'static str) -> Result<Self> {
        if data.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what,
            });
        }
        Ok(Self {
            data: data.to_vec(),
            bit_pos: 0,
        })
    }

    /// Create a new reader from a NAL unit body (after the NAL header),
    /// with emulation-prevention byte removal.
    pub fn with_unescape(nal_body: &[u8], what: &'static str) -> Result<Self> {
        let rbsp = unescape(nal_body);
        if rbsp.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what,
            });
        }
        Ok(Self {
            data: rbsp,
            bit_pos: 0,
        })
    }

    fn has_bits(&self, n: usize) -> bool {
        self.bit_pos + n <= self.data.len() * 8
    }

    /// Read `n` bits as an unsigned integer (`u(n)` / `f(n)`).
    ///
    /// Extraction delegates to the shared `read_bits_at` cursor. This type
    /// stays an owning wrapper (it owns the unescaped RBSP `Vec<u8>`; a
    /// borrowing reader cannot live in the same struct without a
    /// self-referential lifetime).
    pub fn read_bits(&mut self, n: usize, what: &'static str) -> Result<u64> {
        if n > 64 || !self.has_bits(n) {
            return Err(Error::BufferTooShort {
                need: self.bit_pos + n,
                have: self.data.len() * 8,
                what,
            });
        }
        if n == 0 {
            return Ok(0);
        }
        let mut pos = self.bit_pos;
        let val = read_bits_at(&self.data, &mut pos, n).ok_or(Error::BufferTooShort {
            need: self.bit_pos.saturating_add(n),
            have: self.data.len() * 8,
            what,
        })?;
        self.bit_pos = pos;
        Ok(val)
    }

    /// Read one bit as a `bool`.
    pub fn read_flag(&mut self, what: &'static str) -> Result<bool> {
        Ok(self.read_bits(1, what)? != 0)
    }

    /// Consume padding bits up to the next byte boundary (e.g.
    /// `gci_alignment_zero_bit` / `ptl_reserved_zero_bit`, H.266 §7.3.3).
    pub fn align_to_byte(&mut self, what: &'static str) -> Result<()> {
        while !self.bit_pos.is_multiple_of(8) {
            let _ = self.read_bits(1, what)?;
        }
        Ok(())
    }

    /// Parse `ue(v)` — unsigned integer Exp-Golomb-coded syntax element.
    ///
    /// H.264 §9.1: leadingZeroBits (count of zero bits before the first 1-bit),
    /// then read that many bits as the unsigned value `codeNum`.
    pub fn read_ue(&mut self, what: &'static str) -> Result<u64> {
        // H.264 §9.1 / H.265 §9.2.2: a `ue(v)` is a leadingZeroBits run
        // terminated by a 1-bit — with no bits left there is no codeNum to
        // decode, so EOF must be an error. Returning `Ok(0)` here (the
        // pre-fix behaviour) does not consume anything, so every caller that
        // loops on the decoded value (`sps.rs`'s SPS walk) never terminates:
        // a ~20-byte hostile SPS hung the demuxer.
        if !self.has_bits(1) {
            return Err(Error::BufferTooShort {
                need: self.bit_pos + 1,
                have: self.data.len() * 8,
                what,
            });
        }
        let mut leading_zero_bits: u32 = 0;
        while self.read_bits(1, what)? == 0 {
            leading_zero_bits += 1;
            // H.264 §9.1 / H.265 §9.2.2 bound codeNum (leadingZeroBits ≤ 32);
            // a longer run is not a valid `ue(v)` and must never reach a
            // caller's loop count or shift.
            if leading_zero_bits > 32 {
                return Err(Error::InvalidValue {
                    field: what,
                    value: leading_zero_bits as u64,
                    reason: "ue(v) with more than 32 leading zero bits",
                });
            }
            if !self.has_bits(1) {
                return Err(Error::BufferTooShort {
                    need: self.bit_pos + 1,
                    have: self.data.len() * 8,
                    what,
                });
            }
        }
        // The loop exits just past the terminating 1-bit; the
        // `leading_zero_bits` info bits must still be present. (With a
        // leading run of zero this check is trivially true and the result is
        // codeNum 0, matching §9.1.)
        if !self.has_bits(leading_zero_bits as usize) {
            return Err(Error::BufferTooShort {
                need: self.bit_pos + leading_zero_bits as usize,
                have: self.data.len() * 8,
                what,
            });
        }
        let info = self.read_bits(leading_zero_bits as usize, what)?;
        Ok((1u64 << leading_zero_bits) - 1 + info)
    }

    /// Parse `se(v)` — signed integer Exp-Golomb-coded syntax element.
    ///
    /// H.264 §9.1.1: mapping from `codeNum` to signed value.
    pub fn read_se(&mut self, what: &'static str) -> Result<i64> {
        let code_num = self.read_ue(what)?;
        if code_num & 1 == 0 {
            Ok(-((code_num >> 1) as i64))
        } else {
            Ok(((code_num + 1) >> 1) as i64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn unescape_removes_emulation_prevention_bytes() {
        let nal = [0x00, 0x00, 0x03, 0x04, 0x00, 0x00];
        let rbsp = unescape(&nal);
        assert_eq!(rbsp, &[0x00, 0x00, 0x04, 0x00, 0x00]);
    }

    #[test]
    fn unescape_no_epb_passthrough() {
        let nal = [0x01, 0x02, 0x03, 0x04];
        let rbsp = unescape(&nal);
        assert_eq!(rbsp, nal);
    }

    #[test]
    fn ue_simple() {
        let mut r = BitReader::from_rbsp(&[0x80], "test").unwrap();
        assert_eq!(r.read_ue("test").unwrap(), 0);
    }

    #[test]
    fn ue_value_3() {
        let mut r = BitReader::from_rbsp(&[0x20], "test").unwrap();
        assert_eq!(r.read_ue("test").unwrap(), 3);
    }

    #[test]
    fn se_signed() {
        let mut r = BitReader::from_rbsp(&[0x80], "test").unwrap();
        assert_eq!(r.read_se("test").unwrap(), 0);

        let mut r = BitReader::from_rbsp(&[0x40], "test").unwrap();
        assert_eq!(r.read_se("test").unwrap(), 1);

        let mut r = BitReader::from_rbsp(&[0x60], "test").unwrap();
        assert_eq!(r.read_se("test").unwrap(), -1);
    }

    /// H.264 §9.1: `ue(v)` is a leadingZeroBits run terminated by a 1-bit —
    /// with no bits left there is no codeNum to decode, so EOF must be `Err`.
    /// Pre-fix it returned `Ok(0)` **without consuming anything**, and every
    /// caller that loops on a decoded count (`sps.rs`'s
    /// `num_ref_frames_in_pic_order_cnt_cycle`) spun forever — the ~20-byte
    /// SPS demux hang.
    #[test]
    fn read_ue_at_eof_is_error() {
        // Eight 1-bits: eight `ue(v) == 0`, then the reader is empty.
        let mut r = BitReader::from_rbsp(&[0xFF], "test").unwrap();
        for _ in 0..8 {
            assert_eq!(r.read_ue("test").unwrap(), 0);
        }
        assert!(r.read_ue("ue at EOF").is_err());
    }

    /// H.264 §9.1 / H.265 §9.2.2 bound `leadingZeroBits` (codeNum < 2^33):
    /// more than 32 leading zeros cannot be a valid `ue(v)`. Pre-fix this
    /// decoded "fine" to ~2^41, handing callers like an SPS cycle-count loop
    /// an absurd iteration budget.
    #[test]
    fn read_ue_beyond_32_leading_zeros_is_error() {
        // 40 leading zero bits, then a 1-bit and 40 info bits — enough tail
        // (88 bits) that the pre-fix reader returned `Ok(~2^41)` rather than
        // hitting EOF.
        let mut rbsp = vec![0x00u8; 5];
        rbsp.extend_from_slice(&[0xFFu8; 6]);
        let mut r = BitReader::from_rbsp(&rbsp, "test").unwrap();
        assert!(r.read_ue("huge ue").is_err());
    }
}
