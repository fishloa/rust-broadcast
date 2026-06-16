//! Download-on-demand SCTE-35 capture test — exercises the T2-MI → inner TS
//! → SCTE-35 pipeline against a real Russian mux (issue #218).
//!
//! Keyed on `.test-streams/russia-t2mi-scte35.ts` (~4.9 GB). When the capture
//! is present, feeds the outer TS through `InnerTsRecovery` for PLP 0,
//! scans the inner TS for `splice_info_section` (table_id 0xFC), and asserts
//! at least one splice command parses successfully.
//!
//! The capture is large — this is a **scaffold** test. It **skips cleanly** by
//! default (the file won't be present without an explicit download).

use std::fs;
use std::path::Path;

use dvb_common::Parse;
use dvb_scte35::SpliceInfoSection;
use dvb_t2mi::inner_ts::InnerTsRecovery;

const CAPTURE: &str = "russia-t2mi-scte35";

fn capture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".test-streams")
        .join(format!("{CAPTURE}.ts"))
}

/// The T2-MI PID used by the Russian capture (empirically known).
const T2MI_PID: u16 = 0x1000;

#[test]
fn russia_t2mi_scte35_parse() {
    let path = capture_path();
    if !path.exists() {
        eprintln!(
            "downloaded_scte35: SKIPPED — {CAPTURE}.ts not in .test-streams/. \
             Run `tools/fetch-test-streams.sh {CAPTURE}` to enable \
             (warning: ~4.9 GB download)."
        );
        return;
    }

    let data = fs::read(&path).expect("read capture");
    let mut rec = InnerTsRecovery::new_for_plp(T2MI_PID, 0);

    let mut scte35_sections = 0usize;
    let mut splice_commands = 0usize;

    for chunk in data.chunks(188) {
        if chunk.len() != 188 || chunk[0] != 0x47 {
            continue;
        }
        for inner_pkt in rec.feed(chunk) {
            // inner_pkt is [u8; 188]. Look for PUSI + section start.
            if inner_pkt.len() < 188 || inner_pkt[0] != 0x47 {
                continue;
            }
            let pusi = (inner_pkt[1] & 0x40) != 0;
            if !pusi {
                // Non-PUSI packets can contain section continuations, but
                // we only look at start-indicated packets for the table_id.
                continue;
            }
            let af_control = (inner_pkt[3] >> 4) & 0x03;
            if !(1..=3).contains(&af_control) {
                continue;
            }
            // Skip adaptation field if present.
            let af_len = if af_control & 0x02 != 0 {
                inner_pkt[4] as usize
            } else {
                0
            };
            let payload_off = 4 + (if af_control & 0x02 != 0 { 1 } else { 0 }) + af_len;
            if payload_off >= 188 {
                continue;
            }
            let ptr = inner_pkt[payload_off] as usize;
            let sec_start = payload_off + 1 + ptr;
            if sec_start >= 188 {
                continue;
            }
            let table_id = inner_pkt[sec_start];
            if table_id != dvb_scte35::section::TABLE_ID {
                continue;
            }

            // We have the start of a splice_info_section. We need the full
            // section bytes, but the inner TS pump gives us individual packets.
            // For a scaffold test, assert that at least one start is present.
            scte35_sections += 1;

            // Attempt to parse from the packet start (may fail if the section
            // spans multiple packets — this is a best-effort scaffold).
            let section_bytes = &inner_pkt[sec_start..];
            if let Ok(section) = SpliceInfoSection::parse(section_bytes) {
                if section.clear.is_some() {
                    splice_commands += 1;
                }
            }
        }
    }

    eprintln!(
        "downloaded_scte35: {CAPTURE} — {scte35_sections} SCTE-35 section starts, \
         {splice_commands} clear commands parsed, \
         {} bytes processed",
        data.len()
    );

    assert!(
        scte35_sections > 0,
        "{CAPTURE}: expected at least one SCTE-35 splice_info_section"
    );
    assert!(
        splice_commands > 0,
        "{CAPTURE}: expected at least one clear splice command"
    );
}
