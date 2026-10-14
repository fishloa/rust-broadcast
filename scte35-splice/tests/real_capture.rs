//! Real SCTE 35 captures: `splice_info_section`s extracted from transport
//! streams (not authored by this crate), parsed, checked against an
//! INDEPENDENT decode, and re-serialized byte-exactly.
//!
//! The independent oracle is a Python reading of the SCTE 35 2023r1 section /
//! `splice_insert()` syntax tables (`private/specs-md/ansi_scte_35_2023r1*`,
//! clauses 9.6.1 and 9.7.3), written separately from this crate; the expected
//! values below are its output for each section. Sections are found by
//! reassembling PSI on every PID and keeping those that start with
//! `table_id` 0xFC and carry a valid CRC_32.
//!
//! * Committed fixtures (`fixtures/ts/scte35-*.ts`, `fixtures/scte35-ssai`)
//!   always run.
//! * The TSDuck BSD-2 test corpus under `.test-streams/tsduck-test/` is
//!   LOCAL-ONLY (never committed); those cases skip loudly when the archive is
//!   absent. Its `test-091.ts` carries `splice_command_length = 0xFFF`, which
//!   SCTE 35 9.6.1 says receivers shall ignore (parse the command by its own
//!   structure) - the crate used to reject such sections outright.

use std::path::{Path, PathBuf};

use broadcast_common::{Parse, Serialize};
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::{AnyCommand, SpliceInsert};
use scte35_splice::descriptors::AnySpliceDescriptor;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
}

/// Every complete `table_id == 0xFC` PSI section found on any PID.
fn scte35_sections(ts: &[u8]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut open: std::collections::HashMap<u16, Vec<u8>> = std::collections::HashMap::new();
    for pkt in ts.chunks_exact(188) {
        if pkt[0] != 0x47 {
            continue;
        }
        let pid = (u16::from(pkt[1] & 0x1F) << 8) | u16::from(pkt[2]);
        let pusi = pkt[1] & 0x40 != 0;
        let afc = (pkt[3] >> 4) & 3;
        let mut off = 4;
        if afc == 3 {
            off += 1 + usize::from(pkt[4]);
        }
        if afc & 1 == 0 || off >= 188 {
            continue;
        }
        let pl = &pkt[off..];
        if pusi {
            let start = 1 + usize::from(pl[0]);
            if start < pl.len() && pl[start] == 0xFC {
                open.insert(pid, pl[start..].to_vec());
            } else {
                open.remove(&pid);
            }
        } else if let Some(b) = open.get_mut(&pid) {
            b.extend_from_slice(pl);
        }
        if let Some(b) = open.get(&pid)
            && b.len() >= 3
        {
            let sl = (usize::from(b[1] & 0x0F) << 8) | usize::from(b[2]);
            if b.len() >= sl + 3 {
                let mut done = open.remove(&pid).unwrap();
                done.truncate(sl + 3);
                if broadcast_common::crc32_mpeg2::compute(&done) == 0 && !out.contains(&done) {
                    out.push(done);
                }
            }
        }
    }
    out
}

fn insert(s: &SpliceInfoSection<'_>) -> SpliceInsert {
    match &s.clear.as_ref().expect("clear section").command {
        AnyCommand::SpliceInsert(si) => si.clone(),
        other => panic!("expected splice_insert, got {other:?}"),
    }
}

/// parse -> typed checks -> byte-exact re-serialize.
fn check_round_trip(bytes: &[u8]) -> SpliceInfoSection<'_> {
    let s = SpliceInfoSection::parse(bytes).expect("real section must parse");
    assert_eq!(
        s.try_to_bytes().unwrap(),
        bytes,
        "re-serialize must reproduce the captured section byte-for-byte"
    );
    s
}

fn first_insert_with_event(sections: &[Vec<u8>], event: u32) -> &Vec<u8> {
    sections
        .iter()
        .find(|b| {
            SpliceInfoSection::parse(b).ok().is_some_and(|s| {
                s.clear.as_ref().is_some_and(
                    |c| matches!(&c.command, AnyCommand::SpliceInsert(si) if si.splice_event_id == event),
                )
            })
        })
        .unwrap_or_else(|| panic!("no splice_insert with event id {event}"))
}

#[test]
fn committed_scte35_real_ts() {
    let ts = std::fs::read(repo("fixtures/ts/scte35-real.ts")).unwrap();
    let secs = scte35_sections(&ts);
    assert_eq!(secs.len(), 1);
    let s = check_round_trip(&secs[0]);
    assert_eq!(s.pts_adjustment, 0);
    assert!(s.tier_is_ignored());
    let si = insert(&s);
    assert_eq!(si.splice_event_id, 1_207_959_695);
    assert!(si.out_of_network_indicator && si.program_splice_flag && !si.splice_immediate_flag);
    assert_eq!(si.splice_time.unwrap().pts_time, Some(1_936_310_318));
    let bd = si.break_duration.expect("duration_flag set");
    assert!(bd.auto_return);
    assert_eq!(bd.duration, 5_426_421);
    assert_eq!(
        (si.unique_program_id, si.avail_num, si.avails_expected),
        (0, 0, 0)
    );
    let descs: Vec<_> = s.descriptors().collect::<Result<_, _>>().unwrap();
    assert_eq!(descs.len(), 1);
    assert!(matches!(descs[0], AnySpliceDescriptor::Avail(_)));
}

#[test]
fn committed_scte35_other_pid_ts() {
    let ts = std::fs::read(repo("fixtures/ts/scte35-other-pid.ts")).unwrap();
    let secs = scte35_sections(&ts);
    assert_eq!(secs.len(), 1);
    let s = check_round_trip(&secs[0]);
    let si = insert(&s);
    assert_eq!(si.splice_event_id, 777);
    assert!(si.splice_immediate_flag && si.out_of_network_indicator);
    assert!(si.splice_time.is_none() && si.break_duration.is_none());
    assert_eq!(
        (si.unique_program_id, si.avail_num, si.avails_expected),
        (1, 0, 0)
    );
}

#[test]
fn committed_ssai_splice_insert_with_pts_adjustment() {
    let ts = std::fs::read(repo(
        "fixtures/scte35-ssai/ts/video_with_scte35_splice_insert.ts",
    ))
    .unwrap();
    let secs = scte35_sections(&ts);
    assert_eq!(secs.len(), 1);
    let s = check_round_trip(&secs[0]);
    assert_eq!(s.pts_adjustment, 1_899_923_872);
    let si = insert(&s);
    assert_eq!(si.splice_event_id, 1_786_303_510);
    assert_eq!(si.splice_time.unwrap().pts_time, Some(6_690_010_720));
    let bd = si.break_duration.unwrap();
    assert!(bd.auto_return);
    assert_eq!(bd.duration, 1_800_000);
}

/// LOCAL-ONLY: the TSDuck test corpus. `None` (after a loud skip) when absent.
fn tsduck(name: &str) -> Option<Vec<u8>> {
    let p = repo(&format!(".test-streams/tsduck-test/{name}"));
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!(
                "real_capture: SKIPPED - {p:?} not present (local-only archive); \
                 this is a no-op result on this host, not coverage"
            );
            None
        }
    }
}

/// Real section with the deprecated `splice_command_length = 0xFFF`: SCTE 35
/// 9.6.1 says to ignore the value, so it must parse (the command is
/// self-delimiting) and re-serialize byte-exactly (the sentinel is preserved).
#[test]
fn local_archive_splice_command_length_0xfff_sentinel() {
    let Some(ts) = tsduck("input/test-091.ts") else {
        return;
    };
    let secs = scte35_sections(&ts);
    assert_eq!(secs.len(), 1);
    // The sentinel really is on the wire in this capture.
    assert_eq!(
        (usize::from(secs[0][11] & 0x0F) << 8) | usize::from(secs[0][12]),
        0xFFF
    );
    let s = check_round_trip(&secs[0]);
    assert_eq!(s.pts_adjustment, 880_882_211);
    let si = insert(&s);
    assert_eq!(si.splice_event_id, 1_644_174_462);
    assert!(si.out_of_network_indicator);
    assert_eq!(si.splice_time.unwrap().pts_time, Some(7_965_436_329));
    let bd = si.break_duration.unwrap();
    assert!(bd.auto_return);
    assert_eq!(bd.duration, 5_400_000);
}

#[test]
fn local_archive_avail_descriptor_inserts() {
    let Some(ts) = tsduck("input/test-084.ts") else {
        return;
    };
    let secs = scte35_sections(&ts);
    assert_eq!(secs.len(), 2);
    // Event 18, no break_duration, PTS 1 982 351 994, avail 18/255.
    let b = first_insert_with_event(&secs, 18);
    for bytes in &secs {
        let s = check_round_trip(bytes);
        let si = insert(&s);
        assert_eq!(si.splice_event_id, 18);
        assert_eq!(
            (si.unique_program_id, si.avail_num, si.avails_expected),
            (1, 18, 255)
        );
        let descs: Vec<_> = s.descriptors().collect::<Result<_, _>>().unwrap();
        assert!(matches!(descs.as_slice(), [AnySpliceDescriptor::Avail(_)]));
        match si.break_duration {
            None => {
                assert!(!si.out_of_network_indicator);
                assert_eq!(si.splice_time.unwrap().pts_time, Some(1_982_351_994));
            }
            Some(bd) => {
                assert!(si.out_of_network_indicator);
                assert_eq!(si.splice_time.unwrap().pts_time, Some(1_975_601_994));
                assert!(!bd.auto_return);
                assert_eq!(bd.duration, 6_750_000);
            }
        }
    }
    let _ = b;
}

#[test]
fn local_archive_null_and_insert_mix() {
    let Some(ts) = tsduck("input/test-149.ts") else {
        return;
    };
    let secs = scte35_sections(&ts);
    assert_eq!(secs.len(), 3);
    let mut nulls = 0;
    let mut inserts = 0;
    for bytes in &secs {
        let s = check_round_trip(bytes);
        match &s.clear.as_ref().unwrap().command {
            AnyCommand::SpliceNull(_) => nulls += 1,
            AnyCommand::SpliceInsert(si) => {
                inserts += 1;
                assert_eq!(si.splice_event_id, 1);
                match si.break_duration {
                    None => assert_eq!(si.splice_time.unwrap().pts_time, Some(4_897_220_136)),
                    Some(bd) => {
                        assert_eq!(si.splice_time.unwrap().pts_time, Some(4_894_520_136));
                        assert_eq!(bd.duration, 2_700_000);
                        assert_eq!((si.avail_num, si.avails_expected), (1, 1));
                    }
                }
            }
            other => panic!("unexpected command {other:?}"),
        }
    }
    assert_eq!((nulls, inserts), (1, 2));
}
