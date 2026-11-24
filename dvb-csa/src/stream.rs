//! Stream cipher — LFSR-based keystream generator.
//!
//! Match-exact reimplementation of libdvbcsa's dvbcsa_stream_xor (the CSA2 algorithm text is not vendored in private/specs; ETSI TS 100 289 covers CSA v3 signalling only).
//! Uses two 40-bit shift registers (A, B) with sboxes, CFED chain,
//! and output table STREAM_OUT.
use super::tables::{STREAM_CDEF, STREAM_OUT, STREAM_SBOX};

/// Bits in a nibble — the shift registers advance one nibble per round.
const NIBBLE_BITS: u32 = 4;
/// Mask of a nibble.
const NIBBLE_MASK: u32 = 0xf;
/// Bit of `pqzyx` selecting the B-register feedback rotation (`tstp`).
const P_BIT_MASK: u32 = 0x1000;
/// Nibble of A read back after the register shift (`nbget(a, 10)`).
const A_FEEDBACK_NIBBLE: u32 = 10;
/// First nibble of B XOR-folded into its feedback (`nbget(b, 6)`).
const B_FEEDBACK_NIBBLE_0: u32 = 6;
/// Second nibble of B XOR-folded into its feedback (`nbget(b, 9)`).
const B_FEEDBACK_NIBBLE_1: u32 = 9;
/// Bytes in the control-word seed and the initialisation vector.
const SEED_BYTES: usize = 8;
/// Bytes in each of the seed's two register halves.
const SEED_REGISTER_BYTES: usize = core::mem::size_of::<u32>();
/// Masks placing each round's keystream bit pair in the output byte, most
/// significant pair first.
const KEYSTREAM_PAIR_HI_MASK: u8 = 0xc0;
const KEYSTREAM_PAIR_MID_HI_MASK: u8 = 0x30;
const KEYSTREAM_PAIR_MID_LO_MASK: u8 = 0x0c;
const KEYSTREAM_PAIR_LO_MASK: u8 = 0x03;
/// Bits in a `STREAM_SBOX` index.
const SBOX_INDEX_BITS: usize = 5;
/// Mask of a five-bit `STREAM_SBOX` index.
const SBOX_INDEX_MASK: u64 = 0x1f;
/// Positions of the third, fourth and fifth index-bit shifts.
const SHIFT_POS_2: usize = 2;
const SHIFT_POS_3: usize = 3;
const SHIFT_POS_4: usize = 4;
/// `STREAM_SBOX` row each unrolled S-box indexes.
const SBOX_0_ROW: usize = 0;
const SBOX_1_ROW: usize = 1;
const SBOX_2_ROW: usize = 2;
const SBOX_3_ROW: usize = 3;
const SBOX_4_ROW: usize = 4;
const SBOX_5_ROW: usize = 5;
const SBOX_6_ROW: usize = 6;
/// A-register mask selecting the five bits of each `STREAM_SBOX` index.
const SBOX_0_MASK: u64 = 0x1082010040;
const SBOX_1_MASK: u64 = 0x2018004200;
const SBOX_2_MASK: u64 = 0x0004a00180;
const SBOX_3_MASK: u64 = 0x0100048820;
const SBOX_4_MASK: u64 = 0x4201480000;
const SBOX_5_MASK: u64 = 0x8040122000;
const SBOX_6_MASK: u64 = 0x0c20001400;
/// A-register shifts producing each of an S-box's five index bits.
const SBOX_0_SHIFTS: [u32; SBOX_INDEX_BITS] = [36, 30, 23, 3, 12];
const SBOX_1_SHIFTS: [u32; SBOX_INDEX_BITS] = [37, 27, 25, 11, 5];
const SBOX_2_SHIFTS: [u32; SBOX_INDEX_BITS] = [26, 22, 19, 5, 3];
const SBOX_3_SHIFTS: [u32; SBOX_INDEX_BITS] = [32, 17, 9, 2, 11];
const SBOX_4_SHIFTS: [u32; SBOX_INDEX_BITS] = [38, 32, 22, 16, 18];
const SBOX_5_SHIFTS: [u32; SBOX_INDEX_BITS] = [39, 29, 18, 14, 9];
const SBOX_6_SHIFTS: [u32; SBOX_INDEX_BITS] = [35, 33, 27, 9, 6];
/// Bits of `cfed` feeding the `STREAM_CDEF` index (D, E and the C bit).
const CFED_INDEX_MASK: u32 = 0x10ff;
/// Bits of `pqzyx` feeding the `STREAM_CDEF` index (the Z nibble and Q bit).
const PQZYX_INDEX_MASK: u32 = 0x2f00;
/// F nibble of `cfed`, carried down into the next round's E nibble.
const CFED_F_MASK: u32 = 0x0f00;
/// B-register right shift before `csa_stream_b_sel` folds its select bits.
const B_SEL_SHIFT: u32 = 9;
/// Output-bit weights of `csa_stream_b_sel`, named for the `pqzyx`-style
/// bit each one carries.
const B_SEL_OUT_LSB: u32 = 0x1;
const B_SEL_OUT_SECOND: u32 = 0x2;
const B_SEL_OUT_THIRD: u32 = 0x4;
const B_SEL_OUT_HIGH: u32 = 0x8;
/// B-register taps feeding the high output bit.
const B_SEL_HIGH_TAP_PRIMARY: u32 = 27;
/// B-register taps feeding the high and low output bits together.
const B_SEL_HIGH_LSB_TAP: u32 = 18;
/// B-register taps feeding the third output bit.
const B_SEL_THIRD_TAP_PRIMARY: u32 = 22;
const B_SEL_THIRD_TAP_SECONDARY: u32 = 7;
/// B-register tap feeding the third and low output bits together.
const B_SEL_LSB_THIRD_TAP: u32 = 4;
/// B-register taps feeding the second output bit.
const B_SEL_SECOND_TAP_PRIMARY: u32 = 24;
const B_SEL_SECOND_TAP_SECONDARY: u32 = 6;
const B_SEL_SECOND_TAP_TERTIARY: u32 = 11;
/// B-register taps feeding the low output bit.
const B_SEL_LSB_TAP_PRIMARY: u32 = 29;
const B_SEL_LSB_TAP_SECONDARY: u32 = 23;
/// B-register shift folding the three upper output bits together.
const B_SEL_UPPER_TAPS_SHIFT: u32 = 13;

/// Nibble from shift register at position n (nibble index)
fn nbget(r: u64, n: u32) -> u64 {
    r >> (n * NIBBLE_BITS)
}

/// The DVB-CSA2 stream cipher.
///
/// Not `Copy` (every field is LFSR state seeded from the control word; see
/// [`Drop for StreamCipher`](StreamCipher)).
pub(crate) struct StreamCipher {
    a: u64,
    b: u64,
    pqzyx: u32,
    cfed: u32,
}

/// Zero every register on drop — see the `zeroize` module.
impl Drop for StreamCipher {
    fn drop(&mut self) {
        crate::zeroize::zeroize(core::slice::from_mut(&mut self.a));
        crate::zeroize::zeroize(core::slice::from_mut(&mut self.b));
        crate::zeroize::zeroize(core::slice::from_mut(&mut self.pqzyx));
        crate::zeroize::zeroize(core::slice::from_mut(&mut self.cfed));
    }
}

impl StreamCipher {
    /// Initialize the stream cipher with the nibble-swapped CW and an 8-byte IV.
    pub(crate) fn new(cws: &[u8; SEED_BYTES], iv: &[u8; SEED_BYTES]) -> Self {
        let (a_bytes, b_bytes) = cws.split_at(SEED_REGISTER_BYTES);
        let a = u64::from(u32::from_le_bytes(a_bytes.try_into().unwrap()));
        let b = u64::from(u32::from_le_bytes(b_bytes.try_into().unwrap()));

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

    /// The two shift registers and the S-box/feedback words, read back by the
    /// zeroize-on-drop gate.
    #[cfg(test)]
    pub(crate) fn registers(&self) -> [u64; 4] {
        [self.a, self.b, u64::from(self.pqzyx), u64::from(self.cfed)]
    }

    /// A cipher with every register set to a non-zero sentinel, so the
    /// zeroize-on-drop gate can prove each one is cleared.
    #[cfg(test)]
    pub(crate) fn with_sentinel_registers() -> Self {
        Self {
            a: 0x0123_4567_89ab_cdef,
            b: 0xfedc_ba98_7654_3210,
            pqzyx: 0xdead_beef,
            cfed: 0x1fff_ffff,
        }
    }

    /// XOR the stream cipher's keystream into `data` in-place.
    pub(crate) fn xor_stream(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            // 4 rounds produce one keystream byte
            self.generate_round();
            *byte ^= STREAM_OUT[(self.getd() & NIBBLE_MASK) as usize] & KEYSTREAM_PAIR_HI_MASK;

            self.generate_round();
            *byte ^= STREAM_OUT[(self.getd() & NIBBLE_MASK) as usize] & KEYSTREAM_PAIR_MID_HI_MASK;

            self.generate_round();
            *byte ^= STREAM_OUT[(self.getd() & NIBBLE_MASK) as usize] & KEYSTREAM_PAIR_MID_LO_MASK;

            self.generate_round();
            *byte ^= STREAM_OUT[(self.getd() & NIBBLE_MASK) as usize] & KEYSTREAM_PAIR_LO_MASK;
        }
    }

    fn getx(&self) -> u32 {
        self.pqzyx & NIBBLE_MASK
    }
    fn gety(&self) -> u32 {
        (self.pqzyx >> NIBBLE_BITS) & NIBBLE_MASK
    }
    fn getd(&self) -> u32 {
        self.cfed & NIBBLE_MASK
    }
    fn tstp(&self) -> bool {
        (self.pqzyx & P_BIT_MASK) != 0
    }

    fn init_round(&mut self, iv_nibble: u8) {
        let iv = u32::from(iv_nibble);

        self.a <<= NIBBLE_BITS;
        self.a |= (nbget(self.a, A_FEEDBACK_NIBBLE)
            ^ u64::from(self.getx())
            ^ u64::from(self.getd())
            ^ u64::from(iv >> NIBBLE_BITS))
            & u64::from(NIBBLE_MASK);

        let mut tmp = (nbget(self.b, B_FEEDBACK_NIBBLE_0)
            ^ nbget(self.b, B_FEEDBACK_NIBBLE_1)
            ^ u64::from(self.gety())
            ^ u64::from(iv))
            & u64::from(NIBBLE_MASK);
        tmp = csa_stream_rotate(self.tstp(), tmp as u32) as u64;

        self.b <<= NIBBLE_BITS;
        self.b |= tmp;

        self.cfed = csa_stream_cfed(self.pqzyx, self.cfed) ^ csa_stream_b_sel(self.b);

        self.pqzyx = csa_stream_sboxes(self.a);
    }

    fn generate_round(&mut self) {
        self.a <<= NIBBLE_BITS;
        self.a |=
            (nbget(self.a, A_FEEDBACK_NIBBLE) ^ u64::from(self.getx())) & u64::from(NIBBLE_MASK);

        let mut tmp = (nbget(self.b, B_FEEDBACK_NIBBLE_0)
            ^ nbget(self.b, B_FEEDBACK_NIBBLE_1)
            ^ u64::from(self.gety()))
            & u64::from(NIBBLE_MASK);
        tmp = csa_stream_rotate(self.tstp(), tmp as u32) as u64;

        self.b <<= NIBBLE_BITS;
        self.b |= tmp;

        self.cfed = csa_stream_cfed(self.pqzyx, self.cfed) ^ csa_stream_b_sel(self.b);

        self.pqzyx = csa_stream_sboxes(self.a);
    }
}

fn swap_nbl(byte: u8) -> u8 {
    byte.rotate_left(NIBBLE_BITS)
}

fn csa_stream_rotate(p: bool, x: u32) -> u32 {
    if p {
        ((x << 1) | ((x >> (NIBBLE_BITS - 1)) & 1)) & NIBBLE_MASK
    } else {
        x
    }
}

pub(crate) fn csa_stream_sboxes(a: u64) -> u32 {
    let mut t = a & SBOX_1_MASK;
    let mut res = u32::from(
        STREAM_SBOX[SBOX_1_ROW][(((t >> SBOX_1_SHIFTS[0])
            ^ (t >> SBOX_1_SHIFTS[1])
            ^ (t >> SBOX_1_SHIFTS[SHIFT_POS_2])
            ^ (t >> SBOX_1_SHIFTS[SHIFT_POS_3])
            ^ (t >> SBOX_1_SHIFTS[SHIFT_POS_4]))
            & SBOX_INDEX_MASK) as usize],
    );

    t = a & SBOX_4_MASK;
    res |= u32::from(
        STREAM_SBOX[SBOX_4_ROW][(((t >> SBOX_4_SHIFTS[0])
            ^ (t >> SBOX_4_SHIFTS[1])
            ^ (t >> SBOX_4_SHIFTS[SHIFT_POS_2])
            ^ (t >> SBOX_4_SHIFTS[SHIFT_POS_3])
            ^ (t >> SBOX_4_SHIFTS[SHIFT_POS_4]))
            & SBOX_INDEX_MASK) as usize],
    );

    t = a & SBOX_5_MASK;
    res |= u32::from(
        STREAM_SBOX[SBOX_5_ROW][(((t >> SBOX_5_SHIFTS[0])
            ^ (t >> SBOX_5_SHIFTS[1])
            ^ (t >> SBOX_5_SHIFTS[SHIFT_POS_2])
            ^ (t >> SBOX_5_SHIFTS[SHIFT_POS_3])
            ^ (t >> SBOX_5_SHIFTS[SHIFT_POS_4]))
            & SBOX_INDEX_MASK) as usize],
    );

    t = a & SBOX_0_MASK;
    res |= u32::from(
        STREAM_SBOX[SBOX_0_ROW][(((t >> SBOX_0_SHIFTS[0])
            ^ (t >> SBOX_0_SHIFTS[1])
            ^ (t >> SBOX_0_SHIFTS[SHIFT_POS_2])
            ^ (t >> SBOX_0_SHIFTS[SHIFT_POS_3])
            ^ (t >> SBOX_0_SHIFTS[SHIFT_POS_4]))
            & SBOX_INDEX_MASK) as usize],
    );

    t = a & SBOX_2_MASK;
    res |= u32::from(
        STREAM_SBOX[SBOX_2_ROW][(((t >> SBOX_2_SHIFTS[0])
            ^ (t >> SBOX_2_SHIFTS[1])
            ^ (t >> SBOX_2_SHIFTS[SHIFT_POS_2])
            ^ (t >> SBOX_2_SHIFTS[SHIFT_POS_3])
            ^ (t >> SBOX_2_SHIFTS[SHIFT_POS_4]))
            & SBOX_INDEX_MASK) as usize],
    );

    t = a & SBOX_3_MASK;
    res |= u32::from(
        STREAM_SBOX[SBOX_3_ROW][(((t >> SBOX_3_SHIFTS[0])
            ^ (t >> SBOX_3_SHIFTS[1])
            ^ (t >> SBOX_3_SHIFTS[SHIFT_POS_2])
            ^ (t >> SBOX_3_SHIFTS[SHIFT_POS_3])
            ^ (t >> SBOX_3_SHIFTS[SHIFT_POS_4]))
            & SBOX_INDEX_MASK) as usize],
    );

    t = a & SBOX_6_MASK;
    res |= u32::from(
        STREAM_SBOX[SBOX_6_ROW][(((t >> SBOX_6_SHIFTS[0])
            ^ (t >> SBOX_6_SHIFTS[1])
            ^ (t >> SBOX_6_SHIFTS[SHIFT_POS_2])
            ^ (t >> SBOX_6_SHIFTS[SHIFT_POS_3])
            ^ (t >> SBOX_6_SHIFTS[SHIFT_POS_4]))
            & SBOX_INDEX_MASK) as usize],
    );

    res
}

pub(crate) fn csa_stream_b_sel(b: u64) -> u32 {
    // C code: `uint32_t t = B >> 9;` — truncation to 32 bits is load-bearing.
    let t = (b >> B_SEL_SHIFT) as u32;

    ((t ^ (t >> B_SEL_HIGH_TAP_PRIMARY)) & B_SEL_OUT_HIGH)
        ^ ((t >> B_SEL_HIGH_LSB_TAP) & (B_SEL_OUT_HIGH | B_SEL_OUT_LSB))
        ^ (((t >> B_SEL_THIRD_TAP_PRIMARY) ^ (t >> B_SEL_THIRD_TAP_SECONDARY)) & B_SEL_OUT_THIRD)
        ^ ((t >> B_SEL_LSB_THIRD_TAP) & (B_SEL_OUT_THIRD | B_SEL_OUT_LSB))
        ^ (((t >> B_SEL_SECOND_TAP_PRIMARY)
            ^ (t >> B_SEL_SECOND_TAP_SECONDARY)
            ^ (t >> B_SEL_SECOND_TAP_TERTIARY))
            & B_SEL_OUT_SECOND)
        ^ (((t >> B_SEL_LSB_TAP_PRIMARY) ^ (t >> B_SEL_LSB_TAP_SECONDARY)) & B_SEL_OUT_LSB)
        ^ ((t >> B_SEL_UPPER_TAPS_SHIFT) & (B_SEL_OUT_SECOND | B_SEL_OUT_THIRD | B_SEL_OUT_HIGH))
}

fn csa_stream_cfed(pqzyx: u32, cfed: u32) -> u32 {
    let cdef_idx =
        (((cfed & CFED_INDEX_MASK) | (pqzyx & PQZYX_INDEX_MASK)) >> NIBBLE_BITS) as usize;
    ((cfed & CFED_F_MASK) >> NIBBLE_BITS) | u32::from(STREAM_CDEF[cdef_idx])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::STREAM_SBOX_SEL;

    /// r11-O-CSA-2: the S-box input selection is written twice —
    /// hand-unrolled in `csa_stream_sboxes` and as data in
    /// `STREAM_SBOX_SEL` (which the bitsliced circuit generator consumes).
    /// Pin the two to each other over a deterministic sweep of register-A
    /// values so an edit to either one cannot silently drift.
    #[test]
    fn unrolled_sbox_selection_matches_the_selection_table() {
        let mut x = 0x243f_6a88_85a3_08d3_u64;
        for _ in 0..20_000 {
            // xorshift64*
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            let a = x.wrapping_mul(0x2545_f491_4f6c_dd1d) & 0xff_ffff_ffff;
            let mut want = 0u32;
            for &(mask, sbox, shifts) in STREAM_SBOX_SEL.iter() {
                let t = a & mask;
                let mut idx = 0u64;
                for &s in shifts.iter() {
                    idx ^= t >> s;
                }
                want |= u32::from(STREAM_SBOX[sbox][(idx & 0x1f) as usize]);
            }
            assert_eq!(csa_stream_sboxes(a), want, "A = {a:#012x}");
        }
    }
}
