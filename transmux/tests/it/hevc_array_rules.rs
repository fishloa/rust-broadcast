//! `hvcC` array rules of ISO/IEC 14496-15:2017 §8.3.3.1.1 / §8.3.3.1.3 (array
//! `NAL_unit_type` restricted to VPS/SPS/PPS/prefix SEI/suffix SEI) and §8.4.1.1.1
//! (`array_completeness` for `hvc1` / `hev1`), on real `hvcC` boxes.
//!
//! Spec input: `private/specs-verified/mpeg/isobmff/iso-iec-14496-15-2017-avc-hevc-config.md`.
//! The wire stays tolerant (readers ignore unpermitted arrays; a real GPAC
//! `hvc1` file carries `array_completeness` 0 on its parameter sets), so
//! every record here must also round-trip byte for byte.

use broadcast_common::{Parse, Serialize};
use transmux::init_segment::{SampleDescriptionBox, SampleEntryVariant};
use transmux::sample_entries::HEVCSampleEntry;
use transmux::{HEVCDecoderConfigurationRecord, HevcNalArray, HevcNalUnit};

const HVC1_CONFORMANT: &str = "mp4/hevc-frag.mp4";
const HEV1_DEFAULT: &str = "mp4/hevc-main.mp4";
const HVC1_GPAC_INCOMPLETE: &str = "video/dolby-vision/sollevante-dv-p8.1-1080p24-10s.mp4";

const VPS: u8 = 32;
const SPS: u8 = 33;
const PPS: u8 = 34;
const AUD: u8 = 35;
const EOS: u8 = 36;
const PREFIX_SEI: u8 = 39;
const SUFFIX_SEI: u8 = 40;
const RESERVED_NON_IRAP: u8 = 41;

fn find_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let pos = data
        .windows(4)
        .position(|w| w == fourcc)
        .expect("box present");
    let start = pos - 4;
    let size = u32::from_be_bytes(data[start..start + 4].try_into().unwrap()) as usize;
    &data[start..start + size]
}

fn hevc_entry(bytes: &[u8]) -> HEVCSampleEntry {
    let stsd = SampleDescriptionBox::parse(find_box(bytes, b"stsd")).expect("stsd");
    match stsd.entries.into_iter().next() {
        Some(SampleEntryVariant::Hevc1(e)) => e,
        other => panic!("first stsd entry is not HEVC: {other:?}"),
    }
}

fn array(completeness: bool, nal_unit_type: u8) -> HevcNalArray {
    HevcNalArray::new(
        completeness,
        nal_unit_type,
        vec![HevcNalUnit(vec![nal_unit_type << 1, 0x01, 0xAA])],
    )
}

fn record(arrays: Vec<HevcNalArray>) -> HEVCDecoderConfigurationRecord {
    let template = hevc_entry(&test_fixtures::read(HVC1_CONFORMANT));
    HEVCDecoderConfigurationRecord {
        arrays,
        ..template.config.config
    }
}

#[test]
fn the_five_permitted_types_are_accepted_and_every_other_is_reported() {
    let permitted = [VPS, SPS, PPS, PREFIX_SEI, SUFFIX_SEI];
    let rec = record(permitted.iter().map(|&t| array(true, t)).collect());
    assert_eq!(rec.unpermitted_arrays().count(), 0);

    for bad in [0, 1, 19, AUD, EOS, 37, 38, RESERVED_NON_IRAP, 47, 63] {
        let rec = record(vec![array(true, VPS), array(false, bad)]);
        let found: Vec<u8> = rec.unpermitted_arrays().map(|a| a.nal_unit_type).collect();
        assert_eq!(found, [bad], "NAL_unit_type {bad} is not permitted");
    }
}

#[test]
fn unpermitted_arrays_are_kept_and_round_trip() {
    let rec = record(vec![
        array(true, VPS),
        array(true, SPS),
        array(true, PPS),
        array(false, AUD),
        array(false, RESERVED_NON_IRAP),
    ]);
    let bytes = rec.try_to_bytes().unwrap();
    let parsed = HEVCDecoderConfigurationRecord::parse(&bytes).expect("tolerant parse");
    assert_eq!(parsed, rec);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    let found: Vec<u8> = parsed
        .unpermitted_arrays()
        .map(|a| a.nal_unit_type)
        .collect();
    assert_eq!(found, [AUD, RESERVED_NON_IRAP]);
}

#[test]
fn real_hvc1_with_mandated_completeness_has_no_violation() {
    let bytes = test_fixtures::read(HVC1_CONFORMANT);
    let entry = hevc_entry(&bytes);
    assert_eq!(&entry.codec_type, b"hvc1");
    let types: Vec<(bool, u8)> = entry
        .config
        .config
        .arrays
        .iter()
        .map(|a| (a.array_completeness, a.nal_unit_type))
        .collect();
    assert_eq!(
        types,
        [(true, VPS), (true, SPS), (true, PPS), (false, PREFIX_SEI)]
    );
    assert_eq!(entry.array_completeness_violations().count(), 0);
    assert_eq!(entry.config.config.unpermitted_arrays().count(), 0);
}

#[test]
fn real_hev1_default_zero_is_not_a_violation() {
    let entry = hevc_entry(&test_fixtures::read(HEV1_DEFAULT));
    assert_eq!(&entry.codec_type, b"hev1");
    assert!(
        entry
            .config
            .config
            .arrays
            .iter()
            .all(|a| !a.array_completeness)
    );
    assert_eq!(entry.array_completeness_violations().count(), 0);
}

#[test]
fn real_gpac_hvc1_with_incomplete_parameter_sets_is_parsed_and_flagged() {
    let Some(bytes) = test_fixtures::require_bytes(HVC1_GPAC_INCOMPLETE, "hevc_array_rules") else {
        return;
    };
    let entry = hevc_entry(&bytes);
    assert_eq!(&entry.codec_type, b"hvc1");
    let flagged: Vec<u8> = entry
        .array_completeness_violations()
        .map(|a| a.nal_unit_type)
        .collect();
    assert_eq!(flagged, [VPS, SPS, PPS]);

    let original = find_box(&bytes, b"hvcC");
    let reserialized = entry.config.try_to_bytes().unwrap();
    assert_eq!(
        reserialized, original,
        "the tolerant parse must be lossless"
    );
}

#[test]
fn hvc1_completeness_rule_checks_both_directions_and_hev1_is_exempt() {
    let mut entry = hevc_entry(&test_fixtures::read(HVC1_CONFORMANT));
    entry.config.config.arrays = vec![
        array(false, VPS),
        array(true, SPS),
        array(true, PPS),
        array(true, PREFIX_SEI),
        array(false, SUFFIX_SEI),
    ];
    let flagged: Vec<u8> = entry
        .array_completeness_violations()
        .map(|a| a.nal_unit_type)
        .collect();
    assert_eq!(flagged, [VPS, PREFIX_SEI]);

    entry.codec_type = *b"hev1";
    assert_eq!(entry.array_completeness_violations().count(), 0);
}
