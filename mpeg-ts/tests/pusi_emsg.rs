//! Integration test: PUSI-delimited payload reassembly for emsg-on-PID-0x0004.
//!
//! Verifies that:
//! - `PusiReassembler` can reassemble a real TS packet carrying a DASH emsg box
//! - The reassembled bytes are byte-identical to the known-good fixture
//! - The emsg box parses correctly via `mp4-emsg`

use mpeg_ts::pusi::PusiReassembler;
use mpeg_ts::OwnedTsPacket;

/// Path to the TS fixture: one 188-byte packet, PID 0x0004, PUSI=1,
/// adaptation-field-stuffed, payload = 98-byte emsg box.
const TS_FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/emsg-pid4.ts");

/// Path to the known-good emsg box bytes (98 bytes, version 1, SCTE-35).
const EMSG_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/shared/emsg_v1_scte35_livesim.bin"
);

#[test]
fn reassemble_emsg_from_ts() {
    // Read the TS packet fixture.
    let ts_bytes: [u8; 188] = {
        let mut buf = [0u8; 188];
        let data = std::fs::read(TS_FIXTURE).expect("failed to read TS fixture");
        assert_eq!(data.len(), 188, "fixture must be exactly 188 bytes");
        buf.copy_from_slice(&data);
        buf
    };

    let pkt = OwnedTsPacket::parse(ts_bytes).expect("failed to parse TS packet");
    assert_eq!(pkt.pid, 0x0004, "expected PID 0x0004");
    assert!(pkt.pusi, "expected PUSI=1");
    assert!(pkt.has_adaptation, "expected adaptation field");

    let payload = pkt.payload().expect("expected payload bytes in TS packet");

    // Read the expected emsg box bytes.
    let expected = std::fs::read(EMSG_FIXTURE).expect("failed to read emsg fixture");
    assert_eq!(expected.len(), 98, "expected emsg fixture to be 98 bytes");

    // Reassemble via PusiReassembler.
    let mut reasm = PusiReassembler::new(0x0004);
    assert!(reasm.push(0x0004, true, payload).is_none());
    let reassembled = reasm.flush().expect("flush should return the unit");

    // Byte-exact match.
    assert_eq!(
        reassembled, expected,
        "reassembled bytes must match the emsg fixture"
    );

    // Now parse as an emsg box.
    let emsg =
        mp4_emsg::EmsgBox::parse(&reassembled).expect("reassembled bytes must parse as EmsgBox");
    assert_eq!(
        emsg.timescale, 90_000,
        "expected timescale 90000 for SCTE-35 livesim emsg"
    );

    // Round-trip: to_vec should produce byte-identical output.
    let serialized = emsg.to_vec().expect("EmsgBox::to_vec should succeed");
    assert_eq!(
        serialized, reassembled,
        "emsg round-trip must be byte-identical to the reassembled payload"
    );
}
