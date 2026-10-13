//! Allocation bound for SAT serialization (audit r02-O2): sizing a bit-packed
//! SAT body used to trial-encode into a fresh 4 KiB heap buffer, three times
//! per `serialize_into`. The spec-layout `PositionV2` section below is the
//! hand-built vector from the table's own tests (EN 300 468 Table 11d).

use broadcast_common::{Parse, Serialize, crc32_mpeg2};
use dvb_si::tables::sat::SatSection;
use test_alloc::ThreadCounting;

#[global_allocator]
static A: ThreadCounting = ThreadCounting::new();

fn position_v2_section() -> Vec<u8> {
    let mut bytes = vec![
        0x4D, 0xF0, 0x11, 0x00, 0x00, 0xCB, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x00, 0x12, 0x34,
        0x80,
    ];
    let crc = crc32_mpeg2::compute(&bytes);
    bytes.extend_from_slice(&crc.to_be_bytes());
    bytes
}

#[test]
fn sat_serialize_does_not_trial_encode_on_the_heap() {
    let bytes = position_v2_section();
    let sat = SatSection::parse(&bytes).expect("spec-layout SAT");
    let mut out = vec![0u8; sat.serialized_len()];
    let (written, snap) = ThreadCounting::measure(|| {
        let n = sat.serialized_len();
        assert_eq!(n, out.len());
        sat.serialize_into(&mut out).expect("serialize")
    });
    assert_eq!(written, bytes.len());
    assert_eq!(out, bytes, "byte-identical re-serialize");
    assert_eq!(
        snap.allocs, 0,
        "serialized_len/serialize_into allocated {} bytes on the heap",
        snap.bytes
    );
}
