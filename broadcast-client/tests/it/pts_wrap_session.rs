use crate::support;

use broadcast_client::{Action, BufferEvent, Config, Event, Session};
use broadcast_common::{Package, Unpackage};
use transmux::{Media, TsDemux, TsMux};

const WRAP: i64 = 1 << 33;

#[test]
fn pts_33bit_wrap_through_session_playout() {
    let base = test_fixtures::read("ts/h264_aac.ts");
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
        s.pts = Some(pts);
        s.dts = Some(pts);
        s.duration = Some(frame_dur as u32);
    }
    video_track.start_decode_time = video_track.samples[0].dts.unwrap() as u64;

    let video_track_id = video_track.spec.track_id;
    let mut tracks = vec![video_track];
    if let Some(mut a) = audio_track {
        a.samples.truncate(1);
        if let Some(s) = a.samples.first_mut() {
            s.pts = Some(0);
            s.dts = Some(0);
            s.duration = Some(1024);
        }
        tracks.push(a);
    }
    let media = Media::new(tracks, movie_timescale);
    let mut mux = TsMux::default();
    let bytes = mux.package(&media).expect("mux");
    assert!(!bytes.is_empty());

    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        _ => panic!("no request"),
    };
    session.on_response(
        conn,
        200,
        &[("Content-Length".to_string(), bytes.len().to_string())],
        0,
    );
    for chunk in bytes.chunks(1316) {
        session.on_data(conn, chunk, 0);
    }
    session.on_closed(conn, None, 1000);

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
            if let Some(p) = sample.pts {
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
