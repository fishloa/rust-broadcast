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

const SEI_PREFIX_NUT: u8 = 39;
const SEI_MDCV: u8 = 137;
const SEI_CLLI: u8 = 144;
const SEI_T35: u8 = 4;
const SEI_TRAILING: u8 = 0x80;
const HEVC_SEI_HEADER: [u8; 2] = [SEI_PREFIX_NUT << 1, 1];

fn escape(rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut zeros = 0;
    for &b in rbsp {
        if zeros >= 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

fn nal_type(nal: &[u8]) -> u8 {
    (nal[0] >> 1) & 0x3F
}

fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut zeros = 0;
    for &b in nal {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

fn sei_message_types(nal: &[u8]) -> Vec<u8> {
    let nal = unescape(nal);
    let mut types = Vec::new();
    let mut at = 2;
    while at + 1 < nal.len() && nal[at] != SEI_TRAILING {
        types.push(nal[at]);
        at += 2 + usize::from(nal[at + 1]);
    }
    types
}

#[test]
fn demux_keeps_only_declarative_sei_messages_in_the_hvcc_array() {
    use broadcast_common::Package;
    let Some(bytes) = test_fixtures::require_bytes(TS, "hdr10") else {
        return;
    };
    let mut media = TsDemux::new().unpackage(&bytes[..]).expect("demux");
    let idx = media
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .unwrap();
    let hdr = media.tracks[idx].spec.hdr.clone().expect("hdr");
    let mdcv = broadcast_common::Serialize::try_to_bytes(&hdr.mastering_display.unwrap()).unwrap();
    let clli =
        broadcast_common::Serialize::try_to_bytes(&hdr.content_light_level.unwrap()).unwrap();
    let other = [0xB5u8, 0x00, 0x3C, 0x00, 0x01, 0x04];

    let mut rbsp = Vec::new();
    for (t, p) in [
        (SEI_MDCV, &mdcv[..]),
        (SEI_T35, &other[..]),
        (SEI_CLLI, &clli[..]),
    ] {
        rbsp.push(t);
        rbsp.push(p.len() as u8);
        rbsp.extend_from_slice(p);
    }
    rbsp.push(SEI_TRAILING);
    let mut bundled = HEVC_SEI_HEADER.to_vec();
    bundled.extend(escape(&rbsp));

    let track = &mut media.tracks[idx];
    for (i, s) in track.samples.iter_mut().enumerate() {
        let mut out = Vec::new();
        let mut push = |nal: &[u8]| {
            out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            out.extend_from_slice(nal);
        };
        if i == 0 {
            push(&bundled);
        }
        for nal in transmux::annexb::iter_length_prefixed_nals(&s.data).unwrap() {
            if nal_type(nal) != SEI_PREFIX_NUT {
                push(nal);
            }
        }
        s.data = out.into();
    }
    let ts = transmux::TsMux::new().package(&media).expect("mux");
    let spec = video_track(&ts);
    let CodecConfig::Hevc { config, .. } = &spec.config else {
        unreachable!()
    };
    let sei: Vec<&Vec<u8>> = config
        .config
        .arrays
        .iter()
        .filter(|a| a.nal_unit_type == SEI_PREFIX_NUT)
        .flat_map(|a| a.nalus.iter().map(|n| &n.0))
        .collect();
    assert_eq!(sei.len(), 1);
    assert_eq!(sei_message_types(sei[0]), [SEI_MDCV, SEI_CLLI]);
    assert_eq!(
        spec.hdr
            .expect("hdr")
            .content_light_level
            .unwrap()
            .max_content_light_level,
        1000
    );
}
