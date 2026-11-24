//! `transmux::mp4_demux::Mp4Demux` on real fixtures: both input shapes, the
//! review-focus cases, and a compare against the whole-input demuxers.

use test_harness::collect::{CollectMode, media_from_mp4};
use transmux::mp4_demux::Mp4DemuxConfig;

fn seekable(bytes: &[u8]) -> transmux::Media {
    media_from_mp4(bytes, CollectMode::Seekable).expect("seekable demux")
}

#[test]
fn a_progressive_fixture_demuxes_both_ways() {
    let bytes = test_fixtures::read("mp4/progressive/av-faststart.mp4");
    let seek = seekable(&bytes);
    assert!(!seek.tracks.is_empty(), "seekable tracks");
    for chunk in [1usize, 7, 65_536] {
        let seq = media_from_mp4(&bytes, CollectMode::Sequential { chunk }).expect("sequential");
        assert_eq!(
            seq.tracks.len(),
            seek.tracks.len(),
            "sequential chunk {chunk} track count"
        );
        for (a, b) in seek.tracks.iter().zip(&seq.tracks) {
            assert_eq!(a.spec, b.spec, "chunk {chunk} spec");
            assert_eq!(a.samples.len(), b.samples.len(), "chunk {chunk} samples");
        }
    }
}

#[test]
fn a_fragmented_fixture_demuxes_both_ways() {
    let bytes = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let seek = seekable(&bytes);
    assert!(!seek.tracks.is_empty(), "seekable tracks");
    for chunk in [1usize, 7, 65_536] {
        let seq = media_from_mp4(&bytes, CollectMode::Sequential { chunk }).expect("sequential");
        assert_eq!(seq.tracks.len(), seek.tracks.len(), "chunk {chunk} tracks");
    }
}

#[test]
fn feeding_the_wrong_shape_is_a_typed_error() {
    use broadcast_common::stage::{Stage, Timestamp};
    use transmux::mp4_demux::{Mp4Demux, Mp4Input};

    let mut seq = Mp4Demux::sequential(Mp4DemuxConfig::default());
    let err = seq
        .feed(
            Mp4Input::At {
                offset: 0,
                bytes: [0u8; 8].to_vec().into(),
            },
            Timestamp::ZERO,
        )
        .expect_err("At to a sequential stage");
    assert!(matches!(err, transmux::Error::InvalidInput(_)));

    let mut seek = Mp4Demux::seekable(Mp4DemuxConfig::default());
    let err = seek
        .feed(Mp4Input::Next(&[0u8; 8]), Timestamp::ZERO)
        .expect_err("Next to a seekable stage");
    assert!(matches!(err, transmux::Error::InvalidInput(_)));
}

#[test]
fn a_mismatched_range_names_both() {
    use broadcast_common::stage::{Stage, Timestamp};
    use transmux::mp4_demux::{Mp4Demux, Mp4Input};

    let mut seek = Mp4Demux::seekable(Mp4DemuxConfig::default());
    let err = seek
        .feed(
            Mp4Input::At {
                offset: 99,
                bytes: [0u8; 8].to_vec().into(),
            },
            Timestamp::ZERO,
        )
        .expect_err("wrong offset");
    assert!(
        matches!(
            err,
            transmux::Error::InputRangeMismatch {
                expected_offset: 0,
                expected_len: 8,
                got_offset: 99,
                got_len: 8,
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn a_second_moov_is_a_typed_error_telling_the_caller_to_start_a_new_stage() {
    let first = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let mut two = first.clone();
    two.extend_from_slice(&first);
    let err = media_from_mp4(&two, CollectMode::Seekable).expect_err("a second moov");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "got {err:?}"
    );
}

#[test]
fn non_interleaved_progressive_decodes_in_decode_time_order_with_a_window() {
    use broadcast_common::stage::{Stage, Timestamp};
    use std::num::NonZeroUsize;
    use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};

    // `write_big_mp4` lays every video chunk before every audio chunk: the
    // two tracks' data regions are far apart, which is what makes a naive
    // reader hold the distance. One 1 MiB window plus one sample must suffice.
    let path = std::env::temp_dir().join(format!(
        "transmux-mp4-demux-order-{}.mp4",
        std::process::id()
    ));
    let opts = test_harness::big_media::BigMp4Options {
        target_bytes: 2 * 1024 * 1024,
        sample_count: 40,
        moov_first: true,
        ..test_harness::big_media::BigMp4Options::default()
    };
    let written = test_harness::big_media::write_big_mp4(&path, &opts);
    let bytes = std::fs::read(&path).expect("read generated file");
    std::fs::remove_file(&path).expect("remove generated file");

    let window = NonZeroUsize::new(1024 * 1024).expect("non-zero window");
    let config = Mp4DemuxConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_read_window(Some(window));
    let mut demux = Mp4Demux::seekable(config);
    loop {
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(demand.want_bytes).min(bytes.len());
        demux
            .feed(
                Mp4Input::At {
                    offset,
                    bytes: bytes[start..end].to_vec().into(),
                },
                Timestamp::ZERO,
            )
            .expect("feed range");
    }
    demux.finish().expect("finish");

    let mut order = Vec::new();
    let mut last_dts = std::collections::BTreeMap::new();
    let mut count = 0u64;
    while let Some(event) = demux.poll() {
        if let transmux::DemuxEvent::Sample {
            track_id, sample, ..
        } = event
        {
            let dts = sample.timing.and_then(|timing| timing.dts()).expect("dts");
            let previous = last_dts.insert(track_id.value(), dts).unwrap_or(i64::MIN);
            assert!(
                dts >= previous,
                "track {} dts {dts} after {previous}",
                track_id.value()
            );
            order.push(track_id.value());
            count += 1;
        }
    }
    let expected: u64 = written.samples_per_track.iter().map(|(_, n)| n).sum();
    assert_eq!(count, expected, "sample count");
    assert!(
        order
            .iter()
            .take(4)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            >= 2,
        "decode-time order must interleave the two tracks, got {order:?}"
    );
}
