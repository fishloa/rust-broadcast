//! A protected `encv` track maps to its wrapped codec and keeps its
//! `tkhd.track_ID`, so a consumer can address it.
use transmux::TrackId;

const CENC_VIDEO_TRACK_ID: u32 = 1;

#[test]
fn an_encrypted_video_track_maps_to_its_codec_and_carries_its_track_id() {
    let bytes = test_fixtures::read("mp4/h264-cenc.mp4");
    let media = test_harness::collect::media_from_mp4(
        &bytes[..],
        test_harness::collect::CollectMode::Seekable,
    )
    .unwrap();
    assert!(
        media.skipped.is_empty(),
        "the protected entry is reconstructed, not skipped"
    );
    let track = media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == TrackId::new(CENC_VIDEO_TRACK_ID))
        .expect("the protected track keeps its tkhd.track_ID");
    assert!(
        matches!(track.spec.config, transmux::CodecConfig::Avc { .. }),
        "encv maps to its frma original format avc1, got {:?}",
        track.spec.config
    );
    assert!(
        track.spec.protection.is_some(),
        "the sinf defaults stay on the track"
    );
}

#[test]
fn progressive_demux_names_the_protected_track_too() {
    let bytes = test_fixtures::read("mp4/cenc.mp4");
    let media = test_harness::collect::media_from_mp4(
        &bytes[..],
        test_harness::collect::CollectMode::Seekable,
    )
    .unwrap();
    assert!(media.skipped.is_empty());
    assert!(
        media
            .tracks
            .iter()
            .all(|t| t.spec.track_id != TrackId::new(0)),
        "every carried track has its real tkhd.track_ID"
    );
    assert!(
        media.tracks.iter().any(
            |t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. })
                && t.spec.protection.is_some()
        ),
        "the protected track maps to AVC with protection"
    );
}
