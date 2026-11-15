//! The committed `m6-single.ts` capture joins the stream mid-PES: its
//! `private_stream_1` units declare a `PES_packet_length` longer than the bytes
//! captured. [`PesPacket::parse`] reports that as a truncated packet; the
//! lenient [`PesPacket::parse_unit`] returns the unit's bytes as received with
//! the same shortfall typed. (Deleting every
//! `PES_packet_length != 0 && len < 6 + length` filter from the two m6-driven
//! suites below — `fixture_pes.rs` and `dvb-subtitle`'s `support::data_fields` —
//! turns them red on this capture.)

use mpeg_pes::{Error, PesAssembler, PesPacket, Truncation};

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
            "the strict parse reports a truncated unit, never returns it short"
        );
        // The lenient entry returns the same unit's bytes, reported short.
        let (packet, short) = PesPacket::parse_unit(&raw)
            .unwrap_or_else(|e| panic!("parse_unit must accept a joined unit: {e:?}"));
        assert_eq!(
            short,
            Some(Truncation {
                declared,
                available: raw.len() - 6,
            }),
            "parse_unit names the declared shortfall"
        );
        // The payload is every byte after the header, never padded.
        let hdr_len = 9 + usize::from(raw[8]);
        assert_eq!(packet.payload, &raw[hdr_len..], "payload is as received");
    }
    assert!(truncated > 0, "the capture's units are truncated");
}

#[test]
fn a_complete_unit_round_trips_through_parse_unit_without_a_truncation() {
    // A bounded packet whose declared length exactly covers its bytes.
    let b = [
        0x00, 0x00, 0x01, 0xE0, 0x00, 0x08, 0x80, 0x00, 0x00, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
    ];
    let (packet, short) = PesPacket::parse_unit(&b).expect("a complete unit parses");
    assert_eq!(short, None, "a complete unit is not truncated");
    let mut out = vec![0u8; packet.serialized_len()];
    packet.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..], &b[..], "a complete unit round-trips byte-exactly");
}
