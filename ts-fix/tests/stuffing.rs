//! Null packet stuffing / drop tests for `ts-fix`.
//!
//! Tests that the stuffing() operation correctly:
//! 1. Drops all null packets (PID 0x1FFF) when in drop mode,
//! 2. Preserves all non-null packets in the same order and byte-identical,
//! 3. Inserts null packets at a configured rate when in pad mode,
//! 4. The inserted packets are valid null packets (PID 0x1FFF, proper format).

use std::{fs, path::PathBuf};

mod support;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("m6-single.ts")
}

/// Check if a packet is a null packet (PID 0x1FFF).
fn is_null_packet(pkt: &[u8]) -> bool {
    if pkt.len() < 3 {
        return false;
    }
    support::extract_pid(pkt) == 0x1FFF
}

/// Validate that a packet is a properly-formed null packet.
fn validate_null_packet(pkt: &[u8]) -> bool {
    if pkt.len() != 188 {
        return false;
    }
    // Sync byte
    if pkt[0] != 0x47 {
        return false;
    }
    // PID must be 0x1FFF
    if support::extract_pid(pkt) != 0x1FFF {
        return false;
    }
    // Adaptation field control must be 01 (payload only).
    // Byte 3 bits [5:4] = adaptation control.
    let afc = (pkt[3] >> 4) & 0x03;
    if afc != 0b01 {
        return false;
    }
    // Rest should be 0xFF (padding). Check byte 4 onwards.
    pkt[4..].iter().all(|&b| b == 0xFF)
}

#[test]
fn drop_nulls_removes_all_null_packets() {
    let input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");

    // Create a synthetic stream with null packets interleaved.
    let input_with_nulls = support::add_null_packets(&input, 10);

    // Count nulls before processing.
    let null_count_before = input_with_nulls
        .chunks(188)
        .filter(|pkt| is_null_packet(pkt))
        .count();
    assert!(
        null_count_before > 0,
        "test fixture should have null packets inserted"
    );

    // Build engine with drop_nulls.
    let mut engine = ts_fix::TsFix::builder()
        .stuffing(ts_fix::Stuffing::drop_nulls())
        .build()
        .expect("drop_nulls build should not fail");

    let mut output = Vec::with_capacity(input_with_nulls.len());

    for chunk in input_with_nulls.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }

    engine.finish(|pkt| output.extend_from_slice(pkt));

    // Verify no null packets in output.
    let null_count_after = output.chunks(188).filter(|pkt| is_null_packet(pkt)).count();
    assert_eq!(
        null_count_after, 0,
        "output should have zero null packets after drop_nulls, found {null_count_after}"
    );

    // Verify all non-null packets from input are preserved.
    let input_non_null: Vec<&[u8]> = input_with_nulls
        .chunks(188)
        .filter(|pkt| !is_null_packet(pkt))
        .collect();
    let output_packets: Vec<&[u8]> = output.chunks(188).collect();

    assert_eq!(
        input_non_null.len(),
        output_packets.len(),
        "all non-null input packets should be in output"
    );

    for (idx, (input_pkt, output_pkt)) in
        input_non_null.iter().zip(output_packets.iter()).enumerate()
    {
        assert_eq!(
            input_pkt, output_pkt,
            "packet {idx} should be byte-identical after drop_nulls"
        );
    }
}

#[test]
fn drop_nulls_is_identity_on_stream_without_nulls() {
    let input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");

    // Use the clean fixture (no injected nulls).
    let mut engine = ts_fix::TsFix::builder()
        .stuffing(ts_fix::Stuffing::drop_nulls())
        .build()
        .expect("drop_nulls build should not fail");

    let mut output = Vec::with_capacity(input.len());

    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }

    engine.finish(|pkt| output.extend_from_slice(pkt));

    // Verify output is identical to input (assuming m6-single has no nulls).
    assert_eq!(
        output.len(),
        input.len(),
        "output length should match input"
    );
    assert_eq!(
        output, input,
        "output should be byte-identical to input when no nulls present"
    );
}

#[test]
fn pad_to_2_0_doubles_packet_count() {
    let input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");
    let input_packet_count = input.len() / 188;

    // Build engine with pad_to(2.0).
    let mut engine = ts_fix::TsFix::builder()
        .stuffing(ts_fix::Stuffing::pad_to(2.0))
        .build()
        .expect("pad_to build should not fail");

    let mut output = Vec::new();

    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }

    engine.finish(|pkt| output.extend_from_slice(pkt));

    let output_packet_count = output.len() / 188;

    // With pad_to(2.0), output should be approximately 2× the input.
    // Exactly 2× for integer packet counts.
    assert_eq!(
        output_packet_count,
        input_packet_count * 2,
        "pad_to(2.0) should double packet count: {input_packet_count} input → {output_packet_count} output"
    );

    // Verify output is a valid multiple of 188.
    assert_eq!(
        output.len() % 188,
        0,
        "output must be a multiple of 188 bytes"
    );
}

#[test]
fn pad_to_inserts_valid_null_packets() {
    let input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");

    // Build engine with pad_to(1.5) to add 50% more packets.
    let mut engine = ts_fix::TsFix::builder()
        .stuffing(ts_fix::Stuffing::pad_to(1.5))
        .build()
        .expect("pad_to build should not fail");

    let mut output = Vec::new();

    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }

    engine.finish(|pkt| output.extend_from_slice(pkt));

    // Count null packets in output.
    let null_packets: Vec<&[u8]> = output
        .chunks(188)
        .filter(|pkt| is_null_packet(pkt))
        .collect();

    assert!(
        !null_packets.is_empty(),
        "pad_to(1.5) should insert null packets"
    );

    // Verify all inserted null packets are valid.
    for (idx, pkt) in null_packets.iter().enumerate() {
        assert!(
            validate_null_packet(pkt),
            "null packet {idx} is not properly formed (PID 0x1FFF, AFC=01, 0xFF padding)"
        );
    }
}

#[test]
fn pad_to_rate_matches_expected_count() {
    let input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");
    let input_packet_count = input.len() / 188;

    // Test various rates and verify approximate output counts.
    // Note: rates < 1.0 would require dropping packets, which is done via drop_nulls().
    // pad_to() only inserts nulls, so rates < 1.0 are not supported.
    for rate in &[1.0, 1.5, 2.0, 3.0] {
        let mut engine = ts_fix::TsFix::builder()
            .stuffing(ts_fix::Stuffing::pad_to(*rate))
            .build()
            .expect("pad_to build should not fail");

        let mut output = Vec::new();

        for chunk in input.chunks(188) {
            engine
                .push(chunk, |pkt| output.extend_from_slice(pkt))
                .expect("valid 188-byte packet");
        }

        engine.finish(|pkt| output.extend_from_slice(pkt));

        let output_packet_count = output.len() / 188;
        let expected = (input_packet_count as f64 * rate).round() as usize;

        assert_eq!(
            output_packet_count, expected,
            "pad_to({rate}) should produce {expected} packets, got {output_packet_count}"
        );
    }
}

#[test]
fn pad_to_preserves_input_packets() {
    let input = fs::read(fixture_path()).expect("fixture m6-single.ts not found");

    // Build engine with pad_to(2.0).
    let mut engine = ts_fix::TsFix::builder()
        .stuffing(ts_fix::Stuffing::pad_to(2.0))
        .build()
        .expect("pad_to build should not fail");

    let mut output = Vec::new();

    for chunk in input.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }

    engine.finish(|pkt| output.extend_from_slice(pkt));

    // Filter output to non-null packets and compare with input.
    let output_non_null: Vec<&[u8]> = output
        .chunks(188)
        .filter(|pkt| !is_null_packet(pkt))
        .collect();

    let input_packets: Vec<&[u8]> = input.chunks(188).collect();

    assert_eq!(
        output_non_null.len(),
        input_packets.len(),
        "all input packets should appear in output (null packets are additions only)"
    );

    for (idx, (input_pkt, output_pkt)) in
        input_packets.iter().zip(output_non_null.iter()).enumerate()
    {
        assert_eq!(
            input_pkt, output_pkt,
            "packet {idx} should be byte-identical after pad_to"
        );
    }
}
