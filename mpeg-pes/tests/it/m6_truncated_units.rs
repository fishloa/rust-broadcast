//! The committed `m6-single.ts` capture joins the stream mid-PES: its
//! `private_stream_1` units declare a `PES_packet_length` longer than the bytes
//! captured, which `PesPacket::parse` reports as a truncated packet (#2746).
//! (Deleting every `PES_packet_length != 0 && len < 6 + length` filter from the
//! two m6-driven suites below — `fixture_pes.rs` and `dvb-subtitle`'s
//! `support::data_fields` — turns them red on this capture.)

use mpeg_pes::{Error, PesAssembler, PesPacket};

const PKT: usize = 188;
const PES_PID: u16 = 0x0082;

fn units() -> Vec<Vec<u8>> {
    let ts = test_fixtures::read("ts/m6-single.ts");
    let mut asm = PesAssembler::new();
    let mut out = Vec::new();
    for p in ts.chunks(PKT) {
        if p.len() != PKT || p[0] != 0x47 {
            continue;
        }
        let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
        if pid != PES_PID {
            continue;
        }
        let start = match (p[3] >> 4) & 0x03 {
            1 => 4,
            3 => 5 + usize::from(p[4]),
            _ => continue,
        };
        if start >= PKT {
            continue;
        }
        if let Some(v) = asm.feed(p[1] & 0x40 != 0, &p[start..]) {
            out.push(v);
        }
    }
    if let Some(v) = asm.flush() {
        out.push(v);
    }
    out
}

#[test]
fn an_m6_unit_shorter_than_its_declared_length_is_a_truncated_packet() {
    let mut truncated = 0;
    for raw in units() {
        if raw.len() < 6 {
            continue;
        }
        let declared = usize::from(u16::from_be_bytes([raw[4], raw[5]]));
        if declared == 0 || raw.len() >= 6 + declared {
            continue;
        }
        truncated += 1;
        assert_eq!(
            PesPacket::parse(&raw).unwrap_err(),
            Error::BufferTooShort(broadcast_common::error::BufferTooShort {
                need: 6 + declared,
                have: raw.len(),
                what: "PES_packet_length",
            }),
            "a truncated unit is reported, never returned short"
        );
    }
    assert!(truncated > 0, "the capture's units are truncated");
}
