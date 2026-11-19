//! HEVC VPS/SPS/PPS validity gating (issue #1171): garbage sets are counted as
//! `InputDegraded::InvalidParameterSet` and never seed the `hvcC`; a clean copy
//! in the same access unit still resolves the track.
#![cfg(feature = "std")]

use broadcast_common::{Package, Unpackage};
use transmux::annexb::iter_length_prefixed_nals;
use transmux::ir::{CodecConfig, DemuxEvent, InputDegradation};
use transmux::nal::{NalCodec, nal_unit_type};
use transmux::{Media, StreamingTsDemux, Track, TsDemux, TsMux};

fn fixture() -> Vec<u8> {
    test_fixtures::read("ts/hevc/main.ts")
}

fn nals(data: &[u8]) -> Vec<Vec<u8>> {
    iter_length_prefixed_nals(data)
        .expect("length-prefixed NALs")
        .into_iter()
        .map(<[u8]>::to_vec)
        .collect()
}

fn join(nals: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in nals {
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

fn is_parameter_set(nal: &[u8]) -> bool {
    matches!(nal_unit_type(NalCodec::Hevc, nal), Some(32..=34))
}

/// The NAL with the same header/type/length but a body that cannot be a
/// parameter set: `nuh_temporal_id_plus1` cleared, which no valid set has.
fn garbage(nal: &[u8]) -> Vec<u8> {
    let mut out = nal.to_vec();
    for byte in &mut out[2..] {
        *byte = 0xFF;
    }
    out[1] &= 0xF8;
    out
}

fn video_track_mut(media: &mut Media) -> &mut Track {
    media
        .tracks
        .iter_mut()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("HEVC track")
}

fn damaged_ts(insert_clean_copy: bool) -> Vec<u8> {
    let mut media = TsDemux::new()
        .unpackage(fixture().as_slice())
        .expect("demux");
    {
        let track = video_track_mut(&mut media);
        let original = nals(&track.samples[0].data);
        let mut rebuilt: Vec<Vec<u8>> = Vec::new();
        if insert_clean_copy {
            rebuilt.extend(
                original
                    .iter()
                    .filter(|n| is_parameter_set(n))
                    .map(|n| garbage(n)),
            );
        }
        for nal in original {
            if is_parameter_set(&nal) {
                rebuilt.push(if insert_clean_copy {
                    nal
                } else {
                    garbage(&nal)
                });
            } else {
                rebuilt.push(nal);
            }
        }
        track.samples[0].data = join(&rebuilt).into();
    }
    TsMux::new().package(&media).expect("mux damaged TS")
}

fn clean_config() -> CodecConfig {
    let mut media = TsDemux::new()
        .unpackage(fixture().as_slice())
        .expect("demux");
    video_track_mut(&mut media).config().clone()
}

fn feed(ts: &[u8]) -> (usize, Vec<CodecConfig>, bool) {
    let mut demux = StreamingTsDemux::new();
    let mut invalid = 0usize;
    let mut configs = Vec::new();
    let drain = |demux: &mut StreamingTsDemux,
                 invalid: &mut usize,
                 configs: &mut Vec<CodecConfig>| {
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::InputDegraded {
                    kind: InputDegradation::InvalidParameterSet,
                    ..
                } => *invalid += 1,
                DemuxEvent::TrackAdded(spec) if matches!(spec.config, CodecConfig::Hevc { .. }) => {
                    configs.push(spec.config);
                }
                _ => {}
            }
        }
    };
    demux.feed(ts).expect("feed");
    drain(&mut demux, &mut invalid, &mut configs);
    demux.finish().expect("finish");
    drain(&mut demux, &mut invalid, &mut configs);
    let added = !configs.is_empty();
    (invalid, configs, added)
}

#[test]
fn a_garbage_hevc_parameter_set_is_counted_and_skipped() {
    let ts = damaged_ts(true);
    let (invalid, configs, added) = feed(&ts);
    assert_eq!(invalid, 3, "one event per garbage VPS/SPS/PPS");
    assert!(added, "the clean copies still resolve the track");
    assert_eq!(
        configs[0],
        clean_config(),
        "the hvcC comes from the clean copy"
    );
}

#[test]
fn a_garbage_only_hevc_stream_adds_no_track() {
    let ts = damaged_ts(false);
    let (invalid, _, added) = feed(&ts);
    assert_eq!(invalid, 3);
    assert!(!added, "no valid parameter set means no track");
}
