//! Dolby Vision profile 5 and 8.1 on the real Sol Levante captures
//! (`private/fixtures/dolby-vision/`, PROVENANCE.md): the record read from the
//! TS `DOVI_video_stream_descriptor` and from the MP4 `dvcC` / `dvvC` box, the
//! sample entry and box written into the init segment, and RPU NAL units
//! (type 62) surviving a repackage unchanged.
//!
//! Spec inputs: `private/specs-verified/dolby/vision/` (DV-NOTES.md).
#![cfg(feature = "std")]

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::dovi::{DVCC_FOURCC, DVH1_FOURCC, DVVC_FOURCC};
use transmux::ir::{CodecConfig, TrackSpec};
use transmux::ts_demux::TsDemux;
use transmux::{
    CmafMux, DoviBlSignalCompatibilityId, DoviDecoderConfigurationRecord, Fmp4Demux, Media,
    ProgressiveDemux, build_init_segment,
};

const P81_TS: &str = "dolby-vision/sollevante-dv-p8.1-1080p24-10s.ts";
const P81_MP4: &str = "dolby-vision/sollevante-dv-p8.1-1080p24-10s.mp4";
const P5_TS: &str = "dolby-vision/sollevante-dv-p5-1080p24-10s.ts";
const P5_MP4: &str = "dolby-vision/sollevante-dv-p5-1080p24-10s.mp4";
const P84_TS: &str = "dolby-vision/iphone18promax-dv-p8.4-2160p30-7s.ts";
const P84_MP4: &str = "dolby-vision/iphone18promax-dv-p8.4-2160p30-7s.mp4";
const P84_FRAMES: usize = 209;
const HLG_TS: &str = "hlg/sollevante-hlg-derived-1080p24-10s.ts";

const HEVC_RPU_NUT: u8 = 62;
const FRAMES: usize = 241;
const BOX_HEADER: usize = 8;
const VISUAL_ENTRY_PREFIX: usize = 78;
const STSD_PREFIX: usize = 8;
const CONTAINERS: [[u8; 4]; 6] = [*b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"stsd"];
type EntryBoxes = Vec<([u8; 4], Vec<u8>)>;
const MAX_MP4: usize = 64 * 1024 * 1024;

fn ts_media(fixture: &str) -> Option<Media> {
    let bytes = test_fixtures::require_bytes(fixture, "dovi")?;
    Some(TsDemux::new().unpackage(&bytes).expect("demux ts"))
}

fn video(media: &Media) -> &transmux::Track {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("an HEVC track")
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

fn sample_entry(file: &[u8]) -> ([u8; 4], EntryBoxes) {
    let mut level = children(file);
    for container in CONTAINERS {
        let body = level
            .iter()
            .find(|(t, _)| *t == container)
            .unwrap_or_else(|| panic!("no {}", String::from_utf8_lossy(&container)))
            .1;
        if container == *b"stsd" {
            let (fourcc, entry) = children(&body[STSD_PREFIX..])
                .into_iter()
                .next()
                .expect("sample entry");
            let boxes = children(&entry[VISUAL_ENTRY_PREFIX..])
                .into_iter()
                .map(|(t, b)| (t, b.to_vec()))
                .collect();
            return (fourcc, boxes);
        }
        level = children(body);
    }
    unreachable!()
}

fn payload(boxes: &[([u8; 4], Vec<u8>)], fourcc: [u8; 4]) -> Option<Vec<u8>> {
    boxes
        .iter()
        .find(|(t, _)| *t == fourcc)
        .map(|(_, b)| b.clone())
}

fn mp4_record(fixture: &str) -> Option<DoviDecoderConfigurationRecord> {
    let bytes = test_fixtures::require_bytes(fixture, "dovi")?;
    let media = ProgressiveDemux::new(MAX_MP4)
        .unwrap()
        .unpackage(&bytes)
        .expect("demux mp4");
    Some(
        video(&media)
            .spec
            .dovi
            .expect("dvcC/dvvC read from the mp4"),
    )
}

fn nal_types(sample: &[u8]) -> Vec<u8> {
    transmux::annexb::iter_length_prefixed_nals(sample)
        .expect("length-prefixed NALs")
        .iter()
        .map(|n| (n[0] >> 1) & 0x3F)
        .collect()
}

fn rpu_nals(sample: &[u8]) -> Vec<Vec<u8>> {
    transmux::annexb::iter_length_prefixed_nals(sample)
        .expect("length-prefixed NALs")
        .into_iter()
        .filter(|n| (n[0] >> 1) & 0x3F == HEVC_RPU_NUT)
        .map(<[u8]>::to_vec)
        .collect()
}

fn expect_p81(r: DoviDecoderConfigurationRecord) {
    assert_eq!(
        (r.version_major, r.version_minor, r.profile, r.level),
        (1, 0, 8, 3)
    );
    assert!(r.rpu_present && !r.el_present && r.bl_present);
    assert_eq!(
        r.bl_signal_compatibility_id,
        DoviBlSignalCompatibilityId::Hdr10
    );
}

fn expect_p84(r: DoviDecoderConfigurationRecord) {
    assert_eq!(
        (r.version_major, r.version_minor, r.profile, r.level),
        (1, 0, 8, 7)
    );
    assert!(r.rpu_present && !r.el_present && r.bl_present);
    assert_eq!(
        r.bl_signal_compatibility_id,
        DoviBlSignalCompatibilityId::Hlg
    );
}

fn expect_p5(r: DoviDecoderConfigurationRecord) {
    assert_eq!(
        (r.version_major, r.version_minor, r.profile, r.level),
        (1, 0, 5, 3)
    );
    assert!(r.rpu_present && !r.el_present && r.bl_present);
    assert_eq!(
        r.bl_signal_compatibility_id,
        DoviBlSignalCompatibilityId::None
    );
}

#[test]
fn ts_descriptor_yields_the_exact_p81_record() {
    let Some(media) = ts_media(P81_TS) else {
        return;
    };
    expect_p81(video(&media).spec.dovi.expect("dovi"));
}

#[test]
fn ts_descriptor_yields_the_exact_p84_record() {
    let Some(media) = ts_media(P84_TS) else {
        return;
    };
    expect_p84(video(&media).spec.dovi.expect("dovi"));
}

#[test]
fn ts_registration_plus_descriptor_yields_hevc_track_with_the_exact_p5_record() {
    let Some(media) = ts_media(P5_TS) else { return };
    expect_p5(video(&media).spec.dovi.expect("dovi"));
    assert_eq!(video(&media).samples.len(), FRAMES);
}

#[test]
fn mp4_boxes_yield_the_same_records_as_the_ts_descriptors() {
    if let Some(r) = mp4_record(P81_MP4) {
        expect_p81(r);
        assert_eq!(Some(r), ts_media(P81_TS).and_then(|m| video(&m).spec.dovi));
    }
    if let Some(r) = mp4_record(P84_MP4) {
        expect_p84(r);
        assert_eq!(Some(r), ts_media(P84_TS).and_then(|m| video(&m).spec.dovi));
    }
    if let Some(r) = mp4_record(P5_MP4) {
        expect_p5(r);
        assert_eq!(Some(r), ts_media(P5_TS).and_then(|m| video(&m).spec.dovi));
    }
}

#[test]
fn non_dolby_vision_streams_carry_no_record() {
    let Some(media) = ts_media(HLG_TS) else {
        return;
    };
    assert_eq!(video(&media).spec.dovi, None);
    let init = build_init_segment(&[video(&media).spec.clone()], 1000).expect("init");
    let (fourcc, boxes) = sample_entry(&init);
    assert_eq!(fourcc, *b"hvc1");
    assert!(payload(&boxes, DVCC_FOURCC).is_none() && payload(&boxes, DVVC_FOURCC).is_none());
}

fn init_for(fixture: &str) -> Option<([u8; 4], EntryBoxes)> {
    let media = ts_media(fixture)?;
    let init = build_init_segment(&[video(&media).spec.clone()], 1000).expect("init");
    Some(sample_entry(&init))
}

fn fixture_entry(fixture: &str) -> Option<([u8; 4], EntryBoxes)> {
    let bytes = test_fixtures::require_bytes(fixture, "dovi")?;
    Some(sample_entry(&bytes))
}

#[test]
fn p81_init_uses_hvc1_with_the_dvvc_box_equal_to_the_fixture_payload() {
    let Some((fourcc, ours)) = init_for(P81_TS) else {
        return;
    };
    let Some((_, theirs)) = fixture_entry(P81_MP4) else {
        return;
    };
    assert_eq!(fourcc, *b"hvc1");
    let box_ours = payload(&ours, DVVC_FOURCC).expect("dvvC written");
    assert_eq!(
        box_ours,
        payload(&theirs, DVVC_FOURCC).expect("fixture dvvC")
    );
    assert!(payload(&ours, DVCC_FOURCC).is_none());
}

#[test]
fn p84_init_uses_hvc1_with_the_dvvc_box_equal_to_the_fixture_payload() {
    let Some((fourcc, ours)) = init_for(P84_TS) else {
        return;
    };
    let Some((_, theirs)) = fixture_entry(P84_MP4) else {
        return;
    };
    assert_eq!(fourcc, *b"hvc1");
    let box_ours = payload(&ours, DVVC_FOURCC).expect("dvvC written");
    assert_eq!(
        box_ours,
        payload(&theirs, DVVC_FOURCC).expect("fixture dvvC")
    );
    assert!(payload(&ours, DVCC_FOURCC).is_none());
}

#[test]
fn p5_init_uses_dvh1_with_the_dvcc_box_equal_to_the_fixture_payload() {
    let Some((fourcc, ours)) = init_for(P5_TS) else {
        return;
    };
    let Some((_, theirs)) = fixture_entry(P5_MP4) else {
        return;
    };
    assert_eq!(fourcc, DVH1_FOURCC);
    let box_ours = payload(&ours, DVCC_FOURCC).expect("dvcC written");
    assert_eq!(
        box_ours,
        payload(&theirs, DVCC_FOURCC).expect("fixture dvcC")
    );
    assert!(payload(&ours, DVVC_FOURCC).is_none());
}

#[test]
fn init_round_trips_the_record_through_the_fmp4_demuxer() {
    for (fixture, check) in [
        (P81_TS, expect_p81 as fn(DoviDecoderConfigurationRecord)),
        (P5_TS, expect_p5),
        (P84_TS, expect_p84),
    ] {
        let Some(media) = ts_media(fixture) else {
            return;
        };
        let init = build_init_segment(&[video(&media).spec.clone()], 1000).expect("init");
        let back = Fmp4Demux::new().unpackage(&init).expect("fmp4 demux");
        let spec: &TrackSpec = &back.tracks[0].spec;
        check(spec.dovi.expect("dovi after round trip"));
    }
}

#[test]
fn rpu_nals_survive_the_cmaf_repackage_unchanged() {
    for (fixture, frames) in [(P81_TS, FRAMES), (P5_TS, FRAMES), (P84_TS, P84_FRAMES)] {
        let Some(media) = ts_media(fixture) else {
            return;
        };
        let source = video(&media);
        assert_eq!(source.samples.len(), frames);
        assert!(
            source.samples.iter().all(|s| rpu_nals(&s.data).len() == 1),
            "{fixture}: the source has exactly one RPU per frame"
        );
        let bytes = CmafMux::new(1).package(&media).expect("package");
        let out = Fmp4Demux::new().unpackage(&bytes).expect("re-demux");
        let out_video = video(&out);
        assert_eq!(out_video.samples.len(), frames);
        for (a, b) in source.samples.iter().zip(&out_video.samples) {
            assert_eq!(rpu_nals(&a.data), rpu_nals(&b.data), "{fixture}");
            assert!(nal_types(&b.data).contains(&HEVC_RPU_NUT));
        }
    }
}

#[test]
fn record_bytes_round_trip() {
    let Some(media) = ts_media(P81_TS) else {
        return;
    };
    let r = video(&media).spec.dovi.unwrap();
    assert_eq!(
        DoviDecoderConfigurationRecord::parse(&r.try_to_bytes().unwrap()).unwrap(),
        r
    );
}

const DESCRIPTOR_TAG: u8 = 0xB0;
const NONZERO_RESERVED_DESCRIPTOR: [u8; 5] = [0x01, 0x00, 0x10, 0x1D, 0x1F];

#[test]
fn descriptor_with_reserved_bits_set_still_signals_dolby_vision() {
    let Some(media) = ts_media(P81_TS) else {
        return;
    };
    let mut es_info = vec![DESCRIPTOR_TAG, NONZERO_RESERVED_DESCRIPTOR.len() as u8];
    es_info.extend_from_slice(&NONZERO_RESERVED_DESCRIPTOR);
    let spec = video(&media)
        .spec
        .clone()
        .with_source(mpeg_ts::pid::Pid::new(0x65), es_info);
    expect_p81(spec.dovi.expect("dovi despite non-zero reserved bits"));
    let init = build_init_segment(&[spec], 1000).expect("init");
    let (fourcc, boxes) = sample_entry(&init);
    assert_eq!(fourcc, *b"hvc1");
    assert!(payload(&boxes, DVVC_FOURCC).is_some());
}

#[test]
fn record_reserved_bits_survive_an_init_segment_round_trip() {
    let Some(media) = ts_media(P81_TS) else {
        return;
    };
    let mut spec = video(&media).spec.clone();
    let mut record = spec.dovi.expect("dovi");
    record.reserved[0] = 0x0A;
    record.reserved[3] = 0x5A;
    spec.dovi = Some(record);
    let init = build_init_segment(&[spec], 1000).expect("init");
    let (_, boxes) = sample_entry(&init);
    assert_eq!(
        payload(&boxes, DVVC_FOURCC).expect("dvvC"),
        record.try_to_bytes().unwrap()
    );
    let back = Fmp4Demux::new().unpackage(&init).expect("fmp4 demux");
    assert_eq!(back.tracks[0].spec.dovi, Some(record));
}

fn codecs_attrs(mpd: &str) -> Vec<String> {
    mpd.split("codecs=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap().to_string())
        .collect()
}

#[test]
fn p5_dash_codecs_name_the_dvh1_sample_entry() {
    let Some(media) = ts_media(P5_TS) else {
        return;
    };
    let spec = &video(&media).spec;
    let record = spec.dovi.as_ref().expect("p5 record");
    let expected = format!("dvh1.05.{:02}", record.level);
    assert_eq!(
        transmux::rfc6381_track_codec_string(spec).unwrap(),
        expected
    );
    let mpd = transmux::DashPackager::default().package(&media).unwrap();
    assert!(
        codecs_attrs(&mpd).contains(&expected),
        "DASH codecs {:?} must contain {expected}",
        codecs_attrs(&mpd)
    );
    let mut ll = transmux::ll_dash::LlDashPackager::new(2.0, 0.5, 3000, "2026-01-01T00:00:00Z")
        .unwrap();
    let ll_mpd = ll.package(&media).unwrap();
    assert!(codecs_attrs(&ll_mpd).contains(&expected));
}

#[test]
fn p81_dash_codecs_stay_hvc1() {
    let Some(media) = ts_media(P81_TS) else {
        return;
    };
    let codec = transmux::rfc6381_track_codec_string(&video(&media).spec).unwrap();
    assert!(codec.starts_with("hvc1."), "{codec}");
}
