//! TSDuck as an independent oracle for `mux::SiMux` table-mixing on a shared
//! PID (issue #1000).
//!
//! `SiMux` used to key its entries by PID alone, so `upsert_tot` after
//! `upsert_tdt` (both PID `0x0014`) silently replaced the TDT entry — same
//! trap for SDT/BAT on `0x0011`. The fix keys entries by `(pid, table_id)`
//! and shares one packetiser (and so one continuity counter) per PID.
//!
//! Rather than re-validate the fix with our own `SectionReassembler` only
//! (self-consistent, but blind to a shared misreading of continuity-counter
//! rules), this test writes the muxed packets to a real `.ts` file and runs
//! them through TSDuck's own `tsp -P tables` (an independent PSI/SI table
//! collector) and `tsanalyze` (which reports continuity-counter errors per
//! PID) — the reference DVB toolkit, not our own code, doing the check.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration as StdDuration;

use mpeg_ts::mux::SiMux;
use mpeg_ts::ts::TS_PACKET_SIZE;

fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/tsduck-simux-oracle-tmp")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn tsduck_available() -> bool {
    Command::new("tsp")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Skip loudly rather than silently — a silently-skipped oracle reads as a
/// pass, which is worse than no oracle at all.
macro_rules! skip_unless_tsduck_available {
    () => {
        if !tsduck_available() {
            eprintln!(
                "SKIP tsduck_simux_oracle: `tsp` not on PATH (TSDuck, the reference DVB \
                 toolkit; see CLAUDE.md's command list). This test is a no-op result on \
                 this host, not real coverage."
            );
            return;
        }
    };
}

/// Build a short-form section (`section_syntax_indicator` = 0, no CRC) —
/// the form TDT/TOT use (ETSI EN 300 468 §5.2.5/§5.2.6). TSDuck's generic
/// PSI decoder validates the declared `section_length` structurally even
/// for garbage table content, so the SSI bit must match: SSI=1 with a body
/// too short for the long-form fields + CRC is a hard "invalid section
/// length" that TSDuck drops before it ever reaches per-table decoding.
fn build_short_section(table_id: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len() as u16;
    let mut v = Vec::with_capacity(3 + body.len());
    v.push(table_id);
    v.push(0x30 | ((len >> 8) as u8 & 0x0F));
    v.push((len & 0xFF) as u8);
    v.extend_from_slice(body);
    v
}

/// Build a long-form section (SDT/BAT's actual form): the mandatory 5-byte
/// long header (`table_id_extension`(16), reserved/version/current_next(8),
/// section_number(8), last_section_number(8)) plus `body_extra`, a real
/// CRC-32/MPEG (`broadcast_common::crc32_mpeg2`) computed over everything
/// ahead of it. TSDuck decodes real garbage content fine (see the reserved-
/// bit warnings this deliberately triggers) but needs a length-consistent,
/// CRC-terminated section to accept it as valid at all.
fn build_long_section(table_id: u8, table_id_extension: u16, body_extra: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(5 + body_extra.len());
    inner.extend_from_slice(&table_id_extension.to_be_bytes());
    inner.push(0xC1); // reserved(2)='11' + version_number(5)=0 + current_next_indicator(1)=1
    inner.push(0x00); // section_number
    inner.push(0x00); // last_section_number
    inner.extend_from_slice(body_extra);

    let length = (inner.len() + 4) as u16; // + CRC_32
    let mut section = Vec::with_capacity(3 + inner.len() + 4);
    section.push(table_id);
    section.push(0xB0 | ((length >> 8) as u8 & 0x0F));
    section.push((length & 0xFF) as u8);
    section.extend_from_slice(&inner);

    let crc = broadcast_common::crc32_mpeg2::compute(&section);
    section.extend_from_slice(&crc.to_be_bytes());
    section
}

fn write_ts(dir: &Path, name: &str, packets: &[[u8; TS_PACKET_SIZE]]) -> PathBuf {
    let path = dir.join(name);
    let mut f = fs::File::create(&path).expect("create scratch .ts file");
    for pkt in packets {
        f.write_all(pkt).expect("write TS packet");
    }
    path
}

/// TSDuck's own PSI/SI table collector: `tsp -I file <path> -P tables
/// --all-sections -O drop`, capturing its human-readable section listing on
/// stderr (that's where `tables` logs by default without `--output-file`).
fn tsduck_collect_tables(ts_path: &Path) -> String {
    let out = Command::new("tsp")
        .args([
            "-I",
            "file",
            ts_path.to_str().unwrap(),
            "-P",
            "tables",
            "--all-sections",
            "-O",
            "drop",
        ])
        .output()
        .expect("run tsp -P tables");
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// TSDuck's own conformance analyzer: reports continuity-counter
/// discontinuities per PID.
fn tsduck_analyze(ts_path: &Path) -> String {
    let out = Command::new("tsanalyze")
        .arg(ts_path.to_str().unwrap())
        .output()
        .expect("run tsanalyze");
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn tdt_and_tot_both_present_with_continuous_cc_per_tsduck() {
    skip_unless_tsduck_available!();

    let tdt = build_short_section(0x70, &[0xAA; 5]); // TDT: UTC_time(40 bits)
    let tot = build_short_section(0x73, &[0xBB; 8]); // TOT: UTC_time(40 bits) + garbage tail
    let mut mux = SiMux::new();
    mux.upsert_tdt(tdt);
    mux.upsert_tot(tot);
    let packets = mux.poll(StdDuration::ZERO);
    assert!(!packets.is_empty());

    let dir = scratch_dir("tdt_tot");
    let ts_path = write_ts(&dir, "tdt_tot.ts", &packets);

    let tables_out = tsduck_collect_tables(&ts_path);
    assert!(
        tables_out.contains("TID 0x70") || tables_out.contains("TDT"),
        "TSDuck did not report a TDT (table_id 0x70) section: {tables_out}"
    );
    assert!(
        tables_out.contains("TID 0x73") || tables_out.contains("TOT"),
        "TSDuck did not report a TOT (table_id 0x73) section: {tables_out}"
    );

    let analyze_out = tsduck_analyze(&ts_path);
    assert!(
        !analyze_out.to_lowercase().contains("continuity"),
        "TSDuck reported a continuity-counter issue on the shared PID 0x0014: {analyze_out}"
    );
}

#[test]
fn sdt_and_bat_both_present_per_tsduck() {
    skip_unless_tsduck_available!();

    // SDT actual: table_id_extension = transport_stream_id; body_extra =
    // original_network_id(2) + reserved_future_use(1) + garbage service loop.
    let sdt = build_long_section(0x42, 0x0001, &[0xFF, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC]);
    // BAT: table_id_extension = bouquet_id; body_extra = reserved(4 bits) +
    // bouquet_descriptors_length(12 bits, =0) + reserved(4 bits) +
    // transport_stream_loop_length(12 bits, =0) — both loops empty.
    let bat = build_long_section(0x4A, 0x0001, &[0xF0, 0x00, 0xF0, 0x00]);
    let mut mux = SiMux::new();
    mux.upsert_sdt_actual(sdt);
    mux.upsert(0x0011, bat, StdDuration::from_millis(1000));
    let packets = mux.poll(StdDuration::ZERO);
    assert!(!packets.is_empty());

    let dir = scratch_dir("sdt_bat");
    let ts_path = write_ts(&dir, "sdt_bat.ts", &packets);

    let tables_out = tsduck_collect_tables(&ts_path);
    assert!(
        tables_out.contains("TID 0x42") || tables_out.contains("SDT"),
        "TSDuck did not report an SDT actual (table_id 0x42) section: {tables_out}"
    );
    assert!(
        tables_out.contains("TID 0x4A") || tables_out.contains("BAT"),
        "TSDuck did not report a BAT (table_id 0x4A) section: {tables_out}"
    );
}
