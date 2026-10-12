// SPDX-License-Identifier: MIT OR Apache-2.0
//! Issue #1162 R3 — per-track [`VideoScan`] and per-sample [`PictureFacts`]
//! over the committed real/spec-built streams. The expected values were
//! observed from the real captures and cross-checked against the slice headers
//! (`field_pic_flag`/`bottom_field_flag`) and SEI `pic_struct` those streams
//! actually carry; the synthetic HEVC/HRD layouts live in `sps.rs`'s tests.

use broadcast_common::Unpackage;
use std::path::PathBuf;
use transmux::{Media, PicStruct, PictureFacts, TsDemux, VideoScan, video_picture_facts};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel)
}

/// The track of `rel` that has a video scan, with its per-sample facts.
fn facts_of(rel: &str) -> (VideoScan, Vec<PictureFacts>) {
    let ts = std::fs::read(fixture(rel)).expect("fixture");
    let m: Media = TsDemux::new().unpackage(&ts).expect("demux");
    let t = m
        .tracks
        .iter()
        .find(|t| t.spec.config.video_scan().is_some())
        .expect("a video track");
    let scan = t.spec.config.video_scan().unwrap();
    let facts = t
        .samples
        .iter()
        .map(|s| video_picture_facts(&t.spec.config, &s.data).expect("facts for every sample"))
        .collect();
    (scan, facts)
}

#[test]
fn progressive_streams_are_progressive_frames() {
    for rel in [
        "fixtures/ts/h264/main.ts",
        "fixtures/ts/h264/high.ts",
        "fixtures/ts/h264/baseline.ts",
        "fixtures/ts/h264_aac.ts",
        "fixtures/ts/hevc/main.ts",
        "fixtures/ts/hevc/main10.ts",
    ] {
        let (scan, facts) = facts_of(rel);
        assert_eq!(scan, VideoScan::Progressive, "{rel}");
        assert!(!facts.is_empty());
        for f in &facts {
            assert!(!f.field_pic && f.fields == 0, "{rel}: {f:?}");
            assert_eq!((f.bottom_field_first, f.pic_struct), (None, None), "{rel}");
        }
    }
}

#[test]
fn mbaff_stream_reports_frames_with_pic_struct_top_bottom() {
    let (scan, facts) = facts_of("fixtures/ts/h264/interlaced.ts");
    assert_eq!(scan, VideoScan::Mbaff);
    assert_eq!(facts.len(), 16);
    for f in &facts {
        // MBAFF pictures are frames (field_pic_flag 0); the stream's picture
        // timing SEI says top field first.
        assert!(!f.field_pic && f.fields == 0);
        assert_eq!(f.pic_struct, Some(PicStruct::TopBottom));
        assert_eq!(f.bottom_field_first, Some(false));
    }
}

#[test]
fn mbaff_capable_stream_may_still_code_field_pairs() {
    // France 2 DVB capture: an MBAFF-capable SPS whose pictures are a mix of
    // MBAFF frames and field pairs ("mixed").
    let (scan, facts) = facts_of("fixtures/ts/france2.ts");
    assert_eq!(scan, VideoScan::Mbaff);
    let frames = facts.iter().filter(|f| !f.field_pic).count();
    let pairs = facts
        .iter()
        .filter(|f| f.field_pic && f.fields == 2)
        .count();
    assert_eq!((frames, pairs, facts.len()), (18, 48, 66));
    for f in facts.iter().filter(|f| f.field_pic) {
        assert_eq!(f.pic_struct, Some(PicStruct::TopField));
        assert_eq!(f.bottom_field_first, Some(false));
    }
}

#[test]
fn real_paff_capture_reports_field_pairs() {
    let (scan, facts) = facts_of("fixtures/ts/gulli-opengop.ts");
    assert_eq!(scan, VideoScan::Paff);
    assert_eq!(facts.len(), 147);
    // 146 complementary pairs (top field first) plus the lone leading field of
    // the capture that the pairing could not complete.
    assert_eq!(facts.iter().filter(|f| f.fields == 2).count(), 146);
    assert_eq!(facts.iter().filter(|f| f.fields == 1).count(), 1);
    assert!(facts.iter().all(|f| f.field_pic));
    assert!(facts.iter().all(|f| f.bottom_field_first == Some(false)));
}

#[test]
fn synthetic_paff_reports_parity_of_the_first_field() {
    let (scan, tff) = facts_of("transmux/tests/fixtures/paff/paff_tff_12f_64x64.ts");
    assert_eq!(scan, VideoScan::Paff);
    assert_eq!(tff.len(), 12);
    assert!(
        tff.iter()
            .all(|f| f.field_pic && f.fields == 2 && f.bottom_field_first == Some(false))
    );
    let (_, bff) = facts_of("transmux/tests/fixtures/paff/paff_bff_12f_64x64.ts");
    assert!(
        bff.iter()
            .all(|f| f.field_pic && f.fields == 2 && f.bottom_field_first == Some(true))
    );
    // No SEI in the synthetic streams: nothing is invented.
    assert!(tff.iter().chain(&bff).all(|f| f.pic_struct.is_none()));
}

#[test]
fn non_video_and_garbage_yield_none() {
    let ts = std::fs::read(fixture("fixtures/ts/h264_aac.ts")).unwrap();
    let m = TsDemux::new().unpackage(&ts).unwrap();
    let audio = m
        .tracks
        .iter()
        .find(|t| t.spec.config.video_scan().is_none())
        .unwrap();
    assert!(video_picture_facts(&audio.spec.config, &audio.samples[0].data).is_none());
    let video = m
        .tracks
        .iter()
        .find(|t| t.spec.config.video_scan().is_some())
        .unwrap();
    // A length prefix running past the end of the sample.
    assert!(video_picture_facts(&video.spec.config, &[0, 0, 0xFF, 0xFF, 1, 2]).is_none());
    assert!(video_picture_facts(&video.spec.config, &[]).is_none());
}
