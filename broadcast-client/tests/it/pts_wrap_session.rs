use crate::support::replay::{bitrate_bps, replay_with_host};
use crate::support::ts_shift::{PTS_WRAP, Shift, shift_pes};
use broadcast_client::{Action, BufferEvent, Config, Event, Session};
use broadcast_common::clock33::PCR_WRAP_27MHZ;
use broadcast_common::stage::Timestamp;
use broadcast_common::{Package, Unpackage};
use transmux::{Media, TsDemux, TsMux};
use url::Url;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const WRAP: i64 = 1 << 33;

#[test]
fn pts_33bit_wrap_through_session_playout() {
    let base = test_fixtures::read("ts/h264-aac.ts");
    let media0 = TsDemux::new().unpackage(&base[..]).unwrap();
    let movie_timescale = media0.movie_timescale;

    // Keep an audio track as an origin anchor at DTS 0 so the muxer does not
    // rebase the video timeline to zero, preserving the 33-bit wrap on the wire.
    let audio_track = media0
        .tracks
        .iter()
        .find(|t| t.spec.config.audio_sample_rate().is_some())
        .cloned();

    // Build a synthetic Media whose video track crosses the 33-bit PTS wrap.
    let mut video_track = media0
        .tracks
        .into_iter()
        .find(|t| t.spec.config.is_video())
        .expect("base fixture has no video track");
    let frame_dur: i64 = 3000;
    let n = video_track.samples.len().clamp(4, 8);
    video_track.samples.truncate(n);
    for (i, s) in video_track.samples.iter_mut().enumerate() {
        let pts = (WRAP - 10000) + i as i64 * frame_dur;
        if let Some(t) = s.timing.as_mut() {
            t.pts = pts;
        } else {
            s.timing = Some(transmux::ir::SampleTiming::new(pts, pts, 0));
        }
        if let Some(t) = s.timing.as_mut() {
            t.dts = pts;
        } else {
            s.timing = Some(transmux::ir::SampleTiming::new(pts, pts, 0));
        }
        if let Some(t) = s.timing.as_mut() {
            t.duration = frame_dur as u32;
        } else {
            s.timing = Some(transmux::ir::SampleTiming::new(0, 0, frame_dur as u32));
        }
    }
    video_track.start_decode_time = video_track.samples[0].timing.map(|t| t.dts).unwrap() as u64;

    let video_track_id = video_track.spec.track_id;
    let mut tracks = vec![video_track];
    if let Some(mut a) = audio_track {
        a.samples.truncate(1);
        if let Some(s) = a.samples.first_mut() {
            if let Some(t) = s.timing.as_mut() {
                t.pts = 0;
            } else {
                s.timing = Some(transmux::ir::SampleTiming::new(0, 0, 0));
            }
            if let Some(t) = s.timing.as_mut() {
                t.dts = 0;
            } else {
                s.timing = Some(transmux::ir::SampleTiming::new(0, 0, 0));
            }
            if let Some(t) = s.timing.as_mut() {
                t.duration = 1024;
            } else {
                s.timing = Some(transmux::ir::SampleTiming::new(0, 0, 1024));
            }
        }
        tracks.push(a);
    }
    let media = Media::new(tracks, movie_timescale);
    let mut mux = TsMux::default();
    let bytes = mux.package(&media).expect("mux");
    assert!(!bytes.is_empty());

    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let conn = match session.poll(ts(0)).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!("no request"),
    };
    session.on_response(
        conn,
        http::StatusCode::OK,
        &crate::support::headers(&[("Content-Length", &bytes.len().to_string())]),
        ts(0),
    );
    for chunk in bytes.chunks(1316) {
        session.on_data(conn, chunk, ts(0));
    }
    session.on_closed(conn, None, ts(1000));

    // Collect events.
    let mut evs = Vec::new();
    while let Some(e) = session.next_event() {
        evs.push(e);
    }
    // Sample PTS monotone (video track).
    let mut last: Option<i64> = None;
    let mut saw_wrap = false;
    for e in &evs {
        if let Event::Sample { track, sample, .. } = e {
            if *track != video_track_id {
                continue;
            }
            if let Some(p) = sample.timing.map(|t| t.pts) {
                if let Some(l) = last {
                    assert!(p >= l, "non-monotone sample pts: {l} -> {p}");
                }
                if p >= WRAP {
                    saw_wrap = true;
                }
                last = Some(p);
            }
        }
    }
    // No spurious Pause/Resume across the wrap.
    for e in &evs {
        if let Event::Buffer(BufferEvent::Pause { .. }) = e {
            panic!("unexpected stall/pause during wrap: {e:?}");
        }
        if let Event::Buffer(BufferEvent::Resume { .. }) = e {
            panic!("unexpected resume during wrap: {e:?}");
        }
    }
    // Start emitted exactly once.
    let starts = evs
        .iter()
        .filter(|e| matches!(e, Event::Buffer(BufferEvent::Start { .. })))
        .count();
    assert_eq!(starts, 1, "Start not emitted exactly once: {evs:?}");
    // At least one sample after wrap.
    assert!(saw_wrap, "no sample at/after wrap boundary");
}

const JOIN_FIXTURE: &str = "ts/h264-aac-40s.ts";
const JOIN_CHUNK: usize = 188 * 7;
const TICK_US: u64 = 20_000;
const TICKS_27_PER_90K: u64 = 300;
/// Audio muxed 1 s behind video, the usual TS shape (DESIGN.md 0.1c).
const AUDIO_BEHIND_90KHZ: i64 = 90_000;

fn first_pts(bytes: &[u8]) -> (i64, i64) {
    let media = TsDemux::new().unpackage(bytes).unwrap();
    let mut video = None;
    let mut audio = None;
    for t in &media.tracks {
        let pts = t
            .samples
            .iter()
            .filter_map(|s| s.timing.map(|t| t.pts))
            .min()
            .unwrap();
        let pts90 =
            i64::try_from(i128::from(pts) * 90_000 / i128::from(t.spec.timescale.get())).unwrap();
        if t.spec.config.is_video() {
            video = Some(pts90);
        } else if t.spec.config.audio_sample_rate().is_some() {
            audio = Some(pts90);
        }
    }
    (video.unwrap(), audio.unwrap())
}

/// (Start position mod the 27 MHz wrap, video join-point PTS x 300 mod the wrap).
fn start_vs_join(bytes: &[u8]) -> (u64, u64) {
    let (replay, _) = replay_with_host(
        bytes,
        Config::default(),
        JOIN_CHUNK,
        bitrate_bps(bytes),
        Some(TICK_US),
    );
    let mut join = None;
    let mut start = None;
    for e in &replay.events {
        match &e.event {
            Event::Sample {
                sample, join: j, ..
            } if join.is_none() && format!("{j:?}").contains("JoinPoint") => {
                join = sample.timing.map(|t| t.pts);
            }
            Event::Buffer(BufferEvent::Start { at_27mhz }) if start.is_none() => {
                start = Some(*at_27mhz);
            }
            _ => {}
        }
    }
    let join = u64::try_from(join.expect("join point")).unwrap() * TICKS_27_PER_90K;
    (
        start.expect("Start") % PCR_WRAP_27MHZ,
        join % PCR_WRAP_27MHZ,
    )
}

/// transmux unrolls each PID's 33-bit PTS from that PID's own first value, so
/// when the 2^33 wrap falls between the first audio PES and the first video
/// PES, audio comes out a whole wrap above video. The join-point screen
/// compares wrap-safely, so pre-join audio ends are still withheld and `Start`
/// is exactly the video join point, as for the same capture without the wrap.
#[test]
fn pts_wrap_between_first_audio_and_first_video_keeps_the_join_contract() {
    let bytes = shift_pes(
        &test_fixtures::read(JOIN_FIXTURE),
        0,
        -AUDIO_BEHIND_90KHZ,
        Shift::Audio,
    );
    let (base_start, base_join) = start_vs_join(&bytes);
    assert_eq!(
        base_start, base_join,
        "precondition: unshifted Start == join point"
    );

    let (video0, audio0) = first_pts(&bytes);
    assert!(
        audio0 < video0,
        "fixture shape: audio's first PTS precedes video's"
    );
    let delta = PTS_WRAP - (audio0 + video0) / 2;
    let shifted = shift_pes(&bytes, 0, delta, Shift::Both);
    let (start, join) = start_vs_join(&shifted);
    let early_ms = broadcast_common::clock33::pcr_signed_distance(start, join) / 27_000;
    assert_eq!(
        start, join,
        "Start is {} ms before the video join point once the PTS wrap lies between the first audio and first video PES",
        early_ms
    );
}
