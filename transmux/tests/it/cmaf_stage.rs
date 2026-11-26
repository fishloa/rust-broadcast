//! `CmafMuxStage`: one init, then one segment per fragment cut, with each
//! track's `tfdt` continuing across fragments.

use broadcast_common::{Stage, Timestamp};
use broadcast_core::timescale::Timescale;
use transmux::ir::CodecConfig;
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{CmafMuxStage, FragmentCut, TrackId};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("non-zero timescale")
}

fn aac_spec() -> TrackSpec {
    TrackSpec::new(
        TrackId::new(1),
        timescale(1000),
        CodecConfig::Aac {
            esds: isobmff::mp4esds::EsdsBox::new(isobmff::mp4esds::ESDescriptor::new(
                1,
                0,
                Some(isobmff::mp4esds::DecoderConfigDescriptor::new(
                    0x40,
                    0x05,
                    false,
                    0,
                    0,
                    0,
                    Some(isobmff::mp4esds::DecoderSpecificInfo::new(vec![0x12, 0x10])),
                )),
                Some(isobmff::mp4esds::SLConfigDescriptor::predefined_two()),
            )),
            channel_count: 2,
            sample_rate: 1000,
            sample_size: 16,
        },
    )
}

fn sample(dts: i64) -> Sample {
    Sample::new(vec![0u8; 4], Some(dts), Some(dts), Some(100), true).expect("valid timing")
}

/// A fragment cut on the anchor's keyframe emits a `Segment`, and the next
/// fragment's `tfdt` continues the track's decode time instead of restarting
/// at 0 (ISO/IEC 14496-12:2026 §8.8.12).
#[test]
fn fragments_continue_each_tracks_tfdt() {
    let spec = aac_spec();
    let track = spec.track_id;
    let mut stage = CmafMuxStage::new(
        timescale(1000),
        FragmentCut::AnchorKeyframe {
            track,
            target: core::time::Duration::from_millis(250),
        },
        1,
    );
    stage
        .feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)
        .expect("announce");
    for index in 0..10 {
        stage
            .feed(
                MuxInput::Sample {
                    track,
                    sample: sample(index * 100),
                },
                Timestamp::ZERO,
            )
            .expect("sample");
    }
    stage.finish().expect("finish");

    let mut init = Vec::new();
    let mut segments = Vec::new();
    while let Some(output) = stage.poll() {
        match output {
            MuxOutput::Init(bytes) => init.extend_from_slice(&bytes),
            MuxOutput::Segment(segment) => segments.push(segment.bytes),
            _ => {}
        }
    }
    assert!(!init.is_empty(), "the init is emitted once");
    assert!(
        segments.len() >= 2,
        "the target duration must cut more than one fragment, got {}",
        segments.len()
    );

    let mut file = init;
    for segment in &segments {
        file.extend_from_slice(segment);
    }
    let media =
        test_harness::collect::media_from_mp4(&file, test_harness::collect::CollectMode::Seekable)
            .expect("the concatenated output re-demuxes");
    assert_eq!(media.tracks.len(), 1);
    let got: Vec<i64> = media.tracks[0]
        .samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.dts()).expect("a timed sample"))
        .collect();
    let want: Vec<i64> = (0..10).map(|index| index * 100).collect();
    assert_eq!(got, want, "every sample keeps its own decode time");
}
