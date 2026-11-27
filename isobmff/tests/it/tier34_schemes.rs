//! Family 3 of issue #4167: restricted, incomplete and scrambled schemes.
//!
//! No installed tool writes these boxes, so each byte sequence is assembled
//! from the clause it names. The reader methods that surface them are driven
//! over an assembled file with an empty sample table.

use broadcast_core::{Parse, Serialize};
use isobmff::cenc::{OriginalFormatBox, SchemeInformationBox, SchemeTypeBox};
use isobmff::image::ItemProperty;
use isobmff::scheme::{
    CompatibleSchemeTypeBox, CompleteTrackInfoBox, RestrictedSchemeInfoBox,
    SamplePackingInformationBox, ScrambleSchemeInfoBox,
};
use isobmff::{Reader, ReaderConfig, Step};

pub(crate) fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

pub(crate) fn full(version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.push(version);
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

fn u32s(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn scheme_type(scheme_type: [u8; 4], version: u32) -> SchemeTypeBox {
    SchemeTypeBox {
        version: 0,
        flags: 0,
        scheme_type,
        scheme_version: version,
        scheme_uri: None,
    }
}

fn schm_bytes(scheme: &SchemeTypeBox) -> Vec<u8> {
    scheme.try_to_bytes().expect("schm")
}

#[test]
fn compatible_scheme_type_round_trips() {
    let bytes = boxed(*b"csch", &full(0, 0, &[b's', b'p', b'k', b't', 0, 1, 0, 0]));
    let parsed = CompatibleSchemeTypeBox::parse(&bytes).expect("csch");
    assert_eq!(parsed.scheme_type(), *b"spkt");
    assert_eq!(parsed.scheme_version(), 0x0001_0000);
    assert_eq!(parsed.scheme_uri(), None);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);

    let with_uri = boxed(
        *b"csch",
        &full(
            0,
            1,
            &[
                b's', b'p', b'k', b't', 0, 1, 0, 0, b'u', b'r', b'n', b':', b'x',
            ],
        ),
    );
    let parsed = CompatibleSchemeTypeBox::parse(&with_uri).expect("csch uri");
    assert_eq!(parsed.scheme_uri_str(), Some("urn:x"));
    assert_eq!(parsed.try_to_bytes().expect("bytes"), with_uri);
}

#[test]
fn sample_packing_information_round_trips() {
    let bytes = boxed(*b"spki", &full(0, 0, &[1]));
    let parsed = SamplePackingInformationBox::parse(&bytes).expect("spki");
    assert_eq!(parsed.mode(), 1);
    assert_eq!(parsed.version(), 0);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn restricted_scheme_information_round_trips() {
    let frma = boxed(*b"frma", b"avc1");
    let schm = schm_bytes(&scheme_type(*b"stvi", 0));
    let spki = boxed(*b"spki", &full(0, 0, &[0]));
    let schi = boxed(*b"schi", &spki);
    let csch = boxed(*b"csch", &full(0, 0, &[b's', b'p', b'k', b't', 0, 1, 0, 0]));
    let bytes = boxed(*b"rinf", &[frma, schm, schi, csch].concat());
    let parsed = RestrictedSchemeInfoBox::parse(&bytes).expect("rinf");
    assert_eq!(
        parsed.original_format().expect("frma").data_format,
        *b"avc1"
    );
    assert_eq!(parsed.scheme().scheme_type, *b"stvi");
    assert_eq!(
        parsed
            .scheme_info()
            .expect("schi")
            .spki
            .as_ref()
            .expect("spki")
            .mode(),
        0
    );
    assert_eq!(parsed.compatible_schemes().len(), 1);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn a_restricted_scheme_information_box_without_a_format_round_trips() {
    let bytes = boxed(*b"rinf", &schm_bytes(&scheme_type(*b"stvi", 1)));
    let parsed = RestrictedSchemeInfoBox::parse(&bytes).expect("rinf");
    assert!(parsed.original_format().is_none());
    assert_eq!(parsed.scheme().scheme_version, 1);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn scramble_scheme_information_round_trips() {
    let schm = schm_bytes(&scheme_type(*b"cenc", 0x0001_0000));
    let schi = boxed(*b"schi", &boxed(*b"tenc", &full(0, 0, &[0u8; 20])));
    let bytes = boxed(*b"scrb", &[schm, schi].concat());
    let parsed = ScrambleSchemeInfoBox::parse(&bytes).expect("scrb");
    assert_eq!(parsed.scheme().scheme_type, *b"cenc");
    assert!(parsed.scheme_info().is_some());
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);

    let in_ipco = boxed(*b"ipco", &bytes);
    let properties = isobmff::ItemPropertyContainerBox::parse_box(&in_ipco).expect("ipco");
    assert!(matches!(properties.properties()[0], ItemProperty::Scrb(_)));
}

#[test]
fn complete_track_information_round_trips() {
    let bytes = boxed(*b"cinf", &boxed(*b"frma", b"avc1"));
    let parsed = CompleteTrackInfoBox::parse(&bytes).expect("cinf");
    assert_eq!(parsed.original_format().data_format, *b"avc1");
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

fn mvhd_body() -> Vec<u8> {
    let mut body = u32s(&[0, 0, 1000, 0, 0x0001_0000]);
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

fn tkhd_body() -> Vec<u8> {
    let mut body = u32s(&[0, 0, 1, 0, 0]);
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

fn empty_table(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    boxed(fourcc, &full(0, 0, body))
}

/// A one-track file whose sample entry is `fourcc` with `children` after the
/// SampleEntry base, and whose sample table is empty.
pub(crate) fn file_with_sample_entry(fourcc: [u8; 4], fixed: usize, children: &[u8]) -> Vec<u8> {
    let ftyp = boxed(
        *b"ftyp",
        &[b"iso5".as_slice(), &[0, 0, 2, 0], b"iso6"].concat(),
    );
    let mut entry_body = vec![0u8; 6];
    entry_body.extend_from_slice(&1u16.to_be_bytes());
    entry_body.resize(fixed, 0);
    entry_body.extend_from_slice(children);
    let entry = boxed(fourcc, &entry_body);
    let stsd = boxed(
        *b"stsd",
        &full(0, 0, &[1u32.to_be_bytes().to_vec(), entry].concat()),
    );
    let stts = empty_table(*b"stts", &0u32.to_be_bytes());
    let stsc = empty_table(*b"stsc", &0u32.to_be_bytes());
    let stsz = empty_table(*b"stsz", &u32s(&[0, 0]));
    let stco = empty_table(*b"stco", &0u32.to_be_bytes());
    let stbl = boxed(*b"stbl", &[stsd, stts, stsc, stsz, stco].concat());
    let vmhd = boxed(*b"vmhd", &full(0, 1, &[0u8; 8]));
    let url = boxed(*b"url ", &full(0, 1, &[]));
    let dref = boxed(
        *b"dref",
        &full(0, 0, &[1u32.to_be_bytes().to_vec(), url].concat()),
    );
    let dinf = boxed(*b"dinf", &dref);
    let minf = boxed(*b"minf", &[vmhd, dinf, stbl].concat());
    let mut mdhd_body = u32s(&[0, 0, 1000, 0]);
    mdhd_body.extend_from_slice(&[0x55, 0xC4, 0, 0]);
    let mdhd = boxed(*b"mdhd", &full(0, 0, &mdhd_body));
    let mut hdlr_body = u32s(&[0, u32::from_be_bytes(*b"vide"), 0, 0, 0]);
    hdlr_body.push(0);
    let hdlr = boxed(*b"hdlr", &full(0, 0, &hdlr_body));
    let mdia = boxed(*b"mdia", &[mdhd, hdlr, minf].concat());
    let tkhd = boxed(*b"tkhd", &full(0, 7, &tkhd_body()));
    let trak = boxed(*b"trak", &[tkhd, mdia].concat());
    let mvhd = boxed(*b"mvhd", &full(0, 0, &mvhd_body()));
    let moov = boxed(*b"moov", &[mvhd, trak].concat());
    [ftyp, moov].concat()
}

pub(crate) fn drive(bytes: &[u8]) -> Vec<isobmff::TrackInfo> {
    let mut reader = Reader::new(ReaderConfig::default().with_total_len(Some(bytes.len() as u64)));
    let mut tracks = Vec::new();
    loop {
        match reader.step().expect("step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                reader
                    .supply(range.offset, bytes.get(start..end).unwrap_or(&[]))
                    .expect("supply");
            }
            Step::Tracks(info) => tracks = info.to_vec(),
            Step::Done => break,
            _ => {}
        }
    }
    tracks
}

#[test]
fn the_reader_surfaces_restricted_scrambled_and_incomplete_schemes() {
    let rinf = boxed(
        *b"rinf",
        &[
            boxed(*b"frma", b"avc1"),
            schm_bytes(&scheme_type(*b"stvi", 0)),
        ]
        .concat(),
    );
    let tracks = drive(&file_with_sample_entry(*b"resv", 78, &rinf));
    let scheme = tracks[0].entries[0].restricted_scheme().expect("rinf");
    assert_eq!(scheme.scheme().scheme_type, *b"stvi");
    assert_eq!(
        scheme.original_format().expect("frma").data_format,
        *b"avc1"
    );

    let scrb = boxed(*b"scrb", &schm_bytes(&scheme_type(*b"cenc", 0)));
    let tracks = drive(&file_with_sample_entry(*b"resv", 78, &scrb));
    assert_eq!(
        tracks[0].entries[0]
            .scramble_scheme()
            .expect("scrb")
            .scheme()
            .scheme_type,
        *b"cenc"
    );

    let cinf = boxed(*b"cinf", &boxed(*b"frma", b"avc1"));
    let tracks = drive(&file_with_sample_entry(*b"icpv", 78, &cinf));
    assert_eq!(
        tracks[0].entries[0]
            .complete_track_info()
            .expect("cinf")
            .original_format()
            .data_format,
        *b"avc1"
    );
}

#[test]
fn a_scheme_information_box_keeps_its_sample_packing() {
    let spki = boxed(*b"spki", &full(0, 0, &[1]));
    let bytes = boxed(*b"schi", &spki);
    let parsed = SchemeInformationBox::parse(&bytes).expect("schi");
    assert_eq!(parsed.spki.as_ref().expect("spki").mode(), 1);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);

    let frma = OriginalFormatBox {
        data_format: *b"avc1",
    };
    assert_eq!(&frma.try_to_bytes().expect("frma")[4..8], b"frma");
}
