//! Fetch-on-demand SCTE-35 capture test — exercises the T2-MI → inner TS
//! → SCTE-35 pipeline against a real Russian mux (issue #218).
//!
//! Keyed on `.test-streams/russia-t2mi-scte35.ts` (~4.9 GB). When the capture
//! is present, feeds the outer TS through `InnerTsRecovery` for PLP 0,
//! scans the inner TS for `splice_info_section` (table_id 0xFC), and asserts
//! at least one splice command parses successfully.
//!
//! The capture is the one real capture too large to mirror in the private repo,
//! so it stays fetch-on-demand (`tools/fetch-test-streams.sh
//! russia-t2mi-scte35`, resolved by `test_fixtures::find`); the test **skips
//! loudly** until fetched. This is a **scaffold** test.

use mpeg_ts::pid::Pid;
use std::fs;

use broadcast_common::Parse;
use dvb_t2mi::inner_ts::InnerTsRecovery;
use scte35_splice::SpliceInfoSection;

const CAPTURE: &str = "russia-t2mi-scte35";

/// The T2-MI PID used by the Russian capture (empirically known).
const T2MI_PID: u16 = 0x1000;

#[test]
fn russia_t2mi_scte35_parse() {
    // ~4.9 GB: too large for the private repo (see `tools/fetch-test-streams.sh`),
    // so it is the one fetch-on-demand capture; resolved through the shared
    // resolver (`.test-streams/` fallback) and skipped loudly when not fetched.
    let Some((path, _)) = test_fixtures::find(&format!("{CAPTURE}.ts")) else {
        eprintln!(
            "downloaded_scte35: SKIPPED - {CAPTURE}.ts (~4.9 GB) not fetched. \
             Run `tools/fetch-test-streams.sh {CAPTURE}` to enable."
        );
        return;
    };

    let data = fs::read(&path).expect("read capture");
    let mut rec = InnerTsRecovery::new_for_plp(Pid::new(T2MI_PID), 0);

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
            if table_id != scte35_splice::section::TABLE_ID {
                continue;
            }

            // We have the start of a splice_info_section. We need the full
            // section bytes, but the inner TS pump gives us individual packets.
            // For a scaffold test, assert that at least one start is present.
            scte35_sections += 1;

            // Attempt to parse from the packet start (may fail if the section
            // spans multiple packets — this is a best-effort scaffold).
            let section_bytes = &inner_pkt[sec_start..];
            if let Ok(section) = SpliceInfoSection::parse(section_bytes)
                && section.clear.is_some()
            {
                splice_commands += 1;
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
