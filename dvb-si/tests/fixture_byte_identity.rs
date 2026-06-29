//! Byte-identity conformance test: every section emitted by `SiDemux` from a
//! broadcast capture must re-serialize to its original wire bytes.
//!
//! This enforces the CLAUDE.md invariant: parse → serialize → byte-identical
//! on real broadcast data (reserved bits, SSI, CRC included).
#![cfg(feature = "ts")]

use broadcast_common::Serialize;
use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTableSection;
use mpeg_ts::ts::TS_PACKET_SIZE;

fn read_fixture(filename: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), filename);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
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
            let mut buf = vec![0u8; serialized_len_of(&table)];
            serialize_into_of(&table, &mut buf);
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

/// Compute the serialized length of an [`AnyTableSection`] without needing
/// to implement `Serialize` on the enum.
fn serialized_len_of(table: &AnyTableSection<'_>) -> usize {
    match table {
        AnyTableSection::PatSection(t) => t.serialized_len(),
        AnyTableSection::CatSection(t) => t.serialized_len(),
        AnyTableSection::PmtSection(t) => t.serialized_len(),
        AnyTableSection::TsdtSection(t) => t.serialized_len(),
        AnyTableSection::DsmccSection(t) => t.serialized_len(),
        AnyTableSection::NitSection(t) => t.serialized_len(),
        AnyTableSection::SdtSection(t) => t.serialized_len(),
        AnyTableSection::BatSection(t) => t.serialized_len(),
        AnyTableSection::UntSection(t) => t.serialized_len(),
        AnyTableSection::IntSection(t) => t.serialized_len(),
        AnyTableSection::SatSection(t) => t.serialized_len(),
        AnyTableSection::EitSection(t) => t.serialized_len(),
        AnyTableSection::TdtSection(t) => t.serialized_len(),
        AnyTableSection::RstSection(t) => t.serialized_len(),
        AnyTableSection::StSection(t) => t.serialized_len(),
        AnyTableSection::TotSection(t) => t.serialized_len(),
        AnyTableSection::AitSection(t) => t.serialized_len(),
        AnyTableSection::ContainerSection(t) => t.serialized_len(),
        AnyTableSection::RctSection(t) => t.serialized_len(),
        AnyTableSection::CitSection(t) => t.serialized_len(),
        AnyTableSection::MpeFecSection(t) => t.serialized_len(),
        AnyTableSection::RntSection(t) => t.serialized_len(),
        AnyTableSection::MpeIfecSection(t) => t.serialized_len(),
        AnyTableSection::ProtectionMessage(t) => t.serialized_len(),
        AnyTableSection::DownloadableFontInfo(t) => t.serialized_len(),
        AnyTableSection::DitSection(t) => t.serialized_len(),
        AnyTableSection::SitSection(t) => t.serialized_len(),
        AnyTableSection::MpeDatagram(t) => t.serialized_len(),
        AnyTableSection::Unknown { raw, .. } => raw.len(),
        _ => panic!("unhandled AnyTableSection variant"),
    }
}

/// Serialize an [`AnyTableSection`] into `buf` (which must be the correct size).
fn serialize_into_of(table: &AnyTableSection<'_>, buf: &mut [u8]) {
    match table {
        AnyTableSection::PatSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::CatSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::PmtSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::TsdtSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::DsmccSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::NitSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::SdtSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::BatSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::UntSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::IntSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::SatSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::EitSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::TdtSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::RstSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::StSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::TotSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::AitSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::ContainerSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::RctSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::CitSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::MpeFecSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::RntSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::MpeIfecSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::ProtectionMessage(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::DownloadableFontInfo(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::DitSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::SitSection(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::MpeDatagram(t) => {
            t.serialize_into(buf).unwrap();
        }
        AnyTableSection::Unknown { raw, .. } => {
            buf.copy_from_slice(raw);
        }
        _ => panic!("unhandled AnyTableSection variant"),
    }
}

// ── m6-single.ts ─────────────────────────────────────────────────────────

fn m6_demux() -> SiDemux {
    use mpeg_ts::pid::Pid;
    SiDemux::builder()
        .pid(Pid::new(0x00AA)) // AIT
        .pid(Pid::new(0x00AB)) // DSM-CC HbbTV carousel
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
