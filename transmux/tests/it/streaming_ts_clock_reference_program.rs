//! `DemuxEvent::ClockReference` attribution on multi-program TS (issue #1354).
//!
//! ISO/IEC 13818-1 §2.4.3.5 / §2.4.4.8-9: a program's PCRs ride the `PCR_PID`
//! its PMT names, and a multiplex has one independent clock per program. Before
//! #1354 `StreamingTsDemux` emitted a `ClockReference` for the PCR on *any*
//! PID and carried no program number — on the real Hot Bird capture that
//! included one stray PCR on PID 985 that no PMT names as a PCR_PID.
//!
//! The oracle here is independent of the demuxer under test: it reassembles
//! the PAT and every PMT itself with `mpeg_ts`'s section reassembler and reads
//! `program_number` / `PCR_PID` straight out of the section bytes (PMT: bytes
//! 3..5 = program_number, bytes 8..10 = reserved(3) + PCR_PID(13)).
//!
//! Fixtures are real multi-program captures (France TNT UHF32: 5 programs;
//! Hot Bird MHP: 19), loaded through `test_fixtures` (private submodule; skip
//! only where it is not checked out).

use std::collections::{BTreeMap, BTreeSet};

use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};

const PAT_PID: u16 = 0;
const PAT_TABLE_ID: u8 = 0x00;
const PMT_TABLE_ID: u8 = 0x02;
/// Long-form section header is 8 bytes; the CRC is the last 4.
const LONG_HDR: usize = 8;
const CRC_LEN: usize = 4;
const NO_PCR_PID: u16 = 0x1FFF;

struct Oracle {
    /// program_number -> PCR_PID (first PMT version seen).
    pcr_pid: BTreeMap<u16, u16>,
    /// Packet index by which every announced PMT had completed once.
    all_pmts_known_at: u64,
}

fn oracle(data: &[u8]) -> Oracle {
    let mut pat = SectionReassembler::default();
    let mut pmt_pids: BTreeMap<u16, u16> = BTreeMap::new(); // pmt pid -> program
    let mut pmt_reasm: BTreeMap<u16, SectionReassembler> = BTreeMap::new();
    let mut pcr_pid = BTreeMap::new();
    let mut all_known_at = 0u64;
    for (idx, raw) in data.chunks_exact(TS_PACKET_SIZE).enumerate() {
        let Ok(pkt) = TsPacket::parse(raw) else {
            continue;
        };
        let Some(payload) = pkt.payload else { continue };
        let pid = pkt.header.pid.value();
        if pid == PAT_PID {
            pat.feed(payload, pkt.header.pusi);
            while let Some(s) = pat.pop_section() {
                let s: &[u8] = s.as_ref();
                if s[0] != PAT_TABLE_ID || s.len() < LONG_HDR + CRC_LEN {
                    continue;
                }
                for e in s[LONG_HDR..s.len() - CRC_LEN].chunks_exact(4) {
                    let program = u16::from_be_bytes([e[0], e[1]]);
                    let p = u16::from_be_bytes([e[2], e[3]]) & 0x1FFF;
                    if program != 0 {
                        pmt_pids.insert(p, program);
                        pmt_reasm.entry(p).or_default();
                    }
                }
            }
        } else if let Some(r) = pmt_reasm.get_mut(&pid) {
            r.feed(payload, pkt.header.pusi);
            while let Some(s) = r.pop_section() {
                let s: &[u8] = s.as_ref();
                if s[0] != PMT_TABLE_ID {
                    continue;
                }
                let program = u16::from_be_bytes([s[3], s[4]]);
                let pcr = u16::from_be_bytes([s[8], s[9]]) & 0x1FFF;
                if pcr_pid.insert(program, pcr).is_none() && pcr_pid.len() == pmt_pids.len() {
                    all_known_at = idx as u64;
                }
            }
        }
    }
    assert_eq!(pcr_pid.len(), pmt_pids.len(), "every announced PMT seen");
    Oracle {
        pcr_pid,
        all_pmts_known_at: all_known_at,
    }
}

struct Obs {
    pid: u16,
    packet_index: u64,
    program: Option<u16>,
}

fn clock_refs(data: &[u8]) -> Vec<Obs> {
    let mut demux = StreamingTsDemux::new();
    let mut out = Vec::new();
    for chunk in data.chunks(TS_PACKET_SIZE * 512 + 7) {
        demux.feed(chunk);
        while let Some(ev) = demux.poll_event() {
            if let DemuxEvent::ClockReference { provenance, .. } = ev {
                out.push(Obs {
                    pid: provenance
                        .pid
                        .expect("TS ClockReference carries a pid")
                        .value(),
                    packet_index: provenance.packet_index.expect("packet index"),
                    program: provenance.program_number,
                });
            }
        }
    }
    out
}

fn check(fixture: &str, min_programs: usize) -> Option<Oracle> {
    let path = test_fixtures::require(fixture, "streaming_ts_clock_reference_program")?;
    let data = std::fs::read(path).unwrap();
    let o = oracle(&data);
    assert!(
        o.pcr_pid.len() >= min_programs,
        "{fixture}: need a real multi-program capture, got {}",
        o.pcr_pid.len()
    );
    let pcr_pids: BTreeSet<u16> = o
        .pcr_pid
        .values()
        .copied()
        .filter(|p| *p != NO_PCR_PID)
        .collect();
    let obs = clock_refs(&data);

    // 1. Every attributed event is on exactly the PCR_PID its PMT names.
    for ob in &obs {
        if let Some(program) = ob.program {
            assert_eq!(
                o.pcr_pid.get(&program),
                Some(&ob.pid),
                "{fixture}: ClockReference attributed to program {program} on pid {} but its PMT PCR_PID is {:?}",
                ob.pid,
                o.pcr_pid.get(&program)
            );
        }
    }
    // 2. Past the point every PMT is known, nothing is unattributed and no
    //    PCR from a non-PCR_PID leaks (Hot Bird PID 985 carries one such PCR).
    for ob in obs
        .iter()
        .filter(|ob| ob.packet_index > o.all_pmts_known_at)
    {
        assert!(
            ob.program.is_some() && pcr_pids.contains(&ob.pid),
            "{fixture}: post-PMT ClockReference on pid {} (packet {}) is not from a program PCR_PID",
            ob.pid,
            ob.packet_index
        );
    }
    // 3. Every program's PCR_PID is observed with its own program number
    //    (one clock per program, no program silently lost).
    let attributed: BTreeSet<(u16, u16)> = obs
        .iter()
        .filter_map(|ob| ob.program.map(|p| (p, ob.pid)))
        .collect();
    for (program, pid) in o.pcr_pid.iter().filter(|(_, p)| **p != NO_PCR_PID) {
        assert!(
            attributed.contains(&(*program, *pid)),
            "{fixture}: program {program} (PCR_PID {pid}) never produced a ClockReference"
        );
    }
    eprintln!(
        "{fixture}: {} programs, {} PCR pids, {} events, {} pre-PMT unattributed",
        o.pcr_pid.len(),
        pcr_pids.len(),
        obs.len(),
        obs.iter().filter(|ob| ob.program.is_none()).count()
    );
    Some(o)
}

#[test]
fn france_tnt_five_programs() {
    check("ts/france-tnt-uhf32-si-audio.ts", 5);
}

#[test]
fn hotbird_many_programs_no_stray_pcr_pid() {
    // The measured defect: PID 985 carries exactly one PCR (packet 68185) and
    // no PMT names it as a PCR_PID; it used to surface as a ClockReference.
    // `check` step 2 rejects it once every PMT is known, which the premise
    // below guarantees for packet 68185.
    let Some(o) = check("ts/hotbird-mhp-si.ts", 10) else {
        return;
    };
    assert!(!o.pcr_pid.values().any(|p| *p == 985));
    assert!(o.all_pmts_known_at < 68185);
}
