//! HDR10 static metadata: detection from the real `meridian-hdr10-60f` HEVC
//! stream, and the `colr` / `mdcv` / `clli` boxes of the init segment built from
//! it, byte-compared against the boxes GPAC wrote for the same bitstream.
//!
//! Spec inputs: `private/specs-verified/mpeg/hdr/` (HDR10-NOTES.md,
//! isobmff-mdcv-clli-boxes.md). Fixtures load through `test_fixtures` and skip
//! loudly when the private submodule is absent.
#![cfg(feature = "std")]

use broadcast_common::Unpackage;
use transmux::ir::{CodecConfig, TrackSpec};
use transmux::ts_demux::TsDemux;
use transmux::{ChromaticityXy, HdrStaticMetadata, build_init_segment};

const TS: &str = "hdr10/meridian-hdr10-60f.ts";
const GPAC_MP4: &str = "hdr10/meridian-hdr10-60f-hvc1-mdcv-clli.mp4";
const NO_HDR_TS: &str = "ts/hevc/main10.ts";

const COLR: [u8; 4] = *b"colr";
const MDCV: [u8; 4] = *b"mdcv";
const CLLI: [u8; 4] = *b"clli";
const BOX_HEADER: usize = 8;
const HVC1_FIXED_PREFIX: usize = 78;
const STSD_PREFIX: usize = 8;
const CONTAINERS: [[u8; 4]; 6] = [*b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"stsd"];

fn video_track(bytes: &[u8]) -> TrackSpec {
    let media = TsDemux::new().unpackage(bytes).expect("demux");
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("an HEVC track")
        .spec
        .clone()
}

fn children(mut data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    while data.len() >= BOX_HEADER {
        let size = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
        if size < BOX_HEADER || size > data.len() {
            break;
        }
        out.push((data[4..8].try_into().unwrap(), &data[BOX_HEADER..size]));
        data = &data[size..];
    }
    out
}

fn sample_entry_children(file: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut level = children(file);
    for container in CONTAINERS {
        let body = level
            .iter()
            .find(|(t, _)| *t == container)
            .unwrap_or_else(|| panic!("no {}", String::from_utf8_lossy(&container)))
            .1;
        let body = if container == *b"stsd" {
            return entry_children(&body[STSD_PREFIX..]);
        } else {
            body
        };
        level = children(body);
    }
    unreachable!()
}

fn entry_children(entries: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let (_, entry) = children(entries).into_iter().next().expect("sample entry");
    children(&entry[HVC1_FIXED_PREFIX..])
        .into_iter()
        .map(|(t, b)| (t, b.to_vec()))
        .collect()
}

fn payload(boxes: &[([u8; 4], Vec<u8>)], fourcc: [u8; 4]) -> Vec<u8> {
    boxes
        .iter()
        .find(|(t, _)| *t == fourcc)
        .unwrap_or_else(|| panic!("no {} box", String::from_utf8_lossy(&fourcc)))
        .1
        .clone()
}

fn hdr_spec() -> Option<TrackSpec> {
    let bytes = test_fixtures::require_bytes(TS, "hdr10")?;
    Some(video_track(&bytes))
}

#[test]
fn detects_pq_bt2020_and_exact_static_metadata() {
    let Some(spec) = hdr_spec() else { return };
    let colour = spec
        .display
        .expect("display")
        .video_signal
        .expect("video signal")
        .colour
        .expect("colour description");
    assert_eq!(colour.primaries.code(), 9);
    assert_eq!(colour.transfer.code(), 16);
    assert_eq!(colour.matrix.code(), 9);
    assert_eq!(
        broadcast_hls::VideoRange::from_transfer_characteristics(colour.transfer.code()),
        Some(broadcast_hls::VideoRange::Pq)
    );

    let hdr = spec.hdr.expect("hdr static metadata");
    let m = hdr.mastering_display.expect("mastering display");
    let xy = |x, y| ChromaticityXy { x, y };
    assert_eq!(
        m.display_primaries,
        [xy(13250, 34500), xy(7500, 3000), xy(34000, 16000)]
    );
    assert_eq!(m.white_point, xy(15635, 16450));
    assert_eq!(m.max_display_mastering_luminance, 10_000_000);
    assert_eq!(m.min_display_mastering_luminance, 1);
    let c = hdr.content_light_level.expect("content light level");
    assert_eq!(c.max_content_light_level, 1000);
    assert_eq!(c.max_pic_average_light_level, 400);
}

#[test]
fn init_segment_boxes_equal_the_gpac_written_boxes() {
    let Some(spec) = hdr_spec() else { return };
    let Some(gpac) = test_fixtures::require_bytes(GPAC_MP4, "hdr10") else {
        return;
    };
    let init = build_init_segment(&[spec], 1000).expect("init");
    let ours = sample_entry_children(&init);
    let theirs = sample_entry_children(&gpac);
    for fourcc in [COLR, MDCV, CLLI] {
        assert_eq!(
            payload(&ours, fourcc),
            payload(&theirs, fourcc),
            "{fourcc:?}"
        );
    }
    let order: Vec<[u8; 4]> = ours.iter().map(|(t, _)| *t).collect();
    let pos = |f: [u8; 4]| order.iter().position(|t| *t == f).unwrap();
    assert!(pos(COLR) < pos(MDCV) && pos(MDCV) < pos(CLLI), "{order:?}");
}

#[test]
fn init_segment_round_trips_through_the_fmp4_demuxer() {
    let Some(spec) = hdr_spec() else { return };
    let mut boxes_only = spec.clone();
    let CodecConfig::Hevc { config, .. } = &mut boxes_only.config else {
        panic!("HEVC track");
    };
    config
        .config
        .arrays
        .retain(|a| a.nal_unit_type != transmux::hdr::HEVC_PREFIX_SEI_NUT);
    assert!(HdrStaticMetadata::from_hevc_record(&config.config).is_none());
    let init = build_init_segment(&[boxes_only], 1000).expect("init");
    let media = transmux::Fmp4Demux::new()
        .unpackage(&init)
        .expect("fmp4 demux");
    assert_eq!(media.tracks[0].spec.hdr, spec.hdr);
    assert!(spec.hdr.is_some());
}

#[test]
fn non_hdr_stream_gets_no_static_metadata_boxes() {
    let Some(bytes) = test_fixtures::require_bytes(NO_HDR_TS, "hdr10") else {
        return;
    };
    let spec = video_track(&bytes);
    assert_eq!(spec.hdr, None);
    let init = build_init_segment(&[spec], 1000).expect("init");
    let boxes = sample_entry_children(&init);
    assert!(
        boxes.iter().all(|(t, _)| *t != MDCV && *t != CLLI),
        "{boxes:?}"
    );
}

#[test]
fn codec_string_is_main10_with_decimal_level_from_the_real_sps() {
    let Some(spec) = hdr_spec() else { return };
    let CodecConfig::Hevc { config, .. } = &spec.config else {
        panic!("HEVC track");
    };
    assert_eq!(config.config.rfc6381(), "hvc1.2.4.L153.90");
}
