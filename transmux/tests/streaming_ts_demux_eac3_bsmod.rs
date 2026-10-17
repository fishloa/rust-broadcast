//! `dec3.bsmod` per E-AC-3 track of the real French TNT (M6) captures,
//! checked against an INDEPENDENT parse of every intact syncframe's `bsi()`
//! (ETSI TS 102 366 V1.4.1 Annex E §E.1.2.2, page 114-116; intact = `crc2`
//! valid, §E.2.2 page 145) and against the PMT `enhanced_AC-3_descriptor`
//! (ETSI EN 300 468 V1.20.1 Table D.7 page 176 / Table D.1-D.4).
//!
//! Facts established with this parser (both captures carry the same three
//! audio PIDs; the audio ES of `m6-single.ts` is a video-less cut of the
//! zenith capture):
//!
//! * `bsmod` is **constant across a stream's intact frames that carry it**
//!   (0x82: 0, 0x83: 2, 0x84: 0), but not every frame carries it: the encoder
//!   sets `infomdate` in some frames only (0x83 has intact frames with
//!   `infomdate` 1 / `bsmod` 2 *and* intact frames with `infomdate` 0). A
//!   frame without `infomdate` gives `dec3.bsmod` 0 in isolation (§F.6.2.8,
//!   page 199), which is why a `dec3` built from such a frame mislabelled the
//!   audio-description track as a main service (the `0` that #1186 produced
//!   for `m6-single.ts` track 2).
//! * The PMT agrees: PID 0x83 declares `component_type` 0xD2 = `1 1 010 010`
//!   (Enhanced AC-3, full service, service type 010 = Visually Impaired,
//!   stereo; EN 300 468 Tables D.2-D.5), PIDs 0x82/0x84 declare 0xC2
//!   (service type 000 = Complete Main). The service-type numbering of Table D.4
//!   is `bsmod` of ETSI TS 102 366 Table 4.2 (page 33): 010 = VI, 000 = CM.
//!   `ffprobe` reports `visual_impaired`/`descriptions` on that PID.
//!
//! Skips loudly when the private repo is not checked out (zenith capture only).

use std::collections::BTreeMap;
use transmux::pipeline::CodecConfig;
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};

const WHO: &str = "transmux/tests/streaming_ts_demux_eac3_bsmod.rs";
const AUDIO_PIDS: [u16; 3] = [0x82, 0x83, 0x84];
/// The independently established truth (see the module docs): PID -> bsmod.
const TRUTH: [(u16, u8); 3] = [(0x82, 0), (0x83, 2), (0x84, 0)];
/// E-AC-3 `bsid` (§E.1.3.1.6).
const EAC3_BSID: u8 = 16;

fn packets(ts: &[u8], pid: u16) -> impl Iterator<Item = &[u8]> {
    ts.chunks_exact(188)
        .filter(move |p| p[0] == 0x47 && (u16::from(p[1] & 0x1F) << 8 | u16::from(p[2])) == pid)
}

fn payload(p: &[u8]) -> Option<&[u8]> {
    let afc = (p[3] >> 4) & 3;
    let mut off = 4;
    if afc & 2 != 0 {
        off += 1 + usize::from(p[4]);
    }
    (afc & 1 != 0 && off < 188).then(|| &p[off..])
}

/// The PID's PES payload bytes concatenated, with duplicate TS packets (same
/// continuity counter twice) removed.
fn es_of(ts: &[u8], pid: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut in_pes = false;
    let mut last_cc = None;
    for p in packets(ts, pid) {
        let Some(mut pl) = payload(p) else { continue };
        let cc = p[3] & 0x0F;
        if last_cc == Some(cc) {
            continue;
        }
        last_cc = Some(cc);
        if p[1] & 0x40 != 0 {
            in_pes = true;
            pl = &pl[9 + usize::from(pl[8])..];
        }
        if in_pes {
            out.extend_from_slice(pl);
        }
    }
    out
}

/// CRC-16, generator x^16 + x^15 + x^2 + 1, init 0, MSB first, no final XOR
/// (ETSI TS 102 366 §6.10.1); zero over a whole intact frame after its syncword.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

struct Bits<'a>(&'a [u8], usize);
impl Bits<'_> {
    fn get(&mut self, n: usize) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | u32::from(self.0[self.1 >> 3] >> (7 - (self.1 & 7)) & 1);
            self.1 += 1;
        }
        v
    }
}

/// `(infomdate, bsmod)` of an independent, substream-0 E-AC-3 frame `bsi()`
/// (page 114-116), or `None` for any other frame. `mixmdate` must be 0 (true of
/// every frame of these captures); a frame that sets it would need the mixing
/// walk and panics rather than be misread.
fn bsi_bsmod(f: &[u8]) -> Option<(bool, Option<u8>)> {
    let mut r = Bits(f, 16); // after the 16-bit syncword
    let strmtyp = r.get(2);
    let substreamid = r.get(3);
    let _frmsiz = r.get(11);
    let fscod = r.get(2);
    let numblkscod_or_fscod2 = r.get(2);
    assert!(
        fscod != 3 || numblkscod_or_fscod2 <= 3,
        "half-rate frame not expected"
    );
    let acmod = r.get(3);
    let _lfeon = r.get(1);
    let bsid = r.get(5);
    if bsid != u32::from(EAC3_BSID) || strmtyp != 0 || substreamid != 0 {
        return None;
    }
    let _dialnorm = r.get(5);
    if r.get(1) == 1 {
        r.get(8); // compr
    }
    if acmod == 0 {
        r.get(5); // dialnorm2
        if r.get(1) == 1 {
            r.get(8); // compr2
        }
    }
    assert_eq!(
        r.get(1),
        0,
        "mixmdate set: mixing walk not implemented here"
    );
    let infomdate = r.get(1) == 1;
    Some((infomdate, infomdate.then(|| r.get(3) as u8)))
}

/// Every CRC-valid independent substream-0 frame of the ES, in order:
/// `(offset, infomdate, bsmod)`.
fn intact_frames(es: &[u8]) -> Vec<(usize, bool, Option<u8>)> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + 8 <= es.len() {
        if es[off..off + 2] == [0x0B, 0x77] {
            let frmsiz = (usize::from(es[off + 2] & 7) << 8) | usize::from(es[off + 3]);
            let len = (frmsiz + 1) * 2;
            if es.len() >= off + len && crc16(&es[off + 2..off + len]) == 0 {
                if let Some((info, bsmod)) = bsi_bsmod(&es[off..off + len]) {
                    out.push((off, info, bsmod));
                }
                off += len;
                continue;
            }
        }
        off += 1;
    }
    out
}

/// `dec3.bsmod` per PID from a live (chunked) `StreamingTsDemux` run.
fn demuxed_bsmod(ts: &[u8]) -> BTreeMap<u16, u8> {
    let mut d = StreamingTsDemux::new();
    let mut out = BTreeMap::new();
    for c in ts.chunks(188 * 7) {
        d.feed(c);
    }
    d.finish();
    while let Some(e) = d.poll_event() {
        if let DemuxEvent::TrackAdded(s) = e
            && let CodecConfig::Eac3 { config, .. } = &s.config
        {
            out.insert(
                s.source_pid.expect("TS track has a PID").value(),
                config.substreams[0].bsmod,
            );
        }
    }
    out
}

/// `component_type` of the PMT `enhanced_AC-3_descriptor` (0x7A) per ES PID, for
/// `program_number` (the PMT section for it on `pmt_pid`).
fn pmt_component_types(ts: &[u8], pmt_pid: u16, program: u16) -> BTreeMap<u16, u8> {
    let mut out = BTreeMap::new();
    let mut sect = Vec::new();
    for p in packets(ts, pmt_pid) {
        let Some(pl) = payload(p) else { continue };
        if p[1] & 0x40 != 0 {
            sect.clear();
            sect.extend_from_slice(&pl[1 + usize::from(pl[0])..]);
        } else {
            sect.extend_from_slice(pl);
        }
        if sect.len() < 3 {
            continue;
        }
        let len = 3 + (usize::from(sect[1] & 0x0F) << 8 | usize::from(sect[2]));
        if sect.len() < len || sect[0] != 0x02 || u16::from_be_bytes([sect[3], sect[4]]) != program
        {
            continue;
        }
        let pil = usize::from(sect[10] & 0x0F) << 8 | usize::from(sect[11]);
        let mut at = 12 + pil;
        while at + 5 <= len - 4 {
            let pid = u16::from(sect[at + 1] & 0x1F) << 8 | u16::from(sect[at + 2]);
            let il = usize::from(sect[at + 3] & 0x0F) << 8 | usize::from(sect[at + 4]);
            let mut d = at + 5;
            while d + 2 <= at + 5 + il {
                let (tag, dl) = (sect[d], usize::from(sect[d + 1]));
                // Table D.7: flags byte (component_type_flag is the MSB), then
                // component_type when that flag is set.
                if tag == 0x7A && dl >= 2 && sect[d + 2] & 0x80 != 0 {
                    out.insert(pid, sect[d + 3]);
                }
                d += 2 + dl;
            }
            at += 5 + il;
        }
        return out;
    }
    out
}

fn check_capture(name: &str, rel: &str) {
    let Some(path) = test_fixtures::require(rel, WHO) else {
        return;
    };
    let ts = std::fs::read(path).unwrap();

    // 1. The independent parse: constant where carried, truth as documented.
    for (pid, want) in TRUTH {
        let frames = intact_frames(&es_of(&ts, pid));
        let carried: Vec<u8> = frames.iter().filter_map(|f| f.2).collect();
        assert!(
            !carried.is_empty(),
            "{name} PID {pid:#x}: no intact frame carries bsmod"
        );
        assert!(
            carried.iter().all(|&b| b == carried[0]),
            "{name} PID {pid:#x}: bsmod is not constant across intact frames: {carried:?}"
        );
        assert_eq!(carried[0], want, "{name} PID {pid:#x}: independent bsmod");
    }
    // The audio-description track really alternates infomdate (the hazard).
    let vi = intact_frames(&es_of(&ts, 0x83));
    assert!(vi.iter().any(|f| f.1) && vi.iter().any(|f| !f.1), "{vi:?}");

    // 2. The PMT (program 0x0401, PAT-referenced) agrees: service type flags
    //    (component_type bits 5..3, Table D.4) equal bsmod (Table 4.2).
    let ct = pmt_component_types(&ts, 0x64, 0x0401);
    for (pid, want) in TRUTH {
        let component_type = ct[&pid];
        assert_eq!(component_type & 0xC0, 0xC0, "Enhanced AC-3, full service");
        assert_eq!(
            (component_type >> 3) & 7,
            want,
            "{name} PID {pid:#x}: PMT service type vs bsmod"
        );
    }

    // 3. The demuxer's dec3 equals it, per track.
    let got = demuxed_bsmod(&ts);
    for (pid, want) in TRUTH {
        assert_eq!(
            got.get(&pid),
            Some(&want),
            "{name} PID {pid:#x}: dec3.bsmod"
        );
    }
}

#[test]
fn public_m6_single_dec3_bsmod_matches_the_intact_frames_and_the_pmt() {
    check_capture("m6-single", "ts/m6-single.ts");
}

#[test]
fn zenith_capture_dec3_bsmod_matches_the_intact_frames_and_the_pmt() {
    check_capture("zenith", "ts/france-tnt-m6-eac3-zenith.ts");
}

/// Facts behind "service 0x0601 / PID 0x8D produced no tracks" (see the
/// `ts_demux` module docs, "Programs and declared-but-silent PIDs"): the PAT
/// references only program 0x0401, the PMT PID carries a second PMT for 0x0601
/// that the PAT never points at, and PIDs 0x8D (0x0401's HD DVB subtitles) and
/// 0x96/0x97 (0x0601's own subtitles) have no packets at all. Nothing present
/// in the stream is dropped.
fn check_programs(name: &str, rel: &str) {
    let Some(path) = test_fixtures::require(rel, WHO) else {
        return;
    };
    let ts = std::fs::read(path).unwrap();

    // PAT: program numbers it lists (PID 0, single section).
    let mut programs = Vec::new();
    for p in packets(&ts, 0) {
        let pl = payload(p).unwrap();
        if p[1] & 0x40 == 0 {
            continue;
        }
        let s = &pl[1 + usize::from(pl[0])..];
        let end = 3 + (usize::from(s[1] & 0x0F) << 8 | usize::from(s[2])) - 4;
        for e in s[8..end].chunks_exact(4) {
            programs.push(u16::from_be_bytes([e[0], e[1]]));
        }
    }
    programs.sort_unstable();
    programs.dedup();
    assert_eq!(programs, [0x0401], "{name}: PAT lists only program 0x0401");

    // The PMT PID also carries a PMT for 0x0601, which differs from 0x0401's on
    // the same PIDs (it swaps the qaa/qad component types of 0x83/0x84), so
    // following it would have contradicted the PAT-referenced program.
    let other = pmt_component_types(&ts, 0x64, 0x0601);
    assert_eq!(other.get(&0x83), Some(&0xC2), "{name}: 0x0601 PMT present");
    assert_eq!(other.get(&0x84), Some(&0xD2));
    assert_eq!(pmt_component_types(&ts, 0x64, 0x0401)[&0x83], 0xD2);

    // No media packets for the declared-but-silent PIDs.
    for pid in [0x8D, 0x96, 0x97] {
        assert_eq!(
            packets(&ts, pid).count(),
            0,
            "{name}: PID {pid:#x} is silent"
        );
    }

    // Every announced track belongs to the one PAT-referenced program.
    let mut d = StreamingTsDemux::new();
    d.feed(&ts);
    d.finish();
    let mut announced = Vec::new();
    while let Some(e) = d.poll_event() {
        if let DemuxEvent::TrackAdded(s) = e {
            assert_eq!(s.program_number.map(|p| p.value()), Some(0x0401));
            announced.push(s.source_pid.unwrap().value());
        }
    }
    assert!(
        !announced.contains(&0x8D),
        "{name}: silent PID not announced"
    );
    for pid in AUDIO_PIDS {
        assert!(announced.contains(&pid), "{name}: {pid:#x} announced");
    }
}

#[test]
fn public_m6_single_programs_and_silent_pids() {
    check_programs("m6-single", "ts/m6-single.ts");
}

#[test]
fn zenith_programs_and_silent_pids() {
    check_programs("zenith", "ts/france-tnt-m6-eac3-zenith.ts");
}
