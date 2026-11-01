//! `StreamingTsMux` (issue #1024): feeding a live stream one sample at a time
//! must give ONE continuous transport stream — continuity counters carried on,
//! PAT/PMT periodic rather than per call, one shared time origin — and the
//! same per-PID packets as muxing the whole `Media` at once.
#![cfg(feature = "std")]

use std::collections::BTreeMap;
use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use transmux::{Media, Sample, StreamingTsMux, Track, TsDemux, TsMux};

const PKT: usize = 188;
const PAT_PID: u16 = 0;
const PMT_PID: u16 = 0x1000;

fn media(rel: &str) -> Media {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    TsDemux::new()
        .unpackage(&std::fs::read(p).expect("fixture"))
        .expect("demux")
}

fn pid(p: &[u8]) -> u16 {
    (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])
}

/// Decode-ordered `(track, sample)` feed.
fn feed(m: &Media) -> Vec<(usize, Sample)> {
    let mut v: Vec<(i128, usize, Sample)> = Vec::new();
    for (ti, t) in m.tracks.iter().enumerate() {
        for s in &t.samples {
            let k = i128::from(s.dts.unwrap()) * 90_000 / i128::from(t.spec.timescale);
            v.push((k, ti, s.clone()));
        }
    }
    v.sort_by_key(|(k, ti, _)| (*k, *ti));
    v.into_iter().map(|(_, ti, s)| (ti, s)).collect()
}

fn es(ts: &[u8]) -> BTreeMap<u16, Vec<&[u8]>> {
    let mut m: BTreeMap<u16, Vec<&[u8]>> = BTreeMap::new();
    for p in ts.chunks_exact(PKT) {
        let id = pid(p);
        if id != PAT_PID && id != PMT_PID && p[3] & 0x10 != 0 {
            m.entry(id).or_default().push(p);
        }
    }
    m
}

fn run(mux: &mut StreamingTsMux, m: &Media) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let samples = feed(m);
    for (ti, s) in &samples {
        let tracks = m
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                Track::new(
                    t.spec.clone(),
                    if i == *ti { vec![s.clone()] } else { vec![] },
                )
            })
            .collect();
        out.extend_from_slice(&mux.package(&Media::new(tracks, m.movie_timescale)).unwrap());
    }
    (out, samples.len())
}

#[test]
fn per_sample_feed_is_one_stream_equal_to_the_whole_media_mux() {
    for rel in [
        "../fixtures/ts/h264_aac.ts",
        "tests/fixtures/timeline/av_offset.ts",
    ] {
        let m = media(rel);
        let (out, n) = run(&mut StreamingTsMux::new(), &m);
        let whole = TsMux::new().package(&m).unwrap();

        // Same packets on every PID as the one-shot mux (this also pins the CC
        // sequence and the PTS/DTS/PCR bytes, which live inside them).
        assert_eq!(es(&out), es(&whole), "{rel}");

        // PSI: written, but periodically — nowhere near once per sample.
        let pats = out
            .chunks_exact(PKT)
            .filter(|p| pid(p) == PAT_PID && p[1] & 0x40 != 0)
            .count();
        assert!(pats >= 1 && pats * 4 < n, "{rel}: {pats} PATs, {n} samples");

        // PAT/PMT continuity counters run on across calls.
        for want in [PAT_PID, PMT_PID] {
            let ccs: Vec<u8> = out
                .chunks_exact(PKT)
                .filter(|p| pid(p) == want)
                .map(|p| p[3] & 0x0F)
                .collect();
            assert!(
                ccs.windows(2).all(|w| w[1] == (w[0] + 1) & 0x0F),
                "{rel}: pid {want:#x} CC {ccs:?}"
            );
        }
    }
}

#[test]
fn psi_once_only_when_interval_is_none() {
    let m = media("../fixtures/ts/h264_aac.ts");
    let (out, _) = run(&mut StreamingTsMux::with_psi_interval_ms(None), &m);
    let pats = out.chunks_exact(PKT).filter(|p| pid(p) == PAT_PID).count();
    assert_eq!(pats, 1);
}
