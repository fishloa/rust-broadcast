//! PSI regeneration tests for `ts-fix`.
//!
//! Tests verify that PAT regeneration works correctly after PID filtering and
//! corruption scenarios.  All tests use either a real capture (m6-single.ts)
//! with fault injection, or the synthetic 2-program mux from pid_filter tests.

use broadcast_common::traits::{Parse, Serialize};
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{self, PmtSection, PmtStream, StreamType};
use mpeg_ts::mux::SectionPacketizer;
use mpeg_ts::ts::{TsHeader, extract_ts_payload};
use std::fs;
use ts_fix::{PidFilter, TsFix};

// ── PID constants ────────────────────────────────────────────────────────────

const PAT_PID: u16 = 0x0000;
/// PMT table_id (ISO/IEC 13818-1 Table 2-30).
const PMT_TABLE_ID: u8 = pmt::TABLE_ID;
const PMT1_PID: u16 = 0x0100;
const PMT2_PID: u16 = 0x0200;

const P1_PCR_PID: u16 = 0x0101;
const P1_VIDEO_PID: u16 = 0x0101;
const P1_AUDIO_PID: u16 = 0x0102;

const P2_PCR_PID: u16 = 0x0201;
const P2_VIDEO_PID: u16 = 0x0201;
const P2_AUDIO_PID: u16 = 0x0202;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build a 188-byte TS packet carrying a dummy payload on `pid`.
fn dummy_es_packet(pid: u16, cc: u8) -> [u8; 188] {
    let mut pkt = [0u8; 188];
    pkt[0] = 0x47; // sync byte
    pkt[1] = ((pid >> 8) as u8) & 0x1F;
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10 | (cc & 0x0F); // adaptation_field_control=01 (payload only), CC
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
fn pid_from_packet(pkt: &[u8]) -> u16 {
    (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16
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
    let orig_map: std::collections::BTreeMap<u16, u16> = orig_pat
        .entries
        .iter()
        .map(|e| (e.program_number, e.pid))
        .collect();
    let regen_map: std::collections::BTreeMap<u16, u16> = regen_pat
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
fn pmt_derived_mapping(ts: &[u8]) -> std::collections::BTreeMap<u16, u16> {
    use mpeg_ts::ts::SectionReassembler;
    let mut reasm: std::collections::BTreeMap<u16, SectionReassembler> =
        std::collections::BTreeMap::new();
    let mut map = std::collections::BTreeMap::new();
    for pkt in ts.chunks_exact(188) {
        let pid = pid_from_packet(pkt);
        if pid == PAT_PID || pid == 0x1FFF {
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
        let r = match reasm.entry(pid) {
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
fn output_pat_mappings(ts: &[u8]) -> Vec<std::collections::BTreeMap<u16, u16>> {
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
