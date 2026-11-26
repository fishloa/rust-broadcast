//! MSB-first bit writer for the workspace test suites that hand-build H.264 /
//! HEVC RBSP bytes and SEI payloads: fixed-width `u`, Exp-Golomb `ue`/`se`
//! (ITU-T H.264 §9.1), and RBSP trailing bits (§7.3.2.11).
//!
//! One copy, shared by every suite that builds a bitstream: the parsers stay
//! the only readers of the format, and a suite never re-implements the writer
//! (a second writer would let a test drift from the syntax it claims to build).

use broadcast_core::bits::BitBuf;

/// MSB-first bit writer over the workspace bit buffer.
#[derive(Default)]
pub struct BitWriter {
    buf: BitBuf,
}

impl BitWriter {
    /// A writer positioned at the first bit.
    pub fn new() -> Self {
        Self::default()
    }

    /// One bit.
    pub fn bit(&mut self, b: bool) -> &mut Self {
        self.u(1, u64::from(b))
    }

    /// `bits` bits of `v`, most significant first.
    ///
    /// # Panics
    /// If `v` does not fit in `bits` bits (a hand-built fixture that would
    /// silently truncate is a broken fixture).
    pub fn u(&mut self, bits: u32, v: u64) -> &mut Self {
        self.buf
            .write_bits(v, bits)
            .unwrap_or_else(|e| panic!("{v} in {bits} bits: {e}"));
        self
    }

    /// Exp-Golomb `ue(v)`.
    pub fn ue(&mut self, v: u64) -> &mut Self {
        let code = v + 1;
        let len = 64 - code.leading_zeros();
        self.u(len - 1, 0);
        self.u(len, code)
    }

    /// Exp-Golomb `se(v)`.
    pub fn se(&mut self, v: i64) -> &mut Self {
        self.ue(if v > 0 {
            (2 * v - 1).cast_unsigned()
        } else {
            (-2 * v).cast_unsigned()
        })
    }

    /// Whether the next bit starts a byte.
    pub fn aligned(&self) -> bool {
        self.buf.is_byte_aligned()
    }

    /// Pad with zero bits to the byte boundary.
    pub fn align_zero(&mut self) -> &mut Self {
        self.buf.align_to_byte();
        self
    }

    /// The bytes so far, the last partial byte zero-padded.
    pub fn finish(self) -> Vec<u8> {
        self.buf.into_vec()
    }

    /// `rbsp_trailing_bits()`: a one bit, then zeros to the byte boundary.
    pub fn trailing(mut self) -> Vec<u8> {
        self.bit(true);
        self.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_golomb_and_trailing_bits_have_the_h264_layout() {
        let mut w = BitWriter::new();
        w.ue(0).ue(1).ue(2).ue(3).se(1).se(-1);
        assert!(!w.aligned());
        // 1 010 011 00100 010 011 + stop bit + zero pad
        assert_eq!(w.trailing(), [0b1010_0110, 0b0100_0100, 0b1110_0000]);
    }

    #[test]
    fn a_wide_value_and_a_64_bit_field() {
        let mut w = BitWriter::new();
        w.u(64, u64::MAX).u(3, 0b101).align_zero();
        assert!(w.aligned());
        assert_eq!(
            w.finish(),
            [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0b1010_0000]
        );
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn a_value_wider_than_its_field_panics() {
        BitWriter::new().u(2, 4);
    }
}
