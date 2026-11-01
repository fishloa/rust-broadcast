//! PSI regeneration tests for `ts-fix`.
//!
//! Tests verify that PAT regeneration works correctly after PID filtering and
//! corruption scenarios.  All tests use either a real capture (m6-single.ts)
//! with fault injection, or the synthetic 2-program mux from pid_filter tests.

use broadcast_common::traits::{Parse, Serialize};
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{self, PmtSection, PmtStream, StreamType};
use mpeg_ts::mux::SectionPacketiser;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::{TsHeader, extract_ts_payload};
use std::fs;
use ts_fix::{PidFilter, Stuffing, TsFix};

// ── PID constants ────────────────────────────────────────────────────────────

const PAT_PID: Pid = Pid::new(0x0000);
/// PMT table_id (ISO/IEC 13818-1 Table 2-30).
const PMT_TABLE_ID: u8 = pmt::TABLE_ID;
const PMT1_PID: Pid = Pid::new(0x0100);
const PMT2_PID: Pid = Pid::new(0x0200);
/// Null-packet PID (ISO/IEC 13818-1 §2.4.1).
const NULL_PID: Pid = Pid::new(0x1FFF);

const P1_PCR_PID: Pid = Pid::new(0x0101);
const P1_VIDEO_PID: Pid = Pid::new(0x0101);
const P1_AUDIO_PID: Pid = Pid::new(0x0102);

const P2_PCR_PID: Pid = Pid::new(0x0201);
const P2_VIDEO_PID: Pid = Pid::new(0x0201);
const P2_AUDIO_PID: Pid = Pid::new(0x0202);

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build a 188-byte TS packet carrying a dummy payload on `pid`.
fn dummy_es_packet(pid: Pid, cc: u8) -> [u8; 188] {
    let mut pkt = [0u8; 188];
    pkt[0] = 0x47; // sync byte
    pkt[1] = ((pid.value() >> 8) as u8) & 0x1F;
    pkt[2] = (pid.value() & 0xFF) as u8;
    pkt[3] = 0x10 | (cc & 0x0F); // adaptation_field_control=01 (payload only), CC
    for (i, b) in pkt[4..].iter_mut().enumerate() {
        *b = (i as u8).wrapping_add(pid.value() as u8);
    }
    pkt
}

/// Build a 188-byte null packet (PID 0x1FFF).
fn null_packet() -> [u8; 188] {
    let mut pkt = [0u8; 188];
    pkt[0] = 0x47;
    pkt[1] = 0x1F;
    pkt[2] = 0xFF;
    pkt[3] = 0x10;
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
fn build_two_program_ts(cycles: usize) -> Vec<u8> {
    // ── PAT ──────────────────────────────────────────────────────────────────
    let pat = PatSection::new(
        1,
        0,
        true,
        0,
        0,
        vec![PatEntry::new(1, PMT1_PID), PatEntry::new(2, PMT2_PID)],
    );
    let pat_section_bytes = serialize_pat(&pat);
    let mut pat_pktz = SectionPacketiser::new(PAT_PID);
    let pat_pkts = pat_pktz.packetise(&[&pat_section_bytes]);

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
    let pmt1_section_bytes = serialize_pmt(&pmt1);
    let mut pmt1_pktz = SectionPacketiser::new(PMT1_PID);
    let pmt1_pkts = pmt1_pktz.packetise(&[&pmt1_section_bytes]);

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
    let pmt2_section_bytes = serialize_pmt(&pmt2);
    let mut pmt2_pktz = SectionPacketiser::new(PMT2_PID);
    let pmt2_pkts = pmt2_pktz.packetise(&[&pmt2_section_bytes]);

    // ── Assemble interleaved stream ──────────────────────────────────────────
    let mut stream: Vec<u8> = Vec::new();

    // PSI packets appear first.
    for pkt in &pat_pkts {
        stream.extend_from_slice(pkt);
    }
    for pkt in &pmt1_pkts {
        stream.extend_from_slice(pkt);
    }
    for pkt in &pmt2_pkts {
        stream.extend_from_slice(pkt);
    }

    // Then interleave ES packets over `cycles` iterations.  A real mux cycles
    // the PAT (and PMTs) throughout the stream, not just once at the top, so we
    // re-emit a PAT and the PMTs each cycle.  This guarantees PAT slots appear
    // AFTER the PMTs (the realistic case the in-position regen relies on).
    for i in 0..cycles {
        let cc = (i as u8) & 0x0F;
        for pkt in &pat_pkts {
            stream.extend_from_slice(pkt);
        }
        for pkt in &pmt1_pkts {
            stream.extend_from_slice(pkt);
        }
        for pkt in &pmt2_pkts {
            stream.extend_from_slice(pkt);
        }
        stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P2_VIDEO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P1_AUDIO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P2_AUDIO_PID, cc));
        stream.extend_from_slice(&null_packet());
    }

    stream
}

/// Extract the PID from a TS packet header.
fn pid_from_packet(pkt: &[u8]) -> Pid {
    Pid::new((((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16)
}

/// Find a PAT packet in a TS (first packet on PID 0x0000).
fn find_pat_packet(ts: &[u8]) -> Option<&[u8]> {
    ts.chunks_exact(188)
        .find(|pkt| pid_from_packet(pkt) == PAT_PID)
}

/// Locate the PAT section bytes inside a PAT packet, including its trailing CRC.
///
/// Assumes PUSI is set (single section, no continuation). The returned slice
/// runs from `table_id` through the 4-byte CRC inclusive.
fn pat_section_with_crc(pkt: &[u8]) -> Result<&[u8], String> {
    // PUSI flag is in byte 1, bit 6.
    let pusi = (pkt[1] & 0x40) != 0;
    if !pusi {
        return Err("PAT packet must have PUSI set".to_string());
    }

    // Section starts at byte 5 (after 4-byte header + pointer_field at byte 4).
    let pointer = pkt[4] as usize;
    let section_start = 5 + pointer;
    if section_start >= pkt.len() {
        return Err("pointer_field out of range".to_string());
    }
    let section = &pkt[section_start..];

    // section_length is the low 12 bits of bytes [1..3]; total = 3 + section_length.
    if section.len() < 3 {
        return Err("section too short for header".to_string());
    }
    let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
    let total = 3 + section_length;
    if total > section.len() {
        return Err("declared section_length exceeds packet payload".to_string());
    }
    Ok(&section[..total])
}

/// Extract and parse a PAT section from a PAT packet.
fn parse_pat_from_packet(pkt: &[u8]) -> Result<PatSection, String> {
    let section = pat_section_with_crc(pkt)?;
    PatSection::parse(section).map_err(|e| format!("failed to parse PAT: {e:?}"))
}

/// Verify the CRC-32 (MPEG-2) trailing a PAT section in a PAT packet.
///
/// `PatSection::parse` does not validate the CRC, so the test checks it
/// explicitly: the last 4 bytes must equal `crc32_mpeg2(section[..len-4])`.
fn pat_crc_is_valid(pkt: &[u8]) -> bool {
    let section = match pat_section_with_crc(pkt) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if section.len() < 4 {
        return false;
    }
    let crc_pos = section.len() - 4;
    let computed = broadcast_common::crc32_mpeg2::compute(&section[..crc_pos]);
    let stored = u32::from_be_bytes([
        section[crc_pos],
        section[crc_pos + 1],
        section[crc_pos + 2],
        section[crc_pos + 3],
    ]);
    computed == stored
}

// ── Run engine ───────────────────────────────────────────────────────────────

fn run(input: &[u8], builder: impl Fn(ts_fix::TsFixBuilder) -> ts_fix::TsFixBuilder) -> Vec<u8> {
    let base_builder = TsFix::builder();
    let configured = builder(base_builder);
    let mut engine = configured.build().expect("build should not fail");

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

/// Zero out the section body of every PAT packet in a TS stream.
///
/// Leaves the 4-byte TS header (and adaptation, if any) intact but corrupts the
/// PSI payload so the original PAT no longer parses, while keeping the PID-0x0000
/// slot present (so regen has a slot to replace in-position).
fn zero_pat_section_bodies(ts: &mut [u8]) {
    for pkt in ts.chunks_exact_mut(188) {
        if pid_from_packet(pkt) == PAT_PID {
            for b in &mut pkt[4..] {
                *b = 0;
            }
        }
    }
}

/// Fault-inject: corrupt the PAT (zero its section body), then regen.
///
/// regen_psi derives the program → PMT-PID mapping from the PMT sections (which
/// are intact), so even with a destroyed PAT it rebuilds a correct one in the
/// PID-0x0000 slot. Oracle = the original (pre-corruption) PAT mapping.
#[test]
fn regen_pat_from_zero_corruption() {
    let ts = build_two_program_ts(4);

    // Read the original PAT mapping BEFORE corrupting (the oracle).
    let orig_pat_pkt = find_pat_packet(&ts).expect("original stream must have PAT");
    let orig_pat = parse_pat_from_packet(orig_pat_pkt).expect("original PAT must parse");
    assert_eq!(
        orig_pat.entries.len(),
        2,
        "original PAT should have 2 programs"
    );

    // Corrupt the PAT section body (keep the slot present).
    let mut corrupted = ts.clone();
    zero_pat_section_bodies(&mut corrupted);

    // Sanity: the corrupted PAT slot must no longer parse (fault really injected).
    let corrupt_pat_pkt = find_pat_packet(&corrupted).expect("corrupted stream still has PAT slot");
    assert!(
        parse_pat_from_packet(corrupt_pat_pkt).is_err(),
        "corruption must make the original PAT unparseable"
    );

    // Run through regen_psi.
    let output = run(&corrupted, |b| b.regen_psi());

    // Output must contain a PAT on PID 0x0000 that parses with a valid CRC.
    let regen_pat_pkt = find_pat_packet(&output).expect("output must have a regenerated PAT");
    assert!(
        pat_crc_is_valid(regen_pat_pkt),
        "regenerated PAT must carry a valid CRC-32"
    );
    let regen_pat = parse_pat_from_packet(regen_pat_pkt).expect("regen PAT must parse");

    // The regen PAT lists exactly the original program → PMT-PID mapping.
    let orig_map: std::collections::BTreeMap<u16, Pid> = orig_pat
        .entries
        .iter()
        .map(|e| (e.program_number, e.pid))
        .collect();
    let regen_map: std::collections::BTreeMap<u16, Pid> = regen_pat
        .entries
        .iter()
        .map(|e| (e.program_number, e.pid))
        .collect();
    assert_eq!(
        orig_map, regen_map,
        "regen PAT mapping must match the original (PMT-derived)"
    );
}

/// Combine filter + regen: extract program 1, then regenerate PAT.
///
/// Output PAT should list ONLY program 1.
#[test]
fn filter_then_regen_lists_only_surviving_program() {
    let ts = build_two_program_ts(4);

    // Run through filter_pids + regen_psi.
    let output = run(&ts, |b| b.filter_pids(PidFilter::service(1)).regen_psi());

    // Find and parse the regenerated PAT.
    let pat_pkt = find_pat_packet(&output).expect("output must have PAT");
    let pat = parse_pat_from_packet(pat_pkt).expect("output PAT must parse");

    // Verify: only program 1 listed.
    assert_eq!(pat.entries.len(), 1, "filtered PAT should have 1 program");
    assert_eq!(pat.entries[0].program_number, 1);
    assert_eq!(pat.entries[0].pid, PMT1_PID);
}

/// Combine filter + regen: extract program 2, then regenerate PAT.
///
/// Output PAT should list ONLY program 2.
#[test]
fn filter_program_2_then_regen() {
    let ts = build_two_program_ts(4);

    // Run through filter_pids + regen_psi.
    let output = run(&ts, |b| b.filter_pids(PidFilter::service(2)).regen_psi());

    // Find and parse the regenerated PAT.
    let pat_pkt = find_pat_packet(&output).expect("output must have PAT");
    let pat = parse_pat_from_packet(pat_pkt).expect("output PAT must parse");

    // Verify: only program 2 listed.
    assert_eq!(pat.entries.len(), 1, "filtered PAT should have 1 program");
    assert_eq!(pat.entries[0].program_number, 2);
    assert_eq!(pat.entries[0].pid, PMT2_PID);
}

/// No-op regen on a corrupted PAT FAILS (test harness verification).
///
/// This test proves that `regen_pat_from_zero_corruption` bites: without regen,
/// the corrupted (zeroed) PAT remains unparseable, so the regen assertions could
/// not pass under an identity pass-through.
#[test]
fn identity_fails_to_repair_corrupted_pat() {
    let ts = build_two_program_ts(4);

    // Corrupt the PAT section body.
    let mut corrupted = ts.clone();
    zero_pat_section_bodies(&mut corrupted);

    // Run through identity (no regen).
    let output = run(&corrupted, |b| b);

    // Find the (still-corrupted) PAT in output.
    let pat_pkt = find_pat_packet(&output).expect("output must have a PAT packet");

    // Attempt to parse: this should FAIL because the section is still corrupted.
    assert!(
        parse_pat_from_packet(pat_pkt).is_err(),
        "identity pass-through must fail to parse a corrupted PAT (proves regen is needed)"
    );
}

/// Load the committed m6-single.ts fixture (hard panic if missing — never skip).
fn load_m6_single() -> Vec<u8> {
    // CWD when running `cargo test -p ts-fix` is the crate dir (ts-fix/), so the
    // fixture is read from the copy committed inside this crate.
    let fixture_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m6-single.ts");
    fs::read(fixture_path)
        .unwrap_or_else(|e| panic!("required fixture {fixture_path} missing: {e}"))
}

/// Scan a TS for the authoritative program → PMT-PID mapping by parsing the
/// PMT sections present (table_id 0x02; each PMT carries its own program_number).
///
/// This mirrors the op's own discovery and is the correct oracle: the PMTs are
/// the authoritative source, independent of whatever the PAT claims.
fn pmt_derived_mapping(ts: &[u8]) -> std::collections::BTreeMap<u16, Pid> {
    use mpeg_ts::ts::SectionReassembler;
    let mut reasm: std::collections::BTreeMap<u16, SectionReassembler> =
        std::collections::BTreeMap::new();
    let mut map: std::collections::BTreeMap<u16, Pid> = std::collections::BTreeMap::new();
    for pkt in ts.chunks_exact(188) {
        let pid = pid_from_packet(pkt);
        if pid == PAT_PID || pid == Pid::new(0x1FFF) {
            continue;
        }
        // Parse the header to validate the packet; PUSI is read from byte 1.
        if TsHeader::parse(&pkt[..4]).is_err() {
            continue;
        }
        let pusi = (pkt[1] & 0x40) != 0;
        let payload = match extract_ts_payload(pkt) {
            Some(p) => p,
            None => continue,
        };
        // Only track PIDs whose first PUSI section starts with the PMT table_id.
        let r = match reasm.entry(pid.value()) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::btree_map::Entry::Vacant(slot) => {
                if !pusi {
                    continue;
                }
                let ptr = payload[0] as usize;
                match payload.get(1 + ptr) {
                    Some(&tid) if tid == PMT_TABLE_ID => {}
                    _ => continue,
                }
                slot.insert(SectionReassembler::default())
            }
        };
        r.feed(payload, pusi);
        while let Some(section) = r.pop_section() {
            if let Ok(pmt) = PmtSection::parse(&section) {
                map.insert(pmt.program_number, pid);
            }
        }
    }
    map
}

/// Collect every parseable output PAT's program→PMT mapping (in order).
fn output_pat_mappings(ts: &[u8]) -> Vec<std::collections::BTreeMap<u16, Pid>> {
    ts.chunks_exact(188)
        .filter(|pkt| pid_from_packet(pkt) == PAT_PID)
        .filter_map(|pkt| parse_pat_from_packet(pkt).ok())
        .map(|pat| {
            pat.entries
                .iter()
                .map(|e| (e.program_number, e.pid))
                .collect()
        })
        .collect()
}

/// Fault-inject on m6-single.ts: CORRUPT (zero) the PAT section bodies, regen,
/// verify the output PAT parses, has a valid CRC-32, and lists exactly the
/// program → PMT-PID mapping derived from the PMTs present.
///
/// Oracle = the PMT-derived mapping (authoritative).  Cross-check: the original
/// PAT's programs (read BEFORE corrupting) must be a subset of it.
#[test]
fn regen_pat_on_m6_single() {
    let ts_data = load_m6_single();

    // Authoritative oracle: the program mapping derived from the PMTs present.
    let oracle = pmt_derived_mapping(&ts_data);
    assert!(
        !oracle.is_empty(),
        "m6-single.ts must have at least one PMT"
    );

    // Cross-check: every program the original PAT listed must be in the oracle.
    let orig_pat_pkt = find_pat_packet(&ts_data).expect("m6-single.ts must have PAT");
    let orig_pat = parse_pat_from_packet(orig_pat_pkt).expect("original PAT must parse");
    assert!(
        !orig_pat.entries.is_empty(),
        "m6-single.ts must have at least one program"
    );
    for e in &orig_pat.entries {
        assert_eq!(
            oracle.get(&e.program_number),
            Some(&e.pid),
            "original PAT program {} must appear in the PMT-derived mapping",
            e.program_number
        );
    }

    // Corrupt the PAT in-position (zero the section bodies); keep the slots.
    let mut corrupted = ts_data.clone();
    zero_pat_section_bodies(&mut corrupted);

    // Sanity: the first corrupted PAT slot must no longer parse.
    let corrupt_pat_pkt = find_pat_packet(&corrupted).expect("corrupted stream keeps PAT slots");
    assert!(
        parse_pat_from_packet(corrupt_pat_pkt).is_err(),
        "corruption must make the original PAT unparseable"
    );

    // Run through regen_psi.
    let output = run(&corrupted, |b| b.regen_psi());

    // Every output PAT must parse with a valid CRC-32, and at least one must
    // list exactly the authoritative PMT-derived mapping.
    for pkt in output
        .chunks_exact(188)
        .filter(|p| pid_from_packet(p) == PAT_PID)
    {
        assert!(
            pat_crc_is_valid(pkt),
            "every regenerated PAT must carry a valid CRC-32"
        );
    }
    let mappings = output_pat_mappings(&output);
    assert!(
        mappings.contains(&oracle),
        "a regenerated PAT must list exactly the PMT-derived mapping {oracle:?}; got {mappings:?}"
    );
}

/// Identity (no regen) on the corrupted m6-single.ts leaves the PAT unparseable
/// — proves `regen_pat_on_m6_single` bites on a real capture.
#[test]
fn identity_fails_on_corrupted_m6_single() {
    let ts_data = load_m6_single();
    let mut corrupted = ts_data.clone();
    zero_pat_section_bodies(&mut corrupted);

    let output = run(&corrupted, |b| b);
    let pat_pkt = find_pat_packet(&output).expect("output must have a PAT slot");
    assert!(
        parse_pat_from_packet(pat_pkt).is_err(),
        "identity must leave the corrupted m6 PAT unparseable (proves regen test bites)"
    );
}

/// Stripped-PAT (drop-all) on m6-single.ts: with NO PAT slot at all, regen_psi
/// must still discover the program mapping from the PMTs and emit a valid
/// regenerated PAT *early* (before the end of the stream).
#[test]
fn regen_pat_on_m6_single_fully_stripped() {
    let ts_data = load_m6_single();

    // Authoritative oracle from the intact original.
    let oracle = pmt_derived_mapping(&ts_data);
    assert!(
        !oracle.is_empty(),
        "m6-single.ts must have at least one PMT"
    );

    // Drop ALL PAT packets (fully stripped).
    let mut stripped = Vec::new();
    for pkt in ts_data.chunks_exact(188) {
        if pid_from_packet(pkt) != PAT_PID {
            stripped.extend_from_slice(pkt);
        }
    }
    assert!(
        find_pat_packet(&stripped).is_none(),
        "stripped stream must contain no PAT packet"
    );

    // Run through regen_psi.
    let output = run(&stripped, |b| b.regen_psi());

    // Every output PAT must carry a valid CRC, and at least one must list the
    // authoritative PMT-derived mapping.
    for pkt in output
        .chunks_exact(188)
        .filter(|p| pid_from_packet(p) == PAT_PID)
    {
        assert!(
            pat_crc_is_valid(pkt),
            "every regenerated PAT must carry a valid CRC-32"
        );
    }
    let mappings = output_pat_mappings(&output);
    assert!(
        mappings.contains(&oracle),
        "a regenerated PAT must list exactly the PMT-derived mapping {oracle:?}; got {mappings:?}"
    );

    // The regenerated PAT must appear EARLY, not only at the very end.
    let total_packets = output.len() / 188;
    let pat_index = output
        .chunks_exact(188)
        .position(|pkt| pid_from_packet(pkt) == PAT_PID)
        .expect("regen PAT present");
    assert!(
        pat_index < total_packets - 1,
        "regenerated PAT must be emitted in-stream (early), not only as the final packet \
         (index {pat_index} of {total_packets})"
    );
}

/// Verify that regen_psi passes through non-PAT packets unchanged.
///
/// regen_psi will drop incoming PAT packets and emit a regenerated PAT on flush.
/// All other packets should pass through byte-identical to the input.
#[test]
fn regen_psi_preserves_non_pat_packets() {
    let ts = build_two_program_ts(4);

    // Count non-PAT packets in the input.
    let input_non_pat_count: usize = ts
        .chunks_exact(188)
        .filter(|pkt| pid_from_packet(pkt) != PAT_PID)
        .count();

    // Run through regen_psi.
    let output = run(&ts, |b| b.regen_psi());

    // Count non-PAT packets in the output. Since regen_psi drops the original PAT
    // and emits a new one on flush, we have:
    // - All non-PAT packets from input (input_non_pat_count)
    // - One new PAT packet from regen_psi's flush
    let output_non_pat_count: usize = output
        .chunks_exact(188)
        .filter(|pkt| pid_from_packet(pkt) != PAT_PID)
        .count();
    let output_pat_count: usize = output
        .chunks_exact(188)
        .filter(|pkt| pid_from_packet(pkt) == PAT_PID)
        .count();

    // Verify: non-PAT count should match.
    assert_eq!(
        input_non_pat_count, output_non_pat_count,
        "non-PAT packets should pass through unchanged"
    );

    // Verify: there should be at least one PAT in output (the regenerated one).
    assert!(output_pat_count > 0, "output should have a regenerated PAT");

    // Verify every non-PAT packet in the output matches the input.
    // (This is a strong invariant: identity for non-PAT packets.)
    let input_non_pat: Vec<_> = ts
        .chunks_exact(188)
        .filter(|pkt| pid_from_packet(pkt) != PAT_PID)
        .collect();
    let output_non_pat: Vec<_> = output
        .chunks_exact(188)
        .filter(|pkt| pid_from_packet(pkt) != PAT_PID)
        .collect();

    assert_eq!(input_non_pat.len(), output_non_pat.len());
    for (input_pkt, output_pkt) in input_non_pat.iter().zip(output_non_pat.iter()) {
        assert_eq!(
            input_pkt, output_pkt,
            "non-PAT packets must be byte-identical"
        );
    }
}

// ── Regression: continuity_counter, version_number, network_pid (#1037) ──────

/// Find every PAT packet (PID 0x0000) in a TS stream, in order.
fn find_all_pat_packets(ts: &[u8]) -> Vec<&[u8]> {
    ts.chunks_exact(188)
        .filter(|pkt| pid_from_packet(pkt) == PAT_PID)
        .collect()
}

/// Extract the 4-bit continuity_counter from a TS packet header (byte 3, low nibble).
fn continuity_counter(pkt: &[u8]) -> u8 {
    pkt[3] & 0x0F
}

/// Build the synthetic 2-program TS (as `build_two_program_ts`) but with a
/// `network_pid` entry (`program_number == 0`, ISO/IEC 13818-1 §2.4.4.3) in
/// the original PAT — the entry `rebuild_pat` cannot derive from any PMT.
fn build_two_program_ts_with_nit(cycles: usize, network_pid: u16) -> Vec<u8> {
    let pat = PatSection::new(
        1,
        0,
        true,
        0,
        0,
        vec![
            PatEntry::new(0, Pid::new(network_pid)),
            PatEntry::new(1, PMT1_PID),
            PatEntry::new(2, PMT2_PID),
        ],
    );
    let pat_section_bytes = serialize_pat(&pat);
    let mut pat_pktz = SectionPacketiser::new(PAT_PID);
    let pat_pkts = pat_pktz.packetise(&[&pat_section_bytes]);

    let pmt1 = PmtSection::new(
        1,
        0,
        true,
        0,
        0,
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
    let pmt1_section_bytes = serialize_pmt(&pmt1);
    let mut pmt1_pktz = SectionPacketiser::new(PMT1_PID);
    let pmt1_pkts = pmt1_pktz.packetise(&[&pmt1_section_bytes]);

    let pmt2 = PmtSection::new(
        2,
        0,
        true,
        0,
        0,
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
    let pmt2_section_bytes = serialize_pmt(&pmt2);
    let mut pmt2_pktz = SectionPacketiser::new(PMT2_PID);
    let pmt2_pkts = pmt2_pktz.packetise(&[&pmt2_section_bytes]);

    let mut stream: Vec<u8> = Vec::new();
    for pkt in &pat_pkts {
        stream.extend_from_slice(pkt);
    }
    for pkt in &pmt1_pkts {
        stream.extend_from_slice(pkt);
    }
    for pkt in &pmt2_pkts {
        stream.extend_from_slice(pkt);
    }
    for i in 0..cycles {
        let cc = (i as u8) & 0x0F;
        for pkt in &pat_pkts {
            stream.extend_from_slice(pkt);
        }
        for pkt in &pmt1_pkts {
            stream.extend_from_slice(pkt);
        }
        for pkt in &pmt2_pkts {
            stream.extend_from_slice(pkt);
        }
        stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P2_VIDEO_PID, cc));
        stream.extend_from_slice(&null_packet());
    }
    stream
}

/// Regression: the pre-fix code created a fresh `SectionPacketiser` (CC=0)
/// on every single regenerated PAT emission — see `emit_regen_pat`'s old
/// `let mut packetiser = SectionPacketiser::new(PAT_PID);` inside the
/// per-call body. A real receiver treats a PID whose continuity_counter
/// never advances as a stall/duplicate, not a healthy repeating PAT cycle.
#[test]
fn regen_pat_continuity_counter_increments_across_cycles() {
    let ts = build_two_program_ts(6);
    let output = run(&ts, |b| b.regen_psi());

    let pats = find_all_pat_packets(&output);
    assert!(
        pats.len() >= 4,
        "expected several regenerated PAT packets, got {}",
        pats.len()
    );

    let ccs: Vec<u8> = pats.iter().map(|p| continuity_counter(p)).collect();
    for w in ccs.windows(2) {
        let expected = (w[0] + 1) & 0x0F;
        assert_eq!(
            w[1], expected,
            "continuity_counter must increment by 1 mod 16 across regenerated \
             PAT packets, got sequence {ccs:?}"
        );
    }
}

/// version_number (ISO/IEC 13818-1 §2.4.4.4) must stay constant while the
/// discovered program mapping does not change — a real PAT is not supposed
/// to bump version on every repeated, unchanged cycle.
#[test]
fn regen_pat_version_number_stable_when_mapping_unchanged() {
    let ts = build_two_program_ts(6);
    let output = run(&ts, |b| b.regen_psi());

    let pats = find_all_pat_packets(&output);
    assert!(pats.len() >= 4);

    let versions: Vec<u8> = pats
        .iter()
        .map(|p| {
            parse_pat_from_packet(p)
                .expect("regen PAT parses")
                .version_number
        })
        .collect();
    assert!(
        versions.iter().all(|&v| v == versions[0]),
        "version_number must not change while the mapping is unchanged, got {versions:?}"
    );
}

/// version_number must bump by exactly 1 the first time the discovered
/// mapping changes (a third program's PMT appears mid-stream), then stay
/// stable again across the following unchanged repeats.
#[test]
fn regen_pat_version_number_bumps_once_when_mapping_changes() {
    // Phase 1: 2-program mapping, repeated 3 times.
    let phase1 = build_two_program_ts(3);

    // Phase 2: a 3rd program (PMT on a fresh PID) is added, repeated 3 times.
    const PMT3_PID: Pid = Pid::new(0x0300);
    const P3_PCR_PID: Pid = Pid::new(0x0301);
    let pmt3 = PmtSection::new(
        3,
        0,
        true,
        0,
        0,
        P3_PCR_PID,
        DescriptorLoop::new(&[]),
        vec![PmtStream::new(
            StreamType::Mpeg2Video,
            P3_PCR_PID,
            DescriptorLoop::new(&[]),
        )],
    );
    let pmt3_section_bytes = serialize_pmt(&pmt3);
    let mut pmt3_pktz = SectionPacketiser::new(PMT3_PID);
    let pmt3_pkts = pmt3_pktz.packetise(&[&pmt3_section_bytes]);

    let pat3 = PatSection::new(
        1,
        1,
        true,
        0,
        0,
        vec![
            PatEntry::new(1, PMT1_PID),
            PatEntry::new(2, PMT2_PID),
            PatEntry::new(3, PMT3_PID),
        ],
    );
    let pat3_bytes = serialize_pat(&pat3);
    let mut pat3_pktz = SectionPacketiser::new(PAT_PID);
    let pat3_pkts = pat3_pktz.packetise(&[&pat3_bytes]);

    // Reuse phase1's PMT1/PMT2 bytes by re-deriving them the same way
    // build_two_program_ts does, so PMT PIDs/content match phase 1 exactly.
    let pmt1 = PmtSection::new(
        1,
        0,
        true,
        0,
        0,
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
    let pmt1_bytes = serialize_pmt(&pmt1);
    let mut pmt1_pktz = SectionPacketiser::new(PMT1_PID);
    let pmt2 = PmtSection::new(
        2,
        0,
        true,
        0,
        0,
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
    let pmt2_bytes = serialize_pmt(&pmt2);
    let mut pmt2_pktz = SectionPacketiser::new(PMT2_PID);

    let mut phase2 = Vec::new();
    for _ in 0..3 {
        for pkt in &pat3_pkts {
            phase2.extend_from_slice(pkt);
        }
        for pkt in &pmt1_pktz.packetise(&[&pmt1_bytes]) {
            phase2.extend_from_slice(pkt);
        }
        for pkt in &pmt2_pktz.packetise(&[&pmt2_bytes]) {
            phase2.extend_from_slice(pkt);
        }
        for pkt in &pmt3_pkts {
            phase2.extend_from_slice(pkt);
        }
    }

    let mut ts = phase1;
    ts.extend_from_slice(&phase2);

    let output = run(&ts, |b| b.regen_psi());
    let pats = find_all_pat_packets(&output);
    let parsed: Vec<PatSection> = pats
        .iter()
        .map(|p| parse_pat_from_packet(p).expect("regen PAT parses"))
        .collect();

    // Split by mapping size: 2-program entries (phase 1) vs 3-program (phase 2).
    let phase1_versions: Vec<u8> = parsed
        .iter()
        .filter(|p| p.entries.len() == 2)
        .map(|p| p.version_number)
        .collect();
    let phase2_versions: Vec<u8> = parsed
        .iter()
        .filter(|p| p.entries.len() == 3)
        .map(|p| p.version_number)
        .collect();

    assert!(!phase1_versions.is_empty(), "expected some 2-program PATs");
    assert!(!phase2_versions.is_empty(), "expected some 3-program PATs");
    assert!(
        phase1_versions.iter().all(|&v| v == phase1_versions[0]),
        "phase-1 (unchanged 2-program) versions must be stable: {phase1_versions:?}"
    );
    assert!(
        phase2_versions.iter().all(|&v| v == phase2_versions[0]),
        "phase-2 (unchanged 3-program) versions must be stable: {phase2_versions:?}"
    );
    assert_eq!(
        phase2_versions[0],
        (phase1_versions[0] + 1) % 32,
        "version_number must bump by exactly 1 when the mapping first changes \
         (phase1={:?} phase2={:?})",
        phase1_versions,
        phase2_versions
    );
}

/// Regression: `rebuild_pat` only derived entries from parsed PMT sections,
/// so a `network_pid` entry (`program_number == 0`) in the *original* PAT —
/// which no PMT carries — was silently dropped from every regenerated PAT.
#[test]
fn regen_pat_preserves_network_pid_entry() {
    const NIT_PID: Pid = Pid::new(0x0010);
    let ts = build_two_program_ts_with_nit(4, NIT_PID.value());

    // Sanity: the original PAT really does carry the network_pid entry.
    let orig_pat_pkt = find_pat_packet(&ts).expect("original stream must have PAT");
    let orig_pat = parse_pat_from_packet(orig_pat_pkt).expect("original PAT must parse");
    assert!(
        orig_pat
            .entries
            .iter()
            .any(|e| e.program_number == 0 && e.pid == NIT_PID),
        "test precondition: original PAT must carry a network_pid entry"
    );

    let output = run(&ts, |b| b.regen_psi());
    let regen_pat_pkt = find_pat_packet(&output).expect("output must have a regenerated PAT");
    let regen_pat = parse_pat_from_packet(regen_pat_pkt).expect("regen PAT must parse");

    assert!(
        regen_pat
            .entries
            .iter()
            .any(|e| e.program_number == 0 && e.pid == NIT_PID),
        "regenerated PAT must preserve the network_pid entry from the original \
         PAT (not derivable from any PMT), got entries {:?}",
        regen_pat.entries
    );
    // Programs must still be present too.
    assert_eq!(
        regen_pat
            .entries
            .iter()
            .filter(|e| e.program_number != 0)
            .count(),
        2,
        "regenerated PAT must still list both programs alongside the network_pid entry"
    );
}

// ── W-TF-4 (#1101): flush output must flow through later ops ─────────────────

/// Build a stream carrying ONLY PMT1 + ES packets — no PAT slot anywhere — so
/// `regen_psi`'s only PAT is the one emitted from `PsiRegenOp::flush`.  Each
/// PMT cycle ends with a null packet so the later `StuffingOp` has something
/// observable to do.
fn build_patless_p1_ts() -> Vec<u8> {
    let pmt1 = PmtSection::new(
        1,
        0,
        true,
        0,
        0,
        P1_PCR_PID,
        DescriptorLoop::new(&[]),
        vec![PmtStream::new(
            StreamType::Mpeg2Video,
            P1_VIDEO_PID,
            DescriptorLoop::new(&[]),
        )],
    );
    let mut pmt_bytes = vec![0u8; pmt1.serialized_len()];
    pmt1.serialize_into(&mut pmt_bytes).expect("PMT serialize");
    let mut pmt_pktz = SectionPacketiser::new(PMT1_PID);
    let pmt_pkts = pmt_pktz.packetise(&[&pmt_bytes]);

    // A SINGLE PMT cycle: the mapping never completes mid-stream (the early
    // emit only fires on a *repeat* of a known program's PMT), so the PAT is
    // emitted exclusively from PsiRegenOp::flush at end of stream.
    let mut stream = Vec::new();
    for p in &pmt_pkts {
        stream.extend_from_slice(p);
    }
    stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, 0));
    stream.extend_from_slice(&null_packet());
    stream
}

/// `regen_psi` + `drop_nulls` with a PAT-less input: the fallback PAT emitted
/// from `PsiRegenOp::flush` must reach the output by flowing through every
/// later op, exactly like packets flushed by an earlier op during `push`.
/// Before #1101 the multi-op `finish` only drained the staging buffer at the
/// *last* op, so op 0's flush output was silently dropped and the output
/// carried no PAT at all.
#[test]
fn regen_psi_flush_pat_flows_through_later_ops() {
    let ts = build_patless_p1_ts();

    let mut engine = TsFix::builder()
        .regen_psi()
        .stuffing(Stuffing::drop_nulls())
        .build()
        .expect("build");

    let mut output = Vec::new();
    for chunk in ts.chunks_exact(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid packet");
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    let pids: Vec<Pid> = output
        .chunks_exact(188)
        .map(|p| Pid::new((((p[1] & 0x1F) as u16) << 8) | p[2] as u16))
        .collect();
    assert!(
        pids.contains(&PAT_PID),
        "the PAT flushed by regen_psi must be chained through later ops, not dropped"
    );
    assert!(
        !pids.contains(&NULL_PID),
        "nulls must still be dropped by the later StuffingOp"
    );

    // Independent cross-check with TSDuck tsanalyze (the tsduck_oracle.rs
    // pattern): the chained flush PAT must be a clean PAT section.
    if tsanalyze_available() {
        let path = write_temp_ts(&output, "tsfix-wtf4");
        let (disc, dups) = tsanalyze_pid_stats(&path, Pid::new(0));
        assert_eq!(
            disc, 0,
            "tsanalyze: chained flush PAT must have no discontinuities"
        );
        assert_eq!(
            dups, 0,
            "tsanalyze: chained flush PAT must have no duplicates"
        );
        let _ = std::fs::remove_file(&path);
    }
}

fn tsanalyze_available() -> bool {
    std::process::Command::new("tsanalyze")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn write_temp_ts(bytes: &[u8], prefix: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}.ts",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&path, bytes).expect("write temp ts");
    path
}

/// Run `tsanalyze --normalized` and return (discontinuities, duplicates) for `pid`.
fn tsanalyze_pid_stats(path: &std::path::Path, pid: Pid) -> (i64, i64) {
    let output = std::process::Command::new("tsanalyze")
        .arg("--normalized")
        .arg(path)
        .output()
        .expect("tsanalyze must run");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find(|l| {
            l.starts_with("pid:")
                && l.split(':')
                    .find_map(|kv| kv.strip_prefix("pid="))
                    .and_then(|v| v.parse::<i64>().ok())
                    == Some(pid.value() as i64)
        })
        .unwrap_or_else(|| panic!("no tsanalyze pid: line for PID {pid} in:\n{text}"));
    let field = |key: &str| -> i64 {
        line.split(':')
            .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
            .and_then(|v| v.parse().ok())
            .unwrap_or(-1)
    };
    (field("discontinuities"), field("duplicated"))
}

/// #1037, real capture + independent oracle: `--regen-psi` on the committed
/// real fixture must not make the PAT PID (0x0000) violate TR 101 290
/// indicator 1.4 (Continuity_count_error), judged by `dvb-conformance`'s own
/// monitor rather than by ts-fix's CC bookkeeping. With a fresh packetiser
/// per PAT every regenerated PAT has CC=0, so any two consecutive PATs trip it.
#[test]
fn regen_psi_on_real_capture_keeps_pat_pid_continuity_clean() {
    use dvb_conformance::{ConformanceMonitor, Indicator};

    let input = load_m6_single();
    let output = run(&input, |b| b.regen_psi());
    assert!(
        find_all_pat_packets(&output).len() >= 2,
        "precondition: several regenerated PAT packets on the real fixture"
    );

    let mut monitor = ConformanceMonitor::new();
    let mut pat_cc_errors = Vec::new();
    for (i, pkt) in output.chunks_exact(188).enumerate() {
        let t = std::time::Duration::from_micros(40 * i as u64);
        for event in monitor.feed(pkt, t) {
            if event.indicator == Indicator::ContinuityCountError && event.pid == Some(PAT_PID) {
                pat_cc_errors.push((i, event.detail.clone()));
            }
        }
    }
    assert!(
        pat_cc_errors.is_empty(),
        "regenerated PATs raised Continuity_count_error on PID 0: {pat_cc_errors:?}"
    );
}

// ── #1288: filter that drops PMT(s), then regen ─────────────────────────────

/// Every PAT packet's continuity_counter must follow the previous one (mod 16).
fn assert_pat_pid_continuous(ts: &[u8]) {
    let ccs: Vec<u8> = ts
        .chunks_exact(188)
        .filter(|p| pid_from_packet(p) == PAT_PID)
        .map(|p| p[3] & 0x0F)
        .collect();
    for w in ccs.windows(2) {
        assert_eq!(
            w[1],
            (w[0] + 1) % 16,
            "PAT PID CC must be continuous: {ccs:?}"
        );
    }
}

/// PIDs in `ts` whose PUSI packets begin a PMT section.
fn pmt_pids_in(ts: &[u8]) -> std::collections::BTreeSet<Pid> {
    ts.chunks_exact(188)
        .filter(|p| p[1] & 0x40 != 0)
        .filter_map(|p| {
            let payload = extract_ts_payload(p)?;
            let tid = *payload.get(1 + *payload.first()? as usize)?;
            (tid == PMT_TABLE_ID).then(|| pid_from_packet(p))
        })
        .collect()
}

/// Output must carry a valid, continuous PAT whose non-network entries all
/// reference a PMT PID present in the output (never a dangling PMT PID).
fn assert_pat_consistent_with_pmts(output: &[u8]) -> PatSection {
    let pat_pkt = find_pat_packet(output).expect("output must have a PAT (#1288)");
    assert!(pat_crc_is_valid(pat_pkt), "PAT CRC must be valid");
    assert_pat_pid_continuous(output);
    let pat = parse_pat_from_packet(pat_pkt).expect("PAT must parse");
    let pmts = pmt_pids_in(output);
    for e in &pat.entries {
        if e.program_number != dvb_si::tables::pat::PROGRAM_NUMBER_NIT {
            assert!(
                pmts.contains(&e.pid),
                "PAT references absent PMT PID {:?}",
                e.pid
            );
        }
    }
    pat
}

/// The issue repro on the real capture: keep only ES PIDs (PMT dropped).
#[test]
fn keep_pids_dropping_every_pmt_then_regen_keeps_a_programless_pat() {
    let ts = test_fixtures::read("ts/m6-single.ts");
    let output = run(&ts, |b| {
        b.filter_pids(PidFilter::keep([Pid::new(120), Pid::new(130)]))
            .regen_psi()
    });
    assert!(!output.is_empty());
    let pat = assert_pat_consistent_with_pmts(&output);
    assert!(
        pat.entries
            .iter()
            .all(|e| e.program_number == dvb_si::tables::pat::PROGRAM_NUMBER_NIT),
        "no PMT survives, so no program may be listed"
    );
    // The kept ES PIDs are still present.
    let pids: std::collections::BTreeSet<Pid> =
        output.chunks_exact(188).map(pid_from_packet).collect();
    assert!(pids.contains(&Pid::new(130)));
}

/// Two programs, both PMTs dropped (ES of program 1 kept).
#[test]
fn two_program_keep_without_any_pmt_keeps_programless_pat() {
    let ts = build_two_program_ts(4);
    let output = run(&ts, |b| {
        b.filter_pids(PidFilter::keep([P1_VIDEO_PID, P1_AUDIO_PID]))
            .regen_psi()
    });
    let pat = assert_pat_consistent_with_pmts(&output);
    assert!(
        pat.entries.is_empty(),
        "no PMT survives: PAT lists no program"
    );
}

/// Two programs: PMT1 kept, PMT2 dropped. PAT must list only program 1.
#[test]
fn two_program_keep_one_pmt_lists_only_that_program() {
    let ts = build_two_program_ts(4);
    let output = run(&ts, |b| {
        b.filter_pids(PidFilter::keep([PMT1_PID, P1_VIDEO_PID, P1_AUDIO_PID]))
            .regen_psi()
    });
    let pat = assert_pat_consistent_with_pmts(&output);
    assert_eq!(pat.entries.len(), 1);
    assert_eq!(pat.entries[0].program_number, 1);
    assert_eq!(pat.entries[0].pid, PMT1_PID);
}

// ── Audit 2026-10: PAT hints across packets / unknown tsid / PMT parse failure ─

/// Hand-framed single TS packet: PUSI set, pointer_field 0, `section` bytes,
/// 0xFF stuffing. Used for deliberately malformed sections the packetiser
/// would never produce.
fn raw_section_packet(pid: Pid, section: &[u8]) -> [u8; 188] {
    let mut pkt = [0xFFu8; 188];
    pkt[0] = 0x47;
    pkt[1] = 0x40 | ((pid.value() >> 8) as u8);
    pkt[2] = pid.value() as u8;
    pkt[3] = 0x10;
    pkt[4] = 0x00;
    pkt[5..5 + section.len()].copy_from_slice(section);
    pkt
}

fn pmt1_packets() -> Vec<[u8; 188]> {
    let pmt1 = PmtSection::new(
        1,
        0,
        true,
        0,
        0,
        P1_PCR_PID,
        DescriptorLoop::new(&[]),
        vec![PmtStream::new(
            StreamType::Mpeg2Video,
            P1_VIDEO_PID,
            DescriptorLoop::new(&[]),
        )],
    );
    SectionPacketiser::new(PMT1_PID).packetise(&[&serialize_pmt(&pmt1)])
}

/// An original PAT spanning several TS packets (60 programs) must still supply
/// its `transport_stream_id` and `network_PID` to the regenerated PAT. The old
/// hint extraction parsed one packet only, so a multi-packet PAT failed to
/// parse and the regenerated PAT silently got transport_stream_id 0.
#[test]
fn regen_pat_hints_survive_a_multi_packet_original_pat() {
    const TSID: u16 = 0x1234;
    const NIT_PID: Pid = Pid::new(0x0010);
    let mut entries = vec![PatEntry::new(0, NIT_PID), PatEntry::new(1, PMT1_PID)];
    for n in 2..62u16 {
        entries.push(PatEntry::new(n, Pid::new(0x0300 + n)));
    }
    let pat = PatSection::new(TSID, 0, true, 0, 0, entries);
    let pat_pkts = SectionPacketiser::new(PAT_PID).packetise(&[&serialize_pat(&pat)]);
    assert!(pat_pkts.len() > 1, "precondition: PAT must span >1 packet");
    let pmt_pkts = pmt1_packets();

    let mut ts = Vec::new();
    for cycle in 0..3u8 {
        for p in pat_pkts.iter().chain(pmt_pkts.iter()) {
            ts.extend_from_slice(p);
        }
        ts.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, cycle));
    }

    let output = run(&ts, |b| b.regen_psi());
    let pkt = find_pat_packet(&output).expect("output must carry a regenerated PAT");
    let regen = parse_pat_from_packet(pkt).expect("regenerated PAT must parse");
    assert_eq!(
        regen.transport_stream_id, TSID,
        "transport_stream_id must come from the reassembled multi-packet PAT"
    );
    assert!(
        regen
            .entries
            .iter()
            .any(|e| e.program_number == 0 && e.pid == NIT_PID),
        "network_PID must survive, got {:?}",
        regen.entries
    );
}

/// PAT slots exist but the original PAT never parses and a PMT is present: the
/// transport_stream_id is unknown, so nothing is emitted in position (it could
/// be a PAT still arriving across packets). The documented fallback (tsid 0)
/// is used only once, at end of stream.
#[test]
fn unparseable_original_pat_falls_back_only_at_flush() {
    // table_id 0x42 (not a PAT) so the section reassembles but PatSection fails.
    let bad_pat = [0x42, 0xB0, 0x09, 0x00, 0x01, 0xC1, 0x00, 0x00, 0, 0, 0, 0];
    let mut ts = Vec::new();
    for cycle in 0..3u8 {
        ts.extend_from_slice(&raw_section_packet(PAT_PID, &bad_pat));
        for p in pmt1_packets() {
            ts.extend_from_slice(&p);
        }
        ts.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, cycle));
    }
    let output = run(&ts, |b| b.regen_psi());
    let pids: Vec<Pid> = output.chunks_exact(188).map(pid_from_packet).collect();
    let first_pat = pids
        .iter()
        .position(|p| *p == PAT_PID)
        .expect("a fallback PAT is emitted at flush");
    assert!(
        pids[first_pat..].iter().all(|p| *p == PAT_PID),
        "no PAT may be emitted in position while the transport_stream_id is unknown"
    );
    let pat = parse_pat_from_packet(find_pat_packet(&output).unwrap()).unwrap();
    assert_eq!(pat.transport_stream_id, 0, "documented flush-time fallback");
    assert_eq!(pat.entries.len(), 1);
}

/// A PMT that begins but never completes is a parse failure, not "no
/// programs": the flush-time program-less PAT must not be emitted.
#[test]
fn unresolved_pmt_does_not_yield_a_programless_pat() {
    let pat = PatSection::new(0x4321, 0, true, 0, 0, vec![PatEntry::new(1, PMT1_PID)]);
    let pat_pkts = SectionPacketiser::new(PAT_PID).packetise(&[&serialize_pat(&pat)]);
    // PMT header claiming 255 bytes of section body, never completed.
    let truncated_pmt = [pmt::TABLE_ID, 0xB0, 0xFF, 0x00, 0x01, 0xC1, 0x00, 0x00];
    let mut ts = Vec::new();
    for p in &pat_pkts {
        ts.extend_from_slice(p);
    }
    ts.extend_from_slice(&raw_section_packet(PMT1_PID, &truncated_pmt));
    ts.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, 0));
    let output = run(&ts, |b| b.regen_psi());
    assert!(
        find_pat_packet(&output).is_none(),
        "an unresolved PMT must not be reported as an empty program loop"
    );
}
