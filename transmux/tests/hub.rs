//! Any-to-any hub gate (issue #466).
//!
//! Exercises the `broadcast-common` container-mux traits as implemented by
//! transmux: [`Fmp4Demux`] ([`Unpackage`]) and [`CmafMux`] ([`Package`]).
//!
//! EXIT CRITERIA (all bite):
//! 1. `Fmp4Demux.unpackage(av_frag.mp4)` → `Media` with exactly 2 tracks
//!    (AVC video track_id=1, AAC audio track_id=2) with the same per-track
//!    sample counts computed independently from the raw `moof`/`trun` boxes.
//! 2. `CmafMux.package(&media)` → re-`unpackage` → identical track/sample
//!    structure (counts, durations, sync flags, composition offsets, AU bytes).
//! 3. `CmafMux::package` bytes == `build_init_segment ++ build_media_segment`
//!    for the equivalent input (the wrapper is byte-transparent).

use std::fs;
use std::path::PathBuf;

use broadcast_common::{Package, Parse, Unpackage};
use transmux::init_segment::MovieBox;
use transmux::media::{CmafMux, Fmp4Demux, Media};
use transmux::movie_fragment::MovieFragmentBox;
use transmux::pipeline::{
    build_init_segment, build_media_segment, CodecConfig, FragmentTrackData, TrackSpec,
};

fn fixture() -> Vec<u8> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "mp4",
        "cmaf",
        "av_frag.mp4",
    ]
    .iter()
    .collect();
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

/// Independently sum trun sample counts per track_id by walking the raw boxes,
/// so the demux is checked against a ground truth computed a different way.
fn expected_counts(file: &[u8]) -> std::collections::BTreeMap<u32, usize> {
    let mut counts: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    let mut off = 0usize;
    while off + 8 <= file.len() {
        let size =
            u32::from_be_bytes([file[off], file[off + 1], file[off + 2], file[off + 3]]) as usize;
        let ty = &file[off + 4..off + 8];
        if size < 8 {
            break;
        }
        if ty == b"moof" {
            let moof =
                MovieFragmentBox::parse_body(&file[off + 8..off + size]).expect("parse moof body");
            for traf in &moof.traf {
                let n: usize = traf.trun.iter().map(|t| t.samples.len()).sum();
                *counts.entry(traf.tfhd.track_id).or_insert(0) += n;
            }
        }
        off += size;
    }
    counts
}

#[test]
fn fmp4_demux_unpackage_two_tracks_correct_counts() {
    let file = fixture();
    let expected = expected_counts(&file);
    assert_eq!(expected.len(), 2, "fixture must carry exactly 2 tracks");

    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage av_frag.mp4");

    assert_eq!(media.tracks.len(), 2, "Media must have 2 tracks");

    // Track order follows the moov: track 1 = AVC video, track 2 = AAC audio.
    let vid = &media.tracks[0];
    let aud = &media.tracks[1];
    assert_eq!(vid.track_id(), 1, "first track is video track_id=1");
    assert_eq!(aud.track_id(), 2, "second track is audio track_id=2");
    assert!(
        matches!(vid.config(), CodecConfig::Avc { .. }),
        "track 1 must be AVC"
    );
    assert!(
        matches!(aud.config(), CodecConfig::Aac { .. }),
        "track 2 must be AAC"
    );

    for t in &media.tracks {
        let want = expected[&t.track_id()];
        assert!(want > 0, "each track must carry samples");
        assert_eq!(
            t.samples.len(),
            want,
            "track {} sample count must equal the summed trun counts",
            t.track_id()
        );
    }

    // At least one video sample must be a sync sample (random-access point).
    assert!(
        vid.samples.iter().any(|s| s.is_sync),
        "video track must contain a sync sample"
    );
}

#[test]
fn cmaf_package_reunpackage_preserves_structure() {
    let file = fixture();
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage");

    let mut mux = CmafMux::new(1);
    let repacked = mux.package(&media).expect("package");

    let mut demux2 = Fmp4Demux::new();
    let media2 = demux2.unpackage(&repacked).expect("re-unpackage");

    assert_eq!(
        media.tracks.len(),
        media2.tracks.len(),
        "track count preserved through package → unpackage"
    );
    for (a, b) in media.tracks.iter().zip(&media2.tracks) {
        assert_eq!(a.track_id(), b.track_id(), "track_id preserved");
        assert_eq!(a.timescale(), b.timescale(), "timescale preserved");
        assert_eq!(
            a.samples.len(),
            b.samples.len(),
            "track {} sample count preserved",
            a.track_id()
        );
        for (i, (sa, sb)) in a.samples.iter().zip(&b.samples).enumerate() {
            assert_eq!(
                sa.data,
                sb.data,
                "track {} sample {i} AU bytes preserved",
                a.track_id()
            );
            assert_eq!(
                sa.duration,
                sb.duration,
                "track {} sample {i} duration preserved",
                a.track_id()
            );
            assert_eq!(
                sa.is_sync,
                sb.is_sync,
                "track {} sample {i} sync flag preserved",
                a.track_id()
            );
            assert_eq!(
                sa.composition_offset,
                sb.composition_offset,
                "track {} sample {i} composition offset preserved",
                a.track_id()
            );
        }
    }
}

#[test]
fn cmaf_package_is_byte_transparent_over_build_helpers() {
    let file = fixture();
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("unpackage");

    // Package via the hub.
    let mut mux = CmafMux::new(7);
    let via_hub = mux.package(&media).expect("package");

    // Build the equivalent output directly with the existing helpers.
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut expected = build_init_segment(&specs, media.movie_timescale).expect("init");
    let fragments: Vec<FragmentTrackData<'_>> = media
        .tracks
        .iter()
        .map(|t| FragmentTrackData {
            track_id: t.spec.track_id,
            // CmafMux now anchors each fragment at the track's start_decode_time
            // (the demuxed tfdt), so the byte-transparent equivalent must too.
            base_media_decode_time: t.start_decode_time,
            samples: &t.samples,
        })
        .collect();
    let media_seg = build_media_segment(7, &fragments).expect("media segment");
    expected.extend_from_slice(&media_seg);

    assert_eq!(
        via_hub, expected,
        "CmafMux::package must be byte-identical to build_init_segment ++ build_media_segment"
    );

    // The output must itself re-parse as a valid movie + fragment.
    let find = |data: &[u8], fourcc: &[u8; 4]| -> Vec<u8> {
        let mut o = 0usize;
        while o + 8 <= data.len() {
            let sz = u32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as usize;
            if sz < 8 {
                break;
            }
            if &data[o + 4..o + 8] == fourcc {
                return data[o..o + sz].to_vec();
            }
            o += sz;
        }
        panic!("box {:?} not found", core::str::from_utf8(fourcc).unwrap());
    };
    let moov = find(&via_hub, b"moov");
    MovieBox::parse(&moov).expect("packaged moov re-parses");
    let moof = find(&via_hub, b"moof");
    MovieFragmentBox::parse_body(&moof[8..]).expect("packaged moof re-parses");
}

/// A `Media` with no tracks is rejected by the packagers.
#[test]
fn empty_media_rejected() {
    let media = Media::new(vec![], 1000);
    let mut mux = CmafMux::default();
    assert!(mux.package(&media).is_err(), "empty Media must not package");
}
