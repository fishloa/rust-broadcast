//! Real-capture validation against a conformant T2-MI stream.
//!
//! Fixture: `tests/fixtures/colombia-capital-t2mi.ts` — a small slice (PAT +
//! PMT + T2-MI PID 0x0040, stuffing removed, packet-capped) of the Capital TV
//! Colombia stream from <https://tsduck.io/streams/colombia-capital-t2mi/>,
//! described there as a fully conformant embedded T2-MI signalisation.
//!
//! This exercises the pump → `AnyPayload` dispatch → decoded-accessor path on
//! real broadcast bytes (not synthetic vectors): every emitted timestamp
//! decodes to an `emission_offset`, and L1-current packets parse.
#![cfg(feature = "ts")]

use dvb_t2mi::payload::AnyPayload;
use dvb_t2mi::pump::T2miPump;

const T2MI_PID: u16 = 0x0040;
const TS_PACKET: usize = 188;

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/colombia-capital-t2mi.ts"
    );
    std::fs::read(path).expect("fixture present (excluded from published crate)")
}

#[test]
fn colombia_t2mi_timestamps_and_l1_decode() {
    let data = fixture();
    assert_eq!(data.len() % TS_PACKET, 0, "whole TS packets");

    let mut pump = T2miPump::new(T2MI_PID);
    let mut timestamps = 0usize;
    let mut offsets = 0usize;
    let mut l1_current = 0usize;
    let mut bbframes = 0usize;

    for pkt in data.chunks(TS_PACKET) {
        for event in pump.feed_ts(pkt) {
            match event.payload() {
                Ok(AnyPayload::Timestamp(ts)) => {
                    timestamps += 1;
                    // A live broadcast timestamp is absolute, not the all-ones
                    // null sentinel, and must decode to an offset since 2000.
                    assert!(!ts.is_null(), "live timestamp must not be null");
                    if ts.emission_offset().is_some() {
                        offsets += 1;
                    }
                }
                Ok(AnyPayload::L1Current(_)) => l1_current += 1,
                Ok(AnyPayload::Bbframe(_)) => bbframes += 1,
                _ => {}
            }
        }
    }

    // The slice is sized to contain several T2 frames.
    assert!(
        timestamps >= 5,
        "expected real timestamp packets, got {timestamps}"
    );
    assert_eq!(
        offsets, timestamps,
        "every real timestamp must decode to an emission_offset"
    );
    assert!(
        l1_current >= 5,
        "expected L1-current packets, got {l1_current}"
    );
    assert!(bbframes >= 50, "expected the BBFrame bulk, got {bbframes}");
    // Per-packet CRC-32 is checked inside the pump; a conformant stream
    // should not accumulate failures.
    assert_eq!(
        pump.stats().crc_failures,
        0,
        "no CRC failures on conformant stream"
    );
}
