//! Golden-vector tests — bit-exact validation against libdvbcsa 1.1.0.
//!
//! The committed vectors in `tests/fixtures/libdvbcsa-vectors.hex` were
//! generated with libdvbcsa (VideoLAN's reference free implementation).
//! This test requires byte-identical scramble output AND byte-identical
//! descramble recovery for every vector.
use dvb_csa::{ControlWord, descramble, scramble};
use std::fs;

#[derive(Debug)]
struct Vector {
    cw: [u8; 8],
    plain: Vec<u8>,
    scrambled: Vec<u8>,
}

fn parse_hex_byte(s: &str) -> u8 {
    u8::from_str_radix(s, 16).expect("invalid hex byte")
}

fn parse_hex_line(prefix: &str, line: &str) -> Vec<u8> {
    let hex_part = line.strip_prefix(prefix).expect("missing prefix");
    hex_part.split_whitespace().map(parse_hex_byte).collect()
}

fn load_vectors(path: &str) -> Vec<Vector> {
    let content = fs::read_to_string(path).expect("failed to read vectors file");
    let mut vectors = Vec::new();
    let mut current_cw: Option<[u8; 8]> = None;
    let mut current_plain: Option<Vec<u8>> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("CW:") {
            current_cw = Some(parse_hex_line("CW:", line).try_into().unwrap());
        } else if line.starts_with("PLAIN:") {
            current_plain = Some(parse_hex_line("PLAIN:", line));
        } else if line.starts_with("SCRAMBLED:") {
            let scrambled = parse_hex_line("SCRAMBLED:", line);
            if let (Some(cw), Some(plain)) = (current_cw.take(), current_plain.take()) {
                vectors.push(Vector {
                    cw,
                    plain,
                    scrambled,
                });
            }
        }
    }
    vectors
}

/// The same libdvbcsa vectors, driven through the bitsliced batch path.
///
/// The bitsliced path must answer to the **external oracle**, not merely agree
/// with our own scalar code — a shared misreading of the cipher would satisfy
/// the differential test and fail here.
///
/// Each vector is placed in a different lane of a full batch whose other lanes
/// are unrelated decoy payloads of assorted lengths, so the test also fails if
/// a neighbouring lane can perturb the answer.
#[cfg(feature = "bitsliced")]
mod bitsliced {
    use super::{Vector, load_vectors};
    use dvb_csa::ControlWord;
    use dvb_csa::bitsliced::{LANES, descramble_batch, scramble_batch};

    /// Build a full batch of decoys with `v` planted at `lane`.
    fn planted(v: &[u8], lane: usize, salt: u8) -> Vec<Vec<u8>> {
        let lengths = [0usize, 7, 8, 13, 16, 100, 183, 184];
        let mut batch: Vec<Vec<u8>> = (0..LANES)
            .map(|i| {
                let len = lengths[(i + salt as usize) % lengths.len()];
                (0..len).map(|j| (j as u8) ^ salt ^ (i as u8)).collect()
            })
            .collect();
        batch[lane] = v.to_vec();
        batch
    }

    fn run(
        batch_fn: fn(&ControlWord, &mut [&mut [u8]]),
        vectors: &[Vector],
        input: fn(&Vector) -> &Vec<u8>,
        expected: fn(&Vector) -> &Vec<u8>,
        what: &str,
    ) {
        assert!(!vectors.is_empty(), "no vectors loaded");
        for (i, v) in vectors.iter().enumerate() {
            let lane = (i * 7) % LANES;
            let mut batch = planted(input(v), lane, i as u8);
            let mut refs: Vec<&mut [u8]> = batch.iter_mut().map(|b| b.as_mut_slice()).collect();
            batch_fn(&ControlWord::from_bytes(v.cw), &mut refs);
            assert_eq!(
                &batch[lane],
                expected(v),
                "Vector {} bitsliced {what} mismatch at lane {lane}:\n  CW: {:02x?}",
                i + 1,
                v.cw,
            );
        }
    }

    #[test]
    fn golden_vectors_scramble_batch() {
        let path = format!(
            "{}/tests/fixtures/libdvbcsa-vectors.hex",
            env!("CARGO_MANIFEST_DIR")
        );
        let vectors = load_vectors(&path);
        run(
            scramble_batch,
            &vectors,
            |v| &v.plain,
            |v| &v.scrambled,
            "scramble",
        );
    }

    #[test]
    fn golden_vectors_descramble_batch() {
        let path = format!(
            "{}/tests/fixtures/libdvbcsa-vectors.hex",
            env!("CARGO_MANIFEST_DIR")
        );
        let vectors = load_vectors(&path);
        run(
            descramble_batch,
            &vectors,
            |v| &v.scrambled,
            |v| &v.plain,
            "descramble",
        );
    }
}

#[test]
fn golden_vectors_scramble() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = format!("{}/tests/fixtures/libdvbcsa-vectors.hex", manifest_dir);
    let vectors = load_vectors(&path);

    assert!(!vectors.is_empty(), "no vectors loaded");

    for (i, v) in vectors.iter().enumerate() {
        let cw = ControlWord::from_bytes(v.cw);
        let mut data = v.plain.clone();
        scramble(&cw, &mut data);
        assert_eq!(
            data,
            v.scrambled,
            "Vector {} scramble mismatch:\n  CW: {:02x?}\n  plain: {:02x?}\n  expected: {:02x?}\n  got: {:02x?}",
            i + 1,
            v.cw,
            v.plain,
            v.scrambled,
            data,
        );
    }
}

#[test]
fn golden_vectors_descramble() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = format!("{}/tests/fixtures/libdvbcsa-vectors.hex", manifest_dir);
    let vectors = load_vectors(&path);

    assert!(!vectors.is_empty(), "no vectors loaded");

    for (i, v) in vectors.iter().enumerate() {
        let cw = ControlWord::from_bytes(v.cw);
        let mut data = v.scrambled.clone();
        descramble(&cw, &mut data);
        assert_eq!(
            data,
            v.plain,
            "Vector {} descramble mismatch:\n  CW: {:02x?}\n  scrambled: {:02x?}\n  expected: {:02x?}\n  got: {:02x?}",
            i + 1,
            v.cw,
            v.scrambled,
            v.plain,
            data,
        );
    }
}
