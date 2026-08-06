//! Stream cipher — LFSR-based keystream generator.
//!
//! Match-exact reimplementation of libdvbcsa's dvbcsa_stream_xor.
//! Uses two 40-bit shift registers (A, B) with sboxes, CFED chain,
//! and output table STREAM_OUT.
use super::tables::{STREAM_CDEF, STREAM_OUT, STREAM_SBOX};

/// Nibble from shift register at position n (nibble index)
fn nbget(r: u64, n: u64) -> u64 {
    r >> (n * 4)
}

/// The DVB-CSA2 stream cipher.
pub(crate) struct StreamCipher {
    a: u64,
    b: u64,
    pqzyx: u32,
    cfed: u32,
}

impl StreamCipher {
    /// Initialize the stream cipher with the nibble-swapped CW and an 8-byte IV.
    pub(crate) fn new(cws: &[u8; 8], iv: &[u8; 8]) -> Self {
        let a = u64::from(u32::from_le_bytes([cws[0], cws[1], cws[2], cws[3]]));
        let b = u64::from(u32::from_le_bytes([cws[4], cws[5], cws[6], cws[7]]));

        let mut sc = Self {
            a,
            b,
            pqzyx: 0,
            cfed: 0,
        };

        for &iv_byte in iv.iter() {
            sc.init_round(iv_byte);
            sc.init_round(swap_nbl(iv_byte));
            sc.init_round(iv_byte);
            sc.init_round(swap_nbl(iv_byte));
        }

        sc
    }

    /// XOR the stream cipher's keystream into `data` in-place.
    pub(crate) fn xor_stream(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            // 4 rounds produce one keystream byte
            self.generate_round();
            *byte ^= STREAM_OUT[self.getd() as usize & 0xf] & 0xc0;

            self.generate_round();
            *byte ^= STREAM_OUT[self.getd() as usize & 0xf] & 0x30;

            self.generate_round();
            *byte ^= STREAM_OUT[self.getd() as usize & 0xf] & 0x0c;

            self.generate_round();
            *byte ^= STREAM_OUT[self.getd() as usize & 0xf] & 0x03;
        }
    }

    fn getx(&self) -> u32 {
        self.pqzyx & 0xf
    }
    fn gety(&self) -> u32 {
        (self.pqzyx >> 4) & 0xf
    }
    fn getd(&self) -> u32 {
        self.cfed & 0xf
    }
    fn tstp(&self) -> bool {
        (self.pqzyx & 0x1000) != 0
    }

    fn init_round(&mut self, iv_nibble: u8) {
        let iv = u32::from(iv_nibble);

        self.a <<= 4;
        self.a |= (nbget(self.a, 10)
            ^ u64::from(self.getx())
            ^ u64::from(self.getd())
            ^ u64::from(iv >> 4))
            & 0x0f;

        let mut tmp =
            (nbget(self.b, 6) ^ nbget(self.b, 9) ^ u64::from(self.gety()) ^ u64::from(iv)) & 0x0f;
        tmp = csa_stream_rotate(self.tstp(), tmp as u32) as u64;

        self.b <<= 4;
        self.b |= tmp;

        self.cfed = csa_stream_cfed(self.pqzyx, self.cfed) ^ csa_stream_b_sel(self.b);

        self.pqzyx = csa_stream_sboxes(self.a);
    }

    fn generate_round(&mut self) {
        self.a <<= 4;
        self.a |= (nbget(self.a, 10) ^ u64::from(self.getx())) & 0xf;

        let mut tmp = (nbget(self.b, 6) ^ nbget(self.b, 9) ^ u64::from(self.gety())) & 0xf;
        tmp = csa_stream_rotate(self.tstp(), tmp as u32) as u64;

        self.b <<= 4;
        self.b |= tmp;

        self.cfed = csa_stream_cfed(self.pqzyx, self.cfed) ^ csa_stream_b_sel(self.b);

        self.pqzyx = csa_stream_sboxes(self.a);
    }
}

fn swap_nbl(byte: u8) -> u8 {
    byte.rotate_left(4)
}

fn csa_stream_rotate(p: bool, x: u32) -> u32 {
    if p {
        ((x << 1) | ((x >> 3) & 1)) & 0xf
    } else {
        x
    }
}

fn csa_stream_sboxes(a: u64) -> u32 {
    let mut t = a & 0x2018004200u64;
    let mut res = u32::from(
        STREAM_SBOX[1]
            [(((t >> 37) ^ (t >> 27) ^ (t >> 25) ^ (t >> 11) ^ (t >> 5)) & 0x1f) as usize],
    );

    t = a & 0x4201480000u64;
    res |= u32::from(
        STREAM_SBOX[4]
            [(((t >> 38) ^ (t >> 32) ^ (t >> 22) ^ (t >> 16) ^ (t >> 18)) & 0x1f) as usize],
    );

    t = a & 0x8040122000u64;
    res |= u32::from(
        STREAM_SBOX[5]
            [(((t >> 39) ^ (t >> 29) ^ (t >> 18) ^ (t >> 14) ^ (t >> 9)) & 0x1f) as usize],
    );

    t = a & 0x1082010040u64;
    res |= u32::from(
        STREAM_SBOX[0]
            [(((t >> 36) ^ (t >> 30) ^ (t >> 23) ^ (t >> 3) ^ (t >> 12)) & 0x1f) as usize],
    );

    t = a & 0x0004a00180u64;
    res |= u32::from(
        STREAM_SBOX[2][(((t >> 26) ^ (t >> 22) ^ (t >> 19) ^ (t >> 5) ^ (t >> 3)) & 0x1f) as usize],
    );

    t = a & 0x0100048820u64;
    res |= u32::from(
        STREAM_SBOX[3][(((t >> 32) ^ (t >> 17) ^ (t >> 9) ^ (t >> 2) ^ (t >> 11)) & 0x1f) as usize],
    );

    t = a & 0x0c20001400u64;
    res |= u32::from(
        STREAM_SBOX[6][(((t >> 35) ^ (t >> 33) ^ (t >> 27) ^ (t >> 9) ^ (t >> 6)) & 0x1f) as usize],
    );

    res
}

fn csa_stream_b_sel(b: u64) -> u32 {
    // C code: `uint32_t t = B >> 9;` — truncation to 32 bits is load-bearing.
    let t = (b >> 9) as u32;

    ((t ^ (t >> 27)) & 0x8)
        ^ ((t >> 18) & 0x9)
        ^ (((t >> 22) ^ (t >> 7)) & 0x4)
        ^ ((t >> 4) & 0x5)
        ^ (((t >> 24) ^ (t >> 6) ^ (t >> 11)) & 0x2)
        ^ (((t >> 29) ^ (t >> 23)) & 0x1)
        ^ ((t >> 13) & 0xe)
}

fn csa_stream_cfed(pqzyx: u32, cfed: u32) -> u32 {
    let cdef_idx = (((cfed & 0x10ff) | (pqzyx & 0x2f00)) >> 4) as usize;
    ((cfed & 0x0f00) >> 4) | u32::from(STREAM_CDEF[cdef_idx])
}
