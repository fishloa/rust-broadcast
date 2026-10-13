//! SRT push delivers ONE continuous MPEG-2 TS, not one restarted TS per frame
//! (issue #1024, audit r07-C1).
//!
//! Before the fix every pushed sample went through a fresh one-shot
//! `TsMux::package`, so the receiver saw PAT/PMT before every frame, the
//! continuity counters restarting at 0 and the same PTS/PCR on every frame.
//! Here a committed fixture is demuxed, fed through the **real**
//! `PushTransportEgress<SrtTransport>` one sample at a time (exactly what a
//! live push output does) into a loopback `SrtListener`, and the bytes the
//! listener receives are checked with
//!
//! - a minimal independent packet walker: continuity counters valid on every
//!   PID, PAT/PMT no more often than the repetition interval (and at least
//!   every 500 ms), per-PID DTS strictly monotonic and equal to the source
//!   timeline (including the cross-track offset), and
//! - the independent ETSI TR 101 290 monitor (`dvb-conformance`).
//!
//! A sample-by-sample feed must also equal a whole-`Media` mux packet for
//! packet on every PID; only the PAT/PMT placement may differ.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use dvb_conformance::{ConformanceMonitor, Indicator};
use media_plane::PushEgress;
use media_plane::SampleCursorItem;
use multimux::push::{PushTransport, SrtTransport, SrtTransportConfig};
use srt_runtime::HandshakeConfig;
use srt_runtime::io::SrtListener;
use transmux::{Media, Sample, StreamingTsMux, Track, TsDemux, TsMux};

use broadcast_common::{Package, Unpackage};

const PKT: usize = 188;
const PAT_PID: u16 = 0x0000;
const PMT_PID: u16 = 0x1000;
const TS_HZ: i128 = 90_000;
/// Receiver idle time that ends a capture (the sender is finished by then).
const IDLE: Duration = Duration::from_millis(1500);
const GUARD: Duration = Duration::from_secs(60);
/// The PSI interval the default muxer promises, plus one sample of slack.
const PSI_BOUND_TICKS: u64 = 500 * 90;

fn fixture(rel: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {rel}: {e}"))
}

fn demux(rel: &str) -> Media {
    TsDemux::new().unpackage(&fixture(rel)).expect("demux")
}

/// Every sample of `media`, in decode order across tracks (what a live feed
/// delivers), as `(track index, sample)`.
fn interleaved(media: &Media) -> Vec<(usize, Sample)> {
    let mut all: Vec<(i128, usize, Sample)> = Vec::new();
    for (ti, t) in media.tracks.iter().enumerate() {
        let ts = i128::from(t.spec.timescale);
        for s in &t.samples {
            let dts = i128::from(s.dts.expect("timed sample"));
            all.push((dts * TS_HZ / ts, ti, s.clone()));
        }
    }
    all.sort_by_key(|(k, ti, _)| (*k, *ti));
    all.into_iter().map(|(_, ti, s)| (ti, s)).collect()
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
    assert_eq!(ts.len() % PKT, 0, "received bytes are whole TS packets");
    ts.chunks_exact(PKT)
        .map(|c| {
            assert_eq!(c[0], 0x47, "sync byte");
            Pkt(c)
        })
        .collect()
}

fn read_ts33(b: &[u8]) -> u64 {
    (u64::from(b[0] >> 1) & 0x07) << 30
        | u64::from(b[1]) << 22
        | (u64::from(b[2]) >> 1) << 15
        | u64::from(b[3]) << 7
        | u64::from(b[4]) >> 1
}

/// `(pts, dts)` of the PES that starts in `p` (DTS falls back to PTS).
fn pes_times(p: &Pkt<'_>) -> Option<(u64, u64)> {
    let pl = p.payload();
    if !p.pusi() || pl.len() < 14 || pl[..3] != [0, 0, 1] {
        return None;
    }
    let flags = pl[7] >> 6;
    if flags < 2 {
        return None;
    }
    let pts = read_ts33(&pl[9..14]);
    let dts = if flags == 3 {
        read_ts33(&pl[14..19])
    } else {
        pts
    };
    Some((pts, dts))
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
            assert_eq!(p.cc(), want, "pid {:#x} packet {i}: CC broke", p.pid());
        }
        last.insert(p.pid(), p.cc());
    }
}

/// PCR-clock time (90 kHz) of each PAT start.
fn pat_times(ts: &[u8]) -> Vec<u64> {
    let mut now = 0u64;
    let mut out = Vec::new();
    for p in packets(ts) {
        if let Some(t) = p.pcr90() {
            now = t;
        }
        if p.pid() == PAT_PID && p.pusi() {
            out.push(now);
        }
    }
    out
}

/// Per-PID packet lists with PAT/PMT and PCR-only packets dropped.
fn es_packets(ts: &[u8]) -> BTreeMap<u16, Vec<Vec<u8>>> {
    let mut m: BTreeMap<u16, Vec<Vec<u8>>> = BTreeMap::new();
    for p in packets(ts) {
        if p.pid() == PAT_PID || p.pid() == PMT_PID || !p.has_payload() {
            continue;
        }
        m.entry(p.pid()).or_default().push(p.0.to_vec());
    }
    m
}

/// Push `samples` through the real SRT push egress into a loopback listener
/// and return every byte the listener received.
async fn push_over_srt(media: &Media, samples: &[(usize, Sample)]) -> Vec<u8> {
    let mut listener = SrtListener::bind(
        "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap(),
        HandshakeConfig::default(),
    )
    .await
    .expect("bind");
    let bound = listener.local_addr().expect("addr");

    let receiver = tokio::spawn(async move {
        let mut sock = tokio::time::timeout(GUARD, listener.accept())
            .await
            .expect("accept must not hang")
            .expect("accept");
        let mut got = Vec::new();
        loop {
            match tokio::time::timeout(IDLE, sock.recv()).await {
                Ok(Ok(Some(data))) => got.extend_from_slice(&data),
                Ok(Ok(None)) | Err(_) => break,
                Ok(Err(e)) => panic!("recv failed: {e}"),
            }
        }
        got
    });

    let url = format!("srt://{bound}?latency=120");
    let transport = tokio::time::timeout(
        GUARD,
        SrtTransport::connect(&url, &SrtTransportConfig::default()),
    )
    .await
    .expect("connect must not hang")
    .expect("connect");
    let mut egress = multimux::push::PushTransportEgress::new(transport, "no carriable track");
    let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let ids = specs.iter().map(|s| s.track_id).collect::<Vec<_>>();
    assert!(matches!(
        egress.negotiate(&specs),
        media_plane::NegotiationOutcome::Accepted(_)
    ));
    for (ti, sample) in samples {
        let item = SampleCursorItem::Timed {
            track_id: ids[*ti],
            sample: sample.clone(),
        };
        egress.send(&item).expect("encode");
        egress.flush_transmit().await.expect("flush");
    }
    tokio::time::timeout(GUARD, receiver)
        .await
        .expect("receiver must finish")
        .expect("receiver task")
}

/// Source timeline in 90 kHz relative to the earliest first DTS.
fn expected_dts(media: &Media) -> Vec<Vec<f64>> {
    let origin = media
        .tracks
        .iter()
        .map(|t| {
            let d = t.samples[0].dts.unwrap() as f64;
            d * 90_000.0 / f64::from(t.spec.timescale)
        })
        .fold(f64::MAX, f64::min);
    media
        .tracks
        .iter()
        .map(|t| {
            t.samples
                .iter()
                .map(|s| s.dts.unwrap() as f64 * 90_000.0 / f64::from(t.spec.timescale) - origin)
                .collect()
        })
        .collect()
}

async fn check_fixture(rel: &str) {
    let media = demux(rel);
    assert!(media.tracks.len() >= 2, "{rel}: want video+audio");
    let samples = interleaved(&media);
    let got = push_over_srt(&media, &samples).await;
    assert!(!got.is_empty(), "{rel}: nothing received");

    // 1. Continuity counters valid on every PID across the whole push.
    assert_cc_valid(&got);

    // 2. PAT/PMT: not per frame, but at least every 500 ms of media time.
    let pats = pat_times(&got);
    assert!(
        pats.len() * 4 < samples.len(),
        "{rel}: {} PATs for {} samples — PSI per frame",
        pats.len(),
        samples.len()
    );
    let max_gap = pats.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0);
    assert!(max_gap <= PSI_BOUND_TICKS, "{rel}: PAT gap {max_gap} ticks");
    assert_eq!(
        pats.len(),
        packets(&got)
            .iter()
            .filter(|p| p.pid() == PMT_PID && p.pusi())
            .count(),
        "{rel}: PAT/PMT travel together"
    );

    // 3. Per-PID DTS strictly monotonic and equal to the source timeline,
    //    including the cross-track offset (one shared origin).
    let mut by_pid: BTreeMap<u16, Vec<(u64, u64)>> = BTreeMap::new();
    for p in packets(&got) {
        if let Some(t) = pes_times(&p) {
            by_pid.entry(p.pid()).or_default().push(t);
        }
    }
    assert_eq!(by_pid.len(), media.tracks.len(), "{rel}: one PID per track");
    let want = expected_dts(&media);
    let mut shift: Option<f64> = None;
    for (times, want_t) in by_pid.values().zip(&want) {
        assert_eq!(times.len(), want_t.len(), "{rel}: one PES per sample");
        assert!(
            times.windows(2).all(|w| w[1].1 > w[0].1),
            "{rel}: DTS not strictly monotonic"
        );
        for ((_, dts), w) in times.iter().zip(want_t) {
            let s = *dts as f64 - w;
            let base = *shift.get_or_insert(s);
            assert!(
                (s - base).abs() <= 1.5,
                "{rel}: DTS {dts} off the source timeline (shift {s} vs {base})"
            );
        }
    }

    // 4. Independent TR 101 290 monitor: PAT/PMT/CC/CRC clean.
    let mut mon = ConformanceMonitor::new();
    let mut now = 0u64;
    let mut bad = Vec::new();
    for p in packets(&got) {
        if let Some(t) = p.pcr90() {
            now = t;
        }
        let at = Duration::from_micros(now * 1_000_000 / 90_000);
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
    assert!(bad.is_empty(), "{rel}: TR 101 290: {bad:?}");

    // 5. Sample-by-sample == whole-Media, per PID, byte for byte.
    let whole = TsMux::new().package(&media).expect("whole mux");
    assert_eq!(
        es_packets(&got),
        es_packets(&whole),
        "{rel}: per-sample push differs from a whole-Media mux beyond PSI placement"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn srt_push_av_offset_is_one_continuous_ts() {
    check_fixture("../transmux/tests/fixtures/timeline/av_offset.ts").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn srt_push_h264_aac_is_one_continuous_ts() {
    check_fixture("../fixtures/ts/h264_aac.ts").await;
}

/// The transport-level pin without a socket: feeding `StreamingTsMux` sample by
/// sample is the default `encode_media` path's whole behaviour.
#[test]
fn streaming_mux_sample_by_sample_equals_whole_media() {
    let media = demux("../fixtures/ts/h264_aac.ts");
    let mut mux = StreamingTsMux::new();
    let mut out = Vec::new();
    for (ti, s) in interleaved(&media) {
        let tracks: Vec<Track> = media
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                Track::new(
                    t.spec.clone(),
                    if i == ti { vec![s.clone()] } else { vec![] },
                )
            })
            .collect();
        out.extend_from_slice(
            &mux.package(&Media::new(tracks, media.movie_timescale))
                .unwrap(),
        );
    }
    let whole = TsMux::new().package(&media).unwrap();
    assert_cc_valid(&out);
    assert_eq!(es_packets(&out), es_packets(&whole));
}
