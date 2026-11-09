//! Bug audit A (2026-11-09): reproductions that fail on main fed51ad27.

use crate::support::replay::{bitrate_bps, replay_with_host};
use crate::support::ts_shift::{PTS_WRAP, Shift, shift_pes};

use broadcast_client::{BufferEvent, Config, Event};
use broadcast_common::Unpackage;
use transmux::TsDemux;

const FIXTURE: &str = "ts/h264-aac-40s.ts";
const CHUNK: usize = 188 * 7;
const TICK_US: u64 = 20_000;
const PCR_WRAP_27MHZ: u64 = (1 << 33) * 300;
const TICKS_27_PER_90K: u64 = 300;
/// Audio muxed 1 s behind video, the usual TS shape (DESIGN.md 0.1c).
const AUDIO_BEHIND_90KHZ: i64 = 90_000;

fn first_pts(bytes: &[u8]) -> (i64, i64) {
    let media = TsDemux::new().unpackage(bytes).unwrap();
    let mut video = None;
    let mut audio = None;
    for t in &media.tracks {
        let pts = t.samples.iter().filter_map(|s| s.pts).min().unwrap();
        let pts90 = i64::try_from(i128::from(pts) * 90_000 / i128::from(t.spec.timescale)).unwrap();
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
        CHUNK,
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
                join = sample.pts;
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
/// PES, audio is unrolled ~2^33 above video. The feeder's join-point screen
/// `end_27 > join_pts` (broadcast-client/src/buffer.rs `queue_audio_end`) is a
/// plain integer compare, so every pre-join audio end passes it, the
/// controller's origin is anchored at audio's first end (before the join
/// point) and `Start` fires with that much less cushion. Same capture without
/// the wrap: `Start` is exactly the join point.
#[test]
fn pts_wrap_between_first_audio_and_first_video_keeps_the_join_contract() {
    let bytes = shift_pes(
        &test_fixtures::read(FIXTURE),
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

/// The demuxer's tail (the last PES of every PID, held until the next PES
/// start) is flushed when the connection closes. `Session::on_closed` drains
/// those samples to the host and queues their ends in the feeder, but the
/// feeder reports queued ends only from `on_chunk`, and the reconnect's
/// `rejoin` clears the queue: the controller never learns media the host
/// already holds, so it judges the lead short by the tail.
#[test]
fn media_flushed_at_connection_close_reaches_the_controller() {
    use broadcast_client::BufferInput;
    use std::collections::BTreeMap;

    let bytes = test_fixtures::read(FIXTURE);
    let bps = bitrate_bps(&bytes);
    let mut config = Config::default();
    config.record_buffer_inputs = true;
    let (mut s, conn) = crate::support::replay::open_live(config);
    let mut timescale: BTreeMap<u32, u32> = BTreeMap::new();
    let mut host_end: BTreeMap<u32, u64> = BTreeMap::new();
    let mut reported: BTreeMap<u32, u64> = BTreeMap::new();
    let mut now = 0u64;
    let mut drain = |s: &mut broadcast_client::Session,
                     host_end: &mut BTreeMap<u32, u64>,
                     reported: &mut BTreeMap<u32, u64>| {
        while let Some(e) = s.next_event() {
            match e {
                Event::Tracks(specs) => {
                    for t in specs {
                        timescale.insert(t.track_id.value(), t.timescale);
                    }
                }
                Event::Sample { track, sample, .. } => {
                    let ts = u64::from(timescale[&track.value()]);
                    let end = u64::try_from(
                        sample.pts.unwrap() + i64::from(sample.duration.unwrap_or(0)),
                    )
                    .unwrap();
                    let e27 = end * 27_000_000 / ts;
                    let slot = host_end.entry(track.value()).or_insert(0);
                    *slot = (*slot).max(e27);
                }
                _ => {}
            }
        }
        for i in s.drain_buffer_inputs() {
            if let BufferInput::TrackMedia { track, end, .. } = i {
                let slot = reported.entry(track).or_insert(0);
                *slot = (*slot).max(end);
            }
        }
    };
    for part in bytes.chunks(CHUNK) {
        s.on_data(conn, part, now);
        drain(&mut s, &mut host_end, &mut reported);
        now += part.len() as u64 * 8 * 1_000_000 / bps;
    }
    let before_close = reported.clone();
    s.on_closed(conn, None, now);
    drain(&mut s, &mut host_end, &mut reported);
    let mut short = Vec::new();
    for (track, rep) in &reported {
        let host = host_end[track];
        if host > *rep {
            short.push((*track, (host - rep) / 27_000));
        }
    }
    assert!(
        short.is_empty(),
        "controller told less than the host holds after close, (track, ms short): {short:?}; reported before close {before_close:?}"
    );
}
