//! The one TS muxer stage (issue #1024, Phase 4): feeding a live stream one
//! sample at a time must give ONE continuous transport stream — continuity
//! counters carried on, PAT/PMT periodic rather than per call, one shared time
//! origin — and the same bytes as muxing the whole `Media` at once.
#![cfg(feature = "std")]

use std::collections::BTreeMap;

use broadcast_common::stage::Timestamp;
use broadcast_common::{Stage, Unpackage};
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{DEFAULT_PSI_INTERVAL, Media, Sample, TsDemux, TsMuxStage};

use crate::ts_mux_stage;

const PKT: usize = 188;
const PAT_PID: u16 = 0;
const PMT_PID: u16 = 0x1000;

fn media(rel: &str) -> Media {
    let bytes = test_fixtures::read(rel);
    TsDemux::new().unpackage(bytes.as_slice()).expect("demux")
}

fn pid(p: &[u8]) -> u16 {
    (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])
}

/// Decode-ordered `(track index, sample)` feed.
fn feed(m: &Media) -> Vec<(usize, Sample)> {
    let mut v: Vec<(i128, usize, Sample)> = Vec::new();
    for (ti, t) in m.tracks.iter().enumerate() {
        for s in &t.samples {
            let k = i128::from(s.timing.and_then(|t| t.dts()).unwrap()) * 90_000
                / i128::from(t.spec.timescale.get());
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

fn run(mux: &mut TsMuxStage, m: &Media) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    for track in &m.tracks {
        Stage::feed(
            mux,
            MuxInput::Track(Box::new(track.spec.clone())),
            Timestamp::ZERO,
        )
        .expect("announce");
    }
    let samples = feed(m);
    for (ti, sample) in &samples {
        Stage::feed(
            mux,
            MuxInput::Sample {
                track: m.tracks[*ti].spec.track_id,
                sample: sample.clone(),
            },
            Timestamp::ZERO,
        )
        .expect("feed");
        while let Some(output) = Stage::poll(mux) {
            if let MuxOutput::Bytes(chunk) = output {
                out.extend_from_slice(&chunk);
            }
        }
    }
    Stage::finish(mux).expect("finish");
    while let Some(output) = Stage::poll(mux) {
        if let MuxOutput::Bytes(chunk) = output {
            out.extend_from_slice(&chunk);
        }
    }
    (out, samples.len())
}

#[test]
fn per_sample_feed_is_one_stream_equal_to_the_whole_media_mux() {
    for rel in ["ts/h264-aac.ts", "dash/timeline/av_offset.ts"] {
        let m = media(rel);
        let (out, n) = run(&mut TsMuxStage::new(Some(DEFAULT_PSI_INTERVAL)), &m);
        let whole = ts_mux_stage::mux(&m);

        // Byte-identical to the whole-`Media` mux: one continuous stream, so
        // the PSI schedule, the packet order and every PES byte agree.
        assert_eq!(out, whole, "{rel}");
        assert_eq!(es(&out), es(&whole), "{rel}");

        // PSI: written, but periodically — nowhere near once per sample.
        let pats = out
            .chunks_exact(PKT)
            .filter(|p| pid(p) == PAT_PID && p[1] & 0x40 != 0)
            .count();
        assert!(pats >= 1 && pats * 4 < n, "{rel}: {pats} PATs, {n} samples");

        // PAT/PMT continuity counters run on across feeds.
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
    let m = media("ts/h264-aac.ts");
    let (out, _) = run(&mut TsMuxStage::new(None), &m);
    let pats = out.chunks_exact(PKT).filter(|p| pid(p) == PAT_PID).count();
    assert_eq!(pats, 1);
}
