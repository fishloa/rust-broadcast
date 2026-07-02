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
        let mut val: u64 = 0;
        for _ in 0..n {
            let byte_idx = self.bit_pos / 8;
            let bit_in_byte = 7 - (self.bit_pos % 8);
            let bit = ((self.data[byte_idx] >> bit_in_byte) & 1) as u64;
            val = (val << 1) | bit;
            self.bit_pos += 1;
        }
        Ok(val)
    }

    /// Read one bit as a `bool`.
    pub fn read_flag(&mut self, what: &'static str) -> Result<bool> {
        Ok(self.read_bits(1, what)? != 0)
    }

    /// Consume padding bits up to the next byte boundary (e.g.
    /// `gci_alignment_zero_bit` / `ptl_reserved_zero_bit`, H.266 §7.3.3).
    pub fn align_to_byte(&mut self, what: &'static str) -> Result<()> {
        while self.bit_pos % 8 != 0 {
            let _ = self.read_bits(1, what)?;
        }
        Ok(())
    }

    /// Parse `ue(v)` — unsigned integer Exp-Golomb-coded syntax element.
    ///
    /// H.264 §9.1: leadingZeroBits (count of zero bits before the first 1-bit),
    /// then read that many bits as the unsigned value `codeNum`.
    pub fn read_ue(&mut self, what: &'static str) -> Result<u64> {
        let mut leading_zero_bits: u32 = 0;
        while self.has_bits(1) && self.read_bits(1, what)? == 0 {
            leading_zero_bits += 1;
        }
        if leading_zero_bits > 0 && !self.has_bits(leading_zero_bits as usize) {
            return Err(Error::BufferTooShort {
                need: self.bit_pos + leading_zero_bits as usize,
                have: self.data.len() * 8,
                what,
            });
        }
        if leading_zero_bits == 0 {
            return Ok(0);
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
}
