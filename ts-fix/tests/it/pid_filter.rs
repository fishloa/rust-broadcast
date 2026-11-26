//! PID filter / service extract tests for `ts-fix`.
//!
//! All tests use a hermetic synthetic 2-program MPEG-TS built from the real
//! dvb-si PAT/PMT serializers and the mpeg-ts SectionPacketiser.  The known
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

use crate::support::{
    NULL_PID, P1_AUDIO_PID, P1_PCR_PID, P1_VIDEO_PID, P2_AUDIO_PID, P2_PCR_PID, P2_VIDEO_PID,
    PAT_PID, PMT1_PID, PMT2_PID, build_two_program_ts, dummy_es_packet, null_packet, serialize_pat,
    serialize_pmt,
};
use broadcast_core::traits::{Parse, Serialize};
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::cat::CatSection;
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{PmtSection, PmtStream, StreamType};
use mpeg_ts::mux::SectionPacketiser;
use mpeg_ts::pid::Pid;
use ts_fix::{PidFilter, TsFix};

// ── Helper: extract all PIDs present in a TS byte slice ─────────────────────

fn pid_from_packet(pkt: &[u8]) -> Pid {
    Pid::from_wire_masked((((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16)
}

fn all_pids_in(ts: &[u8]) -> std::collections::BTreeSet<Pid> {
    ts.chunks_exact(188).map(pid_from_packet).collect()
}

fn pid_count(ts: &[u8], pid: Pid) -> usize {
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
    let expected: std::collections::BTreeSet<Pid> =
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

    let expected: std::collections::BTreeSet<Pid> =
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

    let expected: std::collections::BTreeSet<Pid> = [PAT_PID, P1_VIDEO_PID].into();
    assert_eq!(pids, expected);
}

/// Keep-set with an empty set: only PAT must pass (PAT is always implicitly added).
#[test]
fn keep_set_empty_keeps_pat() {
    let ts = build_two_program_ts(4);
    let output = run(&ts, PidFilter::keep([]));

    let pids = all_pids_in(&output);
    let expected: std::collections::BTreeSet<Pid> = [PAT_PID].into();
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
    let packets: Vec<Pid> = ts.chunks_exact(188).map(pid_from_packet).collect();
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

// ── W-TF-3 (#1101): CA PIDs + PAT/PMT version re-observation ─────────────────

/// Build a CA_descriptor (tag 0x09, ISO/IEC 13818-1 §2.6.16) for `ca_pid`.
fn ca_descriptor_bytes(ca_pid: Pid) -> Vec<u8> {
    let high = 0xE0 | ((ca_pid.value() >> 8) as u8);
    vec![0x09, 0x04, 0x0E, 0x09, high, (ca_pid.value() & 0xFF) as u8]
}

/// Build a 1-program scrambled TS:
///
/// ```text
/// PID 0x0000 — PAT  (program 1 → PMT 0x0100)
/// PID 0x0001 — CAT  (CA_descriptor → EMM PID 0x0300)
/// PID 0x0100 — PMT1 (PCR 0x0101; video 0x0101; audio 0x0102;
///                    program_info CA_descriptor → ECM PID 0x0200)
/// PID 0x0101/0x0102 — ES, PID 0x0200 — ECM, PID 0x0300 — EMM, nulls
/// ```
fn build_scrambled_ts(cycles: usize) -> Vec<u8> {
    let pat = PatSection::new(1, 0, true, 0, 0, vec![PatEntry::new(1, PMT1_PID)]);
    let pat_bytes = serialize_pat(&pat);
    let mut pat_pktz = SectionPacketiser::new(PAT_PID);
    let pat_pkts = pat_pktz.packetise(&[&pat_bytes]);

    // CAT with one CA_descriptor pointing at the EMM PID.
    let emm_ca = ca_descriptor_bytes(EMM_PID);
    let cat = CatSection::new(0, true, 0, 0, DescriptorLoop::new(&emm_ca));
    let mut cat_bytes = vec![0u8; cat.serialized_len()];
    cat.serialize_into(&mut cat_bytes).expect("CAT serialize");
    let mut cat_pktz = SectionPacketiser::new(CAT_PID);
    let cat_pkts = cat_pktz.packetise(&[&cat_bytes]);

    let ecm_ca = ca_descriptor_bytes(ECM_PID);
    let pmt1 = PmtSection::new(
        1,
        dvb_si::tables::SectionVersion::new(0, true, 0, 0),
        P1_PCR_PID,
        DescriptorLoop::new(&ecm_ca),
        vec![
            PmtStream::new(
                StreamType::Mpeg2Video,
                P1_VIDEO_PID,
                DescriptorLoop::new(&[]),
            ),
            PmtStream::new(
                StreamType::Mpeg2Audio,
                P1_AUDIO_PID,
                DescriptorLoop::new(&[]),
            ),
        ],
    );
    let pmt1_bytes = serialize_pmt(&pmt1);
    let mut pmt1_pktz = SectionPacketiser::new(PMT1_PID);
    let pmt1_pkts = pmt1_pktz.packetise(&[&pmt1_bytes]);

    let mut stream = Vec::new();
    for _ in 0..cycles {
        for p in &pat_pkts {
            stream.extend_from_slice(p);
        }
        for p in &cat_pkts {
            stream.extend_from_slice(p);
        }
        for p in &pmt1_pkts {
            stream.extend_from_slice(p);
        }
        stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(P1_AUDIO_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(ECM_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(EMM_PID, 0));
        stream.extend_from_slice(&null_packet());
    }
    stream
}

const CAT_PID: Pid = Pid::from_const(0x0001);
const ECM_PID: Pid = Pid::from_const(0x0200);
const EMM_PID: Pid = Pid::from_const(0x0300);

/// Service extract of a scrambled service must keep the CAT, the EMM PID, and
/// the ECM PID from the PMT's CA_descriptors — otherwise the output is
/// undecryptable (#1101 W-TF-3).
#[test]
fn service_extract_keeps_ca_pids() {
    let ts = build_scrambled_ts(4);
    let output = run(&ts, PidFilter::service(1));
    let pids = all_pids_in(&output);

    let expected: std::collections::BTreeSet<Pid> = [
        PAT_PID,
        CAT_PID,
        PMT1_PID,
        P1_VIDEO_PID,
        P1_AUDIO_PID,
        ECM_PID,
        EMM_PID,
    ]
    .into();
    assert_eq!(
        pids, expected,
        "scrambled service extract must keep CAT/EMM/ECM PIDs"
    );
}

/// After the service resolves, a PAT version change that moves the PMT to a
/// new PID must be re-observed; the terminal-`Resolved` state silently dropped
/// the moved PMT for the rest of the stream (#1101 W-TF-3).
#[test]
fn service_extract_re_observes_pat_version_change() {
    // Phase 1: PAT v0 maps program 1 → PMT 0x0100, resolved.
    let base = build_two_program_ts(2);
    let mut stream = base.clone();

    // Phase 2: PAT v1 moves program 1 → PMT 0x0200 (PMT2 content).
    let pat_v1 = PatSection::new(1, 1, true, 0, 0, vec![PatEntry::new(1, PMT2_PID)]);
    let pat_v1_bytes = serialize_pat(&pat_v1);
    let mut pktz = SectionPacketiser::new(PAT_PID);
    let pat_v1_pkts = pktz.packetise(&[&pat_v1_bytes]);
    let mut pmt2_pktz = SectionPacketiser::new(PMT2_PID);
    let pmt2_pkts = pmt2_pktz.packetise(&[&{
        // v1, program_number 1: this is the *moved* PMT for program 1.
        let pmt2 = PmtSection::new(
            1,
            dvb_si::tables::SectionVersion::new(1, true, 0, 0),
            P2_PCR_PID,
            DescriptorLoop::new(&[]),
            vec![
                PmtStream::new(
                    StreamType::Mpeg2Video,
                    P2_VIDEO_PID,
                    DescriptorLoop::new(&[]),
                ),
                PmtStream::new(
                    StreamType::Mpeg2Audio,
                    P2_AUDIO_PID,
                    DescriptorLoop::new(&[]),
                ),
            ],
        );
        serialize_pmt(&pmt2)
    }]);
    for _ in 0..2 {
        for p in &pat_v1_pkts {
            stream.extend_from_slice(p);
        }
        for p in &pmt2_pkts {
            stream.extend_from_slice(p);
        }
        stream.extend_from_slice(&dummy_es_packet(P2_VIDEO_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(P2_AUDIO_PID, 0));
    }

    let output = run(&stream, PidFilter::service(1));
    let pids = all_pids_in(&output);

    assert!(
        pids.contains(&PMT2_PID),
        "moved PMT PID 0x0200 must be kept after PAT version change, got {pids:?}"
    );
    assert!(
        pids.contains(&P2_AUDIO_PID),
        "new ES PID 0x0202 must be kept after PAT version change, got {pids:?}"
    );
    // Packets already kept in phase 1 necessarily carry the old PMT PID; the
    // re-observation is proven by phase 2's packets (moved PMT + new ES PIDs).
    let phase2 = &output[output.len() / 2..];
    let phase2_pids = all_pids_in(phase2);
    assert!(
        !phase2_pids.contains(&PMT1_PID),
        "old PMT PID 0x0100 must be dropped after the PAT moved the programme, got {phase2_pids:?}"
    );
}

#[test]
fn service_extract_ignores_a_next_pat_and_a_next_pmt() {
    let next_pat = PatSection::new(1, 3, false, 0, 0, vec![PatEntry::new(1, PMT2_PID)]);
    let mut stream = Vec::new();
    for p in SectionPacketiser::new(PAT_PID).packetise(&[&serialize_pat(&next_pat)]) {
        stream.extend_from_slice(&p);
    }
    let next_pmt = PmtSection::new(
        1,
        dvb_si::tables::SectionVersion::new(3, false, 0, 0),
        P2_PCR_PID,
        DescriptorLoop::new(&[]),
        vec![PmtStream::new(
            StreamType::Mpeg2Video,
            P2_VIDEO_PID,
            DescriptorLoop::new(&[]),
        )],
    );
    for p in SectionPacketiser::new(PMT1_PID).packetise(&[&serialize_pmt(&next_pmt)]) {
        stream.extend_from_slice(&p);
    }
    stream.extend_from_slice(&build_two_program_ts(2));

    let output = run(&stream, PidFilter::service(1));
    let pids = all_pids_in(&output);
    assert!(
        pids.contains(&P1_VIDEO_PID) && pids.contains(&PMT1_PID),
        "the current PAT/PMT must define the service, got {pids:?}"
    );
    assert!(
        !pids.contains(&PMT2_PID) && !pids.contains(&P2_VIDEO_PID),
        "a table with current_next_indicator=0 must not be adopted, got {pids:?}"
    );
}

/// Index (in 188-byte packets) of the first output packet byte-identical to
/// `marker`.
fn position_of_packet(output: &[u8], marker: &[u8; 188]) -> Option<usize> {
    output.chunks_exact(188).position(|pkt| pkt == marker)
}

/// Build one loop of a clear (no CA_descriptor) single-program service with a
/// CAT that advertises `emm_pid`.
fn clear_service_with_cat_cycle(
    stream: &mut Vec<u8>,
    pat_pkts: &[[u8; 188]],
    pmt_pkts: &[[u8; 188]],
    cat_pkts: &[[u8; 188]],
    emm_pid: Pid,
) {
    for p in pat_pkts
        .iter()
        .chain(cat_pkts.iter())
        .chain(pmt_pkts.iter())
    {
        stream.extend_from_slice(p);
    }
    stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, 0));
    stream.extend_from_slice(&dummy_es_packet(P1_AUDIO_PID, 0));
    stream.extend_from_slice(&dummy_es_packet(emm_pid, 0));
    stream.extend_from_slice(&null_packet());
}

/// A service that signals no CA_descriptor must not carry the CAT, nor any
/// EMM PID the CAT advertises — the CAT is only part of the keep-set when the
/// service itself signals CA (audit a23cp #3455).
#[test]
fn service_extract_drops_the_cat_and_emm_pids_for_a_clear_service() {
    let pat = PatSection::new(1, 0, true, 0, 0, vec![PatEntry::new(1, PMT1_PID)]);
    let pat_pkts = SectionPacketiser::new(PAT_PID).packetise(&[&serialize_pat(&pat)]);
    let pmt = PmtSection::new(
        1,
        dvb_si::tables::SectionVersion::new(0, true, 0, 0),
        P1_PCR_PID,
        DescriptorLoop::new(&[]),
        vec![
            PmtStream::new(
                StreamType::Mpeg2Video,
                P1_VIDEO_PID,
                DescriptorLoop::new(&[]),
            ),
            PmtStream::new(
                StreamType::Mpeg2Audio,
                P1_AUDIO_PID,
                DescriptorLoop::new(&[]),
            ),
        ],
    );
    let pmt_pkts = SectionPacketiser::new(PMT1_PID).packetise(&[&serialize_pmt(&pmt)]);
    let emm_ca = ca_descriptor_bytes(EMM_PID);
    let cat = CatSection::new(0, true, 0, 0, DescriptorLoop::new(&emm_ca));
    let mut cat_bytes = vec![0u8; cat.serialized_len()];
    cat.serialize_into(&mut cat_bytes).expect("CAT serialize");
    let cat_pkts = SectionPacketiser::new(CAT_PID).packetise(&[&cat_bytes]);

    let mut stream = Vec::new();
    for _ in 0..3 {
        clear_service_with_cat_cycle(&mut stream, &pat_pkts, &pmt_pkts, &cat_pkts, EMM_PID);
    }

    let output = run(&stream, PidFilter::service(1));
    let pids = all_pids_in(&output);
    assert!(
        pids.contains(&PMT1_PID) && pids.contains(&P1_VIDEO_PID),
        "the service itself is kept, got {pids:?}"
    );
    assert!(
        !pids.contains(&CAT_PID),
        "a clear service keeps no CAT, got {pids:?}"
    );
    assert!(
        !pids.contains(&EMM_PID),
        "a clear service must not learn EMM PIDs from a CAT, got {pids:?}"
    );
}

/// The other side of the same condition: a CA-signalling PMT keeps the CAT
/// even when the CAT itself advertises no EMM PID (audit a23cp #3455).
#[test]
fn service_extract_keeps_the_cat_for_a_ca_service_with_an_empty_cat() {
    let pat = PatSection::new(1, 0, true, 0, 0, vec![PatEntry::new(1, PMT1_PID)]);
    let pat_pkts = SectionPacketiser::new(PAT_PID).packetise(&[&serialize_pat(&pat)]);
    let ecm_ca = ca_descriptor_bytes(ECM_PID);
    let pmt = PmtSection::new(
        1,
        dvb_si::tables::SectionVersion::new(0, true, 0, 0),
        P1_PCR_PID,
        DescriptorLoop::new(&ecm_ca),
        vec![PmtStream::new(
            StreamType::Mpeg2Video,
            P1_VIDEO_PID,
            DescriptorLoop::new(&[]),
        )],
    );
    let pmt_pkts = SectionPacketiser::new(PMT1_PID).packetise(&[&serialize_pmt(&pmt)]);
    let cat = CatSection::new(0, true, 0, 0, DescriptorLoop::new(&[]));
    let mut cat_bytes = vec![0u8; cat.serialized_len()];
    cat.serialize_into(&mut cat_bytes).expect("CAT serialize");
    let cat_pkts = SectionPacketiser::new(CAT_PID).packetise(&[&cat_bytes]);

    let mut stream = Vec::new();
    for _ in 0..3 {
        for p in pat_pkts
            .iter()
            .chain(cat_pkts.iter())
            .chain(pmt_pkts.iter())
        {
            stream.extend_from_slice(p);
        }
        stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(ECM_PID, 0));
        stream.extend_from_slice(&null_packet());
    }

    let output = run(&stream, PidFilter::service(1));
    let pids = all_pids_in(&output);
    assert!(
        pids.contains(&CAT_PID),
        "a CA service needs the CAT, got {pids:?}"
    );
    assert!(
        pids.contains(&ECM_PID),
        "the PMT's ECM PID is kept, got {pids:?}"
    );
}

/// Before the PAT has been parsed, only PAT packets may pass: a leading ES
/// packet must not lead the extracted service (audit a23cp #3456).
#[test]
fn service_extract_passes_only_the_pat_before_the_pat_is_seen() {
    let mut stream = Vec::new();
    stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, 0));
    stream.extend_from_slice(&dummy_es_packet(P2_VIDEO_PID, 0));
    stream.extend_from_slice(&build_two_program_ts(2));

    let output = run(&stream, PidFilter::service(1));
    let first = output
        .chunks_exact(188)
        .next()
        .expect("output is not empty");
    assert_eq!(
        pid_from_packet(first),
        PAT_PID,
        "the PAT must lead the extracted service"
    );
    assert_eq!(
        pid_count(&output, P2_VIDEO_PID),
        0,
        "the leading program-2 packet must not pass"
    );
    assert_eq!(
        pid_count(&output, P1_VIDEO_PID),
        2,
        "only the packets after the PMT resolved the service pass"
    );
}

/// A PMT version change must re-resolve the keep-set from the new PMT while
/// preserving the EMM PIDs already learned from the CAT: dropping them breaks
/// descrambling mid-stream, and keeping the replaced ES PIDs leaks another
/// elementary stream into the extracted service (audit a23cp #3457).
#[test]
fn service_extract_keeps_emm_pids_and_drops_replaced_es_on_a_pmt_version_change() {
    const NEW_AUDIO_PID: Pid = Pid::from_const(0x0103);

    let pat = PatSection::new(1, 0, true, 0, 0, vec![PatEntry::new(1, PMT1_PID)]);
    let pat_pkts = SectionPacketiser::new(PAT_PID).packetise(&[&serialize_pat(&pat)]);

    let ecm_ca = ca_descriptor_bytes(ECM_PID);
    let emm_ca = ca_descriptor_bytes(EMM_PID);
    let cat = CatSection::new(0, true, 0, 0, DescriptorLoop::new(&emm_ca));
    let mut cat_bytes = vec![0u8; cat.serialized_len()];
    cat.serialize_into(&mut cat_bytes).expect("CAT serialize");
    let cat_pkts = SectionPacketiser::new(CAT_PID).packetise(&[&cat_bytes]);

    let pmt_v0 = PmtSection::new(
        1,
        dvb_si::tables::SectionVersion::new(0, true, 0, 0),
        P1_PCR_PID,
        DescriptorLoop::new(&ecm_ca),
        vec![
            PmtStream::new(
                StreamType::Mpeg2Video,
                P1_VIDEO_PID,
                DescriptorLoop::new(&[]),
            ),
            PmtStream::new(
                StreamType::Mpeg2Audio,
                P1_AUDIO_PID,
                DescriptorLoop::new(&[]),
            ),
        ],
    );
    let pmt_v0_pkts = SectionPacketiser::new(PMT1_PID).packetise(&[&serialize_pmt(&pmt_v0)]);
    let pmt_v1 = PmtSection::new(
        1,
        dvb_si::tables::SectionVersion::new(1, true, 0, 0),
        P1_PCR_PID,
        DescriptorLoop::new(&ecm_ca),
        vec![
            PmtStream::new(
                StreamType::Mpeg2Video,
                P1_VIDEO_PID,
                DescriptorLoop::new(&[]),
            ),
            PmtStream::new(
                StreamType::Mpeg2Audio,
                NEW_AUDIO_PID,
                DescriptorLoop::new(&[]),
            ),
        ],
    );
    let pmt_v1_pkts = SectionPacketiser::new(PMT1_PID).packetise(&[&serialize_pmt(&pmt_v1)]);

    let mut stream = Vec::new();
    for _ in 0..2 {
        for p in pat_pkts
            .iter()
            .chain(cat_pkts.iter())
            .chain(pmt_v0_pkts.iter())
        {
            stream.extend_from_slice(p);
        }
        stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(P1_AUDIO_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(ECM_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(EMM_PID, 0));
        stream.extend_from_slice(&null_packet());
    }
    for _ in 0..2 {
        for p in pat_pkts
            .iter()
            .chain(cat_pkts.iter())
            .chain(pmt_v1_pkts.iter())
        {
            stream.extend_from_slice(p);
        }
        stream.extend_from_slice(&dummy_es_packet(EMM_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(P1_AUDIO_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(NEW_AUDIO_PID, 0));
        stream.extend_from_slice(&null_packet());
    }

    let output = run(&stream, PidFilter::service(1));
    let v1_at =
        position_of_packet(&output, &pmt_v1_pkts[0]).expect("the version-1 PMT passes the filter");
    let after = &output[v1_at * 188..];
    let pids = all_pids_in(after);
    assert!(
        pids.contains(&PAT_PID) && pids.contains(&CAT_PID) && pids.contains(&PMT1_PID),
        "PAT, CAT and PMT stay in the keep-set, got {pids:?}"
    );
    assert!(
        pids.contains(&EMM_PID),
        "the CAT-learned EMM PID must survive the PMT version change, got {pids:?}"
    );
    assert!(
        pids.contains(&NEW_AUDIO_PID),
        "the new ES PID must be kept, got {pids:?}"
    );
    assert!(
        !pids.contains(&P1_AUDIO_PID),
        "the replaced ES PID must be dropped, got {pids:?}"
    );
}

/// A CAT of a NEW version must be parsed and its EMM PIDs added, while a
/// repeat of the same version is ignored (audit a23cp #3458).
#[test]
fn service_extract_learns_emm_pids_from_a_changed_cat_version() {
    const EMM2_PID: Pid = Pid::from_const(0x0301);

    let pat = PatSection::new(1, 0, true, 0, 0, vec![PatEntry::new(1, PMT1_PID)]);
    let pat_pkts = SectionPacketiser::new(PAT_PID).packetise(&[&serialize_pat(&pat)]);
    let ecm_ca = ca_descriptor_bytes(ECM_PID);
    let pmt = PmtSection::new(
        1,
        dvb_si::tables::SectionVersion::new(0, true, 0, 0),
        P1_PCR_PID,
        DescriptorLoop::new(&ecm_ca),
        vec![PmtStream::new(
            StreamType::Mpeg2Video,
            P1_VIDEO_PID,
            DescriptorLoop::new(&[]),
        )],
    );
    let pmt_pkts = SectionPacketiser::new(PMT1_PID).packetise(&[&serialize_pmt(&pmt)]);

    let emm_ca = ca_descriptor_bytes(EMM_PID);
    let cat_v0 = CatSection::new(0, true, 0, 0, DescriptorLoop::new(&emm_ca));
    let mut cat_v0_bytes = vec![0u8; cat_v0.serialized_len()];
    cat_v0
        .serialize_into(&mut cat_v0_bytes)
        .expect("CAT serialize");
    let cat_v0_pkts = SectionPacketiser::new(CAT_PID).packetise(&[&cat_v0_bytes]);

    let mut two_emms = ca_descriptor_bytes(EMM_PID);
    two_emms.extend_from_slice(&ca_descriptor_bytes(EMM2_PID));
    let cat_v1 = CatSection::new(1, true, 0, 0, DescriptorLoop::new(&two_emms));
    let mut cat_v1_bytes = vec![0u8; cat_v1.serialized_len()];
    cat_v1
        .serialize_into(&mut cat_v1_bytes)
        .expect("CAT serialize");
    let cat_v1_pkts = SectionPacketiser::new(CAT_PID).packetise(&[&cat_v1_bytes]);

    let mut stream = Vec::new();
    for _ in 0..2 {
        for p in pat_pkts
            .iter()
            .chain(pmt_pkts.iter())
            .chain(cat_v0_pkts.iter())
        {
            stream.extend_from_slice(p);
        }
        stream.extend_from_slice(&dummy_es_packet(EMM_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(EMM2_PID, 0));
        stream.extend_from_slice(&null_packet());
    }
    for _ in 0..2 {
        for p in pat_pkts
            .iter()
            .chain(pmt_pkts.iter())
            .chain(cat_v1_pkts.iter())
        {
            stream.extend_from_slice(p);
        }
        stream.extend_from_slice(&dummy_es_packet(EMM_PID, 0));
        stream.extend_from_slice(&dummy_es_packet(EMM2_PID, 0));
        stream.extend_from_slice(&null_packet());
    }

    let output = run(&stream, PidFilter::service(1));
    let v1_at =
        position_of_packet(&output, &cat_v1_pkts[0]).expect("the version-1 CAT passes the filter");
    let before = &output[..v1_at * 188];
    let after = &output[v1_at * 188..];
    assert_eq!(
        pid_count(before, EMM2_PID),
        0,
        "an EMM PID only listed by CAT v1 must not be kept before it"
    );
    assert!(
        pid_count(after, EMM2_PID) >= 2,
        "CAT v1's new EMM PID must be kept (and a repeated CAT v1 changes nothing)"
    );
    assert!(
        pid_count(after, EMM_PID) >= 2,
        "the already-learned EMM PID stays kept"
    );
}
