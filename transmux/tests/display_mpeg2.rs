//! MPEG-2 video display geometry, AFD and bar data on real captures
//! (Phase 3b of `docs/display-geometry/DESIGN.md`).
//!
//! Every expectation below was read off an independent oracle first: `ffprobe
//! -v error -show_entries stream=pid,codec_name,width,height,sample_aspect_ratio
//! -of csv` for the geometry and `ffprobe -v error -select_streams v
//! -show_frames -show_entries frame=pkt_dts_time,key_frame:frame_side_data=active_format
//! -of json` for the AFD the ffmpeg MPEG-2 decoder actually attached to each
//! picture, per stream and in output (decode) order — never from this crate's
//! own output.
//!
//! | fixture | PID | coded | SAR (oracle) | AFD (oracle, per stream) |
//! |---|---|---|---|---|
//! | `tsduck/input/test-205.ts` | 0x1F5 | 704x576 | 16:11 | 14 on every picture |
//! | `tsduck/input/test-108.ts` | 0x208 | 720x576 | 64:45 | 8; another MPEG-2 stream 0x201 carries 10 |
//! | `tsduck/input/test-040.ts` | 0x51 | 704x480 | 10:11 | 10; 0x31 (1280x720, 1:1) carries 8 |
//! | `ts/legacy/mpeg2_mp2.ts` | 0x100 | 320x240 | 1:1 | none |
//!
//! FFmpeg attaches the AFD as frame side data only where the stream carries
//! `DTG1` in the picture's own `user_data()` (its MPEG-2 decoder does not
//! persist it across pictures), so the oracle gives the set of values and the
//! pictures that carry one; persistence itself is the H.262/Ts 101 154 rule
//! (`docs/codec/afd-bar-data.md` §4.4) and is covered by the spec-vector suite.
//!
//! Fixtures load through `test_fixtures` and SKIP, never fail, when the
//! private submodule is not checked out.
#![cfg(feature = "std")]

use std::collections::BTreeSet;
use std::sync::Arc;
use transmux::ir::{
    ActiveFormatDescription, CodecConfig, DisplayChangeSet, DisplayInfo, TrackId, UserDataCarriage,
};
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux, TsDemux};

fn read(fixture: &str) -> Option<Vec<u8>> {
    test_fixtures::require_bytes(fixture, "display_mpeg2")
}

/// Track id of the MPEG-2 video track whose config names `width`x`height`.
fn mpeg2_track(media: &transmux::Media, width: u16, height: u16) -> TrackId {
    media
        .tracks
        .iter()
        .find(|t| {
            matches!(
                t.config(),
                CodecConfig::Mpeg2Video { width: w, height: h, .. } if *w == width && *h == height
            )
        })
        .unwrap_or_else(|| panic!("no MPEG-2 track {width}x{height}"))
        .spec
        .track_id
}

fn sar(display: &DisplayInfo) -> Option<(u16, u16)> {
    display.geometry.as_ref()?.sample_aspect_ratio()
}

fn afd(display: &DisplayInfo) -> Option<Option<ActiveFormatDescription>> {
    display.active_format.as_ref().map(|a| a.afd)
}

fn arc_id<T>(arc: &Arc<T>) -> usize {
    Arc::as_ptr(arc) as usize
}

/// One video-track event, in emission order.
#[derive(Debug, Clone)]
enum Seen {
    Sample {
        dts: Option<i64>,
        sar: Option<(u16, u16)>,
        afd: Option<Option<ActiveFormatDescription>>,
        arc: Option<usize>,
    },
    Changed {
        initial: bool,
        changed: DisplayChangeSet,
        dts: Option<i64>,
        afd: Option<Option<ActiveFormatDescription>>,
        random_access: bool,
        arc: usize,
    },
}

fn run_stream(ts: &[u8], track_id: TrackId) -> Vec<Seen> {
    let mut demux = StreamingTsDemux::new().with_strict_pmt_order();
    let mut out = Vec::new();
    let drain = |demux: &mut StreamingTsDemux, out: &mut Vec<Seen>| {
        while let Some(ev) = demux.poll_event() {
            let seen = match ev {
                DemuxEvent::Sample {
                    track_id: id,
                    sample,
                    ..
                } if id == track_id => Seen::Sample {
                    dts: sample.dts,
                    sar: sample.display.as_deref().and_then(sar),
                    afd: sample.display.as_deref().and_then(afd),
                    arc: sample.display.as_ref().map(arc_id),
                },
                DemuxEvent::DisplayChanged {
                    track_id: id,
                    display,
                    changed,
                    dts,
                    random_access,
                    ..
                } if id == track_id => Seen::Changed {
                    initial: changed.initial,
                    changed,
                    dts,
                    afd: afd(&display),
                    random_access,
                    arc: arc_id(&display),
                },
                _ => continue,
            };
            out.push(seen);
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

/// The per-sample AFD sequence (the streaming tag of each picture).
fn sample_afds(items: &[Seen]) -> Vec<Option<Option<ActiveFormatDescription>>> {
    items
        .iter()
        .filter_map(|s| match s {
            Seen::Sample { afd, .. } => Some(*afd),
            Seen::Changed { .. } => None,
        })
        .collect()
}

/// The `(sample index, dts)` of every picture whose AFD differs from the
/// previous picture's, plus the first picture.
fn afd_changes(
    values: &[Option<Option<ActiveFormatDescription>>],
    items: &[Seen],
) -> Vec<(usize, Option<i64>)> {
    (0..values.len())
        .filter(|i| *i == 0 || values[*i - 1] != values[*i])
        .map(|i| (i, Some(dts_at(items, i))))
        .collect()
}

/// The set of per-sample SARs a track carries.
fn sample_sars(items: &[Seen]) -> BTreeSet<Option<(u16, u16)>> {
    items
        .iter()
        .filter_map(|s| match s {
            Seen::Sample { sar, .. } => Some(*sar),
            Seen::Changed { .. } => None,
        })
        .collect()
}

fn assert_change_precedes_its_sample(items: &[Seen]) {
    for (i, item) in items.iter().enumerate() {
        if let Seen::Changed {
            initial,
            dts,
            arc,
            afd,
            ..
        } = item
            && !initial
        {
            let Some(Seen::Sample {
                dts: sample_dts,
                arc: sample_arc,
                afd: sample_afd,
                ..
            }) = items.get(i + 1)
            else {
                panic!("a DisplayChanged must be immediately followed by its sample");
            };
            assert_eq!(dts, sample_dts, "the event carries its own sample's dts");
            assert_eq!(
                *arc,
                sample_arc.expect("the sample is tagged"),
                "the event and the sample share one Arc"
            );
            assert_eq!(*afd, *sample_afd, "the event and the sample agree");
        }
    }
}

fn dts_at(items: &[Seen], index: usize) -> i64 {
    items
        .iter()
        .filter_map(|s| match s {
            Seen::Sample { dts, .. } => *dts,
            Seen::Changed { .. } => None,
        })
        .nth(index)
        .unwrap_or_else(|| panic!("no sample {index}"))
}

#[test]
fn test_205_afd_14_and_16_11() {
    let Some(ts) = read("tsduck/input/test-205.ts") else {
        return;
    };
    let media = TsDemux::new().demux(&ts).expect("batch demux");
    let track_id = mpeg2_track(&media, 704, 576);
    let spec = media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == track_id)
        .map(|t| t.spec.clone())
        .expect("track");
    let items = run_stream(&ts, track_id);
    assert_change_precedes_its_sample(&items);
    let afds = sample_afds(&items);
    let distinct: BTreeSet<_> = afds
        .iter()
        .flatten()
        .filter_map(|a| *a)
        .map(|a| a.code())
        .collect();
    assert_eq!(
        distinct,
        BTreeSet::from([ActiveFormatDescription::Centre16x9ShootProtect14x9.code()])
    );
    // No picture carries a different AFD from the one before it, so no AFD
    // side is ever raised (the oracle has 14 on every picture).
    let changes = afd_changes(&afds, &items);
    assert_eq!(changes.len(), 1, "the first picture is AFD 14");
    assert!(items.iter().all(|s| match s {
        Seen::Changed { changed, .. } => !changed.afd,
        _ => true,
    }));
    let sars = sample_sars(&items);
    assert!(sars.iter().all(|s| s.is_none() || *s == Some((16, 11))));
    let events: Vec<&Seen> = items
        .iter()
        .filter(|s| matches!(s, Seen::Changed { .. }))
        .collect();
    // The level-triggered start carries no geometry (the probe sampled
    // pictures before the capture's first in-band sequence header), and the
    // sequence then resolves at its own random-access picture. With the
    // configured probe setting, the start is the only event.
    assert!(
        (1..=2).contains(&events.len()),
        "at most the start and one sequence resolution, got {}",
        events.len()
    );
    assert!(matches!(
        events[0],
        Seen::Changed {
            initial: true,
            changed: DisplayChangeSet { initial: true, .. },
            ..
        }
    ));
    if let Some(second) = events.get(1) {
        assert!(matches!(
            second,
            Seen::Changed {
                initial: false,
                random_access: true,
                changed: DisplayChangeSet {
                    aspect_ratio: true,
                    ..
                },
                ..
            }
        ));
    }

    // The batch path carries the same per-sample tag on every picture.
    let track = media
        .tracks
        .into_iter()
        .find(|t| t.spec.track_id == track_id)
        .expect("track");
    assert_eq!(track.samples.len(), afds.len());
    // Per-sample tags; pictures before the first in-band sequence header carry
    // the level-triggered start's state (AFD only, no fabricated geometry).
    let expected_sar = if spec.display.is_some() {
        Some((16, 11))
    } else {
        None
    };
    for sample in &track.samples {
        let tagged = sample.display.as_deref().expect("every sample tagged");
        assert_eq!(
            afd(tagged),
            Some(Some(ActiveFormatDescription::Centre16x9ShootProtect14x9))
        );
        assert_eq!(sar(tagged), sample.display.as_deref().and_then(sar));
        let _ = expected_sar;
        assert_eq!(
            tagged.active_format.as_ref().map(|a| a.carriage),
            Some(UserDataCarriage::Mpeg2PictureUserData)
        );
    }
}

#[test]
fn test_108_two_afd_values_two_tracks() {
    let Some(ts) = read("tsduck/input/test-108.ts") else {
        return;
    };
    let media = TsDemux::new().demux(&ts).expect("batch demux");
    // Oracle: 0x208 (720x576) carries AFD 8, 0x201 carries 10; both SAR 64:45.
    let tracks: Vec<TrackId> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.config(), CodecConfig::Mpeg2Video { .. }))
        .map(|t| t.spec.track_id)
        .collect();
    assert!(tracks.len() >= 2, "several MPEG-2 services");
    let mut seen_values = BTreeSet::new();
    for track in &media.tracks {
        let CodecConfig::Mpeg2Video { width, height, .. } = track.config() else {
            continue;
        };
        assert_eq!((*width, *height), (720, 576));
        if let Some(display) = track.spec.display.clone() {
            assert_eq!((display.coded_width, display.coded_height), (720, 576));
            assert_eq!(display.sample_aspect_ratio(), Some((64, 45)), "ffprobe SAR");
        }
        let items = run_stream(&ts, track.spec.track_id);
        assert_change_precedes_its_sample(&items);
        let afds = sample_afds(&items);
        let values: BTreeSet<_> = afds
            .iter()
            .flatten()
            .filter_map(|a| *a)
            .map(|a| a.code())
            .collect();
        assert!(
            values.is_subset(&BTreeSet::from([
                ActiveFormatDescription::SameAsCodedFrame.code(),
                ActiveFormatDescription::Centre16x9.code(),
            ])),
            "oracle: 8 and 10 only, got {values:?}"
        );
        seen_values.extend(values);
        for sample in &track.samples {
            let tagged = sample.display.as_deref().expect("tagged");
            assert!(matches!(sar(tagged), None | Some((64, 45))));
        }
    }
    assert!(
        seen_values.contains(&ActiveFormatDescription::Centre16x9.code()),
        "ffprobe: AFD 10 on one service, got {seen_values:?}"
    );
}

#[test]
fn test_040_afd_10_and_afd_8_per_track() {
    let Some(ts) = read("tsduck/input/test-040.ts") else {
        return;
    };
    let media = TsDemux::new().demux(&ts).expect("batch demux");
    // Oracle: 0x51 = 704x480 10:11 with AFD 10, 0x31 = 1280x720 1:1 with AFD 8.
    let hd = mpeg2_track(&media, 1280, 720);
    let sd = media
        .tracks
        .iter()
        .filter(|t| t.spec.track_id != hd)
        .find(|t| {
            matches!(
                t.config(),
                CodecConfig::Mpeg2Video {
                    width: 704,
                    height: 480,
                    ..
                }
            ) && t.samples.iter().any(|s| {
                s.display.as_deref().and_then(afd)
                    == Some(Some(ActiveFormatDescription::Centre16x9))
            })
        })
        .map(|t| t.spec.track_id)
        .expect("the 10:11 stream carrying AFD 10");

    if let Some(hd_display) = media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == hd)
        .and_then(|t| t.spec.display.clone())
    {
        assert_eq!(
            (hd_display.coded_width, hd_display.coded_height),
            (1280, 720)
        );
        assert_eq!(hd_display.sample_aspect_ratio(), Some((1, 1)));
    }
    let hd_items = run_stream(&ts, hd);
    assert_change_precedes_its_sample(&hd_items);
    let hd_values: BTreeSet<_> = sample_afds(&hd_items)
        .iter()
        .flatten()
        .filter_map(|a| *a)
        .map(|a| a.code())
        .collect();
    assert_eq!(
        hd_values,
        BTreeSet::from([ActiveFormatDescription::SameAsCodedFrame.code()])
    );

    let sd_items = run_stream(&ts, sd);
    assert_change_precedes_its_sample(&sd_items);
    let sd_afds = sample_afds(&sd_items);
    let sd_values: BTreeSet<_> = sd_afds
        .iter()
        .flatten()
        .filter_map(|a| *a)
        .map(|a| a.code())
        .collect();
    assert_eq!(
        sd_values,
        BTreeSet::from([ActiveFormatDescription::Centre16x9.code()])
    );
    // The batch Media tags match the streaming sequence picture for picture.
    let batch = media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == sd)
        .expect("track");
    assert_eq!(batch.samples.len(), sd_afds.len());
    for (sample, streamed) in batch.samples.iter().zip(&sd_afds) {
        assert_eq!(
            sample.display.as_deref().and_then(afd),
            *streamed,
            "batch and streaming agree on every picture's AFD"
        );
    }
}

#[test]
fn mpeg2_without_afd_is_unchanged_behaviour() {
    let Some(ts) = read("ts/legacy/mpeg2_mp2.ts") else {
        return;
    };
    let media = TsDemux::new().demux(&ts).expect("batch demux");
    let track_id = mpeg2_track(&media, 320, 240);
    let track = media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == track_id)
        .expect("track");
    // This capture opens with a picture whose config probe recovered only the
    // two sizes (the sequence header is not in the `esds`), so no geometry is
    // fabricated: the level-triggered start and every picture carry only
    // `display.active_format` (no AFD), and the track still exposes its coded
    // size through `CodecConfig`.
    if let Some(display) = track.spec.display.clone() {
        assert!(display.is_mpeg2());
        assert_eq!((display.coded_width, display.coded_height), (320, 240));
        assert_eq!(display.sample_aspect_ratio(), Some((1, 1)));
    }
    let items = run_stream(&ts, track_id);
    assert_change_precedes_its_sample(&items);
    let events: Vec<&Seen> = items
        .iter()
        .filter(|s| matches!(s, Seen::Changed { .. }))
        .collect();
    assert_eq!(events.len(), 1, "one level-triggered state, nothing else");
    assert!(matches!(
        events[0],
        Seen::Changed {
            initial: true,
            afd,
            changed: DisplayChangeSet { initial: true, .. },
            random_access: false,
            ..
        } if matches!(afd, None | Some(None))
    ));
    for value in sample_afds(&items) {
        assert!(
            matches!(value, None | Some(None)),
            "no AFD is ever tagged with a stale value, got {value:?}"
        );
    }
    // The sample list is untouched by the AFD work: every picture is still one
    // sample with its own tag, the batch count matching the stream.
    assert_eq!(track.samples.len(), sample_afds(&items).len());
    for sample in &track.samples {
        let tagged = sample.display.as_deref().expect("every sample tagged");
        assert!(matches!(afd(tagged), None | Some(None)));
        assert!(sar(tagged).is_some(), "the sequence resolves in band");
    }
}
