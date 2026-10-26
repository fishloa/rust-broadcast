//! The live client against REAL captures: the ffmpeg live MPDs and segments
//! exactly as written (samples compared with a direct `Fmp4Demux` of the same
//! files), and the real livesim2 MPD refresh pair.

mod common;

use std::collections::BTreeMap;

use common::capture::{CaptureOrigin, Livesim2Pair};
use common::loopback::{Sig, oracle, sigs};
use common::synth::Pool;
use common::world::{Kind, MICROS, Sim, World};
use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::client::{DashConfig, Output, PullLimits};
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::RepRef;
use dash_runtime::timing::datetime::parse_xs_datetime;

fn ts(text: &str) -> i64 {
    parse_xs_datetime(text).unwrap().unix_us
}

fn samples_by_track(sim: &Sim) -> BTreeMap<u32, Vec<Sig>> {
    let mut m: BTreeMap<u32, Vec<Sig>> = BTreeMap::new();
    for o in &sim.outputs {
        if let Output::Samples { track_id, samples } = o {
            m.entry(track_id.value()).or_default().extend(sigs(samples));
        }
    }
    m
}

fn requested_files(sim: &Sim, kind: Kind) -> Vec<String> {
    sim.requests(kind)
        .iter()
        .map(|l| l.url.rsplit('/').next().unwrap().to_owned())
        .collect()
}

#[test]
fn the_ffmpeg_number_capture_plays_exactly_the_segments_the_clock_selects() {
    let ast = ts("2026-10-26T09:18:04.725Z");
    let mut origin = CaptureOrigin::new("ffmpeg-number", "live-number");
    for n in 5..=8i64 {
        let at = ast + 2 * MICROS * n;
        origin = origin
            .available(&format!("seg-0-{n:03}.m4s"), at)
            .available(&format!("seg-1-{n:03}.m4s"), at);
    }
    let url = origin.url("live.mpd");
    let mut sim = Sim::new(
        &url,
        DashConfig::default(),
        PullLimits::default(),
        World::new(ast + 15_300_000),
        Box::new(origin),
    );
    sim.give_host_clock();
    sim.run_until(1_300_000);
    assert!(sim.fatal().is_none(), "{:?}", sim.fatal());
    let mut segments = requested_files(&sim, Kind::Segment);
    segments.sort();
    assert_eq!(
        segments,
        [
            "seg-0-007.m4s",
            "seg-0-008.m4s",
            "seg-1-007.m4s",
            "seg-1-008.m4s"
        ],
        "T_join = utc - SPD - AST = 13.34 s lies in segment 7; 8 completes at AST + 16 s"
    );
    let seg8 = sim
        .requests(Kind::Segment)
        .into_iter()
        .find(|l| l.url.ends_with("seg-0-008.m4s"))
        .unwrap();
    assert!(
        seg8.send_utc_us >= ast + 16 * MICROS,
        "segment 8 is not requested before it is complete"
    );
    let got = samples_by_track(&sim);
    let video = oracle(&[
        "live-number/init-0.m4s",
        "live-number/seg-0-007.m4s",
        "live-number/seg-0-008.m4s",
    ]);
    let audio = oracle(&[
        "live-number/init-1.m4s",
        "live-number/seg-1-007.m4s",
        "live-number/seg-1-008.m4s",
    ]);
    assert_eq!(
        got.get(&1),
        Some(&video[0]),
        "video samples equal the demux oracle"
    );
    assert_eq!(
        got.get(&2),
        Some(&audio[0]),
        "audio samples equal the demux oracle"
    );
}

#[test]
fn the_ffmpeg_timeline_capture_joins_on_s_t_not_on_tfdt() {
    let ast = ts("2026-10-26T09:18:20.766Z");
    let publish = ts("2026-10-26T09:18:36.438Z");
    let mut origin = CaptureOrigin::new("ffmpeg-timeline", "live-timeline");
    let micros = |ticks: i64, scale: i64| ticks * MICROS / scale;
    for k in 0..4i64 {
        let start = 365_853 + k * 273_763;
        let at = ast + micros(start + 273_763, 90_000);
        origin = origin.available(&format!("seg-0-{:03}.m4s", 3 + k), at);
    }
    let audio = [
        (353_280i64, 89_087i64),
        (442_367, 89_088),
        (531_455, 89_089),
        (620_544, 89_088),
    ];
    for (k, (t, d)) in audio.iter().enumerate() {
        let at = ast + micros(t + d, 44_100);
        origin = origin.available(&format!("seg-1-{:03}.m4s", 5 + k), at);
    }
    let url = origin.url("live.mpd");
    let mut sim = Sim::new(
        &url,
        DashConfig::default(),
        PullLimits::default(),
        World::new(publish),
        Box::new(origin),
    );
    sim.give_host_clock();
    sim.run_until(1_700_000);
    assert!(sim.fatal().is_none(), "{:?}", sim.fatal());
    let mut segments = requested_files(&sim, Kind::Segment);
    segments.sort();
    assert_eq!(
        segments,
        [
            "seg-0-005.m4s",
            "seg-0-006.m4s",
            "seg-1-007.m4s",
            "seg-1-008.m4s"
        ],
        "video joins in 5 and audio in 7: boundaries of each Representation, not by number"
    );
    let got = samples_by_track(&sim);
    let video = oracle(&[
        "live-timeline/init-0.m4s",
        "live-timeline/seg-0-005.m4s",
        "live-timeline/seg-0-006.m4s",
    ]);
    let aud = oracle(&[
        "live-timeline/init-1.m4s",
        "live-timeline/seg-1-007.m4s",
        "live-timeline/seg-1-008.m4s",
    ]);
    assert_eq!(got.get(&1), Some(&video[0]));
    assert_eq!(got.get(&2), Some(&aud[0]));
}

fn pair() -> (Livesim2Pair, i64, i64) {
    let a = test_fixtures::read_to_string("dash/livesim2/segtimeline_time_a.mpd");
    let b = test_fixtures::read_to_string("dash/livesim2/segtimeline_time_b.mpd");
    let mut avail = BTreeMap::new();
    for text in [&a, &b] {
        let mpd = Mpd::parse(text.as_bytes()).unwrap();
        for (set, name) in [(0usize, "A48"), (1usize, "V300")] {
            let planned =
                plan_representation(&mpd, RepRef::at(0, set, 0), &PlanOptions::default()).unwrap();
            let scale = planned.plan.timescale() as i64;
            for s in planned.plan.iter() {
                let end = s.time_ticks as i64 + s.duration_ticks as i64;
                avail.insert(
                    (name.to_owned(), s.time_ticks),
                    (i128::from(end) * i128::from(MICROS) / i128::from(scale)) as i64,
                );
            }
        }
    }
    let publish_a = Mpd::parse(a.as_bytes()).unwrap().publish_time_us.unwrap();
    let publish_b = Mpd::parse(b.as_bytes()).unwrap().publish_time_us.unwrap();
    (
        Livesim2Pair {
            a,
            b,
            switch_utc_us: publish_b,
            avail_utc_us: avail,
            pool: Pool::load(),
            segment_requests: Vec::new(),
        },
        publish_a,
        publish_b,
    )
}

#[test]
fn the_real_livesim2_refresh_pair_plays_across_the_update_without_gap_or_early_request() {
    let (origin, publish_a, publish_b) = pair();
    let handle = std::rc::Rc::new(std::cell::RefCell::new(origin));
    let mut sim = Sim::new(
        Livesim2Pair::url(),
        DashConfig::default(),
        PullLimits::default(),
        World::new(publish_a + 100_000),
        Box::new(common::world::Shared(handle.clone())),
    );
    sim.run_until((publish_b - publish_a) as u64 + 4 * MICROS as u64);
    assert!(sim.fatal().is_none(), "{:?}", sim.fatal());
    let o = handle.borrow();
    let video: Vec<(u64, i64, u16)> = o
        .segment_requests
        .iter()
        .filter(|r| r.0 == "V300")
        .map(|r| (r.1, r.2, r.3))
        .collect();
    let ok: Vec<u64> = video.iter().filter(|r| r.2 == 200).map(|r| r.0).collect();
    assert_eq!(
        ok.len(),
        5,
        "3 segments of a up to the live edge, 2 added by b: {ok:?}"
    );
    let mut sorted = ok.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ok.len(), "no segment twice");
    for w in sorted.windows(2) {
        assert_eq!(
            w[1] - w[0],
            180_000,
            "one 2 s video segment after the other, none skipped"
        );
    }
    assert_eq!(
        sorted.last(),
        Some(&161_274_106_080_000),
        "the newest segment MPD b lists was played: the refresh to b was applied"
    );
    assert!(
        video.iter().all(|r| r.2 == 200),
        "never early, never missing: {video:?}"
    );
    let audio_ok: Vec<u64> = o
        .segment_requests
        .iter()
        .filter(|r| r.0 == "A48" && r.3 == 200)
        .map(|r| r.1)
        .collect();
    let mut a_sorted = audio_ok.clone();
    a_sorted.sort_unstable();
    for w in a_sorted.windows(2) {
        let d = w[1] - w[0];
        assert!(d == 96_256 || d == 95_232, "audio step {d}");
    }
    assert!(sim.count(|o| matches!(o, Output::Discontinuity)) == 0);
}
