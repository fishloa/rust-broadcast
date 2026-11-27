//! The MPEG-2 TS slice of the one best-effort-reading report: every skip the
//! streaming demuxer tolerates reaches the caller as a
//! `broadcast_core::damage::Damage` naming what happened, where in the input,
//! on which track and at what media time — and everything after the damage is
//! still delivered.
//!
//! The fixture is a real broadcast capture; the damage is injected into it
//! (whole packets deleted, a PES header corrupted in place, the tail cut), so
//! the test cannot pass on hand-made bytes that never carry the real packet
//! layout.

use std::collections::BTreeMap;

use broadcast_core::damage::{Damage, DamageKind, Severity};
use broadcast_core::timescale::{Rounding, Timescale, rescale_i64};
use transmux::{DemuxEvent, StreamingTsDemux};

const TS_PACKET: usize = 188;
const SYNC: u8 = 0x47;
const FIXTURE: &str = "ts/h264-aac-40s.ts";
const TIMESCALE_90K: u32 = 90_000;

fn fixture() -> Vec<u8> {
    let bytes = test_fixtures::read(FIXTURE);
    assert_eq!(bytes.len() % TS_PACKET, 0, "fixture is whole packets");
    bytes
}

fn drain(bytes: &[u8]) -> Vec<DemuxEvent> {
    let mut demux = StreamingTsDemux::new();
    demux.feed(bytes).expect("feed");
    demux.finish().expect("finish");
    std::iter::from_fn(|| demux.poll_event()).collect()
}

fn packets(bytes: &[u8]) -> Vec<usize> {
    (0..bytes.len() / TS_PACKET)
        .map(|i| i * TS_PACKET)
        .filter(|&offset| bytes[offset] == SYNC)
        .collect()
}

fn pid_at(bytes: &[u8], offset: usize) -> u16 {
    (u16::from(bytes[offset + 1] & 0x1F) << 8) | u16::from(bytes[offset + 2])
}

fn pusi_at(bytes: &[u8], offset: usize) -> bool {
    bytes[offset + 1] & 0x40 != 0
}

/// Offset of a packet's payload, when it carries one.
fn payload_offset(bytes: &[u8], offset: usize) -> Option<usize> {
    match (bytes[offset + 3] >> 4) & 0b11 {
        0b01 => Some(offset + 4),
        0b11 => Some(offset + 5 + usize::from(bytes[offset + 4])),
        _ => None,
    }
}

/// The track carrying the most bytes (the video track), with its PID and
/// media timescale.
fn busiest_track(events: &[DemuxEvent]) -> (transmux::TrackId, u16, Timescale) {
    let mut bytes: BTreeMap<u32, usize> = BTreeMap::new();
    for event in events {
        if let DemuxEvent::Sample {
            track_id, sample, ..
        } = event
        {
            *bytes.entry(track_id.value()).or_default() += sample.data.len();
        }
    }
    let (&id, _) = bytes
        .iter()
        .max_by_key(|(_, size)| **size)
        .expect("the fixture has samples");
    let spec = events
        .iter()
        .find_map(|event| match event {
            DemuxEvent::TrackAdded(spec) if spec.track_id.value() == id => Some(spec),
            _ => None,
        })
        .expect("the busiest track was announced");
    let pid = spec.source_pid.expect("a TS track names its PID").value();
    (transmux::TrackId::new(id), pid, spec.timescale)
}

fn damages(events: &[DemuxEvent]) -> Vec<&Damage> {
    events
        .iter()
        .filter_map(|event| match event {
            DemuxEvent::Damage(damage) => Some(damage),
            _ => None,
        })
        .collect()
}

fn sample_pts(events: &[DemuxEvent], track: transmux::TrackId) -> Vec<i64> {
    events
        .iter()
        .filter_map(|event| match event {
            DemuxEvent::Sample {
                track_id, sample, ..
            } if *track_id == track => match sample.timing {
                Some(transmux::ir::SampleTiming::Timed { pts, .. }) => Some(pts),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_clean_stream_produces_no_report() {
    let events = drain(&fixture());
    assert!(sample_pts(&events, busiest_track(&events).0).len() > 50);
    assert!(
        damages(&events).is_empty(),
        "a clean stream reports nothing: {:?}",
        damages(&events)
    );
}

#[test]
fn deleted_packets_reach_the_caller_with_place_track_and_time() {
    let bytes = fixture();
    let clean = drain(&bytes);
    let (track, pid, track_timescale) = busiest_track(&clean);
    let offsets = packets(&bytes);
    let video: Vec<usize> = offsets
        .iter()
        .copied()
        .filter(|&offset| pid_at(&bytes, offset) == pid)
        .collect();
    let victim = video[video.len() / 2];
    assert_eq!(
        pid_at(&bytes, victim + TS_PACKET),
        pid,
        "the run to delete is two consecutive packets of the same PID"
    );

    let mut damaged = bytes.clone();
    damaged.drain(victim..victim + 2 * TS_PACKET);
    let events = drain(&damaged);

    let report = damages(&events)
        .into_iter()
        .find(|damage| {
            damage.kind() == DamageKind::MediaUnitDropped
                && damage.severity() == Severity::Degraded
                && damage.location().track() == Some(track.value())
        })
        .unwrap_or_else(|| panic!("no track-attributed report: {:?}", damages(&events)));

    let range = report.location().byte_range().expect("a byte range");
    assert_eq!(
        range.start(),
        victim as u64,
        "the loss is named at the offset that revealed it in the delivered input"
    );
    assert!(
        range.is_empty(),
        "the lost bytes are not in the input at all"
    );
    let media = report.location().media_time().expect("a media time");
    assert_eq!(media.timescale().get(), TIMESCALE_90K);
    assert!(media.start() >= 0);
    assert!(!report.recoverable_bytes());

    // Everything after the damage is still delivered, on the same track and
    // further along the timeline than what came before it.
    let pts = sample_pts(&events, track);
    let clean_pts = sample_pts(&clean, track);
    assert!(
        pts.len() < clean_pts.len(),
        "the fixture must lose samples with the packets"
    );
    let last = *pts.last().expect("samples after the damage");
    let last_90k = rescale_i64(
        last,
        track_timescale,
        broadcast_core::timescale!(90_000),
        Rounding::Floor,
    );
    assert!(
        last_90k > media.start(),
        "playback continues past the damaged media time (last {last_90k}, damage {})",
        media.start()
    );
}

#[test]
fn a_corrupted_pes_is_reported_instead_of_silently_dropped() {
    let bytes = fixture();
    let clean = drain(&bytes);
    let (track, pid, _) = busiest_track(&clean);

    // A unit-start packet past the middle whose payload opens with a PES
    // start code: corrupting those four bytes in place leaves the TS framing
    // intact and only the unit unusable.
    let all = packets(&bytes);
    let (victim, payload) = all
        .iter()
        .copied()
        .skip(all.len() / 2)
        .find_map(|offset| {
            let payload = payload_offset(&bytes, offset)?;
            let starts_pes = bytes.get(payload..payload + 3) == Some(&[0x00, 0x00, 0x01][..]);
            (pid_at(&bytes, offset) == pid && pusi_at(&bytes, offset) && starts_pes)
                .then_some((offset, payload))
        })
        .expect("a PES start in the second half");

    let mut damaged = bytes.clone();
    damaged[payload..payload + 4].fill(0x00);
    let events = drain(&damaged);

    let report = damages(&events)
        .into_iter()
        .find(|damage| damage.what() == "PES packet")
        .unwrap_or_else(|| panic!("no PES report: {:?}", damages(&events)));

    assert_eq!(report.kind(), DamageKind::MediaUnitDropped);
    assert_eq!(report.severity(), Severity::Degraded);
    let range = report.location().byte_range().expect("a byte range");
    assert_eq!(
        range.start(),
        victim as u64,
        "the dropped unit's first packet"
    );
    assert!(!range.is_empty(), "the dropped unit's bytes are named");
    assert_eq!(report.location().track(), Some(track.value()));
    assert!(report.location().media_time().is_some());

    // The stream carries on: later samples of the same track still arrive.
    let mut after = 0usize;
    let mut seen_damage = false;
    for event in &events {
        if matches!(event, DemuxEvent::Damage(_)) {
            seen_damage = true;
        } else if seen_damage
            && let DemuxEvent::Sample { track_id, .. } = event
            && *track_id == track
        {
            after += 1;
        }
    }
    assert!(
        after > 50,
        "playback continues after the dropped unit: {after}"
    );
}

#[test]
fn a_truncated_tail_is_reported() {
    let bytes = fixture();
    let mut cut = bytes.clone();
    cut.truncate(bytes.len() - 100);
    let events = drain(&cut);

    let report = damages(&events)
        .into_iter()
        .find(|damage| damage.kind() == DamageKind::InputTruncated)
        .unwrap_or_else(|| panic!("no truncation report: {:?}", damages(&events)));
    assert_eq!(report.severity(), Severity::Degraded);
    assert_eq!(report.what(), "TS packet");
    let range = report.location().byte_range().expect("a byte range");
    assert_eq!(
        range.end(),
        cut.len() as u64,
        "the truncated tail ends with the input"
    );
    assert!(
        !range.is_empty() && range.len() < TS_PACKET as u64,
        "fewer than a whole packet was cut"
    );
    assert_eq!(
        report.location().track(),
        None,
        "the tail carries no identity, so no track is fabricated"
    );
    assert!(!report.recoverable_bytes());
}

#[test]
fn the_input_degradation_events_and_the_reports_say_the_same_thing() {
    let bytes = fixture();
    let clean = drain(&bytes);
    let (_, pid, _) = busiest_track(&clean);
    let video: Vec<usize> = packets(&bytes)
        .into_iter()
        .filter(|&offset| pid_at(&bytes, offset) == pid)
        .collect();
    let victim = video[video.len() / 2];
    let mut damaged = bytes.clone();
    damaged.drain(victim..victim + TS_PACKET);
    let events = drain(&damaged);

    let degraded = events
        .iter()
        .find_map(|event| match event {
            DemuxEvent::InputDegraded { kind, .. } => Some(*kind),
            _ => None,
        })
        .expect("the existing degradation channel still reports it");
    assert!(events.iter().any(|event| matches!(
        event,
        DemuxEvent::Damage(damage) if damage.kind() == degraded.damage_kind()
    )));
}
