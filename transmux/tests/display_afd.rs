//! AFD and bar data for H.264/H.265 (Phase 3a) — whole-pipeline tests.

#![cfg(feature = "std")]

use broadcast_common::{Package, Unpackage};
use std::sync::Arc;
use transmux::annexb::iter_length_prefixed_nals;
use transmux::ir::{ActiveFormatDescription, AfdPersistence, BarData, CodecConfig, DisplayInfo};
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux, TsDemux};
use transmux::{Media, Track, TsMux};

fn avc_track_mut(media: &mut Media) -> &mut Track {
    media
        .tracks
        .iter_mut()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("avc track")
}

fn hevc_track_mut(media: &mut Media) -> &mut Track {
    media
        .tracks
        .iter_mut()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("hevc track")
}

fn push_nal(out: &mut Vec<u8>, nal: &[u8]) {
    out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
    out.extend_from_slice(nal);
}

fn strip_afd_sei(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in iter_length_prefixed_nals(data).expect("lp") {
        let is_afd = (nal[0] & 0x1F) == 6 && nal.windows(4).any(|w| w == [0x44, 0x54, 0x47, 0x31]);
        if is_afd {
            continue;
        }
        push_nal(&mut out, nal);
    }
    out
}

fn strip_hevc_afd_sei(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in iter_length_prefixed_nals(data).expect("lp") {
        let is_afd = (nal[0] >> 1) == 39 && nal.windows(4).any(|w| w == [0x44, 0x54, 0x47, 0x31]);
        if is_afd {
            continue;
        }
        push_nal(&mut out, nal);
    }
    out
}

fn afd_nal(code: u8, active: bool) -> Vec<u8> {
    let mut t35 = vec![0xB5u8, 0x00, 0x31, 0x44, 0x54, 0x47, 0x31];
    if active {
        t35.push(0x41);
        t35.push(0xF0 | code);
    } else {
        t35.push(0x01);
    }
    let mut body = vec![0x04u8, t35.len() as u8];
    body.extend(t35);
    body.push(0x80);
    let mut nal = vec![0x06u8];
    nal.extend(body);
    nal
}

fn bar_nal(flags: u8, values: &[u16]) -> Vec<u8> {
    let mut t35 = vec![0xB5u8, 0x00, 0x31, 0x47, 0x41, 0x39, 0x34, 0x06, flags];
    for v in values {
        t35.extend_from_slice(&(0xC000u16 | v).to_be_bytes());
    }
    let mut body = vec![0x04u8, t35.len() as u8];
    body.extend(t35);
    body.push(0x80);
    let mut nal = vec![0x06u8];
    nal.extend(body);
    nal
}

fn hevc_afd_nal(code: u8, active: bool, prefix: bool) -> Vec<u8> {
    let avc = afd_nal(code, active);
    let body = &avc[1..];
    let mut nal = if prefix {
        vec![0x4Eu8, 0x01]
    } else {
        vec![0x50u8, 0x01]
    };
    nal.extend_from_slice(body);
    nal
}

fn build_avc_from_n(fixture: &str, sei_per_au: &[Vec<Vec<u8>>], n: Option<usize>) -> Vec<u8> {
    let data = test_fixtures::require_bytes(fixture, "display_afd").expect(fixture);
    let mut media = TsDemux::new().unpackage(&data).expect("demux");
    let track = avc_track_mut(&mut media);
    for (i, sample) in track.samples.iter_mut().enumerate() {
        let mut v = strip_afd_sei(&sample.data);
        if let Some(nals) = sei_per_au.get(i) {
            for nal in nals {
                push_nal(&mut v, nal);
            }
        }
        sample.data = v.into();
    }
    if let Some(n) = n {
        track.samples.truncate(n);
    }
    TsMux::new().package(&media).expect("mux")
}

fn build_avc_from(fixture: &str, sei_per_au: &[Vec<Vec<u8>>]) -> Vec<u8> {
    build_avc_from_n(fixture, sei_per_au, None)
}

fn build_avc(sei_per_au: &[Vec<Vec<u8>>]) -> Vec<u8> {
    build_avc_from("ts/gulli-opengop.ts", sei_per_au)
}

fn build_hevc(sei_per_au: &[Vec<Vec<u8>>]) -> Vec<u8> {
    let data = test_fixtures::require_bytes("ts/hevc/main.ts", "display_afd").expect("hevc");
    let mut media = TsDemux::new().unpackage(&data).expect("demux");
    let track = hevc_track_mut(&mut media);
    for (i, sample) in track.samples.iter_mut().enumerate() {
        let mut v = strip_hevc_afd_sei(&sample.data);
        if let Some(nals) = sei_per_au.get(i) {
            for nal in nals {
                push_nal(&mut v, nal);
            }
        }
        sample.data = v.into();
    }
    TsMux::new().package(&media).expect("mux")
}

fn arc_id<T>(arc: &Arc<T>) -> usize {
    Arc::as_ptr(arc) as usize
}

#[derive(Clone)]
struct Seen {
    kind: &'static str,
    arc: usize,
    afd: Option<ActiveFormatDescription>,
    bar: Option<BarData>,
    changed: Option<transmux::ir::DisplayChangeSet>,
    random_access: bool,
    initial: bool,
}

fn afd_of(d: &DisplayInfo) -> Option<ActiveFormatDescription> {
    d.active_format.as_ref().and_then(|a| a.afd)
}

fn bar_of(d: &DisplayInfo) -> Option<BarData> {
    d.active_format.as_ref().and_then(|a| a.bar_data.clone())
}

fn run_stream(ts: &[u8], policy: AfdPersistence) -> Vec<Seen> {
    let video_id = TsDemux::new()
        .unpackage(ts)
        .expect("batch")
        .tracks
        .iter()
        .find(|t| {
            matches!(
                t.spec.config,
                CodecConfig::Avc { .. } | CodecConfig::Hevc { .. }
            )
        })
        .expect("video track")
        .spec
        .track_id;
    let mut demux = StreamingTsDemux::new().with_afd_persistence(policy);
    let mut out = Vec::new();
    let drain = |demux: &mut StreamingTsDemux, out: &mut Vec<Seen>| {
        while let Some(ev) = demux.poll_event() {
            match ev {
                DemuxEvent::Sample {
                    track_id, sample, ..
                } if track_id == video_id => {
                    out.push(Seen {
                        kind: "sample",
                        arc: sample.display.as_ref().map(arc_id).unwrap_or(0),
                        afd: sample.display.as_deref().and_then(afd_of),
                        bar: sample.display.as_deref().and_then(bar_of),
                        changed: None,
                        random_access: sample.flags.is_sync,
                        initial: false,
                    });
                }
                DemuxEvent::DisplayChanged {
                    track_id,
                    display,
                    changed,
                    random_access,
                    ..
                } if track_id == video_id => {
                    out.push(Seen {
                        kind: "changed",
                        arc: arc_id(&display),
                        afd: afd_of(&display),
                        bar: bar_of(&display),
                        changed: Some(changed),
                        random_access,
                        initial: changed.initial,
                    });
                }
                _ => {}
            }
        }
    };
    for chunk in ts.chunks(188 * 512) {
        demux.feed(chunk);
        drain(&mut demux, &mut out);
    }
    demux.finish();
    drain(&mut demux, &mut out);
    out
}

fn sample_positions(items: &[Seen]) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, s)| if s.kind == "sample" { Some(i) } else { None })
        .collect()
}

#[test]
fn afd_mid_gop_change() {
    let sei = vec![
        vec![afd_nal(8, true)],
        vec![afd_nal(8, true)],
        vec![afd_nal(9, true)],
    ];
    let ts = build_avc(&sei);
    let items = run_stream(&ts, AfdPersistence::UntilReplaced);
    let initials: Vec<&Seen> = items
        .iter()
        .filter(|s| s.kind == "changed" && s.initial)
        .collect();
    assert_eq!(initials.len(), 1);
    let changes: Vec<&Seen> = items
        .iter()
        .filter(|s| s.kind == "changed" && !s.initial)
        .collect();
    assert_eq!(changes.len(), 1);
    let c = changes[0];
    assert!(c.changed.unwrap().afd);
    assert!(!c.random_access);
    assert_eq!(c.afd, Some(ActiveFormatDescription::Centre4x3));
    let sp = sample_positions(&items);
    let changed_pos = items
        .iter()
        .position(|s| s.kind == "changed" && !s.initial)
        .unwrap();
    assert!(changed_pos > sp[1] && changed_pos < sp[2]);
    let samples: Vec<&Seen> = items.iter().filter(|s| s.kind == "sample").collect();
    assert_eq!(
        samples[0].afd,
        Some(ActiveFormatDescription::SameAsCodedFrame)
    );
    assert_eq!(
        samples[1].afd,
        Some(ActiveFormatDescription::SameAsCodedFrame)
    );
    for s in &samples[2..] {
        assert_eq!(s.afd, Some(ActiveFormatDescription::Centre4x3));
    }
    assert_eq!(samples[2].arc, c.arc);
    // batch
    let media = TsDemux::new().unpackage(&ts).expect("batch");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .unwrap();
    assert_eq!(
        afd_of(track.samples[0].display.as_ref().unwrap()),
        Some(ActiveFormatDescription::SameAsCodedFrame)
    );
    assert_eq!(
        afd_of(track.samples[2].display.as_ref().unwrap()),
        Some(ActiveFormatDescription::Centre4x3)
    );
}

#[test]
fn afd_repeated_same_value_no_extra_event() {
    let sei = vec![vec![afd_nal(8, true)]; 6];
    let ts = build_avc(&sei);
    let items = run_stream(&ts, AfdPersistence::UntilReplaced);
    let changes: Vec<&Seen> = items
        .iter()
        .filter(|s| s.kind == "changed" && !s.initial)
        .collect();
    assert!(changes.is_empty());
    let samples: Vec<&Seen> = items.iter().filter(|s| s.kind == "sample").collect();
    assert!(samples.iter().all(|s| s.arc == samples[0].arc));
}

#[test]
fn afd_initial_carries_value() {
    let sei = vec![vec![afd_nal(9, true)]];
    let ts = build_avc(&sei);
    let items = run_stream(&ts, AfdPersistence::UntilReplaced);
    let changes: Vec<&Seen> = items.iter().filter(|s| s.kind == "changed").collect();
    assert_eq!(changes.len(), 1);
    assert!(changes[0].initial);
    assert_eq!(changes[0].afd, Some(ActiveFormatDescription::Centre4x3));
}

#[test]
fn afd_until_replaced_keeps_across_idr() {
    let mut sei: Vec<Vec<Vec<u8>>> = vec![vec![]; 33];
    sei[0] = vec![afd_nal(9, true)];
    let ts = build_avc(&sei);
    let items = run_stream(&ts, AfdPersistence::UntilReplaced);
    let changes: Vec<&Seen> = items
        .iter()
        .filter(|s| s.kind == "changed" && !s.initial)
        .collect();
    assert!(changes.is_empty());
    let samples: Vec<&Seen> = items.iter().filter(|s| s.kind == "sample").collect();
    assert!(
        samples
            .iter()
            .all(|s| s.afd == Some(ActiveFormatDescription::Centre4x3))
    );
}

#[test]
fn afd_reset_at_random_access() {
    let mut sei: Vec<Vec<Vec<u8>>> = vec![vec![]; 26];
    sei[0] = vec![afd_nal(9, true)];
    let ts = build_avc_from_n("ts/h264_aac.ts", &sei, Some(26));
    let items = run_stream(&ts, AfdPersistence::ResetAtRandomAccess);
    let changes: Vec<&Seen> = items
        .iter()
        .filter(|s| s.kind == "changed" && !s.initial)
        .collect();
    assert_eq!(changes.len(), 1);
    let c = changes[0];
    assert!(c.changed.unwrap().afd);
    assert!(c.random_access);
    assert_eq!(c.afd, Some(ActiveFormatDescription::SameAsCodedFrame));
    let sp = sample_positions(&items);
    let changed_pos = items
        .iter()
        .position(|s| s.kind == "changed" && !s.initial)
        .unwrap();
    assert!(changed_pos > sp[24] && changed_pos < sp[25]);
    let samples: Vec<&Seen> = items.iter().filter(|s| s.kind == "sample").collect();
    for s in &samples[..25] {
        assert_eq!(s.afd, Some(ActiveFormatDescription::Centre4x3));
    }
    for s in &samples[25..] {
        assert_eq!(s.afd, Some(ActiveFormatDescription::SameAsCodedFrame));
    }
    assert_eq!(samples[25].arc, c.arc);
}

#[test]
fn afd_reset_idr_with_own_afd_keeps_value() {
    let mut sei: Vec<Vec<Vec<u8>>> = vec![vec![]; 26];
    sei[0] = vec![afd_nal(9, true)];
    sei[25] = vec![afd_nal(9, true)];
    let ts = build_avc_from_n("ts/h264_aac.ts", &sei, Some(26));
    let items = run_stream(&ts, AfdPersistence::ResetAtRandomAccess);
    let changes: Vec<&Seen> = items
        .iter()
        .filter(|s| s.kind == "changed" && !s.initial)
        .collect();
    assert!(changes.is_empty());
    let samples: Vec<&Seen> = items.iter().filter(|s| s.kind == "sample").collect();
    assert!(
        samples
            .iter()
            .all(|s| s.afd == Some(ActiveFormatDescription::Centre4x3))
    );
}

#[test]
fn afd_non_idr_without_afd_does_not_reset() {
    let mut sei: Vec<Vec<Vec<u8>>> = vec![vec![]; 4];
    sei[0] = vec![afd_nal(9, true)];
    let ts = build_avc(&sei);
    let items = run_stream(&ts, AfdPersistence::ResetAtRandomAccess);
    let changes: Vec<&Seen> = items
        .iter()
        .filter(|s| s.kind == "changed" && !s.initial)
        .collect();
    assert!(changes.is_empty());
}

#[test]
fn afd_bar_data_replace_and_clear() {
    let sei = vec![
        vec![afd_nal(9, true)],
        vec![bar_nal(0xC0, &[70, 506])],
        vec![afd_nal(10, true)],
    ];
    let ts = build_avc(&sei);
    let items = run_stream(&ts, AfdPersistence::UntilReplaced);
    let changes: Vec<&Seen> = items
        .iter()
        .filter(|s| s.kind == "changed" && !s.initial)
        .collect();
    assert_eq!(changes.len(), 2);
    // bar-only change
    let c1 = changes[0];
    assert!(!c1.changed.unwrap().afd);
    assert!(c1.changed.unwrap().bar_data);
    assert_eq!(c1.afd, Some(ActiveFormatDescription::Centre4x3));
    assert_eq!(
        c1.bar,
        Some(BarData {
            top_bar_end_line: Some(70),
            bottom_bar_start_line: Some(506),
            left_bar_end_pixel: None,
            right_bar_start_pixel: None,
        })
    );
    let sp = sample_positions(&items);
    let pos1 = items
        .iter()
        .position(|s| s.kind == "changed" && !s.initial)
        .unwrap();
    assert!(pos1 > sp[0] && pos1 < sp[1]);
    // afd clears bar
    let c2 = changes[1];
    assert!(c2.changed.unwrap().afd);
    assert!(c2.changed.unwrap().bar_data);
    assert_eq!(c2.afd, Some(ActiveFormatDescription::Centre16x9));
    assert_eq!(c2.bar, None);
    let pos2 = items
        .iter()
        .rposition(|s| s.kind == "changed" && !s.initial)
        .unwrap();
    assert!(pos2 > sp[1] && pos2 < sp[2]);
    let samples: Vec<&Seen> = items.iter().filter(|s| s.kind == "sample").collect();
    assert_eq!(samples[0].bar, None);
    assert_eq!(
        samples[1].bar,
        Some(BarData {
            top_bar_end_line: Some(70),
            bottom_bar_start_line: Some(506),
            left_bar_end_pixel: None,
            right_bar_start_pixel: None,
        })
    );
    for s in &samples[2..] {
        assert_eq!(s.bar, None);
    }
}

#[test]
fn afd_first_wins_in_one_au() {
    let sei = vec![vec![afd_nal(9, true), afd_nal(10, true)]];
    let ts = build_avc(&sei);
    let items = run_stream(&ts, AfdPersistence::UntilReplaced);
    let changes: Vec<&Seen> = items.iter().filter(|s| s.kind == "changed").collect();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].afd, Some(ActiveFormatDescription::Centre4x3));
}

#[test]
fn hevc_prefix_sei_afd_works() {
    let sei = vec![vec![hevc_afd_nal(9, true, true)]];
    let ts = build_hevc(&sei);
    let video_id = TsDemux::new()
        .unpackage(&ts)
        .expect("batch")
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .unwrap()
        .spec
        .track_id;
    let mut demux = StreamingTsDemux::new();
    let mut changed_count = 0;
    let mut afd0 = None;
    let mut drain = |demux: &mut StreamingTsDemux| {
        while let Some(ev) = demux.poll_event() {
            match ev {
                DemuxEvent::DisplayChanged {
                    track_id, display, ..
                } if track_id == video_id => {
                    changed_count += 1;
                    afd0 = afd0.or_else(|| afd_of(&display));
                }
                _ => {}
            }
        }
    };
    for chunk in ts.chunks(188 * 512) {
        demux.feed(chunk);
        drain(&mut demux);
    }
    demux.finish();
    drain(&mut demux);
    assert_eq!(changed_count, 1);
    assert_eq!(afd0, Some(ActiveFormatDescription::Centre4x3));
}

#[test]
fn hevc_suffix_sei_ignored() {
    let sei = vec![vec![hevc_afd_nal(9, true, false)]];
    let ts = build_hevc(&sei);
    let video_id = TsDemux::new()
        .unpackage(&ts)
        .expect("batch")
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .unwrap()
        .spec
        .track_id;
    let mut demux = StreamingTsDemux::new();
    let mut afd0 = None;
    let mut drain = |demux: &mut StreamingTsDemux| {
        while let Some(ev) = demux.poll_event() {
            match ev {
                DemuxEvent::DisplayChanged {
                    track_id, display, ..
                } if track_id == video_id => {
                    afd0 = afd0.or_else(|| afd_of(&display));
                }
                _ => {}
            }
        }
    };
    for chunk in ts.chunks(188 * 512) {
        demux.feed(chunk);
        drain(&mut demux);
    }
    demux.finish();
    drain(&mut demux);
    assert_eq!(afd0, None);
}

#[test]
fn hevc_prefix_plus_suffix_keeps_prefix() {
    let sei = vec![vec![
        hevc_afd_nal(9, true, true),
        hevc_afd_nal(10, true, false),
    ]];
    let ts = build_hevc(&sei);
    let video_id = TsDemux::new()
        .unpackage(&ts)
        .expect("batch")
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .unwrap()
        .spec
        .track_id;
    let mut demux = StreamingTsDemux::new();
    let mut afd0 = None;
    let mut drain = |demux: &mut StreamingTsDemux| {
        while let Some(ev) = demux.poll_event() {
            match ev {
                DemuxEvent::DisplayChanged {
                    track_id, display, ..
                } if track_id == video_id => {
                    afd0 = afd0.or_else(|| afd_of(&display));
                }
                _ => {}
            }
        }
    };
    for chunk in ts.chunks(188 * 512) {
        demux.feed(chunk);
        drain(&mut demux);
    }
    demux.finish();
    drain(&mut demux);
    assert_eq!(afd0, Some(ActiveFormatDescription::Centre4x3));
}
