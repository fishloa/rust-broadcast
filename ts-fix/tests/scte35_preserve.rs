//! SCTE-35 cue preservation through ts-fix operations.
//!
//! Proves that `filter_pids`, `regen_psi`, and `restamp_pcr` do not corrupt
//! SCTE-35 splice_info_sections (PID 0x01F0, table_id 0xFC).
//!
//! Fixture: `fixtures/ts/scte35-pcr.ts` — 6 packets:
//!   - indices 0,1,2,4,5 → PID 0x0100 carrying a PCR (adaptation field).
//!   - index 3          → PID 0x01F0 carrying two SCTE-35 splice_info_sections
//!     (splice_insert, event_id 100, out-of-network then in).
//!
//! See <https://github.com/fishloa/rust-broadcast/issues/387>

use broadcast_common::traits::Parse;
use scte35_splice::commands::AnyCommand;
use scte35_splice::SpliceInfoSection;
use ts_fix::{PcrRestamp, PidFilter, TsFix};

const PKT: usize = 188;
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/scte35-pcr.ts");
const PCR_PID: u16 = 0x0100;
const SCTE35_PID: u16 = 0x01F0;

fn load() -> Vec<u8> {
    std::fs::read(FIXTURE).unwrap_or_else(|e| panic!("fixture {FIXTURE}: {e}"))
}

fn pid(p: &[u8]) -> u16 {
    (((p[1] & 0x1f) as u16) << 8) | p[2] as u16
}

/// Check that a packet is on the SCTE-35 PID and is byte-identical.
fn assert_scte35_pkt_unchanged(input_pkt: &[u8], output_pkt: &[u8]) {
    assert_eq!(
        pid(output_pkt),
        SCTE35_PID,
        "output packet not on SCTE-35 PID"
    );
    assert_eq!(input_pkt, output_pkt, "SCTE-35 packet modified by ts-fix");
}

/// Parse the two splice_info_sections from the SCTE-35 packet payload.
///
/// The packet has PUSI=1, payload starts with a 1-byte pointer_field (0x00),
/// then the sections follow back-to-back (no padding after the second).
fn parse_scte35_sections(payload: &[u8]) -> Vec<SpliceInfoSection<'_>> {
    // Skip the 1-byte pointer_field (PUSI=1 → first byte is pointer).
    let data = if payload[0] == 0 {
        // pointer_field == 0: the section starts immediately.
        &payload[1..]
    } else {
        panic!("unexpected pointer_field value {:#04x}", payload[0]);
    };

    let mut sections = Vec::new();
    let mut cursor: &[u8] = data;

    // Walk until we run out of bytes or hit padding (0xFF).
    while cursor.len() >= 3 && cursor[0] != 0xFF {
        let table_id = cursor[0];
        assert_eq!(table_id, 0xFC, "expected SCTE-35 table_id 0xFC");
        let section_length = ((cursor[1] as usize & 0x0F) << 8) | cursor[2] as usize;
        let section_end = 3 + section_length;
        if section_end > cursor.len() {
            panic!(
                "section_length {section_length} exceeds remaining bytes {}",
                cursor.len()
            );
        }
        let section =
            SpliceInfoSection::parse(&cursor[..section_end]).expect("SCTE-35 section parse");
        sections.push(section);
        cursor = &cursor[section_end..];
    }
    sections
}

fn run<F: FnOnce(ts_fix::TsFixBuilder) -> ts_fix::TsFixBuilder>(input: &[u8], cfg: F) -> Vec<u8> {
    let mut fix = cfg(TsFix::builder()).build().expect("build");
    let mut out = Vec::with_capacity(input.len());
    for p in input.chunks_exact(PKT) {
        let _ = fix.push(p, |o| out.extend_from_slice(o));
    }
    fix.finish(|o| out.extend_from_slice(o));
    out
}

fn assert_valid_scte35_cues(out: &[u8]) {
    // Find the SCTE-35 packet in output.
    let scte35_payload = out
        .chunks_exact(PKT)
        .find(|p| pid(p) == SCTE35_PID)
        .map(|p| {
            // adaptation_field_control = 01 (payload only) per fixture analysis.
            &p[4..]
        })
        .expect("SCTE-35 packet must be present in output");

    let sections = parse_scte35_sections(scte35_payload);
    assert_eq!(
        sections.len(),
        2,
        "expected two SCTE-35 sections, got {}",
        sections.len()
    );

    for (i, section) in sections.iter().enumerate() {
        let clear = section
            .clear
            .as_ref()
            .unwrap_or_else(|| panic!("section[{i}] is encrypted"));

        let event_id = match &clear.command {
            AnyCommand::SpliceInsert(si) => si.splice_event_id,
            _ => panic!(
                "section[{i}] expected SpliceInsert, got {}",
                clear.command.name()
            ),
        };
        assert_eq!(event_id, 100, "section[{i}] splice_event_id mismatch");
    }

    // First section: out-of-network = true; second: false.
    let cmd0 = match &sections[0].clear.as_ref().unwrap().command {
        AnyCommand::SpliceInsert(si) => si,
        _ => panic!("section[0] not SpliceInsert"),
    };
    let cmd1 = match &sections[1].clear.as_ref().unwrap().command {
        AnyCommand::SpliceInsert(si) => si,
        _ => panic!("section[1] not SpliceInsert"),
    };
    assert!(
        cmd0.out_of_network_indicator,
        "section[0] should be out-of-network"
    );
    assert!(
        !cmd1.out_of_network_indicator,
        "section[1] should be in-network"
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Preservation (keep-mode): keep [0x0100, 0x01F0] → SCTE-35 packet byte-identical.
#[test]
fn keep_mode_preserves_scte35_cues() {
    let input = load();
    let out = run(&input, |b| {
        b.filter_pids(PidFilter::keep([PCR_PID, SCTE35_PID]))
    });

    // Find the SCTE-35 packet in input and output.
    let input_scte35 = input
        .chunks_exact(PKT)
        .find(|p| pid(p) == SCTE35_PID)
        .expect("fixture must have SCTE-35 packet");

    let output_scte35 = out
        .chunks_exact(PKT)
        .find(|p| pid(p) == SCTE35_PID)
        .expect("output must have SCTE-35 packet");

    assert_scte35_pkt_unchanged(input_scte35, output_scte35);
    assert_valid_scte35_cues(&out);
}

/// Cue PID survives even when the PCR PID is dropped: keep only [0x01F0].
#[test]
fn scte35_survives_when_pcr_pid_dropped() {
    let input = load();
    let out = run(&input, |b| b.filter_pids(PidFilter::keep([SCTE35_PID])));

    // Only PAT (implicitly added) and SCTE-35 PID should be present.
    let output_pids: std::collections::BTreeSet<u16> = out.chunks_exact(PKT).map(pid).collect();
    assert!(
        output_pids.contains(&SCTE35_PID),
        "SCTE-35 PID must survive even without PCR PID"
    );

    // Byte-identical SCTE-35 packet.
    let input_scte35 = input
        .chunks_exact(PKT)
        .find(|p| pid(p) == SCTE35_PID)
        .unwrap();
    let output_scte35 = out
        .chunks_exact(PKT)
        .find(|p| pid(p) == SCTE35_PID)
        .unwrap();
    assert_scte35_pkt_unchanged(input_scte35, output_scte35);
    assert_valid_scte35_cues(&out);
}

/// Restamp does not corrupt the cue: restamp_pcr only touches the PCR PID.
#[test]
fn restamp_pcr_does_not_corrupt_scte35() {
    let input = load();

    // Use from_bitrate to guarantee PCR values change (interpolate mode preserves
    // valid monotonic PCRs by forwarding observed values unchanged).
    let bps = 27_000_000; // arbitrary bitrate; restamp recomputes from position.
    let out = run(&input, |b| b.restamp_pcr(PcrRestamp::from_bitrate(bps)));

    // SCTE-35 packet must be byte-identical.
    let input_scte35 = input
        .chunks_exact(PKT)
        .find(|p| pid(p) == SCTE35_PID)
        .expect("fixture must have SCTE-35 packet");

    let output_scte35 = out
        .chunks_exact(PKT)
        .find(|p| pid(p) == SCTE35_PID)
        .expect("output must have SCTE-35 packet");

    assert_scte35_pkt_unchanged(input_scte35, output_scte35);
    assert_valid_scte35_cues(&out);

    // At least one PCR PID packet must differ (proving restamp was active).
    let input_pcrs: Vec<&[u8]> = input
        .chunks_exact(PKT)
        .filter(|p| pid(p) == PCR_PID)
        .collect();
    let output_pcrs: Vec<&[u8]> = out
        .chunks_exact(PKT)
        .filter(|p| pid(p) == PCR_PID)
        .collect();
    assert_eq!(
        input_pcrs.len(),
        output_pcrs.len(),
        "same number of PCR PID packets"
    );
    let any_pcr_changed = input_pcrs
        .iter()
        .zip(output_pcrs.iter())
        .any(|(a, b)| a != b);
    assert!(
        any_pcr_changed,
        "at least one PCR PID packet must differ (restamp was not active)"
    );
}

/// #417 design decision, made semantic: a PCR restamp must **not** shift the
/// SCTE-35 `splice_time.pts_time` / `pts_adjustment`. Those live on the 90 kHz
/// *presentation* (PTS) clock — the same timeline as the PES PTS, which this op
/// does not touch — so the cue must stay put relative to the (unchanged) media.
/// Parses the splice PTS from the cue before and after `restamp_pcr` and asserts
/// each is identical. (Stronger-typed complement to the byte-identical check.)
#[test]
fn restamp_pcr_preserves_splice_pts_time() {
    let input = load();
    let out = run(&input, |b| {
        b.restamp_pcr(PcrRestamp::from_bitrate(27_000_000))
    });

    // (pts_adjustment, program splice_time.pts_time) per section, in order.
    fn splice_pts(ts: &[u8]) -> Vec<(u64, Option<u64>)> {
        let payload = ts
            .chunks_exact(PKT)
            .find(|p| pid(p) == SCTE35_PID)
            .map(|p| &p[4..])
            .expect("SCTE-35 packet present");
        parse_scte35_sections(payload)
            .iter()
            .map(|s| {
                let clear = s.clear.as_ref().expect("clear cue");
                let pts = match &clear.command {
                    AnyCommand::SpliceInsert(si) => si.splice_time.and_then(|t| t.pts_time),
                    AnyCommand::TimeSignal(t) => t.splice_time.pts_time,
                    _ => None,
                };
                (s.pts_adjustment, pts)
            })
            .collect()
    }

    let before = splice_pts(&input);
    let after = splice_pts(&out);
    assert!(!before.is_empty(), "fixture must carry at least one cue");
    assert_eq!(
        before, after,
        "PCR restamp must NOT shift SCTE-35 splice PTS / pts_adjustment (#417)"
    );
}
