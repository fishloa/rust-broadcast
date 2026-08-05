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
