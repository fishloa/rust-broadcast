//! Timeline-break reporting (issue #4196): `StreamingTsDemux` reports every
//! track whose timeline stepped, not only the audio one, at the packet that
//! began the new time base, and publishes the matching `TimelineRepaired`
//! damage report beside it.
//!
//! The fixture is the committed `fixtures/ts/h264-aac.ts` (a real 2-track
//! H.264 + AAC MPEG-2 TS capture) concatenated with itself, whose seam is a
//! real backward jump of both tracks' 90 kHz timeline; no bytes are
//! hand-built.

use std::collections::BTreeMap;

use broadcast_core::damage::{ByteRange, DamageKind, Severity};
use transmux::TrackId;
use transmux::ir::SampleTiming;
use transmux::pipeline::CodecConfig;
use transmux::ts_demux::{DemuxEvent, DiscontinuityKind, StreamingTsDemux};

/// One MPEG-2 TS packet (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;

/// The same packet width for byte-offset arithmetic.
const TS_PACKET_BYTES: u64 = 188;

fn fixture() -> Vec<u8> {
    test_fixtures::read("ts/h264-aac.ts")
}

/// Feed `bytes` one TS packet at a time, draining after every `feed`, then
/// `finish` and drain the rest.
fn demux(bytes: &[u8]) -> Vec<DemuxEvent> {
    let mut demux = StreamingTsDemux::new();
    let mut events = Vec::new();
    for chunk in bytes.chunks(TS_PACKET_LEN) {
        demux.feed(chunk).expect("a valid TS feed");
        while let Some(event) = demux.poll_event() {
            events.push(event);
        }
    }
    demux.finish().expect("a valid TS finish");
    while let Some(event) = demux.poll_event() {
        events.push(event);
    }
    events
}

/// One `TimelineReanchored` event and the damage report published beside it.
struct Break<'a> {
    index: usize,
    track: TrackId,
    packet_index: Option<u64>,
    repair: &'a broadcast_core::damage::Damage,
}

fn breaks(events: &[DemuxEvent]) -> Vec<Break<'_>> {
    let mut out = Vec::new();
    for (index, event) in events.iter().enumerate() {
        let DemuxEvent::Discontinuity {
            track: Some(track),
            kind: DiscontinuityKind::TimelineReanchored,
            provenance,
            ..
        } = event
        else {
            continue;
        };
        let repair = match events.get(index + 1) {
            Some(DemuxEvent::Damage(damage)) => damage,
            other => panic!("no damage report after the break at {index}: {other:?}"),
        };
        out.push(Break {
            index,
            track: *track,
            packet_index: provenance.packet_index,
            repair,
        });
    }
    out
}

fn track_timescale(events: &[DemuxEvent], track: TrackId) -> u32 {
    events
        .iter()
        .find_map(|event| match event {
            DemuxEvent::TrackAdded(spec) if spec.track_id == track => Some(spec.timescale.get()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no TrackAdded for track {track}"))
}

fn is_video(events: &[DemuxEvent], track: TrackId) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            DemuxEvent::TrackAdded(spec)
                if spec.track_id == track && matches!(spec.config, CodecConfig::Avc { .. })
        )
    })
}

fn is_audio(events: &[DemuxEvent], track: TrackId) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            DemuxEvent::TrackAdded(spec)
                if spec.track_id == track && matches!(spec.config, CodecConfig::Aac { .. })
        )
    })
}

/// The decode timestamp of the nearest sample of `track` before `index`.
fn dts_before(events: &[DemuxEvent], index: usize, track: TrackId) -> i64 {
    events[..index]
        .iter()
        .rev()
        .find_map(|event| sample_dts(event, track))
        .unwrap_or_else(|| panic!("no sample of track {track} before event {index}"))
}

/// The decode timestamp of the nearest sample of `track` after `index`.
fn dts_after(events: &[DemuxEvent], index: usize, track: TrackId) -> i64 {
    events[index + 1..]
        .iter()
        .find_map(|event| sample_dts(event, track))
        .unwrap_or_else(|| panic!("no sample of track {track} after event {index}"))
}

fn sample_dts(event: &DemuxEvent, track: TrackId) -> Option<i64> {
    match event {
        DemuxEvent::Sample {
            track_id, sample, ..
        } if *track_id == track => match sample.timing {
            Some(SampleTiming::Timed { dts, .. }) => Some(dts),
            _ => None,
        },
        _ => None,
    }
}

/// `ticks` rescaled from `from` to the 90 kHz PES clock
/// (ISO/IEC 13818-1 §2.4.3.7).
fn to_90k(ticks: i64, from: u32) -> i64 {
    i64::try_from(i128::from(ticks) * i128::from(90_000u32) / i128::from(from))
        .expect("rescaled within i64")
}

#[test]
fn a_doubled_capture_reports_one_break_per_track_that_stepped() {
    let single = fixture();
    let seam = u64::try_from(single.len()).expect("fixture length");
    let mut doubled = single.clone();
    doubled.extend_from_slice(&single);

    let events = demux(&doubled);
    let found = breaks(&events);
    let tracks: Vec<TrackId> = found.iter().map(|b| b.track).collect();

    assert_eq!(
        found.len(),
        2,
        "both the video and the audio track step back at the seam: {tracks:?}"
    );
    assert!(
        tracks.iter().any(|track| is_video(&events, *track)),
        "the video track's backward step is reported: {tracks:?}"
    );
    assert!(
        tracks.iter().any(|track| is_audio(&events, *track)),
        "the audio track's re-anchor is reported: {tracks:?}"
    );

    for found in &found {
        let track = found.track;
        let packet = found
            .packet_index
            .unwrap_or_else(|| panic!("track {track} reports the packet that began its new base"));
        let position = packet * TS_PACKET_BYTES;
        assert!(
            position > seam,
            "track {track}'s break is in the second play, at byte {position} (seam {seam})"
        );

        let before = dts_before(&events, found.index, track);
        let after = dts_after(&events, found.index, track);
        assert!(
            after < before,
            "track {track} stepped backwards at the break: {before} -> {after}"
        );

        let repair = found.repair;
        assert_eq!(repair.kind(), DamageKind::TimelineRepaired);
        assert_eq!(repair.severity(), Severity::Informational);
        assert_eq!(repair.what(), "decode timestamp");
        assert_eq!(repair.location().track(), Some(track.value()));
        assert_eq!(
            repair.location().byte_range(),
            Some(ByteRange::new(position, 0)),
            "the report names the packet that began the new time base"
        );
        let media = repair
            .location()
            .media_time()
            .unwrap_or_else(|| panic!("track {track} reports the new base's time"));
        assert_eq!(media.timescale().get(), 90_000);
        let expected = to_90k(after, track_timescale(&events, track));
        assert!(
            (media.start() - expected).abs() <= 3,
            "track {track}'s reported time {} is the new base {expected}",
            media.start()
        );
    }
}

#[test]
fn a_single_track_step_reports_only_that_track() {
    let single = fixture();
    let events = demux(&single);
    let video = events
        .iter()
        .find_map(|event| match event {
            DemuxEvent::TrackAdded(spec) if matches!(spec.config, CodecConfig::Avc { .. }) => {
                spec.source_pid
            }
            _ => None,
        })
        .expect("the capture declares an H.264 track")
        .value();

    let mut doubled = single.clone();
    for packet in single.chunks(TS_PACKET_LEN) {
        let pid = (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2]);
        if pid != video {
            doubled.extend_from_slice(packet);
        }
    }

    let events = demux(&doubled);
    let found = breaks(&events);
    assert_eq!(
        found.len(),
        1,
        "only the audio track steps when the second play carries no video"
    );
    assert!(
        is_audio(&events, found[0].track),
        "the one report is the audio track's, not the video track that did not step"
    );
}

#[test]
fn a_continuous_capture_reports_no_timeline_break() {
    let events = demux(&fixture());
    assert!(
        breaks(&events).is_empty(),
        "a capture whose timestamps never step reports no break"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            DemuxEvent::Damage(damage) if damage.kind() == DamageKind::TimelineRepaired
        )),
        "a capture whose timestamps never step reports no timeline repair"
    );
}

/// A backward step in the emitted samples is not by itself a timeline break:
/// this real capture's DVB subtitle track (PES PID `0x96`) carries `0xBE`
/// padding PES between its sparse stamped access units, and each unstamped one
/// is interpolated forward by one measured frame period
/// (ISO/IEC 13818-1 §2.4.2.6). The interpolation overshoots the next wire
/// stamp, so the samples step back when the wire value takes over again — but
/// the track's timeline never did, and before the wire-stamped rule the demux
/// reported two breaks on a capture where nothing broke.
#[test]
fn an_interpolated_step_back_on_a_real_capture_is_not_a_timeline_break() {
    let Some(bytes) =
        test_fixtures::require_bytes("http/live-channels/tf1-live.ts", "demux_timeline_break")
    else {
        return;
    };
    let events = demux(&bytes);

    let mut stepped: Vec<(TrackId, i64, i64)> = Vec::new();
    let mut last: BTreeMap<TrackId, i64> = BTreeMap::new();
    for event in &events {
        let DemuxEvent::Sample {
            track_id, sample, ..
        } = event
        else {
            continue;
        };
        let Some(dts) = sample.timing.and_then(|t| match t {
            SampleTiming::Timed { dts, .. } => Some(dts),
            _ => None,
        }) else {
            continue;
        };
        if let Some(prev) = last.insert(*track_id, dts)
            && dts < prev
        {
            stepped.push((*track_id, prev, dts));
        }
    }
    assert!(
        !stepped.is_empty(),
        "the capture really does emit a backward step: {stepped:?}"
    );
    let found = breaks(&events);
    assert!(
        found.is_empty(),
        "a step back that only corrects an interpolated stamp is no break: {} found",
        found.len()
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            DemuxEvent::Damage(damage) if damage.kind() == DamageKind::TimelineRepaired
        )),
        "nothing was repaired, so nothing claims a timeline repair"
    );
}
