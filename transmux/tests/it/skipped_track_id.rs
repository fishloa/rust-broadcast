//! A protected `encv` track, once skipped, is now carried as an ISOBMFF data
//! track — still by its `tkhd.track_ID`, so a consumer can address it.
use transmux::TrackId;
use transmux::ir::DataIdentity;

const CENC_VIDEO_TRACK_ID: u32 = 1;

#[test]
fn an_encrypted_video_track_skipped_by_fmp4_demux_carries_its_track_id() {
    let bytes = test_fixtures::read("mp4/h264-cenc.mp4");
    let media = test_harness::collect::media_from_mp4(
        &bytes[..],
        test_harness::collect::CollectMode::Seekable,
    )
    .unwrap();
    assert!(
        media.skipped.is_empty(),
        "the protected entry is carried as data, not skipped"
    );
    let track = media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == TrackId::new(CENC_VIDEO_TRACK_ID))
        .expect("the protected track keeps its tkhd.track_ID");
    let sample_entry = match &track.spec.config {
        transmux::CodecConfig::Data {
            identity:
                DataIdentity::Mp4 {
                    handler,
                    sample_entry,
                },
        } => {
            assert_eq!(handler, b"vide", "the entry's tkhd handler is carried");
            sample_entry
        }
        other => panic!("expected an ISOBMFF data identity, got {other:?}"),
    };
    assert_eq!(
        &sample_entry[4..8],
        b"encv",
        "the raw entry is the encv box"
    );
}

#[test]
fn progressive_demux_names_the_skipped_track_too() {
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
        media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, transmux::CodecConfig::Data { .. })),
        "the protected track arrives as data"
    );
}
