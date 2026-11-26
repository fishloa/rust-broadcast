//! Tracks with no typed `CodecConfig` arrive as ISOBMFF data tracks, and
//! per-sample CENC protection survives demux and re-mux.
//!
//! `Mp4Demux` used to abandon a track whose sample entry this crate cannot
//! map (`TrackAbandoned`, a `Media::skipped` record). It now carries the
//! complete sample entry as a [`DataIdentity::Mp4`] identity, samples
//! included, so a camera telemetry/timecode track is no longer dropped.

use test_harness::collect::Collected as Media;
use transmux::ir::{CodecConfig, DataIdentity};
use transmux::pipeline::{FragmentTrackData, build_media_segment};

fn demux(bytes: &[u8]) -> Media {
    test_harness::collect::media_from_mp4(bytes, test_harness::collect::CollectMode::Seekable)
        .expect("demux")
}

/// The raw sample entry and handler of an ISOBMFF data track, or `None`.
fn mp4_identity(track: &test_harness::collect::CollectedTrack) -> Option<([u8; 4], &[u8])> {
    match &track.spec.config {
        CodecConfig::Data {
            identity:
                DataIdentity::Mp4 {
                    handler,
                    sample_entry,
                },
        } => Some((*handler, sample_entry.as_ref())),
        _ => None,
    }
}

/// The GoPro capture's three unmapped tracks (`tmcd` timecode, `gpmd`
/// telemetry, `fdsc` face/scene metadata) arrive as data tracks with the
/// file's own raw sample-entry bytes and their full sample counts.
#[test]
fn gopro_unmapped_tracks_arrive_as_data_tracks() {
    let Some(path) = test_fixtures::require("mp4/camera/gopro-karma.mp4", "mp4_demux_data_tracks")
    else {
        return;
    };
    let bytes = std::fs::read(&path).expect("read gopro-karma.mp4");
    let media = demux(&bytes);
    assert!(
        media.skipped.is_empty(),
        "no track is abandoned any more, got {:?}",
        media.skipped
    );

    for (fourcc, handler_fourcc, sample_count) in [
        (*b"tmcd", *b"tmcd", 1usize),
        (*b"gpmd", *b"meta", 13),
        (*b"fdsc", *b"meta", 943),
    ] {
        let track = media
            .tracks
            .iter()
            .find(|t| mp4_identity(t).is_some_and(|(_, entry)| entry[4..8] == fourcc))
            .unwrap_or_else(|| {
                panic!(
                    "{:?} must arrive as a data track; tracks: {:?}",
                    core::str::from_utf8(&fourcc),
                    media
                        .tracks
                        .iter()
                        .map(|t| &t.spec.config)
                        .collect::<Vec<_>>()
                )
            });
        let (handler, sample_entry) = mp4_identity(track).expect("checked by the finder");
        assert_eq!(
            handler,
            handler_fourcc,
            "{:?} handler",
            core::str::from_utf8(&fourcc)
        );
        assert_eq!(
            track.samples.len(),
            sample_count,
            "{:?} sample count",
            core::str::from_utf8(&fourcc)
        );
        assert!(
            bytes
                .windows(sample_entry.len())
                .any(|window| window == sample_entry),
            "{:?}: the raw sample entry must be the file's own bytes",
            core::str::from_utf8(&fourcc)
        );
    }
}

/// A protected `encv` track demuxed by `Mp4Demux` carries its per-sample IVs
/// and subsample maps, and a fragmented re-mux writes exactly the source's
/// `senc` entries.
#[test]
#[cfg(feature = "cenc")]
fn protection_survives_demux_and_remux() {
    use isobmff::cenc::SampleEncryptionBox;

    let bytes = test_fixtures::read("mp4/h264-cenc.mp4");
    let media = demux(&bytes);
    let track = media
        .tracks
        .iter()
        .find(|t| t.samples.iter().any(|s| s.protection.is_some()))
        .expect("the protected video track carries per-sample protection");
    let protection = track
        .spec
        .protection
        .as_ref()
        .expect("the sample entry's sinf states the track protection");

    let want: Vec<isobmff::cenc::SampleEncryptionEntry> = track
        .samples
        .iter()
        .map(|sample| {
            let protection = sample.protection.as_ref().expect("per-sample protection");
            isobmff::cenc::SampleEncryptionEntry {
                initialization_vector: protection.iv.to_vec(),
                subsamples: protection.subsamples.clone(),
            }
        })
        .collect();
    assert_eq!(track.samples.len(), want.len(), "one entry per sample");

    let out = build_media_segment(
        1,
        &[FragmentTrackData::new(
            track.spec.track_id,
            track.start_decode_time,
            &track.samples,
        )],
    )
    .expect("re-mux the protected track");

    let moof = isobmff::BoxIter::new(&out)
        .flatten()
        .find(|b| b.four_cc() == isobmff::segments::MOOF)
        .expect("a moof in the re-muxed segment");
    let traf = isobmff::BoxIter::new(&moof.bytes[8..])
        .flatten()
        .find(|b| b.four_cc() == *b"traf")
        .expect("one traf");
    let senc = isobmff::BoxIter::new(&traf.bytes[8..])
        .flatten()
        .find(|b| b.four_cc() == isobmff::cenc::SENC)
        .expect("the fragment writer emitted senc");
    let version = senc.bytes[8];
    let flags = u32::from_be_bytes([0, senc.bytes[9], senc.bytes[10], senc.bytes[11]]);
    let got = SampleEncryptionBox::parse_body(
        &senc.bytes[12..],
        version,
        flags,
        protection.tenc.default_per_sample_iv_size,
    )
    .expect("parse the re-muxed senc");
    assert_eq!(got.entries, want, "the re-muxed senc matches the source");
}

/// A camera capture's data tracks (timecode, telemetry, face/scene metadata)
/// survive MP4 → CMAF → MP4: the track set, every sample's bytes, and each
/// data track's raw sample entry are unchanged.
#[test]
fn gopro_data_tracks_survive_cmaf_mux() {
    let Some(path) =
        test_fixtures::require("mp4/camera/gopro-hero6-ble.mp4", "mp4_demux_data_tracks")
    else {
        return;
    };
    let bytes = std::fs::read(&path).expect("read gopro-hero6-ble.mp4");
    let media = demux(&bytes);
    assert_eq!(media.tracks.len(), 5, "the capture carries five tracks");

    let out = crate::cmaf::package(&media).expect("CMAF mux");
    let remuxed = demux(&out);
    assert_eq!(
        remuxed.tracks.len(),
        media.tracks.len(),
        "every track (typed and data) must come back"
    );

    for track in &media.tracks {
        let other = remuxed
            .tracks
            .iter()
            .find(|t| t.spec.track_id == track.spec.track_id)
            .unwrap_or_else(|| {
                panic!(
                    "track {} is missing after the round trip",
                    track.spec.track_id
                )
            });
        assert_eq!(
            track.samples.len(),
            other.samples.len(),
            "track {} sample count",
            track.spec.track_id
        );
        for (index, (before, after)) in track.samples.iter().zip(&other.samples).enumerate() {
            assert_eq!(
                before.data, after.data,
                "track {} sample {index} bytes",
                track.spec.track_id
            );
        }
        match (mp4_identity(track), mp4_identity(other)) {
            (Some((handler, entry)), Some((other_handler, other_entry))) => {
                assert_eq!(handler, other_handler, "data track handler");
                assert_eq!(
                    entry, other_entry,
                    "data track's raw sample entry must round-trip byte-identically"
                );
            }
            (None, None) => {}
            (before, after) => panic!(
                "track {} changed between a data and a typed track: {before:?} -> {after:?}",
                track.spec.track_id
            ),
        }
    }
}

/// A protected `encv`/`enca` entry maps to the codec its `sinf`/`frma` wraps
/// (ISO/IEC 14496-12:2026 §13.4.1), with the protection kept on the track.
#[test]
#[cfg(feature = "cenc")]
fn protected_entries_map_to_their_original_codec() {
    for name in ["mp4/h264-cenc.mp4", "mp4/cenc.mp4"] {
        let bytes = test_fixtures::read(name);
        let media = demux(&bytes);
        let track = media
            .tracks
            .first()
            .unwrap_or_else(|| panic!("{name}: a video track"));
        assert!(
            matches!(track.spec.config, CodecConfig::Avc { .. }),
            "{name}: encv maps to its frma original format avc1, got {:?}",
            track.spec.config
        );
        assert!(
            track.spec.protection.is_some(),
            "{name}: the sinf defaults stay on the track"
        );
        assert!(
            track.samples.iter().all(|s| s.protection.is_some()),
            "{name}: every sample carries its senc entry"
        );
    }

    let bytes = test_fixtures::read("mp4/cenc-av-enca.mp4");
    let media = demux(&bytes);
    assert_eq!(media.tracks.len(), 2, "encv and enca both map");
    assert!(
        media
            .tracks
            .iter()
            .all(|t| !matches!(t.spec.config, CodecConfig::Data { .. })),
        "neither protected entry stays a data track"
    );
}
