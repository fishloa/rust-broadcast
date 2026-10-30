//! Explicit HE-AAC SBR/PS signalling for ADTS audio demuxed from MPEG-2 TS
//! (issue #1428, Apple HLS authoring rule: HE-AAC in fMP4 MUST use explicit
//! signalling).
//!
//! Real artifacts: `fixtures/ts/heaac/heaac_v{1,2}_adts.ts` (macOS AudioToolbox
//! HE-AAC / HE-AAC v2, core rate 22050 Hz, output rate 44100 Hz; commands in
//! `fixtures/ts/GENERATE.md`) and `fixtures/ts/h264_aac.ts` (plain AAC-LC).
//! The ASC layout is `private/specs-verified/mpeg/transmux/codec/heaac-asc.md`
//! (ISO/IEC 14496-3 §1.6.2). The DVB descriptor path builds its PMT only
//! through `dvb-si`'s serializer and the TS muxer.

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use dvb_si::descriptors::aac::{AacDescriptor, AacExtension};
use transmux::{
    AudioObjectType, AudioSpecificConfig, CodecConfig, EsdsBox, HeAacProfile, Media,
    SamplingFrequencyIndex, TrackSpec, TsDemux, TsMux, build_init_segment, rfc6381_codec_string,
};

const MOVIE_TIMESCALE: u32 = 1000;
const CORE_HZ: u32 = 22_050;
const OUTPUT_HZ: u32 = 44_100;
const PROFILE_AND_LEVEL: u8 = 0x58;
const AAC_TYPES_HE_V1: [u8; 8] = [0x01, 0x03, 0x05, 0x40, 0x41, 0x42, 0x47, 0x48];
const AAC_TYPES_HE_V2: [u8; 6] = [0x43, 0x44, 0x45, 0x46, 0x49, 0x4A];
const AAC_TYPE_HE_V2_STEREO: u8 = 0x43;
const AAC_TYPE_HE_OR_V2_SAOC_DE: u8 = 0xA0;
const ESDS: &[u8; 4] = b"esds";
const BOX_HEADER_LEN: usize = 8;

const V1_ASC: [u8; 4] = [0x2B, 0x92, 0x08, 0x00];
const V2_ASC: [u8; 4] = [0xEB, 0x8A, 0x08, 0x00];

fn demux(fixture: &str) -> Media {
    let bytes = test_fixtures::read(fixture);
    TsDemux::new().unpackage(&bytes).expect("demux fixture")
}

fn audio_spec(media: &Media) -> TrackSpec {
    media
        .tracks
        .iter()
        .map(|t| t.spec.clone())
        .find(|s| matches!(s.config, CodecConfig::Aac { .. }))
        .expect("AAC track")
}

fn asc_in_init(spec: &TrackSpec) -> AudioSpecificConfig {
    let init = build_init_segment(std::slice::from_ref(spec), MOVIE_TIMESCALE).expect("init");
    let at = init
        .windows(ESDS.len())
        .position(|w| w == ESDS)
        .expect("esds box")
        - 4;
    let size = u32::from_be_bytes([init[at], init[at + 1], init[at + 2], init[at + 3]]) as usize;
    assert!(size > BOX_HEADER_LEN);
    let esds = EsdsBox::parse_box(&init[at..at + size]).expect("esds parses");
    let dsi = esds
        .es_descriptor
        .decoder_config
        .and_then(|dc| dc.decoder_specific_info)
        .expect("DecoderSpecificInfo");
    AudioSpecificConfig::parse(&dsi.data).expect("ASC parses")
}

fn dsi_bytes(spec: &TrackSpec) -> Vec<u8> {
    asc_in_init(spec).try_to_bytes().expect("ASC serializes")
}

fn remux_with_aac_descriptor(fixture: &str, aac_type: u8) -> TrackSpec {
    let mut media = demux(fixture);
    let audio = media
        .tracks
        .iter_mut()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("AAC track");
    let descriptor = AacDescriptor {
        profile_and_level: PROFILE_AND_LEVEL,
        extension: Some(AacExtension::new(false, Some(aac_type), &[])),
    };
    audio.spec.es_info_descriptors = descriptor.try_to_bytes().expect("descriptor serializes");
    let muxed = TsMux::new().package(&media).expect("mux");
    let back = TsDemux::new().unpackage(&muxed).expect("re-demux");
    audio_spec(&back)
}

fn plain_init() -> Vec<u8> {
    let media = demux("ts/h264_aac.ts");
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    build_init_segment(&specs, MOVIE_TIMESCALE).expect("init")
}

#[test]
fn heaac_v1_override_writes_explicit_sbr_asc() {
    let plain = audio_spec(&demux("ts/heaac/heaac_v1_adts.ts"));
    assert_eq!(rfc6381_codec_string(&plain.config).unwrap(), "mp4a.40.2");
    let spec = plain
        .with_explicit_he_aac(HeAacProfile::V1)
        .expect("override applies");
    let asc = asc_in_init(&spec);
    assert_eq!(asc.audio_object_type.raw(), 5);
    assert_eq!(
        asc.sampling_frequency_index,
        SamplingFrequencyIndex::Fs22050
    );
    assert_eq!(asc.extension_sampling_frequency(), Some(CORE_HZ * 2));
    assert_eq!(asc.core_audio_object_type(), AudioObjectType::AacLc);
    assert!(asc.heaac_signaling().sbr_present);
    assert!(!asc.heaac_signaling().ps_present);
    assert_eq!(dsi_bytes(&spec), V1_ASC);
    assert_eq!(rfc6381_codec_string(&spec.config).unwrap(), "mp4a.40.5");
    assert_eq!(spec.config.audio_sample_rate(), Some(OUTPUT_HZ));
    assert_eq!(spec.timescale, CORE_HZ);
}

#[test]
fn heaac_v2_override_writes_explicit_ps_asc() {
    let spec = audio_spec(&demux("ts/heaac/heaac_v2_adts.ts"))
        .with_explicit_he_aac(HeAacProfile::V2)
        .expect("override applies");
    let asc = asc_in_init(&spec);
    assert_eq!(asc.audio_object_type.raw(), 29);
    assert_eq!(
        asc.sampling_frequency_index,
        SamplingFrequencyIndex::Fs22050
    );
    assert_eq!(asc.extension_sampling_frequency(), Some(CORE_HZ * 2));
    assert_eq!(asc.core_audio_object_type(), AudioObjectType::AacLc);
    assert!(asc.heaac_signaling().sbr_present);
    assert!(asc.heaac_signaling().ps_present);
    assert_eq!(dsi_bytes(&spec), V2_ASC);
    assert_eq!(rfc6381_codec_string(&spec.config).unwrap(), "mp4a.40.29");
    assert_eq!(spec.config.audio_sample_rate(), Some(OUTPUT_HZ));
    let CodecConfig::Aac { channel_count, .. } = spec.config else {
        panic!("AAC");
    };
    assert_eq!(channel_count, 2);
}

#[test]
fn heaac_descriptor_marks_track_he_aac() {
    let v2 = remux_with_aac_descriptor("ts/heaac/heaac_v2_adts.ts", AAC_TYPE_HE_V2_STEREO);
    assert_eq!(v2.audio_metadata().he_aac, Some(HeAacProfile::V2));
    assert_eq!(rfc6381_codec_string(&v2.config).unwrap(), "mp4a.40.29");
    let asc = asc_in_init(&v2);
    assert_eq!(asc.audio_object_type.raw(), 29);
    assert_eq!(asc.extension_sampling_frequency(), Some(OUTPUT_HZ));
    assert_eq!(asc.core_audio_object_type(), AudioObjectType::AacLc);
    assert_eq!(dsi_bytes(&v2), V2_ASC);
    for ct in AAC_TYPES_HE_V2 {
        let spec = remux_with_aac_descriptor("ts/heaac/heaac_v2_adts.ts", ct);
        assert_eq!(
            rfc6381_codec_string(&spec.config).unwrap(),
            "mp4a.40.29",
            "component_type {ct:#04x}"
        );
    }
}

#[test]
fn heaac_v1_descriptor_alone_leaves_track_aac_lc() {
    let plain = dsi_bytes(&audio_spec(&demux("ts/heaac/heaac_v1_adts.ts")));
    for ct in AAC_TYPES_HE_V1 {
        let spec = remux_with_aac_descriptor("ts/heaac/heaac_v1_adts.ts", ct);
        assert_eq!(
            spec.audio_metadata().he_aac,
            None,
            "component_type {ct:#04x}"
        );
        assert_eq!(rfc6381_codec_string(&spec.config).unwrap(), "mp4a.40.2");
        let asc = asc_in_init(&spec);
        assert_eq!(asc.audio_object_type, AudioObjectType::AacLc);
        assert_eq!(dsi_bytes(&spec), plain, "component_type {ct:#04x}");
    }
}

#[test]
fn heaac_descriptor_that_cannot_tell_v1_from_v2_changes_nothing() {
    let spec = remux_with_aac_descriptor("ts/heaac/heaac_v1_adts.ts", AAC_TYPE_HE_OR_V2_SAOC_DE);
    assert_eq!(spec.audio_metadata().he_aac, None);
    assert_eq!(rfc6381_codec_string(&spec.config).unwrap(), "mp4a.40.2");
}

#[test]
fn plain_aac_without_signal_is_unchanged() {
    let golden = include_bytes!("golden/heaac/h264_aac_init_origin_main.bin");
    assert_eq!(
        plain_init(),
        golden.as_slice(),
        "init of h264_aac.ts must stay byte-identical to origin/main (3ee2808eb)"
    );
}

#[test]
fn heaac_fixture_without_signal_stays_implicit() {
    let spec = audio_spec(&demux("ts/heaac/heaac_v1_adts.ts"));
    assert_eq!(rfc6381_codec_string(&spec.config).unwrap(), "mp4a.40.2");
    assert_eq!(spec.audio_metadata().he_aac, None);
}

#[test]
fn explicit_he_aac_is_rewritable_and_rejects_what_it_cannot_express() {
    let spec = audio_spec(&demux("ts/heaac/heaac_v1_adts.ts"));
    let v1 = spec
        .clone()
        .with_explicit_he_aac(HeAacProfile::V1)
        .expect("first application");
    let v2 = v1
        .clone()
        .with_explicit_he_aac(HeAacProfile::V2)
        .expect("explicit v1 re-expressed as v2");
    assert_eq!(rfc6381_codec_string(&v2.config).unwrap(), "mp4a.40.29");
    let back = v2
        .with_explicit_he_aac(HeAacProfile::V1)
        .expect("explicit v2 re-expressed as v1");
    assert_eq!(dsi_bytes(&back), dsi_bytes(&v1));
    assert_eq!(back.config, v1.config);

    let video = demux("ts/h264_aac.ts")
        .tracks
        .iter()
        .map(|t| t.spec.clone())
        .find(|s| !matches!(s.config, CodecConfig::Aac { .. }))
        .expect("non-AAC track");
    assert!(video.with_explicit_he_aac(HeAacProfile::V1).is_err());

    let asc = asc_in_init(&spec);
    let mut with_extension = asc.clone();
    with_extension.trailing = vec![0xAA];
    assert!(
        with_extension
            .with_explicit_he_aac(HeAacProfile::V1)
            .is_err()
    );

    let mut odd_hierarchical = asc_in_init(&v1);
    odd_hierarchical.trailing_top = Some(0);
    assert!(
        odd_hierarchical
            .with_explicit_he_aac(HeAacProfile::V1)
            .is_err(),
        "a hierarchical config whose tail is not ours is not silently rewritten"
    );
}

#[test]
fn he_aac_profile_labels() {
    assert_eq!(HeAacProfile::V1.to_string(), "HE-AAC v1 (SBR)");
    assert_eq!(HeAacProfile::V2.to_string(), "HE-AAC v2 (SBR+PS)");
}
