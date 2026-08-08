//! Exhaustive verification of the generated circuits in [`super::circuits`].
//!
//! `circuits.rs` is machine-generated, so nothing in it is reviewed by eye.
//! Every circuit is therefore evaluated here over its **entire** input domain
//! (or, where the domain is a 40-bit register, over a randomised sweep that is
//! required to cover every table entry) and compared against the table in
//! [`crate::tables`] it was generated from. A generator bug — a bad variable
//! order, a mis-shared node, a wrong tap — cannot survive this file.
//!
//! These are deliberately kept out of `circuits.rs` so that regenerating it
//! cannot regenerate its own gate.
use super::circuits::{
    CDEF_OUT_BITS, PERM_BIT, STREAM_SBOX_OUT_BITS, block_sbox, stream_b_sel, stream_cdef,
    stream_out, stream_sboxes,
};
use super::{LANES, Word};
use crate::stream::{csa_stream_b_sel, csa_stream_sboxes};
use crate::tables::{PERM, SBOX, STREAM_CDEF, STREAM_OUT, STREAM_SBOX, STREAM_SBOX_SEL};

/// Slice `LANES` integers into one `Word` per bit position.
fn slice<const N: usize>(values: &[u32; LANES]) -> [Word; N] {
    let mut out = [0 as Word; N];
    for (lane, &v) in values.iter().enumerate() {
        for (bit, word) in out.iter_mut().enumerate() {
            if (v >> bit) & 1 == 1 {
                *word |= 1 << lane;
            }
        }
    }
    out
}

/// Recover lane `lane`'s value from a sliced result.
fn unslice(words: &[Word], lane: usize) -> u32 {
    let mut v = 0u32;
    for (bit, word) in words.iter().enumerate() {
        if (word >> lane) & 1 == 1 {
            v |= 1 << bit;
        }
    }
    v
}

/// A deterministic xorshift — the sweeps must be reproducible when they fail.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn block_sbox_matches_the_table_on_every_input() {
    for base in (0..256).step_by(LANES) {
        let mut values = [0u32; LANES];
        for (lane, v) in values.iter_mut().enumerate() {
            *v = (base + lane) as u32;
        }
        let got = block_sbox(&slice(&values));
        for lane in 0..LANES {
            assert_eq!(
                unslice(&got, lane) as u8,
                SBOX[values[lane] as usize],
                "SBOX[{:#04x}]",
                values[lane]
            );
        }
    }
}

#[test]
fn perm_bit_map_matches_the_table_on_every_input() {
    for (s, &want) in PERM.iter().enumerate() {
        let mut got = 0u8;
        for (bit, &dst) in PERM_BIT.iter().enumerate() {
            if (s >> bit) & 1 == 1 {
                got |= 1 << dst;
            }
        }
        assert_eq!(got, want, "PERM[{s:#04x}]");
    }
}

#[test]
fn stream_cdef_matches_the_table_on_every_input() {
    const DOMAIN: usize = 1 << 10;
    for base in (0..DOMAIN).step_by(LANES) {
        let mut values = [0u32; LANES];
        for (lane, v) in values.iter_mut().enumerate() {
            *v = (base + lane) as u32;
        }
        let got = stream_cdef(&slice(&values));
        for lane in 0..LANES {
            let want = STREAM_CDEF[values[lane] as usize];
            let mut assembled = 0u16;
            for (i, &bit) in CDEF_OUT_BITS.iter().enumerate() {
                if (got[i] >> lane) & 1 == 1 {
                    assembled |= 1 << bit;
                }
            }
            assert_eq!(assembled, want, "STREAM_CDEF[{}]", values[lane]);
        }
    }
}

#[test]
fn stream_out_matches_the_table_on_every_input() {
    let mut values = [0u32; LANES];
    for (lane, v) in values.iter_mut().enumerate() {
        *v = (lane % STREAM_OUT.len()) as u32;
    }
    let [high, low] = stream_out(&slice(&values));
    for lane in 0..LANES {
        let want = STREAM_OUT[values[lane] as usize];
        let got_high = ((high >> lane) & 1) as u8;
        let got_low = ((low >> lane) & 1) as u8;
        // The table entry is the pair replicated across the byte.
        let expect = got_high * 0b1010_1010 + got_low * 0b0101_0101;
        assert_eq!(expect, want, "STREAM_OUT[{}]", values[lane]);
    }
}

/// The A register is 40 bits wide, so the sweep is randomised — but it is only
/// accepted if it drove **every** entry of **every** stream S-box, which is
/// what makes it equivalent to an exhaustive check of the S-box circuits.
#[test]
fn stream_sboxes_match_the_scalar_routine_and_cover_every_entry() {
    const SWEEPS: usize = 400;
    const A_BITS: usize = 40;
    const A_MASK: u64 = (1 << A_BITS) - 1;
    const ENTRIES: usize = 32;

    let mut rng = Rng(0x243f_6a88_85a3_08d3);
    let mut seen = [[false; ENTRIES]; STREAM_SBOX.len()];

    for _ in 0..SWEEPS {
        let mut regs = [0u64; LANES];
        for r in regs.iter_mut() {
            *r = rng.next() & A_MASK;
        }
        let mut sliced = [0 as Word; A_BITS];
        for (lane, &r) in regs.iter().enumerate() {
            for (bit, word) in sliced.iter_mut().enumerate() {
                if (r >> bit) & 1 == 1 {
                    *word |= 1 << lane;
                }
            }
        }
        let got = stream_sboxes(&sliced);

        for (lane, &r) in regs.iter().enumerate() {
            let mut assembled = 0u32;
            for (i, &bit) in STREAM_SBOX_OUT_BITS.iter().enumerate() {
                if (got[i] >> lane) & 1 == 1 {
                    assembled |= 1 << bit;
                }
            }
            assert_eq!(assembled, csa_stream_sboxes(r), "A = {r:#012x}");

            for &(mask, sbox, shifts) in STREAM_SBOX_SEL.iter() {
                let t = r & mask;
                let mut index = 0u64;
                for &s in shifts.iter() {
                    index ^= t >> s;
                }
                seen[sbox][(index & 0x1f) as usize] = true;
            }
        }
    }

    for (n, table) in seen.iter().enumerate() {
        let missed: usize = table.iter().filter(|s| !**s).count();
        assert_eq!(
            missed, 0,
            "stream S-box {n} left {missed} entries unexercised"
        );
    }
}

#[test]
fn stream_b_sel_matches_the_scalar_routine() {
    const SWEEPS: usize = 200;
    const B_BITS: usize = 40;
    const B_MASK: u64 = (1 << B_BITS) - 1;

    let mut rng = Rng(0x1319_8a2e_0370_7344);
    for _ in 0..SWEEPS {
        let mut regs = [0u64; LANES];
        for r in regs.iter_mut() {
            *r = rng.next() & B_MASK;
        }
        let mut sliced = [0 as Word; B_BITS];
        for (lane, &r) in regs.iter().enumerate() {
            for (bit, word) in sliced.iter_mut().enumerate() {
                if (r >> bit) & 1 == 1 {
                    *word |= 1 << lane;
                }
            }
        }
        let got = stream_b_sel(&sliced);
        for (lane, &r) in regs.iter().enumerate() {
            assert_eq!(unslice(&got, lane), csa_stream_b_sel(r), "B = {r:#012x}");
        }
    }
}
