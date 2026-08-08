//! Bitsliced stream cipher — the same two 40-bit shift registers as
//! [`crate::stream`], evaluated as a boolean circuit so one round advances
//! [`LANES`](super::LANES) independent keystreams.
//!
//! The register state is held one `Word` per *bit*, lane-major: `a[i]` carries
//! bit `i` of the A register for every lane. Every table the scalar round
//! touches becomes a circuit from [`super::circuits`], and the three linear
//! ones cost almost nothing — the S-box index selection, `csa_stream_b_sel`
//! and `STREAM_OUT` are all GF(2)-linear, so they reduce to XOR trees.
//!
//! Each lane carries its own initialisation vector (its own first block), so
//! the lanes diverge on the first round and never interact.
use super::circuits::{
    CDEF_OUT_BITS, STREAM_SBOX_OUT_BITS, stream_b_sel, stream_cdef, stream_out, stream_sboxes,
};
use super::{BITS_PER_BYTE, BLOCK_BITS, BLOCK_BYTES, Word};

/// Bits in a nibble — the shift registers advance one nibble per round.
const NIBBLE_BITS: usize = 4;
/// Bits in the A shift register (ten nibbles).
const A_BITS: usize = 40;
/// Bits in the B shift register (ten nibbles).
const B_BITS: usize = 40;
/// Live bits of the `pqzyx` S-box output word: X, Y, Z, then P and Q.
const PQZYX_BITS: usize = 14;
/// Live bits of the `cfed` feedback word: D, E, F, then the carried C bit.
const CFED_BITS: usize = 13;
/// Index bits consumed by the `STREAM_CDEF` table.
const CDEF_IN_BITS: usize = 10;
/// Rounds per keystream byte — each yields two bits.
const ROUNDS_PER_BYTE: usize = 4;
/// Rounds per initialisation-vector byte.
const ROUNDS_PER_IV_BYTE: usize = 4;
/// Bits of the control word loaded into each shift register at reset.
const KEY_BITS: usize = 32;

/// Nibble of A that feeds back into the register (`nbget(a, 10)` after the
/// shift is nibble 9 before it).
const A_FEEDBACK_NIBBLE: usize = 9;
/// Nibbles of B that feed back into the register (`nbget(b, 6) ^ nbget(b, 9)`).
const B_FEEDBACK_NIBBLES: [usize; 2] = [6, 9];

/// First bit of the X nibble of `pqzyx` (`getx`).
const X_BASE: usize = 0;
/// First bit of the Y nibble of `pqzyx` (`gety`).
const Y_BASE: usize = 4;
/// First bit of the Z nibble of `pqzyx` — the `STREAM_CDEF` index.
const Z_BASE: usize = 8;
/// The P bit of `pqzyx` (`tstp`), which selects the B feedback rotation.
const P_BIT: usize = 12;
/// The Q bit of `pqzyx` — the top `STREAM_CDEF` index bit.
const Q_BIT: usize = 13;

/// First bit of the D nibble of `cfed` (`getd`) — the keystream source.
const D_BASE: usize = 0;
/// First bit of the E nibble of `cfed` — the low `STREAM_CDEF` index bits.
const E_BASE: usize = 4;
/// First bit of the F nibble of `cfed`, which shifts down into E each round.
const F_BASE: usize = 8;
/// The carried C bit of `cfed` — a `STREAM_CDEF` index bit.
const C_BIT: usize = 12;

/// The bitsliced DVB-CSA2 stream cipher.
pub(super) struct BitslicedStream {
    a: [Word; A_BITS],
    b: [Word; B_BITS],
    pqzyx: [Word; PQZYX_BITS],
    cfed: [Word; CFED_BITS],
}

/// Expand a key word into lane masks — the control word is shared by every
/// lane, so each of its bits is all-ones or all-zeros.
fn broadcast<const N: usize>(value: u32) -> [Word; N] {
    let mut out = [0 as Word; N];
    for (bit, word) in out.iter_mut().enumerate().take(KEY_BITS) {
        *word = if (value >> bit) & 1 == 1 { !0 } else { 0 };
    }
    out
}

impl BitslicedStream {
    /// Seed the cipher from the nibble-swapped control word and one 8-byte
    /// initialisation vector per lane.
    ///
    /// `iv` is the transposed first block: `iv[byte * 8 + bit]`.
    pub(super) fn new(cws: &[u8; BLOCK_BYTES], iv: &[Word; BLOCK_BITS]) -> Self {
        let a = u32::from_le_bytes([cws[0], cws[1], cws[2], cws[3]]);
        let b = u32::from_le_bytes([cws[4], cws[5], cws[6], cws[7]]);
        let mut sc = Self {
            a: broadcast(a),
            b: broadcast(b),
            pqzyx: [0; PQZYX_BITS],
            cfed: [0; CFED_BITS],
        };

        for byte in 0..BLOCK_BYTES {
            let base = byte * BITS_PER_BYTE;
            let mut low = [0 as Word; NIBBLE_BITS];
            let mut high = [0 as Word; NIBBLE_BITS];
            low.copy_from_slice(&iv[base..base + NIBBLE_BITS]);
            high.copy_from_slice(&iv[base + NIBBLE_BITS..base + BITS_PER_BYTE]);
            // The scalar path runs iv, swap_nbl(iv), iv, swap_nbl(iv); the
            // swap exchanges which nibble reaches which register.
            for _ in 0..ROUNDS_PER_IV_BYTE / 2 {
                sc.round::<true>(&high, &low);
                sc.round::<true>(&low, &high);
            }
        }
        sc
    }

    /// Advance one round.
    ///
    /// `INIT` selects the initialisation round, which additionally mixes an
    /// IV nibble into each register's feedback; it is a const parameter so the
    /// generating rounds carry none of the cost.
    #[inline]
    fn round<const INIT: bool>(&mut self, iv_a: &[Word; NIBBLE_BITS], iv_b: &[Word; NIBBLE_BITS]) {
        // --- A register: feed back nibble 9 and X -----------------------
        let mut fa = [0 as Word; NIBBLE_BITS];
        for (k, slot) in fa.iter_mut().enumerate() {
            let mut v = self.a[A_FEEDBACK_NIBBLE * NIBBLE_BITS + k] ^ self.pqzyx[X_BASE + k];
            if INIT {
                // Initialisation alone also folds in the D nibble and the IV;
                // `generate_round` folds in neither.
                v ^= self.cfed[D_BASE + k] ^ iv_a[k];
            }
            *slot = v;
        }
        self.a.copy_within(0..A_BITS - NIBBLE_BITS, NIBBLE_BITS);
        self.a[..NIBBLE_BITS].copy_from_slice(&fa);

        // --- B register: feed back nibbles 6 and 9 and Y, then rotate ---
        let mut fb = [0 as Word; NIBBLE_BITS];
        for (k, slot) in fb.iter_mut().enumerate() {
            let mut v = self.b[B_FEEDBACK_NIBBLES[0] * NIBBLE_BITS + k]
                ^ self.b[B_FEEDBACK_NIBBLES[1] * NIBBLE_BITS + k]
                ^ self.pqzyx[Y_BASE + k];
            if INIT {
                v ^= iv_b[k];
            }
            *slot = v;
        }
        // `csa_stream_rotate`: a one-bit left rotation of the nibble, taken
        // only when P is set — branch-free here, as a per-lane select.
        let p = self.pqzyx[P_BIT];
        let mut rb = [0 as Word; NIBBLE_BITS];
        for (k, slot) in rb.iter_mut().enumerate() {
            let rotated = fb[(k + NIBBLE_BITS - 1) % NIBBLE_BITS];
            *slot = fb[k] ^ ((fb[k] ^ rotated) & p);
        }
        self.b.copy_within(0..B_BITS - NIBBLE_BITS, NIBBLE_BITS);
        self.b[..NIBBLE_BITS].copy_from_slice(&rb);

        // --- C/D/E/F feedback ------------------------------------------
        let mut index = [0 as Word; CDEF_IN_BITS];
        index[..NIBBLE_BITS].copy_from_slice(&self.cfed[E_BASE..E_BASE + NIBBLE_BITS]);
        index[NIBBLE_BITS..2 * NIBBLE_BITS]
            .copy_from_slice(&self.pqzyx[Z_BASE..Z_BASE + NIBBLE_BITS]);
        index[2 * NIBBLE_BITS] = self.cfed[C_BIT];
        index[2 * NIBBLE_BITS + 1] = self.pqzyx[Q_BIT];
        let table = stream_cdef(&index);
        // `csa_stream_b_sel` reads the *post-shift* B register.
        let selected = stream_b_sel(&self.b);

        let mut next = [0 as Word; CFED_BITS];
        next[E_BASE..E_BASE + NIBBLE_BITS]
            .copy_from_slice(&self.cfed[F_BASE..F_BASE + NIBBLE_BITS]);
        for (i, &bit) in CDEF_OUT_BITS.iter().enumerate() {
            next[bit] = table[i];
        }
        for (slot, sel) in next[D_BASE..D_BASE + NIBBLE_BITS].iter_mut().zip(selected) {
            *slot ^= sel;
        }
        self.cfed = next;

        // --- S-boxes over the post-shift A register ---------------------
        let sboxed = stream_sboxes(&self.a);
        let mut pqzyx = [0 as Word; PQZYX_BITS];
        for (i, &bit) in STREAM_SBOX_OUT_BITS.iter().enumerate() {
            pqzyx[bit] = sboxed[i];
        }
        self.pqzyx = pqzyx;
    }

    /// Produce one keystream byte per lane, sliced: `[bit][lane]`.
    ///
    /// Four rounds contribute two bits each, most significant pair first.
    pub(super) fn keystream_byte(&mut self) -> [Word; BITS_PER_BYTE] {
        let idle = [0 as Word; NIBBLE_BITS];
        let mut out = [0 as Word; BITS_PER_BYTE];
        for phase in 0..ROUNDS_PER_BYTE {
            self.round::<false>(&idle, &idle);
            let mut d = [0 as Word; NIBBLE_BITS];
            d.copy_from_slice(&self.cfed[D_BASE..D_BASE + NIBBLE_BITS]);
            let [high, low] = stream_out(&d);
            out[BITS_PER_BYTE - 1 - 2 * phase] = high;
            out[BITS_PER_BYTE - 2 - 2 * phase] = low;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitsliced::{LANES, transpose};
    use crate::stream::StreamCipher;

    /// Every lane must reproduce the scalar keystream for its own IV. Giving
    /// each lane a *different* IV is the point: a bug that let lane state
    /// bleed sideways would survive a uniform batch.
    #[test]
    fn matches_the_scalar_keystream_in_every_lane() {
        const N: usize = 40;
        let cws = [0x12u8, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0];

        let mut ivs = [[0u8; BLOCK_BYTES]; LANES];
        let mut seed: u64 = 0xdead_beef_cafe_f00d;
        for iv in ivs.iter_mut() {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *iv = seed.to_le_bytes();
        }

        let mut m = [0u64; LANES];
        for (lane, iv) in ivs.iter().enumerate() {
            m[lane] = u64::from_le_bytes(*iv);
        }
        transpose(&mut m);

        let mut bs = BitslicedStream::new(&cws, &m);
        let mut got = [[0u8; N]; LANES];
        for byte in 0..N {
            let bits = bs.keystream_byte();
            for (lane, lane_bytes) in got.iter_mut().enumerate() {
                let mut v = 0u8;
                for (bit, word) in bits.iter().enumerate() {
                    if (word >> lane) & 1 == 1 {
                        v |= 1 << bit;
                    }
                }
                lane_bytes[byte] = v;
            }
        }

        for (lane, iv) in ivs.iter().enumerate() {
            let mut want = [0u8; N];
            StreamCipher::new(&cws, iv).xor_stream(&mut want);
            assert_eq!(got[lane], want, "lane {lane} keystream disagrees");
        }
    }
}
