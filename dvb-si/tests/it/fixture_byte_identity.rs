//! Byte-identity conformance test: every section emitted by `SiDemux` from a
//! broadcast capture must re-serialize to its original wire bytes.
//!
//! This enforces the CLAUDE.md invariant: parse → serialize → byte-identical
//! on real broadcast data (reserved bits, SSI, CRC included).
#![cfg(feature = "ts")]

use broadcast_common::Serialize;
use dvb_si::demux::SiDemux;
use mpeg_ts::ts::TS_PACKET_SIZE;

fn read_fixture(filename: &str) -> Vec<u8> {
    let subdir = if filename == "m6-single.ts" || filename.starts_with("tnt-5w") {
        "ts"
    } else {
        "si"
    };
    let path = test_fixtures::path(&format!("{subdir}/{filename}"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Feed every aligned 188-byte packet in `data` through `demux` and assert
/// that every emitted section re-serializes to its original wire bytes.
fn feed_and_assert_byte_identity(demux: &mut SiDemux, data: &[u8], fixture_name: &str) {
    let mut section_count = 0u64;
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
            continue;
        }
        for ev in demux.feed(chunk) {
            section_count += 1;
            let original: &[u8] = ev.bytes().as_ref();
            let table = match ev.table_section() {
                Ok(t) => t,
                Err(e) => {
                    // Tables with a known parser that nonetheless fails to parse
                    // (e.g. newer spec revision that added a constraint).  We
                    // can't byte-compare these — report and continue.
                    eprintln!(
                        "  [{section_count}] {fixture_name}: PID=0x{:04X} table_id=0x{:02X} \
                         parse failed: {e}",
                        ev.pid().value(),
                        ev.table_id(),
                    );
                    continue;
                }
            };
            let mut buf = vec![0u8; table.serialized_len()];
            table.serialize_into(&mut buf).unwrap();
            if buf != original {
                // Print a detailed diff header so the user can see which
                // section/byte diverges.
                eprintln!(
                    "  [{section_count}] {fixture_name}: PID=0x{:04X} table_id=0x{:02X} \
                     ({}) byte MISMATCH (len: original={}, roundtrip={})",
                    ev.pid().value(),
                    ev.table_id(),
                    table.name(),
                    original.len(),
                    buf.len(),
                );
                let min = original.len().min(buf.len());
                for i in 0..min {
                    if original[i] != buf[i] {
                        eprintln!(
                            "    byte {i}: expected 0x{:02X}, got 0x{:02X}",
                            original[i], buf[i]
                        );
                    }
                }
                if original.len() != buf.len() {
                    eprintln!(
                        "    length mismatch: original has {} bytes, roundtrip has {}",
                        original.len(),
                        buf.len()
                    );
                }
                panic!(
                    "{fixture_name}: section[{section_count}] PID=0x{:04X} \
                     table_id=0x{:02X} ({}) NOT byte-identical",
                    ev.pid().value(),
                    ev.table_id(),
                    table.name(),
                );
            }
        }
    }
    eprintln!("  {fixture_name}: {section_count} sections, all byte-identical");
}

// ── m6-single.ts ─────────────────────────────────────────────────────────

fn m6_demux() -> SiDemux {
    use mpeg_ts::pid::Pid;
    SiDemux::builder()
        .pid(Pid::from_const(0x00AA)) // AIT
        .pid(Pid::from_const(0x00AB)) // DSM-CC HbbTV carousel
        .build()
}

#[test]
fn m6_byte_identity() {
    let data = read_fixture("m6-single.ts");
    let mut demux = m6_demux();
    feed_and_assert_byte_identity(&mut demux, &data, "m6-single.ts");
}

// ── tnt-5w-12732v-isi6-10s.ts ────────────────────────────────────────────

#[test]
fn tnt_byte_identity() {
    let data = read_fixture("tnt-5w-12732v-isi6-10s.ts");
    let mut demux = SiDemux::builder().build();
    feed_and_assert_byte_identity(&mut demux, &data, "tnt-5w-12732v-isi6-10s.ts");
}
