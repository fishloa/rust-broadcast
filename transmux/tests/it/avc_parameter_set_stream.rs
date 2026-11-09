//! AVC parameter set stream (ISO/IEC 14496-15:2017 §5.4.3): the `avcp` sample
//! entry, the `AVCParameterSample` and the `avcp` track reference, on real data.
//!
//! The `tref` boxes come from `fixtures/mp4/tref-avcp.mp4` (GPAC `MP4Box -ref`,
//! see `fixtures/mp4/PROVENANCE.md`). No real `avcp` track exists in the
//! workspace, so the entry is the real `avc1` entry of `av-prog.mp4` renamed
//! to `avcp` with its record's parameter sets removed (§5.4.3.1), and the
//! parameter set sample is made of that record's own SPS and PPS.
//!
//! Spec input: `private/specs-verified/mpeg/isobmff/iso-iec-14496-15-2017-avc-hevc-config.md`.

use broadcast_common::error::InvalidValue;
use broadcast_common::{Parse, Serialize};
use transmux::avc_parameter_set::{AVCP_SAMPLE_ENTRY, AVCParameterSample, AVCParameterSampleEntry};
use transmux::init_segment::{MovieBox, SampleDescriptionBox, SampleEntryVariant};
use transmux::track_reference::{AVCP_REFERENCE_TYPE, TrackReferenceBox};
use transmux::{Error, TrackId};

const TREF_FILE: &str = "mp4/tref-avcp.mp4";
const AVC_FILE: &str = "mp4/progressive/av-prog.mp4";
const CDSC: [u8; 4] = *b"cdsc";

fn find_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let pos = data
        .windows(4)
        .position(|w| w == fourcc)
        .expect("box present");
    let start = pos - 4;
    let size = u32::from_be_bytes(data[start..start + 4].try_into().unwrap()) as usize;
    &data[start..start + size]
}

#[test]
fn real_tref_boxes_parse_and_round_trip() {
    let file = test_fixtures::read(TREF_FILE);
    let moov = MovieBox::parse(find_box(&file, b"moov")).expect("moov");
    assert_eq!(moov.tracks.len(), 2);

    let video = moov.tracks[0]
        .track_references()
        .expect("tref parses")
        .expect("video track has a tref");
    assert_eq!(video.references.len(), 1);
    assert_eq!(
        video.track_ids(&AVCP_REFERENCE_TYPE),
        Some(&[TrackId::new(2)][..])
    );

    let audio = moov.tracks[1]
        .track_references()
        .unwrap()
        .expect("audio track has a tref");
    assert_eq!(audio.track_ids(&CDSC), Some(&[TrackId::new(1)][..]));
    assert_eq!(audio.track_ids(&AVCP_REFERENCE_TYPE), None);

    let first_tref = file
        .windows(4)
        .position(|w| w == b"tref")
        .map(|p| &file[p - 4..p - 4 + 20])
        .unwrap();
    assert_eq!(video.try_to_bytes().unwrap(), first_tref);
    assert_eq!(
        TrackReferenceBox::parse(first_tref).unwrap(),
        video,
        "whole-box parse agrees with the body parse"
    );
}

#[test]
fn a_tref_with_a_partial_track_id_is_rejected() {
    let mut body = Vec::new();
    body.extend_from_slice(&13u32.to_be_bytes());
    body.extend_from_slice(&AVCP_REFERENCE_TYPE);
    body.extend_from_slice(&[0, 0, 0, 2, 9]);
    assert!(matches!(
        TrackReferenceBox::parse_body(&body).unwrap_err(),
        Error::InvalidValue(InvalidValue {
            field: "track_IDs",
            ..
        })
    ));
}

fn avc1_entry() -> transmux::sample_entries::AVCSampleEntry {
    let file = test_fixtures::read(AVC_FILE);
    let stsd = SampleDescriptionBox::parse(find_box(&file, b"stsd")).expect("stsd");
    match stsd.entries.into_iter().next() {
        Some(SampleEntryVariant::Avc1(e)) => e,
        other => panic!("first stsd entry is not AVC: {other:?}"),
    }
}

#[test]
fn an_avcp_entry_is_typed_and_round_trips_through_stsd() {
    let mut entry = avc1_entry();
    entry.codec_type = AVCP_SAMPLE_ENTRY;
    entry.config.config.sps.clear();
    entry.config.config.pps.clear();
    let stsd = SampleDescriptionBox {
        version: 0,
        flags: 0,
        entries: vec![SampleEntryVariant::Avc1(entry.clone())],
    };
    let bytes = stsd.try_to_bytes().unwrap();

    let parsed = SampleDescriptionBox::parse(&bytes).unwrap();
    let SampleEntryVariant::AvcParameterSet(avcp) = &parsed.entries[0] else {
        panic!(
            "'avcp' must parse as the typed variant: {:?}",
            parsed.entries[0]
        );
    };
    assert_eq!(avcp.entry, entry);
    assert!(avcp.declares_no_parameter_sets());
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn an_avcp_entry_with_parameter_sets_parses_but_is_flagged() {
    let mut entry = avc1_entry();
    entry.codec_type = AVCP_SAMPLE_ENTRY;
    let bytes = entry.try_to_bytes().unwrap();
    let avcp = AVCParameterSampleEntry::bare_parse(&bytes).unwrap();
    assert!(!avcp.declares_no_parameter_sets());
    assert_eq!(avcp.try_to_bytes().unwrap(), bytes);
}

#[test]
fn bare_parse_refuses_an_entry_that_is_not_avcp() {
    let bytes = avc1_entry().try_to_bytes().unwrap();
    assert!(matches!(
        AVCParameterSampleEntry::bare_parse(&bytes).unwrap_err(),
        Error::InvalidValue(InvalidValue {
            field: "sample entry type",
            ..
        })
    ));
}

fn parameter_set_sample(length_size: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
    let record = avc1_entry().config.config;
    let nals: Vec<Vec<u8>> = record
        .sps
        .iter()
        .map(|s| s.0.clone())
        .chain(record.pps.iter().map(|p| p.0.clone()))
        .collect();
    assert!(nals.len() >= 2, "the fixture carries an SPS and a PPS");
    let mut sample = Vec::new();
    for nal in &nals {
        sample.extend_from_slice(&(nal.len() as u32).to_be_bytes()[4 - length_size..]);
        sample.extend_from_slice(nal);
    }
    (sample, nals)
}

#[test]
fn a_parameter_set_sample_of_real_nal_units_round_trips_at_every_length_size() {
    for length_size_minus_one in [0u8, 1, 3] {
        let (bytes, nals) = parameter_set_sample(usize::from(length_size_minus_one) + 1);
        let sample = AVCParameterSample::parse_with(&bytes, length_size_minus_one).unwrap();
        let parsed: Vec<&[u8]> = sample.nal_units.iter().map(|n| n.0.as_slice()).collect();
        let expected: Vec<&[u8]> = nals.iter().map(Vec::as_slice).collect();
        assert_eq!(parsed, expected);
        assert_eq!(sample.nal_units[0].0[0] & 0x1F, 7, "SPS");
        assert_eq!(sample.try_to_bytes().unwrap(), bytes);
    }
}

#[test]
fn a_parameter_set_sample_rejects_a_bad_length_size_a_truncation_and_an_overflow() {
    let (bytes, _) = parameter_set_sample(4);
    assert!(matches!(
        AVCParameterSample::parse_with(&bytes, 2).unwrap_err(),
        Error::InvalidValue(InvalidValue {
            field: "lengthSizeMinusOne",
            ..
        })
    ));
    assert!(AVCParameterSample::parse_with(&bytes[..bytes.len() - 1], 3).is_err());

    let mut sample = AVCParameterSample::parse_with(&bytes, 3).unwrap();
    sample.length_size_minus_one = 0;
    sample.nal_units[0].0 = vec![0x67; 256];
    assert!(matches!(
        sample.try_to_bytes().unwrap_err(),
        Error::FieldOverflow(_)
    ));
}
