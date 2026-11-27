//! Family 2 of issue #4167: file-level and structure boxes.
//!
//! No installed tool writes these boxes (see `PROVENANCE.md`), so the bytes
//! here are assembled from the clause and labelled as such. The ones a
//! committed fixture carries (`mehd`) are also asserted against that file in
//! `tier34_fixtures.rs`.

use broadcast_core::{Parse, Serialize};
use isobmff::init_segment::MovieExtendsHeaderBox;
use isobmff::metadata::{Udta, UdtaChild};
use isobmff::segments::{ExtendedTypeBox, IdentifiedMediaDataBox, TypeCombinationBox};
use isobmff::track_desc::{SubTrackBox, SubTrackDefinitionBox, SubTrackInformationBox};
use isobmff::{Reader, ReaderConfig, Step};

fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

fn full(version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.push(version);
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

fn u32s(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

#[test]
fn type_combination_and_extended_type_round_trip() {
    let tyco = boxed(*b"tyco", b"iso6iso8");
    let parsed = TypeCombinationBox::parse(&tyco).expect("tyco");
    assert_eq!(parsed.compatible_brands(), &[*b"iso6", *b"iso8"]);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), tyco);

    let bytes = boxed(*b"etyp", &[tyco.clone(), tyco].concat());
    let parsed = ExtendedTypeBox::parse(&bytes).expect("etyp");
    assert_eq!(parsed.combinations().len(), 2);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn original_file_type_round_trips() {
    let ftyp = boxed(
        *b"ftyp",
        &[b"iso5".as_slice(), &[0, 0, 2, 0], b"iso6"].concat(),
    );
    let etyp = boxed(*b"etyp", &boxed(*b"tyco", b"iso6iso8"));
    let bytes = boxed(*b"otyp", &[ftyp, etyp].concat());
    let parsed = isobmff::OriginalFileTypeBox::parse(&bytes).expect("otyp");
    assert_eq!(parsed.file_type().major_brand, *b"iso5");
    assert_eq!(
        parsed.extended_types().expect("etyp").combinations().len(),
        1
    );
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn identified_media_data_round_trips() {
    let bytes = boxed(*b"imda", &[0u8, 0, 0, 7, 0xDE, 0xAD, 0xBE, 0xEF]);
    let parsed = IdentifiedMediaDataBox::parse(&bytes).expect("imda");
    assert_eq!(parsed.identifier(), 7);
    assert_eq!(parsed.data(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
    assert_eq!(
        IdentifiedMediaDataBox::parse_identification(&bytes).expect("identification"),
        (7, 8 + 4)
    );
}

#[test]
fn sub_track_boxes_round_trip_and_sit_in_udta() {
    let stri = boxed(
        *b"stri",
        &full(
            0,
            0,
            &[
                0u8, 7, 0, 3, 0, 0, 0, 5, b'b', b'i', b't', b'r', b'f', b'r', b'a', b'r',
            ],
        ),
    );
    let stri_box = SubTrackInformationBox::parse(&stri).expect("stri");
    assert_eq!(stri_box.switch_group(), 7);
    assert_eq!(stri_box.alternate_group(), 3);
    assert_eq!(stri_box.sub_track_id(), 5);
    assert_eq!(stri_box.attributes(), &[*b"bitr", *b"frar"]);
    assert_eq!(stri_box.try_to_bytes().expect("bytes"), stri);

    let strd = boxed(*b"strd", &[]);
    let definition = SubTrackDefinitionBox::parse(&strd).expect("strd");
    assert!(definition.sample_groups().is_empty());
    assert_eq!(definition.try_to_bytes().expect("bytes"), strd);

    let strk = boxed(*b"strk", &[stri.clone(), strd].concat());
    let parsed = SubTrackBox::parse(&strk).expect("strk");
    assert_eq!(parsed.information().sub_track_id(), 5);
    assert!(parsed.definition().is_some());
    assert_eq!(parsed.try_to_bytes().expect("bytes"), strk);

    let udta_bytes = boxed(*b"udta", &strk);
    let udta = Udta::parse(&udta_bytes).expect("udta");
    let sub = udta
        .children
        .iter()
        .find_map(|child| match child {
            UdtaChild::SubTrack(sub) => Some(sub),
            _ => None,
        })
        .expect("strk");
    assert_eq!(sub.information().sub_track_id(), 5);
    assert_eq!(
        udta.try_to_bytes().expect("bytes"),
        udta_bytes,
        "a sub-track round-trips inside the user-data box"
    );
}

#[test]
fn movie_extends_header_round_trips_in_both_versions() {
    let v0 = boxed(*b"mehd", &full(0, 0, &12000u32.to_be_bytes()));
    let parsed = MovieExtendsHeaderBox::parse(&v0).expect("mehd v0");
    assert_eq!(parsed.version(), 0);
    assert_eq!(parsed.fragment_duration(), 12000);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), v0);

    let v1 = boxed(*b"mehd", &full(1, 0, &(1u64 << 40).to_be_bytes()));
    let parsed = MovieExtendsHeaderBox::parse(&v1).expect("mehd v1");
    assert_eq!(parsed.version(), 1);
    assert_eq!(parsed.fragment_duration(), 1u64 << 40);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), v1);
}

#[test]
fn volumetric_media_header_round_trips() {
    let bytes = boxed(*b"vvhd", &full(0, 0, &[]));
    let parsed = isobmff::VolumetricVisualMediaHeaderBox::parse(&bytes).expect("vvhd");
    assert_eq!(parsed.version(), 0);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

fn drive(bytes: &[u8]) -> Reader {
    let mut reader = Reader::new(ReaderConfig::default().with_total_len(Some(bytes.len() as u64)));
    loop {
        match reader.step().expect("step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                reader
                    .supply(range.offset, bytes.get(start..end).unwrap_or(&[]))
                    .expect("supply");
            }
            Step::Done => break,
            _ => {}
        }
    }
    reader
}

#[test]
fn the_reader_reports_extended_and_original_file_types() {
    let ftyp = boxed(
        *b"ftyp",
        &[b"iso5".as_slice(), &[0, 0, 2, 0], b"iso6"].concat(),
    );
    let etyp = boxed(*b"etyp", &boxed(*b"tyco", b"iso6iso8"));
    let otyp = boxed(*b"otyp", &[ftyp.clone(), etyp.clone()].concat());
    let file = [ftyp, etyp, otyp].concat();
    let reader = drive(&file);
    let extended = reader.extended_types().expect("etyp");
    assert_eq!(
        extended.combinations()[0].compatible_brands(),
        &[*b"iso6", *b"iso8"]
    );
    assert_eq!(
        reader
            .original_type()
            .expect("otyp")
            .file_type()
            .major_brand,
        *b"iso5"
    );
}

fn mvhd_body(timescale: u32, duration: u32) -> Vec<u8> {
    let mut body = u32s(&[0, 0, timescale, duration, 0x0001_0000]);
    body.extend_from_slice(&[0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    body.extend_from_slice(&u32s(&[
        0x0001_0000,
        0,
        0,
        0,
        0x0001_0000,
        0,
        0,
        0,
        0x4000_0000,
    ]));
    body.extend_from_slice(&u32s(&[0, 0, 0, 0, 0, 0]));
    body.extend_from_slice(&2u32.to_be_bytes());
    body
}

fn tkhd_body(track_id: u32, duration: u32) -> Vec<u8> {
    let mut body = u32s(&[0, 0, track_id, 0, duration]);
    body.extend_from_slice(&u32s(&[0, 0]));
    body.extend_from_slice(&[0, 0, 0, 0, 0x01, 0x00, 0, 0]);
    body.extend_from_slice(&u32s(&[
        0x0001_0000,
        0,
        0,
        0,
        0x0001_0000,
        0,
        0,
        0,
        0x4000_0000,
    ]));
    body.extend_from_slice(&u32s(&[0, 0]));
    body
}

fn hdlr_body() -> Vec<u8> {
    let mut body = u32s(&[0, u32::from_be_bytes(*b"vide"), 0, 0, 0]);
    body.push(0);
    body
}

fn imda_referenced_progressive_file(payload: &[u8]) -> (Vec<u8>, u64) {
    let ftyp = boxed(
        *b"ftyp",
        &[b"iso5".as_slice(), &[0, 0, 2, 0], b"iso6"].concat(),
    );
    let imda = boxed(*b"imda", &[&7u32.to_be_bytes()[..], payload].concat());
    let payload_start = (ftyp.len() + 8 + 4) as u64;

    let sample_entry = boxed(*b"zzzz", &[0, 0, 0, 0, 0, 0, 0, 1]);
    let stsd = boxed(
        *b"stsd",
        &full(0, 0, &[1u32.to_be_bytes().to_vec(), sample_entry].concat()),
    );
    let stts = boxed(
        *b"stts",
        &full(
            0,
            0,
            &[1u32.to_be_bytes().to_vec(), u32s(&[1, 100])].concat(),
        ),
    );
    let stsc = boxed(
        *b"stsc",
        &full(
            0,
            0,
            &[1u32.to_be_bytes().to_vec(), u32s(&[1, 1, 1])].concat(),
        ),
    );
    let stsz = boxed(*b"stsz", &full(0, 0, &u32s(&[4, 1])));
    let stco = boxed(
        *b"stco",
        &full(0, 0, &[1u32.to_be_bytes().to_vec(), u32s(&[0])].concat()),
    );
    let stbl = boxed(*b"stbl", &[stsd, stts, stsc, stsz, stco].concat());
    let vmhd = boxed(*b"vmhd", &full(0, 1, &[0u8; 8]));
    let imdt = boxed(*b"imdt", &full(0, 0, &7u32.to_be_bytes()));
    let dref = boxed(
        *b"dref",
        &full(0, 0, &[1u32.to_be_bytes().to_vec(), imdt].concat()),
    );
    let dinf = boxed(*b"dinf", &dref);
    let minf = boxed(*b"minf", &[vmhd, dinf, stbl].concat());
    let mut mdhd_body = u32s(&[0, 0, 1000, 100]);
    mdhd_body.extend_from_slice(&[0x55, 0xC4, 0, 0]);
    let mdhd = boxed(*b"mdhd", &full(0, 0, &mdhd_body));
    let hdlr = boxed(*b"hdlr", &full(0, 0, &hdlr_body()));
    let mdia = boxed(*b"mdia", &[mdhd, hdlr, minf].concat());
    let tkhd = boxed(*b"tkhd", &full(0, 7, &tkhd_body(1, 100)));
    let trak = boxed(*b"trak", &[tkhd, mdia].concat());
    let mvhd = boxed(*b"mvhd", &full(0, 0, &mvhd_body(1000, 100)));
    let moov = boxed(*b"moov", &[mvhd, trak].concat());
    ([ftyp, imda, moov].concat(), payload_start)
}

#[test]
fn the_reader_addresses_samples_through_an_identified_media_data_box() {
    let payload = [0xDE, 0xAD, 0xBE, 0xEF];
    let (file, payload_start) = imda_referenced_progressive_file(&payload);
    let mut reader = Reader::new(ReaderConfig::default().with_total_len(Some(file.len() as u64)));
    let mut samples = Vec::new();
    let mut identified = Vec::new();
    loop {
        match reader.step().expect("step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                reader
                    .supply(range.offset, file.get(start..end).unwrap_or(&[]))
                    .expect("supply");
            }
            Step::Sample(sample) => samples.push(sample),
            Step::Done => break,
            _ => {}
        }
        identified = reader.identified_media().to_vec();
    }
    assert_eq!(identified.len(), 1);
    assert_eq!(identified[0].identifier(), 7);
    assert_eq!(identified[0].payload().offset, payload_start);
    assert_eq!(identified[0].payload().len, payload.len() as u64);
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].offset, payload_start);
    assert_eq!(samples[0].size, payload.len() as u32);
}
