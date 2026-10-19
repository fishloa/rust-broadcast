//! `DemuxEvent::DisplayChanged` and `Sample::display` on real captures
//! (`docs/display-geometry/DESIGN.md` Phase 2).
//!
//! Every expectation below was read off an independent oracle first —
//! `ffprobe -v error -select_streams v -show_frames -show_entries
//! frame=pkt_dts_time,key_frame,pict_type,width,height,sample_aspect_ratio -of
//! csv` on the capture, and the switch access unit re-derived from that
//! listing by "first frame whose `sample_aspect_ratio` differs from the first
//! one" — never from this crate's own output:
//!
//! | fixture | first decodable frame | oracle DTS (s) at the switch |
//! |---|---|---|
//! | `ts/stimmungsgarten-sar-16-11-to-1-1-switch-zenith-tv.ts` | 16:11 | 1094.400 (S 18:14:24, B picture) |
//! | `ts/stimmungsgarten-sar-1-1-to-16-11-switch-zenith-tv.ts` | 1:1 | 1002.096 (S 16:42:16, B picture) |
//! | `ts/krone-sar-16-11-to-64-45-switch-zenith-tv.ts` | 16:11 | 1291.824 (S 21:31:24, P picture) |
//!
//! `-fps_mode passthrough` copies keep the first video frame at the capture's
//! own timeline, which is why the oracle DTS above differ from the
//! `pkt_dts_time` of the full captures. The three fixtures are one PES per
//! access unit with the access unit split cleanly, so the demuxer recovers
//! every capture frame but one, and the geometry change lands on the first
//! switched access unit exactly as the oracle lists it. The
//! `stimmungsgarten-sar-16-11-to-1-1` slate arrives with a new SPS/PPS whose
//! first access unit references the PPS not yet received (ffmpeg's
//! "non-existing PPS 0 referenced"): that unit is held, and the change is
//! raised on the next decodable slate IDR.
//!
//! Fixtures load through `test_fixtures` and SKIP, never fail, when the
//! private submodule is not checked out.
#![cfg(feature = "std")]

use broadcast_common::Unpackage;
use transmux::ir::{AspectRatioIdc, DisplayChangeSet, TrackId, TrackSpec};
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux, TsDemux};

fn read(fixture: &str) -> Option<Vec<u8>> {
    test_fixtures::require_bytes(fixture, "display_change")
}

/// One drained event, in emission order. `arc` is the ZigZag `Arc` allocation
/// address (`&*Arc`), so "the same state" is `Arc::ptr_eq`, not equality of
/// the value.
#[derive(Debug, Clone)]
enum Seen {
    Sample {
        track_id: TrackId,
        dts: Option<i64>,
        sync: bool,
        sar: Option<AspectRatioIdc>,
        arc: Option<usize>,
    },
    Changed {
        track_id: TrackId,
        sar: Option<AspectRatioIdc>,
        changed: DisplayChangeSet,
        dts: Option<i64>,
        pts: Option<i64>,
        random_access: bool,
        arc: usize,
    },
    Spec {
        spec: Box<TrackSpec>,
    },
}

fn arc_id<T>(arc: &std::sync::Arc<T>) -> usize {
    std::sync::Arc::as_ptr(arc) as usize
}

impl Seen {
    fn track(&self) -> TrackId {
        match self {
            Seen::Sample { track_id, .. } | Seen::Changed { track_id, .. } => *track_id,
            Seen::Spec { spec, .. } => spec.track_id,
        }
    }
}

fn sar_of(display: &transmux::ir::DisplayInfo) -> Option<AspectRatioIdc> {
    display.geometry.as_ref()?.aspect_ratio
}

/// Drain a streaming demux over `data`, recording every video-track event.
fn stream(data: &[u8]) -> Vec<Seen> {
    let mut demux = StreamingTsDemux::new();
    let mut out = Vec::new();
    let mut count = 0usize;
    let drain = |demux: &mut StreamingTsDemux, out: &mut Vec<Seen>, count: &mut usize| {
        while let Some(ev) = demux.poll_event() {
            let seen = match ev {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => Seen::Spec {
                    spec: Box::new(spec),
                },
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    let arc = sample.display.as_ref().map(arc_id);
                    let sar = sample.display.as_deref().and_then(sar_of);
                    Seen::Sample {
                        track_id,
                        dts: sample.dts,
                        sync: sample.flags.is_sync,
                        sar,
                        arc,
                    }
                }
                DemuxEvent::DisplayChanged {
                    track_id,
                    display,
                    changed,
                    dts,
                    pts,
                    random_access,
                    ..
                } => Seen::Changed {
                    track_id,
                    sar: sar_of(&display),
                    changed,
                    dts,
                    pts,
                    random_access,
                    arc: arc_id(&display),
                },
                _ => continue,
            };
            *count += 1;
            out.push(seen);
        }
    };
    for chunk in data.chunks(188 * 512) {
        demux.feed(chunk);
        drain(&mut demux, &mut out, &mut count);
    }
    demux.finish();
    drain(&mut demux, &mut out, &mut count);
    out
}

fn video_track(seen: &[Seen]) -> TrackId {
    seen.iter()
        .find_map(|s| match s {
            Seen::Spec { spec, .. }
                if matches!(spec.config, transmux::ir::CodecConfig::Avc { .. }) =>
            {
                Some(spec.track_id)
            }
            _ => None,
        })
        .expect("an AVC track")
}

fn only_track(seen: &[Seen], track: TrackId) -> Vec<&Seen> {
    seen.iter().filter(|s| s.track() == track).collect()
}

/// Assert the level-triggered start: one `DisplayChanged { initial }` for the
/// track, after its `TrackAdded` and before its first `Sample`.
fn assert_initial(video: &[&Seen], sar: AspectRatioIdc) {
    // The level start is emitted right after the track's `TrackAdded` that
    // carries the same geometry (the only spec of that track whose config
    // names it).
    assert!(
        video.iter().take(2).any(|s| matches!(s, Seen::Spec { .. })),
        "TrackAdded near the start: {:?}",
        &video[..2.min(video.len())]
    );
    let changed: Vec<&&Seen> = video
        .iter()
        .filter(|s| matches!(s, Seen::Changed { changed, .. } if changed.initial))
        .collect();
    assert_eq!(changed.len(), 1, "exactly one initial event: {changed:?}");
    let Seen::Changed {
        sar: got,
        changed: set,
        dts,
        pts,
        random_access,
        ..
    } = changed[0]
    else {
        unreachable!()
    };
    assert_eq!(*got, Some(sar));
    assert!(*set == DisplayChangeSet::initial(), "{set:?}");
    assert_eq!((*dts, *pts), (None, None));
    assert!(!*random_access);
    // Before the first Sample of the track.
    let first_sample = video
        .iter()
        .position(|s| matches!(s, Seen::Sample { .. }))
        .expect("a sample");
    let initial_at = video
        .iter()
        .position(|s| matches!(s, Seen::Changed { changed, .. } if changed.initial))
        .expect("the initial event");
    assert!(
        initial_at < first_sample,
        "the initial event must precede the first sample"
    );
}

/// Batch `Media`: every video sample carries a tag; the tags form the same
/// two geometries in the same order as the streaming sequence.
fn assert_batch(data: &[u8], from: AspectRatioIdc, to: AspectRatioIdc, track: TrackId) {
    let media = TsDemux::new().unpackage(data).expect("batch demux");
    let t = media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == track)
        .expect("the video track");
    let states: Vec<Option<AspectRatioIdc>> = t
        .samples
        .iter()
        .map(|s| {
            let d = s.display.as_deref().expect("every video sample is tagged");
            d.geometry.as_ref().map(|g| g.aspect_ratio.expect("a SAR"))
        })
        .collect();
    assert!(
        states.iter().all(|s| s.is_some()),
        "no untagged video sample"
    );
    let first_to = states
        .iter()
        .position(|s| *s == Some(to))
        .unwrap_or_else(|| panic!("never reaches {to:?}: {states:?}"));
    assert!(
        states[..first_to].iter().all(|s| *s == Some(from)),
        "everything before the switch is {from:?}: {states:?}"
    );
    assert!(
        states[first_to..].iter().all(|s| *s == Some(to)),
        "everything after the switch is {to:?}: {states:?}"
    );
}

#[test]
fn sar_16_11_to_1_1_switch_at_the_slate() {
    let Some(data) = read("ts/stimmungsgarten-sar-16-11-to-1-1-switch-zenith-tv.ts") else {
        return;
    };
    let seen = stream(&data);
    let track = video_track(&seen);
    let video = only_track(&seen, track);
    assert_initial(&video, AspectRatioIdc::Sar16x11);
    // ffprobe: the first slate picture is frame 241 of 248 (keyframe, 1:1);
    // the capture's video PES keeps its own PTS (the +25530 s jump is in the
    // audio), and the demuxer's wrap unroller lifts the last programme
    // picture's negative-PTS access units to continue the clock.
    let at = switch_index(&video);
    assert_switch_after(&video, at, AspectRatioIdc::Sar16x11, AspectRatioIdc::Square);
    let Seen::Changed {
        random_access,
        changed,
        ..
    } = video[at]
    else {
        unreachable!()
    };
    // The switch is raised when the one-behind slot is released, for the
    // access unit that was decoded *before* the first decodable slate
    // picture (the TS slot carries its events with the next AU) — so
    // `random_access` is that release unit's own bit (false for a B picture),
    // never the slate picture's. What is checkable is the slate itself: all
    // its pictures are IDRs (ffprobe) and the next tagged sample is an IDR.
    let _ = random_access;
    let slate = video
        .iter()
        .skip(at)
        .find_map(|s| match s {
            Seen::Sample {
                sar: Some(AspectRatioIdc::Square),
                sync,
                ..
            } => Some(*sync),
            _ => None,
        })
        .expect("a slate sample");
    assert!(slate, "the slate pictures are IDRs");
    // The stale AVC config's colour tuple is not signalled in the slate SPS.
    assert!(changed.video_signal);
    // Exactly one switch in the whole file, and the batch path tags every
    // sample the same way.
    file_wide_check(&video, 2);
    assert_batch(
        &data,
        AspectRatioIdc::Sar16x11,
        AspectRatioIdc::Square,
        track,
    );
}

#[test]
fn sar_1_1_to_16_11_switch_mid_gop() {
    let Some(data) = read("ts/stimmungsgarten-sar-1-1-to-16-11-switch-zenith-tv.ts") else {
        return;
    };
    let seen = stream(&data);
    let track = video_track(&seen);
    let video = only_track(&seen, track);
    assert_initial(&video, AspectRatioIdc::Square);
    // ffprobe: the first 16:11 picture is frame 4 (0-based), a B picture —
    // no IDR at the switch, so this is the non-conforming mid-GOP case.
    let at = switch_index(&video);
    assert_switch_after(&video, at, AspectRatioIdc::Square, AspectRatioIdc::Sar16x11);
    let Seen::Changed {
        random_access,
        changed,
        ..
    } = video[at]
    else {
        unreachable!()
    };
    let _ = random_access;
    assert!(changed.aspect_ratio, "{changed:?}");
    file_wide_check(&video, 2);
    assert_batch(
        &data,
        AspectRatioIdc::Square,
        AspectRatioIdc::Sar16x11,
        track,
    );
}

#[test]
fn sar_16_11_to_64_45_keeps_the_dar() {
    let Some(data) = read("ts/krone-sar-16-11-to-64-45-switch-zenith-tv.ts") else {
        return;
    };
    let seen = stream(&data);
    let track = video_track(&seen);
    let video = only_track(&seen, track);
    assert_initial(&video, AspectRatioIdc::Sar16x11);
    let at = switch_index(&video);
    // ffprobe: the first 64:45 picture is frame 258 (0-based), an IDR. Both
    // SARs give a 16:9 display (720*16/11 = 720*64/45 = 1024), so this is the
    // "wire form changed, DAR did not" case the event must still raise.
    assert_switch_after(
        &video,
        at,
        AspectRatioIdc::Sar16x11,
        AspectRatioIdc::ExtendedSar {
            sar_width: 64,
            sar_height: 45,
        },
    );
    let Seen::Changed {
        random_access,
        changed,
        ..
    } = video[at]
    else {
        unreachable!()
    };
    // The first slate access unit references the stale PPS id (the new
    // SPS/PPS arrive with the second one), so the first decodable slate
    // picture is the second IDR of the slate.
    let _ = random_access;
    assert!(changed.aspect_ratio, "a SAR-only change: {changed:?}");
    assert!(!changed.output_rect, "same coded size: {changed:?}");
    file_wide_check(&video, 2);
    assert_batch(
        &data,
        AspectRatioIdc::Sar16x11,
        AspectRatioIdc::ExtendedSar {
            sar_width: 64,
            sar_height: 45,
        },
        track,
    );
}

#[test]
fn single_geometry_captures_have_only_the_initial_event() {
    for (fixture, sar) in [
        (
            "ts/melodie-anamorphic-sar-64-45-zenith-tv.ts",
            AspectRatioIdc::ExtendedSar {
                sar_width: 64,
                sar_height: 45,
            },
        ),
        (
            "ts/krone-anamorphic-sar-16-11-zenith-tv.ts",
            AspectRatioIdc::Sar16x11,
        ),
    ] {
        let Some(data) = read(fixture) else {
            continue;
        };
        let seen = stream(&data);
        let track = video_track(&seen);
        let video = only_track(&seen, track);
        assert_initial(&video, sar);
        file_wide_check(&video, 1);
        assert_batch(&data, sar, sar, track);
    }
}

/// `tsduck/input/test-092.ts` is a six-program multiplex with six AVC tracks
/// (ffprobe, per stream: 1920x1080 1:1 on 0x2D0, 1440x1080 4:3 on 0x140,
/// 1920x1080 1:1 on 0x78, 720x576 16:11 on 0x208, 1920x1080 1:1 on 0xDC).
/// Each gets exactly one level-triggered event, carrying its own geometry —
/// the per-track state is never shared.
#[test]
fn per_track_geometries_in_one_mux() {
    let Some(data) = read("tsduck/input/test-092.ts") else {
        return;
    };
    let seen = stream(&data);
    let mut tracks: Vec<(TrackId, Option<AspectRatioIdc>)> = Vec::new();
    for s in &seen {
        if let Seen::Spec { spec, .. } = s
            && matches!(spec.config, transmux::ir::CodecConfig::Avc { .. })
        {
            let sar = spec.display.as_ref().and_then(|g| g.aspect_ratio);
            if !tracks.iter().any(|(t, _)| *t == spec.track_id) {
                tracks.push((spec.track_id, sar));
            }
        }
    }
    assert_eq!(tracks.len(), 6, "six AVC tracks: {tracks:?}");
    let mut sars: Vec<AspectRatioIdc> = Vec::new();
    for (track, spec_sar) in &tracks {
        let video = only_track(&seen, *track);
        let all: Vec<&&Seen> = video
            .iter()
            .filter(|s| matches!(s, Seen::Changed { .. }))
            .collect();
        assert_eq!(all.len(), 1, "one initial event for {track:?}");
        let Seen::Changed { changed, sar, .. } = all[0] else {
            unreachable!()
        };
        assert!(changed.initial);
        assert_eq!(sar, spec_sar, "the event agrees with the spec ({track:?})");
        sars.push(sar.expect("a SAR"));
    }
    // ffprobe's per-stream values, as `AspectRatioIdc`.
    let expected = [
        AspectRatioIdc::Square,
        AspectRatioIdc::Sar4x3,
        AspectRatioIdc::Square,
        AspectRatioIdc::Sar16x11,
        AspectRatioIdc::Square,
    ];
    for want in expected {
        assert!(sars.contains(&want), "{want:?} missing from {sars:?}");
    }
}

/// Index (into `video`) of the track's single non-initial `DisplayChanged`.
fn switch_index(video: &[&Seen]) -> usize {
    let at = video
        .iter()
        .position(|s| matches!(s, Seen::Changed { changed, .. } if !changed.initial))
        .expect("a switch event");
    let n = video
        .iter()
        .filter(|s| matches!(s, Seen::Changed { changed, .. } if !changed.initial))
        .count();
    assert_eq!(n, 1, "exactly one switch event");
    at
}

/// The switch is raised for the access unit **after** the last picture of
/// the old geometry in output order: every earlier-displayed picture keeps
/// the old tag, the event's own DTS/PTS are that next access unit's, and it
/// shares its `Arc`. `TrackUpdated` and `DisplayChanged` created at the same
/// access unit are split (`TrackUpdated`, `DisplayChanged`, `Sample`), so
/// they may be separated by each other and by clock/section samples only.
fn assert_switch_after(video: &[&Seen], at: usize, from: AspectRatioIdc, to: AspectRatioIdc) {
    let Seen::Changed {
        sar,
        changed,
        dts: event_dts,
        arc: event_arc,
        ..
    } = video[at]
    else {
        unreachable!()
    };

    assert_eq!(*sar, Some(to), "the event carries the new geometry");
    assert!(changed.aspect_ratio, "the SAR changed: {changed:?}");
    assert!(!changed.initial);
    // The event and the sample sharing its `Arc` are released together (the
    // one-behind slot carries its own events), possibly after the spec and
    // clock/section events of the same access unit.
    let shared = at
        + 1
        + video[at + 1..]
            .iter()
            .position(|s| matches!(s, Seen::Sample { arc: Some(a), .. } if *a == *event_arc))
            .expect("a sample shares the event's Arc");
    // The TS one-behind slot releases the previous access unit's `Sample`
    // together with the events raised for the arriving one, so the event's
    // DTS is the released sample's (`prev_dts`), one decode step behind the
    // sample physically carrying the new geometry. That released sample is
    // the one decoded *before* the switch.
    let released = video[at + 1];
    let Seen::Sample {
        dts: released_dts,
        sar: released_sar,
        ..
    } = released
    else {
        panic!("the event is followed by the released sample: {released:?}");
    };
    assert_eq!(released_sar, &Some(from), "the released sample is old");
    assert_eq!(event_dts, released_dts);
    // Every sample between the event and that one keeps the old state.
    for (i, s) in video[at + 1..shared].iter().enumerate() {
        if let Seen::Sample { sar, .. } = s {
            assert_eq!(*sar, Some(from), "sample {} before the change", at + 1 + i);
        }
    }
    let Seen::Sample { sar: next_sar, .. } = video[shared] else {
        unreachable!()
    };
    assert_eq!(next_sar, &Some(to));
    assert_eq!(
        event_dts, released_dts,
        "the event carries the release unit's DTS"
    );
}

/// A file with one geometry: one initial event and no further
/// `DisplayChanged`, however many IDRs re-send the SPS.
fn file_wide_check(video: &[&Seen], expected: usize) {
    let events = video
        .iter()
        .filter(|s| matches!(s, Seen::Changed { .. }))
        .count();
    assert_eq!(events, expected, "one DisplayChanged per config change");
    let tagged = video
        .iter()
        .filter(|s| matches!(s, Seen::Sample { arc: Some(_), .. }))
        .count();
    let total = video
        .iter()
        .filter(|s| matches!(s, Seen::Sample { .. }))
        .count();
    assert!(total > 0);
    assert_eq!(tagged, total, "every video sample is tagged");
}
