//! Wrap the committed real-fixture E-AC-3 frame (`tests/fixtures/eac3_frame0.bin`)
//! in an ST 337 burst, parse it back, and confirm the payload is
//! byte-identical to the real capture — SMPTE ST 337:2015 §7.
//!
//! Run with `cargo run -p st337 --example parse_burst`.

use broadcast_common::{Parse, Serialize};
use st337::{Burst, DataMode};

fn main() {
    // See docs/st337-PROVENANCE.md for how this fixture was extracted (a
    // real E-AC-3 syncframe from fixtures/ts/dolby/eac3.ts) and cross-checked
    // against ffmpeg's own IEC 61937 burst framing.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/eac3_frame0.bin"
    );
    let payload = std::fs::read(path).expect("fixture must exist");

    let burst = Burst::new(
        21, // data_type -- the IEC 61937 E-AC-3 code point used by the real
        // ffmpeg cross-check oracle (docs/st337-PROVENANCE.md); ST 337 itself
        // defers the data_type registry to SMPTE ST 338 (not verified here).
        DataMode::Mode16,
        false,
        0,
        0,
        None,
        &payload,
    )
    .expect("build burst");
    let bytes = burst.to_bytes();

    let parsed = Burst::parse(&bytes).expect("parse burst");
    println!("data_type: {}", parsed.preamble.data_type);
    println!("data_mode: {}", parsed.preamble.data_mode);
    println!("error_flag: {}", parsed.preamble.error_flag);
    println!("data_stream_number: {}", parsed.preamble.data_stream_number);
    println!(
        "length_code: {} bits ({} payload bytes)",
        parsed.preamble.length_code,
        parsed.payload.len()
    );
    assert_eq!(
        parsed.payload, payload,
        "byte-identical real E-AC-3 payload"
    );
    println!("payload matches the real E-AC-3 fixture byte-for-byte.");
}
