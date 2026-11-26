//! Carry an opaque data track across container families (Phase 4 slice 8b).
//!
//! A camera capture's `gpmd` telemetry track is demuxed as an
//! [`DataIdentity::Mp4`] track, mapped to a TS private-data elementary stream
//! ([`DataIdentity::Ts`], `stream_type` `0x06` — ISO/IEC 13818-1 Table 2-34 —
//! plus a `registration_descriptor` (ISO/IEC 13818-1 §2.6.8, tag `0x05`) that
//! names the `gpmd` format), muxed to TS, demuxed back, mapped to the original
//! MP4 identity and muxed to CMAF: the sample bytes and the raw sample entry
//! are unchanged.

use broadcast_common::stage::{Stage, Timestamp};
use broadcast_core::Serialize;
use bytes::Bytes;
use dvb_si::descriptors::registration::RegistrationDescriptor;
use dvb_si::tables::pmt::StreamType;
use test_harness::collect::CollectMode;
use test_harness::collect::Collected as Media;
use transmux::ir::{CodecConfig, DataCarriage, DataIdentity, Sample, TrackSpec};
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{
    CmafMuxStage, DEFAULT_PSI_INTERVAL, DataIdentityMap, DataIdentityStage, FragmentCut,
    MkvMuxStage, StreamingTsDemux, TrackId, TsMuxStage,
};

const GPMD_FOURCC: [u8; 4] = *b"gpmd";
const GPMD_HANDLER: [u8; 4] = *b"meta";
const SAMPLE_ENTRY_LEN: usize = 8;

/// Drive `stage` from one track's samples through a [`DataIdentityStage`] that
/// applies `map`, then return every output the stage produced.
fn relay<S>(
    stage: &mut S,
    map: DataIdentityMap,
    spec: TrackSpec,
    samples: &[Sample],
) -> Result<Vec<MuxOutput>, transmux::Error>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = MuxOutput, Error = transmux::Error>,
{
    let track = spec.track_id;
    let mut identities = DataIdentityStage::new(map);
    identities.feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)?;
    while let Some(input) = identities.poll() {
        stage.feed(input, Timestamp::ZERO)?;
    }
    for sample in samples {
        identities.feed(
            MuxInput::Sample {
                track,
                sample: sample.clone(),
            },
            Timestamp::ZERO,
        )?;
        while let Some(input) = identities.poll() {
            stage.feed(input, Timestamp::ZERO)?;
        }
    }
    identities.finish()?;
    stage.finish()?;
    let mut out = Vec::new();
    while let Some(output) = stage.poll() {
        out.push(output);
    }
    Ok(out)
}

fn demux_ts(bytes: &[u8]) -> Media {
    let mut demux = StreamingTsDemux::new();
    demux.feed(bytes).expect("valid TS");
    demux.finish().expect("valid TS");
    let mut events = Vec::new();
    while let Some(event) = demux.poll_event() {
        events.push(event);
    }
    test_harness::collect::media_from_events(events)
}

/// The index of the capture's `gpmd` data track.
fn gpmd_index(media: &Media) -> Option<usize> {
    media.tracks.iter().position(|track| {
        matches!(
            &track.spec.config,
            CodecConfig::Data {
                identity: DataIdentity::Mp4 { sample_entry, .. },
            } if sample_entry.len() >= SAMPLE_ENTRY_LEN
                && sample_entry[4..SAMPLE_ENTRY_LEN] == GPMD_FOURCC
        )
    })
}

/// The TS identity a `gpmd` track carries in a TS output: PES-carried private
/// data (`stream_type` `0x06`, ISO/IEC 13818-1 Table 2-34) whose
/// `registration_descriptor` (ISO/IEC 13818-1 §2.6.8, tag `0x05`) names the
/// `gpmd` format.
fn gpmd_ts_identity() -> DataIdentity {
    let registration = RegistrationDescriptor {
        format_identifier: GPMD_FOURCC,
        additional_identification_info: &[],
    };
    DataIdentity::Ts {
        stream_type: StreamType::PesPrivateData.to_u8(),
        descriptors: registration
            .try_to_bytes()
            .expect("serialize registration descriptor"),
        carriage: DataCarriage::Pes,
    }
}

fn timescale() -> broadcast_core::timescale::Timescale {
    broadcast_core::timescale::Timescale::try_new(1_000).expect("non-zero timescale")
}

fn sample(dts: i64) -> Sample {
    Sample::new(vec![0xA5u8; 4], Some(dts), Some(dts), Some(1), true).expect("valid timing")
}

/// The stage rewrites only a mapped *data* track's announcement; samples and
/// every other input pass through byte-for-byte.
#[test]
fn data_identity_stage_rewrites_only_mapped_data_tracks() {
    let mp4 = DataIdentity::Mp4 {
        handler: GPMD_HANDLER,
        sample_entry: Bytes::from_static(b"\0\0\0\x08gpmd"),
    };
    let data_spec = TrackSpec::new(
        TrackId::new(1),
        timescale(),
        CodecConfig::Data {
            identity: mp4.clone(),
        },
    );
    let typed_spec = TrackSpec::new(
        TrackId::new(2),
        timescale(),
        CodecConfig::Vp8 {
            width: 640,
            height: 480,
        },
    );
    let map = DataIdentityMap::new().with(TrackId::new(1), gpmd_ts_identity());
    assert!(!map.is_empty());
    let mut stage = DataIdentityStage::new(map);
    stage
        .feed(MuxInput::Track(Box::new(data_spec)), Timestamp::ZERO)
        .expect("announce");
    stage
        .feed(
            MuxInput::Sample {
                track: TrackId::new(1),
                sample: sample(0),
            },
            Timestamp::ZERO,
        )
        .expect("sample");
    stage
        .feed(MuxInput::Track(Box::new(typed_spec)), Timestamp::ZERO)
        .expect("announce");
    stage.finish().expect("finish");

    let announced = match stage.poll() {
        Some(MuxInput::Track(spec)) => spec,
        other => panic!("expected a rewritten track announcement, got {other:?}"),
    };
    assert!(
        matches!(
            &announced.config,
            CodecConfig::Data {
                identity: DataIdentity::Ts { .. },
            }
        ),
        "the mapped data track carries the target identity, got {:?}",
        announced.config
    );

    match stage.poll() {
        Some(MuxInput::Sample { sample, .. }) => {
            assert_eq!(sample.data.as_ref(), &[0xA5u8; 4], "sample bytes unchanged");
        }
        other => panic!("expected the sample to pass through, got {other:?}"),
    }

    let typed = match stage.poll() {
        Some(MuxInput::Track(spec)) => spec,
        other => panic!("expected the typed announcement, got {other:?}"),
    };
    assert!(
        matches!(typed.config, CodecConfig::Vp8 { .. }),
        "a mapped track that is not a data track keeps its config, got {:?}",
        typed.config
    );
}

/// A `gpmd` telemetry track carried MP4 → TS → MP4 keeps every sample's bytes
/// and restores the MP4 sample entry byte-identically (Review Focus).
#[test]
fn gopro_gpmd_mp4_ts_mp4_round_trip() {
    let Some(path) = test_fixtures::require("mp4/camera/gopro-hero6-ble.mp4", "data_identity_map")
    else {
        return;
    };
    let bytes = std::fs::read(&path).expect("read gopro-hero6-ble.mp4");
    let original = test_harness::collect::media_from_mp4(&bytes, CollectMode::Seekable)
        .expect("demux the capture");
    let index = gpmd_index(&original).expect("the capture carries a gpmd data track");
    let source = &original.tracks[index];
    let (handler, sample_entry) = match &source.spec.config {
        CodecConfig::Data {
            identity:
                DataIdentity::Mp4 {
                    handler,
                    sample_entry,
                },
        } => (*handler, sample_entry.clone()),
        other => panic!("gpmd must arrive as a data track, got {other:?}"),
    };
    assert_eq!(handler, GPMD_HANDLER, "gpmd's handler");
    let samples = source.samples.clone();
    assert!(!samples.is_empty(), "the capture carries gpmd samples");

    // MP4 data track -> TS private-data elementary stream.
    let target = gpmd_ts_identity();
    let to_ts = DataIdentityMap::new().with(source.spec.track_id, target.clone());
    let mut ts_mux = TsMuxStage::new(Some(DEFAULT_PSI_INTERVAL));
    let ts = crate::mux_bytes::apply(
        relay(&mut ts_mux, to_ts, source.spec.clone(), &samples).expect("mux the gpmd track to TS"),
    );

    // TS -> IR: the same sample bytes, now with a TS identity whose
    // registration descriptor the PMT carried through.
    let ts_media = demux_ts(&ts);
    assert_eq!(ts_media.tracks.len(), 1, "one data track in the TS output");
    let carried = &ts_media.tracks[0];
    match &carried.spec.config {
        CodecConfig::Data {
            identity:
                DataIdentity::Ts {
                    stream_type,
                    descriptors,
                    carriage,
                },
        } => {
            assert_eq!(*stream_type, StreamType::PesPrivateData.to_u8());
            assert_eq!(*carriage, DataCarriage::Pes);
            match &target {
                DataIdentity::Ts {
                    descriptors: expected,
                    ..
                } => assert_eq!(
                    descriptors, expected,
                    "the PMT carries the mapped registration descriptor"
                ),
                other => panic!("unexpected target identity {other:?}"),
            }
        }
        other => panic!("the carried track keeps a TS identity, got {other:?}"),
    }
    assert_eq!(carried.samples.len(), samples.len(), "sample count");
    for (index, (before, after)) in samples.iter().zip(&carried.samples).enumerate() {
        assert_eq!(before.data, after.data, "gpmd sample {index} bytes");
    }

    // TS -> MP4: map back to the identity the first demux recorded.
    let to_mp4 = DataIdentityMap::new().with(
        carried.spec.track_id,
        DataIdentity::Mp4 {
            handler,
            sample_entry: sample_entry.clone(),
        },
    );
    let mut cmaf = CmafMuxStage::new(original.movie_timescale, FragmentCut::OnFlush, 1);
    let fmp4 = crate::mux_bytes::apply(
        relay(&mut cmaf, to_mp4, carried.spec.clone(), &carried.samples)
            .expect("mux the carried gpmd track to CMAF"),
    );
    let remuxed =
        test_harness::collect::media_from_mp4(&fmp4, CollectMode::Seekable).expect("re-demux CMAF");
    assert_eq!(remuxed.tracks.len(), 1, "one gpmd track in the MP4 output");
    let restored = &remuxed.tracks[0];
    assert_eq!(
        restored.samples.len(),
        samples.len(),
        "round-trip sample count"
    );
    for (index, (before, after)) in samples.iter().zip(&restored.samples).enumerate() {
        assert_eq!(
            before.data, after.data,
            "round-trip gpmd sample {index} bytes"
        );
    }
    match &restored.spec.config {
        CodecConfig::Data {
            identity:
                DataIdentity::Mp4 {
                    handler: restored_handler,
                    sample_entry: restored_entry,
                },
        } => {
            assert_eq!(*restored_handler, handler, "restored handler");
            assert_eq!(
                restored_entry.as_ref(),
                sample_entry.as_ref(),
                "the raw sample entry must be restored byte-identically"
            );
        }
        other => panic!("the gpmd track must come back as an MP4 data track, got {other:?}"),
    }
}

/// An unmapped MP4 data track fed to the TS muxer is a typed refusal naming
/// the track and both container families.
#[test]
fn unmapped_mp4_data_track_is_refused_by_ts_mux() {
    let spec = TrackSpec::new(
        TrackId::new(1),
        timescale(),
        CodecConfig::Data {
            identity: DataIdentity::Mp4 {
                handler: GPMD_HANDLER,
                sample_entry: Bytes::from_static(b"\0\0\0\x08gpmd"),
            },
        },
    );
    let mut ts_mux = TsMuxStage::new(None);
    let err = relay(&mut ts_mux, DataIdentityMap::new(), spec, &[sample(0)])
        .expect_err("an MP4 data track cannot be carried by a TS muxer");
    match err {
        transmux::Error::DataIdentityMismatch { track, have, want } => {
            assert_eq!(track, TrackId::new(1), "the error names the track");
            assert_eq!(have, "mp4", "the error names the track's family");
            assert_eq!(want, "ts", "the error names the muxer's family");
        }
        other => panic!("expected DataIdentityMismatch, got {other:?}"),
    }
}

/// A Matroska data identity round-trips through [`MkvMuxStage`].
#[test]
fn matroska_data_identity_round_trips_through_mkv_mux() {
    let codec_id = String::from("D_GPMD");
    let codec_private = b"\x01\x02\x03";
    let spec = TrackSpec::new(
        TrackId::new(1),
        timescale(),
        CodecConfig::Data {
            identity: DataIdentity::Matroska {
                codec_id: codec_id.clone(),
                codec_private: Bytes::from_static(codec_private),
            },
        },
    );
    let samples: Vec<Sample> = (0..3).map(sample).collect();
    let media = Media::new(
        vec![test_harness::collect::CollectedTrack::new(
            spec,
            samples.clone(),
        )],
        broadcast_core::timescale::Timescale::try_new(1_000).expect("non-zero timescale"),
    );
    let mkv = crate::mux_bytes::mux_bytes(
        &mut MkvMuxStage::new(core::time::Duration::from_millis(1_000)),
        &media,
    );
    let remuxed = test_harness::collect::media_from_matroska(
        &mkv,
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("re-demux the Matroska output");
    assert_eq!(remuxed.tracks.len(), 1, "one data track");
    assert_eq!(
        remuxed.tracks[0].samples.len(),
        samples.len(),
        "sample count"
    );
    for (index, (before, after)) in samples.iter().zip(&remuxed.tracks[0].samples).enumerate() {
        assert_eq!(before.data, after.data, "Matroska sample {index} bytes");
    }
    match &remuxed.tracks[0].spec.config {
        CodecConfig::Data {
            identity:
                DataIdentity::Matroska {
                    codec_id: restored_id,
                    codec_private: restored_private,
                },
        } => {
            assert_eq!(restored_id, &codec_id, "CodecID");
            assert_eq!(restored_private.as_ref(), codec_private, "CodecPrivate");
        }
        other => panic!("the track must come back as a Matroska data track, got {other:?}"),
    }
}
