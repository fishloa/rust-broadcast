//! Bitsliced fast path — [`LANES`] payloads scrambled or descrambled at once.
//!
//! # What is actually parallel in CSA2
//!
//! Bitslicing transposes the data so that bit *i* of every lane lives in one
//! machine word, then evaluates the cipher as a boolean circuit; every gate
//! then operates on all [`LANES`] lanes at once. That only pays off where the
//! algorithm has real independence, and CSA2 has less of it than it looks:
//!
//! - **Block cipher, scramble.** `C[i] = E(P[i] ^ C[i+1])` — a reverse CBC.
//!   Block *i* cannot start until block *i+1* is finished, so there is **no
//!   parallelism within one payload**.
//! - **Block cipher, descramble.** `P[i] = D(C[i]) ^ C[i+1]`. The chaining XOR
//!   uses the *ciphertext* of the next block, which is already in the buffer,
//!   so every `D(C[i])` is independent — this half *is* parallel within a
//!   single payload.
//! - **Stream cipher.** Two chained 40-bit shift registers; round *n+1* needs
//!   round *n*'s state. Sequential within one payload, in both directions.
//!
//! Two of those three are sequential within a payload, and the stream cipher —
//! the sequential one — is about two thirds of the total work. So the honest
//! unit of parallelism for CSA2 is **the payload, not the block**: this module
//! exposes a batch API that scrambles or descrambles up to [`LANES`]
//! *independent* payloads (TS packets, typically) in one pass. There is no
//! bitsliced single-payload entry point, because for a single payload there is
//! nothing worth slicing.
//!
//! # Correctness
//!
//! The bitsliced path is bit-exact with the scalar path — it is the same
//! cipher, re-expressed. Three independent gates hold it there:
//!
//! - every generated circuit in `src/bitsliced/circuits.rs` is checked against
//!   the table it came from over its **entire** input domain;
//! - `tests/bitsliced_differential.rs` compares batch output against
//!   [`crate::scramble`] / [`crate::descramble`] over randomised payloads and
//!   lengths, in both directions;
//! - `tests/golden_vectors.rs` runs the libdvbcsa known-answer vectors through
//!   the batch API too, so the fast path answers to the external oracle and
//!   not merely to our own scalar code.
//!
//! # Example
//!
//! ```
//! use dvb_csa::{ControlWord, bitsliced, descramble, scramble};
//!
//! let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
//! let mut packets = [[0xAAu8; 184], [0xBBu8; 184], [0xCCu8; 184]];
//! let expected = packets;
//!
//! let mut batch: [&mut [u8]; 3] = {
//!     let [a, b, c] = &mut packets;
//!     [a.as_mut_slice(), b.as_mut_slice(), c.as_mut_slice()]
//! };
//! bitsliced::scramble_batch(&cw, &mut batch);
//! bitsliced::descramble_batch(&cw, &mut batch);
//!
//! assert_eq!(packets, expected);
//! ```
mod block;
#[cfg(test)]
mod circuit_tests;
mod circuits;
mod stream;

use crate::key::ControlWord;
use block::BitslicedBlock;
use stream::BitslicedStream;

/// The machine word the cipher state is sliced across. One bit per lane.
type Word = u64;

/// Number of payloads processed per bitsliced pass — the slicing width.
///
/// A batch longer than this is split into consecutive groups of `LANES`; a
/// shorter one simply leaves the surplus lanes idle, so a batch well below
/// `LANES` gets proportionally less of the speed-up.
pub const LANES: usize = Word::BITS as usize;

/// Bytes in one CSA block.
const BLOCK_BYTES: usize = 8;
/// Bits in one byte — the width of the block cipher's byte-wise state words.
const BITS_PER_BYTE: usize = 8;
/// Bits in one CSA block; also the side length of the transpose matrix.
const BLOCK_BITS: usize = BLOCK_BYTES * BITS_PER_BYTE;

const _: () = assert!(BLOCK_BITS == LANES, "the transpose is square");

/// Transpose a `LANES` x `LANES` bit matrix in place.
///
/// Row `i` on input becomes column `i` on output, which is exactly the
/// scalar <-> bitsliced conversion in both directions: feed it one packed
/// `Word` per lane and it returns one `Word` per bit position, and vice versa.
///
/// The recursive block-swap algorithm is Hacker's Delight 2nd ed. §7-3, in its
/// least-significant-bit-is-column-zero form: it costs `log2(LANES)` passes
/// rather than the `LANES^2` of the naive loop, which keeps transposition
/// negligible beside the cipher rounds it feeds.
///
/// Each pass exchanges the two off-diagonal quadrants of every `2j x 2j`
/// sub-matrix — `mask` selects the low `j` columns of each such block, and
/// halves alongside `j`.
fn transpose(m: &mut [Word; LANES]) {
    let mut j = LANES / 2;
    let mut mask: Word = !0 >> (LANES / 2);
    while j != 0 {
        let mut k = 0;
        while k < LANES {
            let t = ((m[k] >> j) ^ m[k | j]) & mask;
            m[k] ^= t << j;
            m[k | j] ^= t;
            k = ((k | j) + 1) & !j;
        }
        j >>= 1;
        mask ^= mask << j;
    }
}

/// Per-lane bookkeeping for one group of at most [`LANES`] payloads.
struct Group {
    /// Payload length per lane; `0` for an idle lane.
    len: [usize; LANES],
    /// Complete 8-byte blocks per lane.
    blocks: [usize; LANES],
    /// Largest `blocks` in the group.
    max_blocks: usize,
    /// Largest stream-ciphered byte count (`len - BLOCK_BYTES`) in the group.
    max_stream: usize,
}

impl Group {
    fn new(payloads: &[&mut [u8]]) -> Self {
        let mut g = Self {
            len: [0; LANES],
            blocks: [0; LANES],
            max_blocks: 0,
            max_stream: 0,
        };
        for (lane, p) in payloads.iter().enumerate() {
            // Payloads shorter than one block pass through untouched, exactly
            // as the scalar path leaves them.
            if p.len() < BLOCK_BYTES {
                continue;
            }
            g.len[lane] = p.len();
            g.blocks[lane] = p.len() / BLOCK_BYTES;
            g.max_blocks = g.max_blocks.max(g.blocks[lane]);
            g.max_stream = g.max_stream.max(p.len() - BLOCK_BYTES);
        }
        g
    }
}

/// Read the 8 bytes at `off` of every lane into one packed `Word` per lane.
///
/// Lanes with no block at that offset contribute zero; their results are
/// discarded, so the value does not matter.
fn gather(payloads: &[&mut [u8]], offsets: &[Option<usize>; LANES], m: &mut [Word; LANES]) {
    *m = [0; LANES];
    for (lane, off) in offsets.iter().enumerate() {
        if let Some(off) = *off {
            let bytes: [u8; BLOCK_BYTES] = payloads[lane][off..off + BLOCK_BYTES]
                .try_into()
                .expect("slice is exactly one block");
            m[lane] = Word::from_le_bytes(bytes);
        }
    }
}

/// Write one packed `Word` per lane back to the 8 bytes at `off`.
fn scatter(payloads: &mut [&mut [u8]], offsets: &[Option<usize>; LANES], m: &[Word; LANES]) {
    for (lane, off) in offsets.iter().enumerate() {
        if let Some(off) = *off {
            payloads[lane][off..off + BLOCK_BYTES].copy_from_slice(&m[lane].to_le_bytes());
        }
    }
}

/// Byte offset of block `index` in each lane, or `None` where the lane is
/// shorter than that.
fn block_offsets(g: &Group, index: usize) -> [Option<usize>; LANES] {
    let mut o = [None; LANES];
    for (slot, &blocks) in o.iter_mut().zip(g.blocks.iter()) {
        if index < blocks {
            *slot = Some(index * BLOCK_BYTES);
        }
    }
    o
}

/// Byte offset of the block `from_end` places before each lane's last block.
fn block_offsets_from_end(g: &Group, from_end: usize) -> [Option<usize>; LANES] {
    let mut o = [None; LANES];
    for (slot, &blocks) in o.iter_mut().zip(g.blocks.iter()) {
        if from_end < blocks {
            *slot = Some((blocks - 1 - from_end) * BLOCK_BYTES);
        }
    }
    o
}

/// Scramble (encrypt) up to [`LANES`] payloads per pass with one control word.
///
/// Bit-for-bit identical to calling [`crate::scramble`] on each payload in
/// turn, including the pass-through of payloads shorter than 8 bytes. The
/// payloads are independent of one another; their lengths may all differ.
pub fn scramble_batch(cw: &ControlWord, payloads: &mut [&mut [u8]]) {
    for group in payloads.chunks_mut(LANES) {
        scramble_group(cw, group);
    }
}

/// Descramble (decrypt) up to [`LANES`] payloads per pass with one control word.
///
/// Bit-for-bit identical to calling [`crate::descramble`] on each payload in
/// turn, including the pass-through of payloads shorter than 8 bytes. The
/// payloads are independent of one another; their lengths may all differ.
pub fn descramble_batch(cw: &ControlWord, payloads: &mut [&mut [u8]]) {
    for group in payloads.chunks_mut(LANES) {
        descramble_group(cw, group);
    }
}

fn scramble_group(cw: &ControlWord, payloads: &mut [&mut [u8]]) {
    let g = Group::new(payloads);
    if g.max_blocks == 0 {
        return;
    }
    let bc = BitslicedBlock::new(cw.expand_block());
    let mut m = [0 as Word; LANES];

    // Phase 1 — block cipher, reverse CBC. Sequential within a payload, so the
    // lanes are aligned on the *last* block of each and walk backwards
    // together; a lane drops out as soon as its payload runs out of blocks.
    for from_end in 0..g.max_blocks {
        let here = block_offsets_from_end(&g, from_end);
        if from_end > 0 {
            // XOR the already-encrypted following block into this one.
            let next = block_offsets_from_end(&g, from_end - 1);
            for lane in 0..LANES {
                if let (Some(h), Some(n)) = (here[lane], next[lane]) {
                    let following: [u8; BLOCK_BYTES] = payloads[lane][n..n + BLOCK_BYTES]
                        .try_into()
                        .expect("slice is exactly one block");
                    for (dst, src) in payloads[lane][h..h + BLOCK_BYTES].iter_mut().zip(following) {
                        *dst ^= src;
                    }
                }
            }
        }
        gather(payloads, &here, &mut m);
        transpose(&mut m);
        bc.encrypt(&mut m);
        transpose(&mut m);
        scatter(payloads, &here, &m);
    }

    // Phase 2 — stream cipher over bytes 8.., seeded from the now-encrypted
    // first block of each payload.
    stream_xor(cw, payloads, &g);
}

fn descramble_group(cw: &ControlWord, payloads: &mut [&mut [u8]]) {
    let g = Group::new(payloads);
    if g.max_blocks == 0 {
        return;
    }
    let bc = BitslicedBlock::new(cw.expand_block());

    // Phase 1 — stream cipher over bytes 8.., seeded from the still-encrypted
    // first block of each payload.
    stream_xor(cw, payloads, &g);

    // Phase 2 — block cipher, forward CBC undo. Every `D(C[i])` is independent
    // here, so the lanes are aligned on block 0 and simply walk forwards.
    let mut m = [0 as Word; LANES];
    let mut cipher = [0 as Word; LANES];
    for index in 0..g.max_blocks {
        let here = block_offsets(&g, index);
        gather(payloads, &here, &mut m);
        // C[index] is needed to un-chain the *previous* block, and decryption
        // is about to overwrite it, so keep it.
        cipher.copy_from_slice(&m);
        transpose(&mut m);
        bc.decrypt(&mut m);
        transpose(&mut m);
        if index > 0 {
            let prev = block_offsets(&g, index - 1);
            for lane in 0..LANES {
                if let (Some(p), Some(_)) = (prev[lane], here[lane]) {
                    let c = cipher[lane].to_le_bytes();
                    for (dst, src) in payloads[lane][p..p + BLOCK_BYTES].iter_mut().zip(c) {
                        *dst ^= src;
                    }
                }
            }
        }
        scatter(payloads, &here, &m);
    }
}

/// XOR the keystream of every lane into its own bytes `8..len`.
///
/// The initialisation vector is each payload's own first block, so the lanes
/// diverge after the first round and stay independent from there.
fn stream_xor(cw: &ControlWord, payloads: &mut [&mut [u8]], g: &Group) {
    if g.max_stream == 0 {
        return;
    }
    let mut iv = [0 as Word; LANES];
    let mut first = [None; LANES];
    for (slot, &len) in first.iter_mut().zip(g.len.iter()) {
        if len >= BLOCK_BYTES {
            *slot = Some(0);
        }
    }
    gather(payloads, &first, &mut iv);
    transpose(&mut iv);

    let mut sc = BitslicedStream::new(&cw.expand_stream(), &iv);

    let mut ks = [0 as Word; LANES];
    let mut done = 0;
    while done < g.max_stream {
        // One transpose covers BLOCK_BYTES keystream bytes for every lane.
        for byte in 0..BLOCK_BYTES {
            let bits = sc.keystream_byte();
            for bit in 0..BITS_PER_BYTE {
                ks[byte * BITS_PER_BYTE + bit] = bits[bit];
            }
        }
        transpose(&mut ks);
        for lane in 0..LANES {
            // A lane drops out once its own payload is exhausted; the group
            // keeps going for whichever lane is longest.
            let base = BLOCK_BYTES + done;
            if base >= g.len[lane] {
                continue;
            }
            let bytes = ks[lane].to_le_bytes();
            let n = (g.len[lane] - base).min(BLOCK_BYTES);
            for (j, b) in bytes.iter().take(n).enumerate() {
                payloads[lane][base + j] ^= b;
            }
        }
        done += BLOCK_BYTES;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpose_is_an_involution() {
        let mut m = [0 as Word; LANES];
        for (i, w) in m.iter_mut().enumerate() {
            // A deterministic, asymmetric fill: any bug that mixed rows and
            // columns up would survive a symmetric one.
            *w = (i as Word).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
        let original = m;
        transpose(&mut m);
        assert_ne!(
            m, original,
            "transpose of an asymmetric matrix is not a no-op"
        );
        transpose(&mut m);
        assert_eq!(m, original);
    }

    #[test]
    fn transpose_moves_row_bits_to_column_bits() {
        let mut m = [0 as Word; LANES];
        m[3] = 1 << 5;
        transpose(&mut m);
        for (i, w) in m.iter().enumerate() {
            let want: Word = if i == 5 { 1 << 3 } else { 0 };
            assert_eq!(*w, want, "row {i}");
        }
    }

    #[test]
    fn short_payloads_pass_through() {
        let cw = ControlWord::from_bytes([1, 2, 3, 4, 5, 6, 7, 8]);
        let mut a = [0xAAu8; 7];
        let mut b = [0xBBu8; 0];
        let mut batch: [&mut [u8]; 2] = [&mut a, &mut b];
        scramble_batch(&cw, &mut batch);
        descramble_batch(&cw, &mut batch);
        assert_eq!(a, [0xAAu8; 7]);
    }

    #[test]
    fn empty_batch_is_a_no_op() {
        let cw = ControlWord::from_bytes([1, 2, 3, 4, 5, 6, 7, 8]);
        scramble_batch(&cw, &mut []);
        descramble_batch(&cw, &mut []);
    }
}
