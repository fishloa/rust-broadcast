//! Fixture-backed tests: parse AIT, DSM-CC, and NIT sections from captured TS.
//!
//! `tests/fixtures/m6-single.ts` contains:
//! - AIT (table_id 0x74) on PID 0x00AA (3 sections)
//! - DSM-CC (table_id 0x3B, 0x3C) on PID 0x00AB (3 sections total)
//!
//! NIT fixtures (table_id 0x40/0x41 on PID 0x0010):
//! - `tests/fixtures/tnt-5w-12732v-isi6-10s.ts` — 2 NIT sections (primary fixture)

use dvb_common::{Parse, Serialize};
use dvb_si::tables::ait::Ait;
use dvb_si::tables::dsmcc::DsmccSection;
use dvb_si::tables::nit::Nit;
use dvb_si::ts::{SectionReassembler, TsPacket, TS_PACKET_SIZE};

/// Read a TS file and return all reassembled sections for a given PID.
fn extract_sections_for_pid(path: &str, target_pid: u16) -> Vec<Vec<u8>> {
    let data = std::fs::read(path).expect("read fixture");
    let mut reassembler = SectionReassembler::default();
    let mut sections = Vec::new();

    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE {
            continue;
        }
        let pkt = match TsPacket::parse(chunk) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if pkt.header.pid != target_pid {
            continue;
        }
        if let Some(payload) = pkt.payload {
            reassembler.feed(payload, pkt.header.pusi);
            while let Some(sec) = reassembler.pop_section() {
                sections.push(sec.to_vec());
            }
        }
    }
    sections
}

#[test]
fn fixture_m6_ait_sections_parse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m6-single.ts");
    let sections = extract_sections_for_pid(path, 0x00AA);

    assert!(
        !sections.is_empty(),
        "expected at least 1 complete AIT section on PID 0x00AA, got {}",
        sections.len()
    );

    let mut parsed_count = 0;
    for sec in &sections {
        if sec.is_empty() || sec[0] != 0x74 {
            continue;
        }
        let _ait = Ait::parse(sec).expect("AIT should parse");
        parsed_count += 1;
    }
    assert_eq!(
        parsed_count, sections.len(),
        "all AIT sections should parse"
    );
}

#[test]
fn fixture_m6_ait_round_trip() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m6-single.ts");
    let sections = extract_sections_for_pid(path, 0x00AA);

    for sec in &sections {
        if sec.is_empty() || sec[0] != 0x74 {
            continue;
        }
        let ait = Ait::parse(sec).expect("AIT parse");
        let mut buf = vec![0u8; ait.serialized_len()];
        ait.serialize_into(&mut buf).expect("AIT serialize");
        let reparsed = Ait::parse(&buf).expect("AIT reparse");
        assert_eq!(ait, reparsed);
    }
}

#[test]
fn fixture_m6_dsmcc_sections_parse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m6-single.ts");
    let sections = extract_sections_for_pid(path, 0x00AB);

    assert!(
        sections.len() >= 2,
        "expected at least 2 complete DSM-CC sections on PID 0x00AB, got {}",
        sections.len()
    );

    let mut parsed_count = 0;
    for sec in &sections {
        if sec.is_empty() {
            continue;
        }
        let tid = sec[0];
        if tid != 0x3B && tid != 0x3C {
            continue;
        }
        let dsm = DsmccSection::parse(sec).expect("DSM-CC should parse");
        assert_eq!(dsm.table_id, tid);
        parsed_count += 1;
    }
    assert_eq!(
        parsed_count, sections.len(),
        "all DSM-CC sections should parse"
    );
}

#[test]
fn fixture_m6_dsmcc_round_trip() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m6-single.ts");
    let sections = extract_sections_for_pid(path, 0x00AB);

    for sec in &sections {
        if sec.is_empty() {
            continue;
        }
        let tid = sec[0];
        if tid != 0x3B && tid != 0x3C {
            continue;
        }
        let dsm = DsmccSection::parse(sec).expect("DSM-CC parse");
        let mut buf = vec![0u8; dsm.serialized_len()];
        dsm.serialize_into(&mut buf).expect("DSM-CC serialize");
        let reparsed = DsmccSection::parse(&buf).expect("DSM-CC reparse");
        assert_eq!(dsm, reparsed);
    }
}

#[test]
fn fixture_tnt_isi6_nit_sections_parse() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tnt-5w-12732v-isi6-10s.ts");
    let sections = extract_sections_for_pid(path, 0x0010);

    assert!(
        !sections.is_empty(),
        "expected at least 1 complete NIT section on PID 0x0010, got {}",
        sections.len()
    );

    let mut parsed_count = 0;
    for sec in &sections {
        if sec.is_empty() {
            continue;
        }
        let tid = sec[0];
        if tid != 0x40 && tid != 0x41 {
            continue;
        }
        if let Ok(_nit) = Nit::parse(sec) {
            parsed_count += 1;
        }
    }
    assert!(
        parsed_count > 0,
        "expected at least 1 NIT section to parse from tnt-5w-12732v-isi6-10s.ts"
    );
}

#[test]
fn fixture_tnt_isi6_nit_round_trip() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tnt-5w-12732v-isi6-10s.ts");
    let sections = extract_sections_for_pid(path, 0x0010);

    let mut round_tripped = 0;
    for sec in &sections {
        if sec.is_empty() || (sec[0] != 0x40 && sec[0] != 0x41) {
            continue;
        }
        let nit = match Nit::parse(sec) {
            Ok(n) => n,
            Err(_) => continue,
        };
        let mut buf = vec![0u8; nit.serialized_len()];
        nit.serialize_into(&mut buf).expect("NIT serialize");
        let reparsed = Nit::parse(&buf).expect("NIT reparse");
        assert_eq!(nit, reparsed);
        round_tripped += 1;
    }
    assert!(round_tripped > 0, "expected at least 1 NIT round-trip success");
}
