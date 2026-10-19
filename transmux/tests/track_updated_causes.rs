//! Issue #1380: what raises `DemuxEvent::TrackUpdated` at a programme/slate
//! switch, in which order, and with which contents.
//!
//! The issue reported "two `TrackUpdated` for one SPS change" on the real
//! slate-switch captures and blamed the track re-probe. That diagnosis was
//! wrong: there are two *independent* changes at the switch, each correctly
//! reported once:
//!
//! 1. the broadcaster's **PMT** changes (the slate lists fewer `es_info`
//!    descriptors than the programme, or more on the way back) — a
//!    descriptors-only `TrackUpdated` whose codec config is **unchanged**
//!    (issue #774), raised when the PMT is processed;
//! 2. the **SPS/PPS** change — a config-changing `TrackUpdated` (r04-W51) that
//!    is released after the last sample of the old config and immediately
//!    before the same access unit's `DisplayChanged` and its first sample
//!    (`docs/display-geometry/DESIGN.md` §6.5).
//!
//! This file pins that contract on the three real captures (private
//! fixtures, loaded through `test_fixtures`, SKIPPED loudly when the private
//! repository is not checked out) and on the public
//! `h264-two-resolutions.ts`, where only the SPS changes and so exactly one
//! `TrackUpdated` must fire.
#![cfg(feature = "std")]

use transmux::ir::{CodecConfig, TrackSpec};
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};

/// One event of the video track, in emission order.
#[derive(Debug)]
enum Ev {
    Added(Box<TrackSpec>),
    Updated(Box<TrackSpec>),
    /// A non-initial `DisplayChanged`; `arc` is the shared state's address.
    Switch {
        arc: usize,
    },
    /// A video sample and the address of the display state it carries.
    Sample {
        arc: Option<usize>,
    },
}

fn is_avc(spec: &TrackSpec) -> bool {
    matches!(spec.config, CodecConfig::Avc { .. })
}

/// Drain a streaming demux over `data` and keep the AVC track's events.
fn video_events(data: &[u8]) -> Vec<Ev> {
    let mut demux = StreamingTsDemux::new();
    let mut out = Vec::new();
    let mut video = None;
    let take = |demux: &mut StreamingTsDemux, out: &mut Vec<Ev>, video: &mut Option<_>| {
        while let Some(ev) = demux.poll_event() {
            match ev {
                DemuxEvent::TrackAdded(spec) if is_avc(&spec) => {
                    *video = Some(spec.track_id);
                    out.push(Ev::Added(Box::new(spec)));
                }
                DemuxEvent::TrackUpdated(spec) if Some(spec.track_id) == *video => {
                    out.push(Ev::Updated(Box::new(spec)));
                }
                DemuxEvent::DisplayChanged {
                    track_id,
                    display,
                    changed,
                    ..
                } if Some(track_id) == *video && !changed.initial => {
                    out.push(Ev::Switch {
                        arc: std::sync::Arc::as_ptr(&display) as usize,
                    });
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } if Some(track_id) == *video => {
                    out.push(Ev::Sample {
                        arc: sample
                            .display
                            .as_ref()
                            .map(|a| std::sync::Arc::as_ptr(a) as usize),
                    });
                }
                _ => {}
            }
        }
    };
    for chunk in data.chunks(188 * 64) {
        demux.feed(chunk);
        take(&mut demux, &mut out, &mut video);
    }
    demux.finish();
    take(&mut demux, &mut out, &mut video);
    out
}

fn updated(events: &[Ev]) -> Vec<(usize, &TrackSpec)> {
    events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Ev::Updated(s) => Some((i, &**s)),
            _ => None,
        })
        .collect()
}

fn added(events: &[Ev]) -> &TrackSpec {
    events
        .iter()
        .find_map(|e| match e {
            Ev::Added(s) => Some(&**s),
            _ => None,
        })
        .expect("the AVC track is announced")
}

/// `TrackSpec` and `CodecConfig` are not `PartialEq`; their `Debug` text is
/// complete (every field, the full avcC), so it is the equality oracle.
fn same_config(a: &TrackSpec, b: &TrackSpec) -> bool {
    format!("{:?}", a.config) == format!("{:?}", b.config)
}

/// The config-changing update is immediately followed (in the video track's
/// event sequence) by the switch's `DisplayChanged` and then by the FIRST
/// sample that carries the switch's display state; no sample before it does.
fn assert_config_update_adjacent_to_switch(events: &[Ev], update_at: usize) {
    let Some(Ev::Switch { arc }) = events.get(update_at + 1) else {
        panic!(
            "the config update must be immediately followed by DisplayChanged: {:?}",
            &events[update_at..(update_at + 3).min(events.len())]
        );
    };
    let first_new = events
        .iter()
        .position(|e| matches!(e, Ev::Sample { arc: Some(a) } if a == arc))
        .expect("a sample carries the switch's display state");
    assert_eq!(
        first_new,
        update_at + 2,
        "the first new-geometry sample directly follows the event: {:?}",
        &events[update_at..(update_at + 4).min(events.len())]
    );
}

/// The three real captures: a PMT change and an SPS change at the same
/// switch give two events, in this order and with these contents.
fn assert_pmt_then_sps(fixture: &str) {
    let Some(data) = test_fixtures::require_bytes(fixture, "track_updated_causes") else {
        return;
    };
    let events = video_events(&data);
    let tracks = added(&events);
    let ups = updated(&events);
    assert_eq!(
        ups.len(),
        2,
        "one descriptors update and one config update, not more: {ups:?}"
    );
    let (first_at, first) = ups[0];
    let (second_at, second) = ups[1];
    // 1. The PMT cause: new descriptors, the codec config exactly as announced.
    assert!(
        same_config(tracks, first),
        "the descriptors-only update keeps the codec config"
    );
    assert_ne!(
        tracks.es_info_descriptors, first.es_info_descriptors,
        "the descriptors-only update carries new descriptors"
    );
    // 2. The SPS cause: a new config, the descriptors already in force.
    assert!(
        !same_config(first, second),
        "the second update changes the codec config"
    );
    assert_eq!(
        first.es_info_descriptors, second.es_info_descriptors,
        "the config update carries the descriptors already in force"
    );
    assert!(first_at < second_at, "the PMT update comes first");
    assert_config_update_adjacent_to_switch(&events, second_at);
}

#[test]
fn stimmungsgarten_16_11_to_1_1_pmt_then_sps() {
    assert_pmt_then_sps("ts/stimmungsgarten-sar-16-11-to-1-1-switch-zenith-tv.ts");
}

#[test]
fn stimmungsgarten_1_1_to_16_11_pmt_then_sps() {
    assert_pmt_then_sps("ts/stimmungsgarten-sar-1-1-to-16-11-switch-zenith-tv.ts");
}

#[test]
fn krone_16_11_to_64_45_pmt_then_sps() {
    assert_pmt_then_sps("ts/krone-sar-16-11-to-64-45-switch-zenith-tv.ts");
}

/// Public fixture: only the SPS changes (a 320x240 clip then a 640x480 one on
/// one PID, an unchanged PMT), so exactly ONE `TrackUpdated` fires, it changes
/// the config, keeps the descriptors, and sits right before the switch.
#[test]
fn two_resolutions_one_update_when_only_the_sps_changes() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ts/h264-two-resolutions.ts");
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let events = video_events(&data);
    let tracks = added(&events);
    let ups = updated(&events);
    assert_eq!(ups.len(), 1, "only the SPS changed: {ups:?}");
    let (at, spec) = ups[0];
    assert!(!same_config(tracks, spec), "the update changes the config");
    assert_eq!(
        tracks.es_info_descriptors, spec.es_info_descriptors,
        "the PMT did not change"
    );
    assert_config_update_adjacent_to_switch(&events, at);
}
