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
use mpeg_ts::pid::Pid;

const T2MI_PID: Pid = Pid::new(0x0040);
const TS_PACKET: usize = 188;

fn fixture() -> Vec<u8> {
    test_fixtures::read("dvb-t2mi/colombia-capital-t2mi.ts")
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

/// The pump filters on a typed [`Pid`]: the capture's T2-MI PID (0x0040)
/// yields packets, a different valid PID yields none, and a value outside the
/// 13-bit range cannot be turned into a filter at all (#1159).
#[test]
fn typed_pid_filter_selects_the_t2mi_pid_on_a_real_capture() {
    let data = fixture();
    let count = |pid: Pid| {
        let mut pump = T2miPump::new(pid);
        data.chunks(TS_PACKET)
            .map(|pkt| pump.feed_ts(pkt).count())
            .sum::<usize>()
    };
    assert!(count(T2MI_PID) > 100, "T2-MI PID must yield packets");
    assert_eq!(count(Pid::try_new(0x0041).unwrap()), 0);
    assert!(
        Pid::try_new(0x2040).is_err(),
        "0x2040 aliases 0x0040 if masked"
    );
}

/// The capture carries 11 individual-addressing (0x21) packets, each with
/// three transmitters (0x000B / 0x000C / 0x000D), each a single
/// `tx_time_offset_function` (tag 0x00; TS 101 191 §6.1.1, Table 7) with
/// offsets -100 / 0 / -50 (units of 100 ns).  Real bytes, per packet:
/// `00 15 | 00 0b 04 00 04 ff 9c | 00 0c 04 00 04 00 00 | 00 0d 04 00 04 ff ce`.
/// This is also the evidence that `function_length` counts the 2-byte
/// function header (`function_length` = 4 for a 2-byte body).
#[test]
fn colombia_individual_addressing_time_offsets_decode_and_round_trip() {
    use broadcast_common::Serialize;
    use dvb_t2mi::packet::Header;
    use dvb_t2mi::payload::individual_addressing::FunctionBody;

    let data = fixture();
    let mut pump = T2miPump::new(T2MI_PID);
    let mut seen = 0usize;
    for pkt in data.chunks(TS_PACKET) {
        for event in pump.feed_ts(pkt) {
            let Ok(AnyPayload::IndividualAddressing(ia)) = event.payload() else {
                continue;
            };
            seen += 1;
            let got: Vec<(u16, i16)> = ia
                .transmitters
                .iter()
                .map(|t| {
                    assert_eq!(t.functions.len(), 1);
                    assert_eq!(t.functions[0].tag, 0x00);
                    match &t.functions[0].body {
                        FunctionBody::TimeOffset(b) => (t.transmitter_id, b.time_offset),
                        other => panic!("expected typed TimeOffset, got {other:?}"),
                    }
                })
                .collect();
            assert_eq!(got, [(0x000B, -100), (0x000C, 0), (0x000D, -50)]);

            // Byte-exact round trip against the wire payload.
            let wire = Header::raw_payload_bytes(event.bytes()).unwrap();
            let mut out = vec![0u8; ia.serialized_len()];
            ia.serialize_into(&mut out).unwrap();
            assert_eq!(out, wire);
        }
    }
    assert_eq!(
        seen, 11,
        "expected the capture's 11 individual-addressing packets"
    );
}
