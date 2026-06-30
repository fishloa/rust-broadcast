//! Identity (no-op) round-trip test for `ts-fix`.
//!
//! Verifies that `TsFix::builder().build()?` with no operations configured is
//! a pure pass-through: the concatenated output is byte-identical to the input,
//! and the packet count matches `input_len / 188`.
//!
//! # Forward-compat proof
//!
//! The test also demonstrates that the public API surface is forward-compatible:
//!
//! - `TsFix::builder()` returns a `TsFixBuilder` (opaque struct, no enum variant).
//! - `TsFixBuilder::build()` returns `Result<TsFix, ts_fix::Error>`.
//! - `Error` is `#[non_exhaustive]` — a new variant in v0.2 cannot cause a
//!   match-exhaustiveness compile error in downstream code.
//! - `TsFix::push` and `TsFix::finish` signatures do not change when new builder
//!   methods are added (the engine is reconfigured internally).
//!
//! A hypothetical future builder method:
//!
//! ```rust,ignore
//! // Adding this in v0.2 is purely additive — the lines below compile identically:
//! let _engine = ts_fix::TsFix::builder().build().unwrap();
//! // and the hypothetical new method:
//! // let _engine = ts_fix::TsFix::builder().wrap_pts().build().unwrap();
//! ```
//!
//! Neither form changes `push`, `finish`, or `build`'s return type.

use std::{fs, path::PathBuf};

fn fixture_path() -> PathBuf {
    // Locate the fixture relative to the manifest directory so `cargo test` from
    // any working directory finds it.  `CARGO_MANIFEST_DIR` is set by Cargo.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("m6-single.ts")
}

#[test]
fn identity_passthrough_byte_identical() {
    let input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");

    // Sanity: fixture must be a non-empty multiple of 188.
    assert!(!input.is_empty(), "fixture is empty");
    assert_eq!(
        input.len() % 188,
        0,
        "fixture length {} is not a multiple of 188",
        input.len()
    );

    let expected_packet_count = input.len() / 188;

    // Build an identity engine (no ops).
    let mut engine = ts_fix::TsFix::builder()
        .build()
        .expect("identity build should not fail");

    let mut output: Vec<u8> = Vec::with_capacity(input.len());
    let mut emitted_count: usize = 0;

    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| {
                output.extend_from_slice(pkt);
                emitted_count += 1;
            })
            .expect("valid 188-byte packet from fixture");
    }

    engine.finish(|pkt| {
        output.extend_from_slice(pkt);
        emitted_count += 1;
    });

    // Packet count matches.
    assert_eq!(
        emitted_count, expected_packet_count,
        "emitted {} packets, expected {}",
        emitted_count, expected_packet_count
    );

    // Output is byte-identical to input.
    assert_eq!(output, input, "identity engine output differs from input");
}

#[test]
fn identity_rejects_short_packet() {
    let mut engine = ts_fix::TsFix::builder().build().unwrap();
    let short = [0x47u8; 100]; // 100 bytes, not 188
    let result = engine.push(&short, |_| {});
    assert!(result.is_err(), "engine should reject a short packet");
    match result.unwrap_err() {
        ts_fix::Error::ShortPacket { len } => assert_eq!(len, 100),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn identity_rejects_bad_sync_byte() {
    let mut engine = ts_fix::TsFix::builder().build().unwrap();
    let mut pkt = [0u8; 188];
    pkt[0] = 0x00; // wrong sync byte
    let result = engine.push(&pkt, |_| {});
    assert!(
        result.is_err(),
        "engine should reject a packet with bad sync byte"
    );
    match result.unwrap_err() {
        ts_fix::Error::NoSyncByte { found } => assert_eq!(found, 0x00),
        other => panic!("unexpected error: {other:?}"),
    }
}

/// Confirm `Error` is `#[non_exhaustive]` — this would fail to compile if
/// `Error` were exhaustive and a future match arm were missing.
///
/// We match with a wildcard arm to prove forward-compat: adding a new variant
/// in v0.2 does not break this match.
#[test]
fn error_is_non_exhaustive() {
    let err = ts_fix::Error::ShortPacket { len: 42 };
    let _label = match err {
        ts_fix::Error::ShortPacket { len } => alloc::format!("short:{len}"),
        ts_fix::Error::NoSyncByte { found } => alloc::format!("sync:{found:#04x}"),
        // Wildcard required because Error is #[non_exhaustive].
        _ => "unknown".to_string(),
    };
}

// Pull in `alloc` for the format! calls above (we're in a std test, so this
// is always available, but we alias it for clarity).
extern crate alloc;
