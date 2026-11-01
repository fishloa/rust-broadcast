//! `TsMux` PAT/PMT repetition (issue #1162 R7): the PSI must recur through the
//! output so a stream cut at any packet-aligned offset can still be joined
//! (ETSI TR 101 290 indicators 1.3/1.5 want PAT/PMT at least every 0.5 s).
//!
//! Inputs are the committed real captures under `fixtures/ts/`; the muxer is fed
//! what `TsDemux` recovers from them and its output is parsed back here with a
//! small independent packet walker, then also fed to the independent
//! `dvb-conformance` TR 101 290 monitor.
#![cfg(feature = "std")]

use std::collections::BTreeMap;
use std::time::Duration;

use broadcast_common::{Package, Unpackage};
use dvb_conformance::{ConformanceMonitor, Indicator};
use transmux::{Media, TsDemux, TsMux};

const PKT: usize = 188;
const PAT_PID: u16 = 0x0000;
const PMT_PID: u16 = 0x1000;
const TICKS_PER_SEC: u64 = 90_000;
/// TR 101 290 1.3/1.5 bound, in 90 kHz ticks.
const TR101290_MAX_TICKS: u64 = TICKS_PER_SEC / 2;
/// Slack on a configured interval: a repeat lands on the next sample boundary,
/// so it can be up to one inter-sample gap late (<= 100 ms for these streams).
const SAMPLE_SLACK_TICKS: u64 = TICKS_PER_SEC / 10;
/// Leading bytes of a PSI section compared across repeats (table_id through
/// the section_number fields: covers version_number).
const SECTION_HEAD: usize = 8;

fn fixture(rel: &str) -> Vec<u8> {
    test_fixtures::read(&format!("ts/{rel}"))
}

fn demux(bytes: &[u8]) -> Media {
    TsDemux::new().unpackage(bytes).expect("demux")
}

struct Pkt<'a>(&'a [u8]);

impl Pkt<'_> {
    fn pid(&self) -> u16 {
        (u16::from(self.0[1] & 0x1F) << 8) | u16::from(self.0[2])
    }
    fn pusi(&self) -> bool {
        self.0[1] & 0x40 != 0
    }
    fn cc(&self) -> u8 {
        self.0[3] & 0x0F
    }
    fn has_payload(&self) -> bool {
        self.0[3] & 0x10 != 0
    }
    /// PCR base in 90 kHz ticks, if the adaptation field carries a PCR.
    fn pcr90(&self) -> Option<u64> {
        if self.0[3] & 0x20 == 0 || self.0[4] < 7 || self.0[5] & 0x10 == 0 {
            return None;
        }
        let b = &self.0[6..11];
        Some(
            (u64::from(b[0]) << 25)
                | (u64::from(b[1]) << 17)
                | (u64::from(b[2]) << 9)
                | (u64::from(b[3]) << 1)
                | u64::from(b[4] >> 7),
        )
    }
    fn payload(&self) -> &[u8] {
        let start = if self.0[3] & 0x20 != 0 {
            5 + usize::from(self.0[4])
        } else {
            4
        };
        &self.0[start.min(PKT)..]
    }
}

fn packets(ts: &[u8]) -> Vec<Pkt<'_>> {
    assert_eq!(ts.len() % PKT, 0);
    ts.chunks_exact(PKT)
        .map(|c| {
            assert_eq!(c[0], 0x47);
            Pkt(c)
        })
        .collect()
}

/// (PCR-derived media time in 90 kHz ticks, section head) of each PSI section
/// start on `pid`.
fn psi_occurrences(ts: &[u8], pid: u16) -> Vec<(u64, Vec<u8>)> {
    let mut now = 0u64;
    let mut out = Vec::new();
    for p in packets(ts) {
        if let Some(t) = p.pcr90() {
            now = t;
        }
        if p.pid() == pid && p.pusi() {
            let pl = p.payload();
            let ptr = usize::from(pl[0]);
            out.push((now, pl[1 + ptr..1 + ptr + SECTION_HEAD].to_vec()));
        }
    }
    out
}

fn max_gap(occ: &[(u64, Vec<u8>)]) -> u64 {
    occ.windows(2).map(|w| w[1].0 - w[0].0).max().unwrap_or(0)
}

fn interval_bound(ms: u32) -> u64 {
    u64::from(ms) * TICKS_PER_SEC / 1000 + SAMPLE_SLACK_TICKS
}

fn assert_cc_valid(ts: &[u8]) {
    let mut last: BTreeMap<u16, u8> = BTreeMap::new();
    for (i, p) in packets(ts).iter().enumerate() {
        if let Some(&prev) = last.get(&p.pid()) {
            let want = if p.has_payload() {
                (prev + 1) & 0x0F
            } else {
                prev
            };
            assert_eq!(p.cc(), want, "pid {:#x} packet {i}: bad CC", p.pid());
        }
        last.insert(p.pid(), p.cc());
    }
}

const FIXTURES: &[&str] = &["h264_aac.ts", "h264_aac_40s.ts"];

#[test]
fn default_repeats_pat_and_pmt_within_the_interval() {
    for &name in FIXTURES {
        let media = demux(&fixture(name));
        let out = TsMux::new().package(&media).unwrap();
        for pid in [PAT_PID, PMT_PID] {
            let occ = psi_occurrences(&out, pid);
            assert!(occ.len() > 2, "{name} pid {pid:#x}: only {} PSI", occ.len());
            let gap = max_gap(&occ);
            assert!(
                gap <= interval_bound(transmux::DEFAULT_PSI_INTERVAL_MS),
                "{name} pid {pid:#x}: gap {gap}"
            );
            assert!(gap <= TR101290_MAX_TICKS);
        }
        assert_cc_valid(&out);
    }
}

#[test]
fn configured_interval_is_honoured() {
    let media = demux(&fixture("h264_aac_40s.ts"));
    let mux = |iv| {
        TsMux::new()
            .with_psi_interval_ms(iv)
            .package(&media)
            .unwrap()
    };
    let pats = |ts: &[u8]| psi_occurrences(ts, PAT_PID);
    assert_eq!(pats(&mux(Some(0))).len(), 1, "Some(0) means once-only");
    assert_eq!(pats(&mux(None)).len(), 1);
    let (fast, slow) = (pats(&mux(Some(100))), pats(&mux(Some(400))));
    assert!(fast.len() > slow.len() && slow.len() > 1);
    assert!(max_gap(&slow) <= interval_bound(400));
    assert_cc_valid(&mux(Some(400)));
}

#[test]
fn repeats_are_identical_sections_with_unchanged_pmt_version() {
    let media = demux(&fixture("h264_aac_40s.ts"));
    let out = TsMux::new().package(&media).unwrap();
    for pid in [PAT_PID, PMT_PID] {
        let occ = psi_occurrences(&out, pid);
        assert!(occ.len() > 2);
        // The head covers table_id, section_length and version_number.
        assert!(occ.iter().all(|(_, s)| *s == occ[0].1), "pid {pid:#x}");
    }
}

#[test]
fn whole_file_round_trip_is_unchanged_but_for_psi_repeats() {
    for &name in FIXTURES {
        let media = demux(&fixture(name));
        let once = TsMux::new()
            .with_psi_interval_ms(None)
            .package(&media)
            .unwrap();
        let periodic = TsMux::new().package(&media).unwrap();
        // Dropping every PSI packet after the leading block reproduces the
        // once-only output byte for byte: media packets are untouched.
        let lead = packets(&once)
            .iter()
            .take_while(|p| p.pid() == PAT_PID || p.pid() == PMT_PID)
            .count();
        let stripped: Vec<u8> = periodic
            .chunks_exact(PKT)
            .enumerate()
            .filter(|(i, c)| {
                let pid = (u16::from(c[1] & 0x1F) << 8) | u16::from(c[2]);
                *i < lead || (pid != PAT_PID && pid != PMT_PID)
            })
            .flat_map(|(_, c)| c.iter().copied())
            .collect();
        assert_eq!(stripped, once, "{name}: media packets changed");
        assert!(periodic.len() > once.len());
        // And it demuxes to the identical IR.
        // Tracks (config + every sample) are identical. `Media::pcr_samples`
        // records each PCR's *packet index*, which legitimately shifts by the
        // inserted PSI packets, so it is excluded here.
        assert_eq!(
            format!("{:?}", demux(&periodic).tracks),
            format!("{:?}", demux(&once).tracks),
            "{name}"
        );
    }
}

#[test]
fn stream_cut_at_packet_aligned_offsets_still_yields_all_tracks() {
    let media = demux(&fixture("h264_aac_40s.ts"));
    let want = media.tracks.len();
    assert!(want >= 2);
    let out = TsMux::new().package(&media).unwrap();
    let once = TsMux::new()
        .with_psi_interval_ms(None)
        .package(&media)
        .unwrap();
    let n = out.len() / PKT;
    // Packet 6 = 1128 bytes in: the zenith-tv failure offset.
    for k in [6, 7, 13, 50, 99, n / 3, n / 2, 2 * n / 3] {
        assert_eq!(
            demux(&out[k * PKT..]).tracks.len(),
            want,
            "periodic PSI: cut at packet {k}"
        );
    }
    // Contrast: the once-only layout loses the programme when cut, which is the
    // failure the repetition fixes.
    assert_eq!(demux(&once[6 * PKT..]).tracks.len(), 0);
}

#[test]
fn tr101290_pat_pmt_and_continuity_checks_pass() {
    for &name in FIXTURES {
        let media = demux(&fixture(name));
        let out = TsMux::new().package(&media).unwrap();
        let mut mon = ConformanceMonitor::new();
        let mut now = 0u64;
        let mut bad: Vec<String> = Vec::new();
        for p in packets(&out) {
            if let Some(t) = p.pcr90() {
                now = t;
            }
            let at = Duration::from_micros(now * 1_000_000 / TICKS_PER_SEC);
            for ev in mon.feed(p.0, at) {
                if matches!(
                    ev.indicator,
                    Indicator::PatError2
                        | Indicator::PmtError2
                        | Indicator::ContinuityCountError
                        | Indicator::CrcError
                ) {
                    bad.push(format!("{:?} {}", ev.indicator, ev.detail));
                }
            }
        }
        assert!(bad.is_empty(), "{name}: {bad:?}");
    }
}
