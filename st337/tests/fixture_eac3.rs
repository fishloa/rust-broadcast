//! Real-fixture gate: wrap a real E-AC-3 syncframe in a hand-built ST 337
//! burst and confirm byte-identical parse/serialize round-tripping.
//!
//! ## Fixture provenance
//!
//! `tests/fixtures/eac3_frame0.bin` is 834 real bytes: the first E-AC-3
//! syncframe of this workspace's own ffmpeg-encoded capture
//! `fixtures/ts/dolby/eac3.ts` (already a committed Dolby oracle fixture,
//! issue #426). SMPTE ST 337 is an AES3 *wire*-layer format with no
//! recordable file capture of its own, so per this project's real-fixture
//! discipline (`docs/CRATE-ACCEPTANCE.md`) the payload bytes are real
//! (extracted from a genuine broadcast-style capture) while the burst
//! framing around them is built directly from this crate's own typed API --
//! the same "re-wrap real payload bytes under new framing" approach already
//! used this session for the `#638`/`#641` TS fixtures.
//!
//! `Pc`'s `data_type=21` here is the real value `ffmpeg -f spdif` uses for
//! E-AC-3 (an independent, running-software cross-check, not a value this
//! crate invented) -- see `docs/st337-PROVENANCE.md` for the full oracle
//! comparison, including the one genuine spec-family discrepancy it
//! surfaced (`length_code` bits-vs-bytes between SMPTE ST 337 and IEC 61937).

use broadcast_common::{Parse, Serialize};
use st337::{Burst, DataMode};

fn real_eac3_frame() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/eac3_frame0.bin"
    ))
    .expect("eac3_frame0.bin fixture must exist")
}

/// The real E-AC-3 IEC 61937 data_type code (docs/st337-PROVENANCE.md).
const EAC3_DATA_TYPE: u8 = 21;

fn build_burst(payload: &[u8]) -> Burst<'_> {
    Burst::new(EAC3_DATA_TYPE, DataMode::Mode16, false, 0, 0, None, payload)
        .expect("build burst from real E-AC-3 frame")
}

#[test]
fn wraps_real_eac3_frame_and_parses_back() {
    let payload = real_eac3_frame();
    assert_eq!(
        payload.len(),
        834,
        "real E-AC-3 frame 0 is 834 bytes (ffprobe-confirmed)"
    );

    let burst = build_burst(&payload);
    // length_code is BITS per SMPTE ST 337 §7.2.5's literal text (see the
    // PROVENANCE doc for why this differs from ffmpeg's own byte-count Pd).
    assert_eq!(burst.preamble.length_code, 834 * 8);

    let bytes = burst.to_bytes();
    // Pa/Pb are the real, independently-confirmed 16-bit-mode sync words.
    assert_eq!(&bytes[0..2], &st337::SYNC_WORD_PA.to_le_bytes());
    assert_eq!(&bytes[2..4], &st337::SYNC_WORD_PB.to_le_bytes());
    // Pc = 0x0015 little-endian, matching the real ffmpeg oracle exactly.
    assert_eq!(&bytes[4..6], &[0x15, 0x00]);

    let parsed = Burst::parse(&bytes).expect("parse the wrapped real burst");
    assert_eq!(parsed.preamble.data_type, EAC3_DATA_TYPE);
    assert_eq!(parsed.preamble.data_mode, DataMode::Mode16);
    assert!(!parsed.preamble.error_flag);
    assert_eq!(parsed.preamble.data_stream_number, 0);
    assert_eq!(
        parsed.payload, payload,
        "payload extracted from the parsed burst must be byte-identical to the real E-AC-3 frame"
    );
}

#[test]
fn byte_identical_round_trip_real_fixture() {
    let payload = real_eac3_frame();
    let burst = build_burst(&payload);

    let mut out = vec![0u8; burst.serialized_len()];
    let n = burst.serialize_into(&mut out).unwrap();
    assert_eq!(n, out.len());

    let reparsed = Burst::parse(&out).unwrap();
    assert_eq!(reparsed, burst, "parse(serialize(burst)) == burst");

    let mut out2 = vec![0u8; reparsed.serialized_len()];
    reparsed.serialize_into(&mut out2).unwrap();
    assert_eq!(
        out, out2,
        "serialize(parse(serialize(burst))) is byte-identical"
    );
}

#[test]
fn mutating_error_flag_changes_only_pc_bytes() {
    // Proves serialize_into recomputes Pc from typed fields rather than
    // echoing a stored raw buffer (guards against a `self.raw` passthrough).
    let payload = real_eac3_frame();
    let mut burst = build_burst(&payload);
    let original = burst.to_bytes();

    burst.preamble.error_flag = true;
    let mutated = burst.to_bytes();

    assert_ne!(
        original, mutated,
        "flipping error_flag must change the wire bytes"
    );
    assert_eq!(&original[0..4], &mutated[0..4], "Pa/Pb unaffected");
    assert_ne!(&original[4..6], &mutated[4..6], "Pc must change");
    assert_eq!(&original[6..], &mutated[6..], "Pd + payload unaffected");
}
