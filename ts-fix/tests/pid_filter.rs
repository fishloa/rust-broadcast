//! PID filter / service extract tests for `ts-fix`.
//!
//! All tests use a hermetic synthetic 2-program MPEG-TS built from the real
//! dvb-si PAT/PMT serializers and the mpeg-ts SectionPacketizer.  The known
//! PID→program mapping is the oracle.
//!
//! Stream layout:
//!
//! ```text
//! PID 0x0000 — PAT  (program 1 → PMT 0x0100; program 2 → PMT 0x0200)
//! PID 0x0100 — PMT1 (PCR 0x0101; video 0x0101; audio 0x0102)
//! PID 0x0200 — PMT2 (PCR 0x0201; video 0x0201; audio 0x0202)
//! PID 0x0101 — program-1 video ES   (dummy payload)
//! PID 0x0102 — program-1 audio ES   (dummy payload)
//! PID 0x0201 — program-2 video ES   (dummy payload)
//! PID 0x0202 — program-2 audio ES   (dummy payload)
//! PID 0x1FFF — null packets
//! ```
//!
//! Packets are interleaved so that naive prefix/suffix cuts cannot pass the
//! "only these PIDs" assertion.

use broadcast_common::traits::{Parse, Serialize};
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{PmtSection, PmtStream, StreamType};
use mpeg_ts::mux::SectionPacketizer;
use ts_fix::{PidFilter, TsFix};

// ── PID constants ────────────────────────────────────────────────────────────

const PAT_PID: u16 = 0x0000;
const PMT1_PID: u16 = 0x0100;
const PMT2_PID: u16 = 0x0200;

const P1_PCR_PID: u16 = 0x0101;
const P1_VIDEO_PID: u16 = 0x0101;
const P1_AUDIO_PID: u16 = 0x0102;

const P2_PCR_PID: u16 = 0x0201;
const P2_VIDEO_PID: u16 = 0x0201;
const P2_AUDIO_PID: u16 = 0x0202;

const NULL_PID: u16 = 0x1FFF;

// ── Synthetic 2-program mux builder ─────────────────────────────────────────

/// Build a 188-byte TS packet carrying a dummy payload on `pid`.
///
/// Sets PUSI=false, adaptation_field_control=01 (payload only), CC=`cc`.
fn dummy_es_packet(pid: u16, cc: u8) -> [u8; 188] {
    let mut pkt = [0u8; 188];
    pkt[0] = 0x47; // sync byte
    pkt[1] = ((pid >> 8) as u8) & 0x1F; // PUSI=0, TEI=0, transport_priority=0
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10 | (cc & 0x0F); // adaptation_field_control=01 (payload only), CC
                                 // payload: fill with a recognisable pattern
    for (i, b) in pkt[4..].iter_mut().enumerate() {
        *b = (i as u8).wrapping_add(pid as u8);
    }
    pkt
}

/// Build a 188-byte null packet (PID 0x1FFF).
fn null_packet() -> [u8; 188] {
    let mut pkt = [0u8; 188];
    pkt[0] = 0x47;
    pkt[1] = 0x1F;
    pkt[2] = 0xFF;
    pkt[3] = 0x10; // payload only
    for b in &mut pkt[4..] {
        *b = 0xFF;
    }
    pkt
}

/// Serialize a `PatSection` to bytes.
fn serialize_pat(pat: &PatSection) -> Vec<u8> {
    let mut buf = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut buf).expect("PAT serialize");
    buf
}

/// Serialize a `PmtSection` to bytes.
fn serialize_pmt(pmt: &PmtSection<'_>) -> Vec<u8> {
    let mut buf = vec![0u8; pmt.serialized_len()];
    pmt.serialize_into(&mut buf).expect("PMT serialize");
    buf
}

/// Build the synthetic 2-program TS as a flat `Vec<u8>`.
///
/// The stream contains, in interleaved order per cycle:
///   PAT | PMT1 | PMT2 | P1-video | P1-audio | P2-video | P2-audio | null
///
/// We repeat the cycle `cycles` times so that every PID appears multiple
/// times and packets from different programs are interleaved.
fn build_two_program_ts(cycles: usize) -> Vec<u8> {
    // ── PAT ─────────────────────────────────────────────────────────────────
    let pat = PatSection {
        transport_stream_id: 1,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        entries: vec![
            PatEntry {
                program_number: 1,
                pid: PMT1_PID,
            },
            PatEntry {
                program_number: 2,
                pid: PMT2_PID,
            },
        ],
    };
    let pat_section_bytes = serialize_pat(&pat);
    let mut pat_pktz = SectionPacketizer::new(PAT_PID);
    let pat_pkts = pat_pktz.packetize(&[&pat_section_bytes]);

    // ── PMT1 ─────────────────────────────────────────────────────────────────
    let pmt1 = PmtSection::new(
        /* program_number */ 1,
        /* version_number */ 0,
        /* current_next_indicator */ true,
        /* section_number */ 0,
        /* last_section_number */ 0,
        /* pcr_pid */ P1_PCR_PID,
        /* program_info */ DescriptorLoop::new(&[]),
        /* streams */
        vec![
            PmtStream {
                stream_type: StreamType::Mpeg2Video,
                elementary_pid: P1_VIDEO_PID,
                es_info: DescriptorLoop::new(&[]),
            },
            PmtStream {
                stream_type: StreamType::Mpeg2Audio,
                elementary_pid: P1_AUDIO_PID,
                es_info: DescriptorLoop::new(&[]),
            },
        ],
    );
    let pmt1_section_bytes = serialize_pmt(&pmt1);
    let mut pmt1_pktz = SectionPacketizer::new(PMT1_PID);
    let pmt1_pkts = pmt1_pktz.packetize(&[&pmt1_section_bytes]);

    // ── PMT2 ─────────────────────────────────────────────────────────────────
    let pmt2 = PmtSection::new(
        /* program_number */ 2,
        /* version_number */ 0,
        /* current_next_indicator */ true,
        /* section_number */ 0,
        /* last_section_number */ 0,
        /* pcr_pid */ P2_PCR_PID,
        /* program_info */ DescriptorLoop::new(&[]),
        /* streams */
        vec![
            PmtStream {
                stream_type: StreamType::Mpeg2Video,
                elementary_pid: P2_VIDEO_PID,
                es_info: DescriptorLoop::new(&[]),
            },
            PmtStream {
                stream_type: StreamType::Mpeg2Audio,
                elementary_pid: P2_AUDIO_PID,
                es_info: DescriptorLoop::new(&[]),
            },
        ],
    );
    let pmt2_section_bytes = serialize_pmt(&pmt2);
    let mut pmt2_pktz = SectionPacketizer::new(PMT2_PID);
    let pmt2_pkts = pmt2_pktz.packetize(&[&pmt2_section_bytes]);

    // ── Assemble interleaved stream ──────────────────────────────────────────
    let mut stream: Vec<u8> = Vec::new();

    // PSI packets appear first (once — they're short enough to fit in one packet).
    for pkt in &pat_pkts {
        stream.extend_from_slice(pkt);
    }
    for pkt in &pmt1_pkts {
        stream.extend_from_slice(pkt);
    }
    for pkt in &pmt2_pkts {
        stream.extend_from_slice(pkt);
    }

    // Then interleave ES packets and nulls over `cycles` iterations.
    for i in 0..cycles {
        let cc = (i as u8) & 0x0F;
        // Interleave both programs' ES so a prefix cut fails.
        stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P2_VIDEO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P1_AUDIO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P2_AUDIO_PID, cc));
        stream.extend_from_slice(&null_packet());
    }

    stream
}

// ── Helper: extract all PIDs present in a TS byte slice ─────────────────────

fn pid_from_packet(pkt: &[u8]) -> u16 {
    (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16
}

fn all_pids_in(ts: &[u8]) -> std::collections::BTreeSet<u16> {
    ts.chunks_exact(188).map(pid_from_packet).collect()
}

fn pid_count(ts: &[u8], pid: u16) -> usize {
    ts.chunks_exact(188)
        .filter(|pkt| pid_from_packet(pkt) == pid)
        .count()
}

// ── Run one stream through the engine ───────────────────────────────────────

fn run(input: &[u8], cfg: PidFilter) -> Vec<u8> {
    let mut engine = TsFix::builder()
        .filter_pids(cfg)
        .build()
        .expect("build should not fail");

    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));
    output
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Service extract: feed a 2-program stream through `PidFilter::service(1)`.
///
/// Output must contain ONLY PIDs {0x0000, 0x0100, 0x0101, 0x0102}.
/// Program-2 PIDs {0x0200, 0x0201, 0x0202} must be entirely absent.
/// Null packets (0x1FFF) must be dropped.
#[test]
fn service_extract_program_1() {
    let ts = build_two_program_ts(8);
    let output = run(&ts, PidFilter::service(1));

    // Must be a multiple of 188 bytes.
    assert_eq!(
        output.len() % 188,
        0,
        "output must be aligned to 188-byte packets"
    );
    assert!(!output.is_empty(), "output must not be empty");

    let pids = all_pids_in(&output);

    // Program-1 PIDs must all be present.
    assert!(pids.contains(&PAT_PID), "PAT (0x0000) must be present");
    assert!(pids.contains(&PMT1_PID), "PMT1 (0x0100) must be present");
    assert!(
        pids.contains(&P1_VIDEO_PID),
        "P1 video (0x0101) must be present"
    );
    assert!(
        pids.contains(&P1_AUDIO_PID),
        "P1 audio (0x0102) must be present"
    );

    // Program-2 PIDs must be entirely absent.
    assert!(
        !pids.contains(&PMT2_PID),
        "PMT2 (0x0200) must NOT be present"
    );
    assert!(
        !pids.contains(&P2_VIDEO_PID),
        "P2 video (0x0201) must NOT be present"
    );
    assert!(
        !pids.contains(&P2_AUDIO_PID),
        "P2 audio (0x0202) must NOT be present"
    );

    // Null packets must be dropped.
    assert!(
        !pids.contains(&NULL_PID),
        "null (0x1FFF) must NOT be present"
    );

    // No unexpected PIDs.
    let expected: std::collections::BTreeSet<u16> =
        [PAT_PID, PMT1_PID, P1_VIDEO_PID, P1_AUDIO_PID].into();
    assert_eq!(pids, expected, "output must contain exactly the P1 PIDs");
}

/// Service extract: feed a 2-program stream through `PidFilter::service(2)`.
///
/// Output must contain ONLY PIDs {0x0000, 0x0200, 0x0201, 0x0202}.
#[test]
fn service_extract_program_2() {
    let ts = build_two_program_ts(8);
    let output = run(&ts, PidFilter::service(2));

    assert_eq!(output.len() % 188, 0);
    assert!(!output.is_empty());

    let pids = all_pids_in(&output);

    assert!(pids.contains(&PAT_PID));
    assert!(pids.contains(&PMT2_PID));
    assert!(pids.contains(&P2_VIDEO_PID));
    assert!(pids.contains(&P2_AUDIO_PID));

    assert!(!pids.contains(&PMT1_PID));
    assert!(!pids.contains(&P1_VIDEO_PID));
    assert!(!pids.contains(&P1_AUDIO_PID));
    assert!(!pids.contains(&NULL_PID));

    let expected: std::collections::BTreeSet<u16> =
        [PAT_PID, PMT2_PID, P2_VIDEO_PID, P2_AUDIO_PID].into();
    assert_eq!(pids, expected);
}

/// Keep-set: `PidFilter::keep([0x0101])` → output has only PID 0x0101 and PAT (0x0000).
#[test]
fn keep_set_single_pid() {
    let ts = build_two_program_ts(8);
    let output = run(&ts, PidFilter::keep([P1_VIDEO_PID]));

    assert_eq!(output.len() % 188, 0);
    assert!(!output.is_empty());

    let pids = all_pids_in(&output);

    // PAT is always implicitly included.
    assert!(pids.contains(&PAT_PID), "PAT must always be present");
    assert!(
        pids.contains(&P1_VIDEO_PID),
        "kept PID 0x0101 must be present"
    );

    // Everything else must be gone.
    assert!(!pids.contains(&PMT1_PID));
    assert!(!pids.contains(&PMT2_PID));
    assert!(!pids.contains(&P1_AUDIO_PID));
    assert!(!pids.contains(&P2_VIDEO_PID));
    assert!(!pids.contains(&P2_AUDIO_PID));
    assert!(!pids.contains(&NULL_PID));

    let expected: std::collections::BTreeSet<u16> = [PAT_PID, P1_VIDEO_PID].into();
    assert_eq!(pids, expected);
}

/// Keep-set with an empty set: only PAT must pass (PAT is always implicitly added).
#[test]
fn keep_set_empty_keeps_pat() {
    let ts = build_two_program_ts(4);
    let output = run(&ts, PidFilter::keep([]));

    let pids = all_pids_in(&output);
    let expected: std::collections::BTreeSet<u16> = [PAT_PID].into();
    assert_eq!(pids, expected, "only PAT should survive an empty keep-set");
}

/// Boundary: the stream interleaves both programs' packets.
///
/// This verifies that the filtering is per-PID (not a prefix cut): we assert
/// that in the INPUT the programs' ES packets are truly interleaved, then
/// verify the output has the correct subset.
#[test]
fn interleaved_programs_are_truly_filtered() {
    let ts = build_two_program_ts(4);

    // Verify interleaving: in the raw stream, a P2 video packet must appear
    // before all P1 audio packets are exhausted (i.e. they genuinely alternate).
    let packets: Vec<u16> = ts.chunks_exact(188).map(pid_from_packet).collect();
    let first_p2_video = packets.iter().position(|&p| p == P2_VIDEO_PID);
    let last_p1_audio = packets.iter().rposition(|&p| p == P1_AUDIO_PID);
    assert!(
        first_p2_video.is_some() && last_p1_audio.is_some(),
        "fixture must contain both programs' ES packets"
    );
    // P2 video must appear before or interspersed with P1 audio.
    assert!(
        first_p2_video.unwrap() < last_p1_audio.unwrap(),
        "packets must be interleaved (P2 video before last P1 audio)"
    );

    // After service-extract, program 2's PIDs must be absent even though
    // program 2's packets appeared before some program 1 packets.
    let output = run(&ts, PidFilter::service(1));
    assert_eq!(pid_count(&output, P2_VIDEO_PID), 0);
    assert_eq!(pid_count(&output, P2_AUDIO_PID), 0);
    assert_eq!(pid_count(&output, PMT2_PID), 0);

    // And program 1 packets must all survive.
    assert!(pid_count(&output, P1_VIDEO_PID) > 0);
    assert!(pid_count(&output, P1_AUDIO_PID) > 0);
}

/// A no-op (identity pass-through) FAILS the "only these PIDs" assertion —
/// this verifies the test would catch a broken implementation.
///
/// This test is deliberately structured to prove the test suite bites.
#[test]
fn noop_fails_pid_exclusivity() {
    let ts = build_two_program_ts(4);

    // Run through an IDENTITY engine (no filter).
    let mut engine = TsFix::builder().build().expect("identity build");
    let mut output = Vec::with_capacity(ts.len());
    for chunk in ts.chunks_exact(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .unwrap();
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    // The identity engine passes BOTH programs through, so the test for
    // program-2 PIDs being absent must FAIL with a no-op engine.
    // We verify this by checking that P2 PIDs ARE present (identity = no filter).
    let pids = all_pids_in(&output);
    assert!(
        pids.contains(&PMT2_PID) && pids.contains(&P2_VIDEO_PID),
        "identity engine must keep all PIDs — proves filter test would catch a broken impl"
    );
}

/// The output PMT for program 1 must still parse correctly after service extract.
#[test]
fn output_pmt_still_parses() {
    let ts = build_two_program_ts(4);
    let output = run(&ts, PidFilter::service(1));

    // Find the PMT1 packet in the output and attempt to parse the section.
    let pmt1_pkt = output
        .chunks_exact(188)
        .find(|pkt| pid_from_packet(pkt) == PMT1_PID)
        .expect("PMT1 packet must be in output");

    // The PMT section starts after the 4-byte header and pointer_field (0x00).
    // For a PUSI packet: byte 4 = pointer_field, section starts at byte 5.
    let pusi = (pmt1_pkt[1] & 0x40) != 0;
    assert!(pusi, "first PMT1 packet should have PUSI set");
    let pointer = pmt1_pkt[4] as usize;
    let section_start = 5 + pointer;
    let section_bytes = &pmt1_pkt[section_start..];

    let pmt = PmtSection::parse(section_bytes).expect("output PMT must parse successfully");
    assert_eq!(pmt.program_number, 1);
    assert_eq!(pmt.pcr_pid, P1_PCR_PID);
    assert_eq!(pmt.streams.len(), 2);
}
