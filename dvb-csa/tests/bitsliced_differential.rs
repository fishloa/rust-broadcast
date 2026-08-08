//! Differential test — the bitsliced batch path against the scalar reference.
//!
//! The bitsliced path is a re-expression of the same cipher, so the only
//! acceptable result is **byte-identical** output. This file drives randomised
//! payloads through both paths and requires them to agree, in both directions.
//!
//! The lengths are chosen to hit every branch the batch driver has: payloads
//! below one block (the pass-through case), exactly one block, lengths that are
//! not a multiple of 8 (so the final partial block is stream-ciphered but never
//! block-ciphered), and long payloads. Batch sizes straddle [`LANES`] so the
//! grouping loop is exercised on both sides of a group boundary, and every
//! batch mixes lengths so that lanes retire at different rounds — a bug that
//! let a finished lane write over a live one would show up there and nowhere
//! else.
#![cfg(feature = "bitsliced")]

use dvb_csa::bitsliced::{LANES, descramble_batch, scramble_batch};
use dvb_csa::{ControlWord, descramble, scramble};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Payload lengths worth mixing into a batch.
///
/// `0`/`1`/`7` are below one block and must pass through untouched; `8` is a
/// single block with an empty stream-cipher region; `9`, `13`, `23`, `100` and
/// `183` are not multiples of 8; the rest are exact block multiples.
const LENGTHS: [usize; 12] = [0, 1, 7, 8, 9, 13, 16, 23, 100, 176, 183, 184];

fn random_cw(rng: &mut StdRng) -> ControlWord {
    let mut cw = [0u8; 8];
    rng.fill(&mut cw);
    ControlWord::from_bytes(cw)
}

fn random_batch(rng: &mut StdRng, count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|_| {
            let len = LENGTHS[rng.random_range(0..LENGTHS.len())];
            let mut v = vec![0u8; len];
            rng.fill(v.as_mut_slice());
            v
        })
        .collect()
}

/// Batch sizes that straddle the grouping boundary.
fn batch_sizes() -> Vec<usize> {
    vec![1, 2, 7, LANES - 1, LANES, LANES + 1, 2 * LANES + 3]
}

fn as_slices(data: &mut [Vec<u8>]) -> Vec<&mut [u8]> {
    data.iter_mut().map(|v| v.as_mut_slice()).collect()
}

/// Run `batch` and `single` over the same inputs and require identical output.
fn differential(
    label: &str,
    batch: fn(&ControlWord, &mut [&mut [u8]]),
    single: fn(&ControlWord, &mut [u8]),
) {
    let mut rng = StdRng::seed_from_u64(0x00DB_C5A2_0000_0001);
    let mut checked = 0usize;
    for size in batch_sizes() {
        for trial in 0..4 {
            let cw = random_cw(&mut rng);
            let mut sliced = random_batch(&mut rng, size);
            let mut scalar = sliced.clone();

            batch(&cw, &mut as_slices(&mut sliced));
            for payload in scalar.iter_mut() {
                single(&cw, payload);
            }

            for (lane, (got, want)) in sliced.iter().zip(scalar.iter()).enumerate() {
                assert_eq!(
                    got,
                    want,
                    "{label}: batch of {size}, trial {trial}, lane {lane} \
                     ({} bytes) disagrees with the scalar path",
                    got.len()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 1000, "{label}: only {checked} payloads compared");
}

#[test]
fn scramble_batch_matches_the_scalar_path() {
    differential("scramble", scramble_batch, scramble);
}

#[test]
fn descramble_batch_matches_the_scalar_path() {
    differential("descramble", descramble_batch, descramble);
}

/// Crossing the paths: whichever path scrambled it, the other must recover it.
#[test]
fn the_two_paths_interoperate_in_both_directions() {
    let mut rng = StdRng::seed_from_u64(0xC5A2_C5A2_C5A2_C5A2);
    for size in batch_sizes() {
        let cw = random_cw(&mut rng);
        let original = random_batch(&mut rng, size);

        // Scalar scramble -> bitsliced descramble.
        let mut data = original.clone();
        for payload in data.iter_mut() {
            scramble(&cw, payload);
        }
        descramble_batch(&cw, &mut as_slices(&mut data));
        assert_eq!(data, original, "scalar scramble / batch descramble, {size}");

        // Bitsliced scramble -> scalar descramble.
        let mut data = original.clone();
        scramble_batch(&cw, &mut as_slices(&mut data));
        for payload in data.iter_mut() {
            descramble(&cw, payload);
        }
        assert_eq!(data, original, "batch scramble / scalar descramble, {size}");
    }
}

/// A batch must not be sensitive to what its *other* lanes contain. Running one
/// payload alone and running it beside 63 strangers must give the same bytes.
#[test]
fn a_lane_is_unaffected_by_its_neighbours() {
    let mut rng = StdRng::seed_from_u64(0x1234_5678_9abc_def0);
    let cw = random_cw(&mut rng);

    for &len in LENGTHS.iter() {
        let mut subject = vec![0u8; len];
        rng.fill(subject.as_mut_slice());

        let mut alone = vec![subject.clone()];
        scramble_batch(&cw, &mut as_slices(&mut alone));

        for position in [0usize, 1, LANES / 2, LANES - 1] {
            let mut crowd = random_batch(&mut rng, LANES);
            crowd[position] = subject.clone();
            scramble_batch(&cw, &mut as_slices(&mut crowd));
            assert_eq!(
                crowd[position], alone[0],
                "a {len}-byte payload at lane {position} was perturbed by its neighbours"
            );
        }
    }
}
