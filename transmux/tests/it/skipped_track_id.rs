//! `Media::skipped` names the skipped `trak` by its `tkhd.track_ID`, so a
//! consumer can report one drop per track id.

use broadcast_common::Unpackage;
use transmux::{Fmp4Demux, ProgressiveDemux, TrackId};

const CENC_VIDEO_TRACK_ID: u32 = 1;
const PROGRESSIVE_CAP: usize = 16 * 1024 * 1024;

#[test]
fn an_encrypted_video_track_skipped_by_fmp4_demux_carries_its_track_id() {
    let bytes = test_fixtures::read("transmux/h264_cenc.mp4");
    let media = Fmp4Demux::new().unpackage(&bytes[..]).unwrap();
    let skipped: Vec<_> = media
        .skipped
        .iter()
        .map(|s| (s.fourcc.as_str(), s.track_id))
        .collect();
    assert_eq!(
        skipped,
        vec![("encv", Some(TrackId::new(CENC_VIDEO_TRACK_ID)))]
    );
    assert!(
        media
            .tracks
            .iter()
            .all(|t| t.spec.track_id != TrackId::new(CENC_VIDEO_TRACK_ID))
    );
}

#[test]
fn progressive_demux_names_the_skipped_track_too() {
    let bytes = test_fixtures::read("mp4/cenc.mp4");
    let media = ProgressiveDemux::new(PROGRESSIVE_CAP)
        .unwrap()
        .unpackage(&bytes[..])
        .unwrap();
    assert!(!media.skipped.is_empty());
    assert!(media.skipped.iter().all(|s| s.track_id.is_some()));
}
