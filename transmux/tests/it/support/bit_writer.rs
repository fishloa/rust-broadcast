//! MSB-first bit writer for the transmux tests that hand-build H.264/HEVC
//! RBSP bytes and SEI payloads: fixed-width `u`, Exp-Golomb `ue`/`se`
//! (ITU-T H.264 §9.1), and RBSP trailing bits (§7.3.2.11).

#[derive(Default)]
pub struct BitWriter {
    out: Vec<u8>,
    cur: u8,
    n: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bit(&mut self, b: bool) -> &mut Self {
        self.cur = (self.cur << 1) | u8::from(b);
        self.n += 1;
        if self.n == 8 {
            self.out.push(self.cur);
            self.cur = 0;
            self.n = 0;
        }
        self
    }

    /// `bits` bits of `v`, most significant first.
    ///
    /// # Panics
    /// If `v` does not fit in `bits` bits.
    pub fn u(&mut self, bits: u32, v: u64) -> &mut Self {
        assert!(bits == 64 || v >> bits == 0, "{v} wider than {bits} bits");
        for i in (0..bits).rev() {
            self.bit((v >> i) & 1 == 1);
        }
        self
    }

    pub fn ue(&mut self, v: u64) -> &mut Self {
        let code = v + 1;
        let len = 64 - code.leading_zeros();
        self.u(len - 1, 0);
        self.u(len, code)
    }

    pub fn se(&mut self, v: i64) -> &mut Self {
        self.ue(if v > 0 {
            (2 * v - 1).cast_unsigned()
        } else {
            (-2 * v).cast_unsigned()
        })
    }

    pub fn aligned(&self) -> bool {
        self.n == 0
    }

    pub fn align_zero(&mut self) -> &mut Self {
        while self.n != 0 {
            self.bit(false);
        }
        self
    }

    /// The bytes so far, the last partial byte zero-padded.
    pub fn finish(mut self) -> Vec<u8> {
        self.align_zero();
        self.out
    }

    /// `rbsp_trailing_bits()`: a one bit, then zeros to the byte boundary.
    pub fn trailing(mut self) -> Vec<u8> {
        self.bit(true);
        self.finish()
    }
}
