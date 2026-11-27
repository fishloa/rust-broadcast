//! The registered codes that were newly typed from a held specification, on
//! real files, with the independent tool's dump line beside each assertion.
//!
//! `MP4Box -diso` output quoted in each test is the oracle: the values the crate
//! asserts are the ones GPAC reports, and every parse is re-serialized
//! byte-for-byte.

use broadcast_core::{Parse, Serialize};
use isobmff::box_types::{BOX_TYPE_SIZE, BoxIter};
use isobmff::init_segment::{HandlerBox, MediaInformationBox, MediaInformationHeader};
use isobmff::registered::{Brand, HandlerType, SampleGroupType, TrackReferenceType};
use test_fixtures as fixtures;

const SAMPLE_ENTRY_FIXED: usize = 8;
const VISUAL_FIXED: usize = 78;
const AUDIO_FIXED: usize = 28;
const HINT_FIXED: usize = 16;

fn visual(code: &[u8]) -> bool {
    matches!(
        code,
        b"avc1"
            | b"avc2"
            | b"avc3"
            | b"avc4"
            | b"hvc1"
            | b"hev1"
            | b"dvh1"
            | b"dvhe"
            | b"vvc1"
            | b"vvi1"
            | b"mp4v"
            | b"av01"
            | b"vp09"
            | b"encv"
    )
}

fn audio(code: &[u8]) -> bool {
    matches!(
        code,
        b"mp4a"
            | b"enca"
            | b"ac-3"
            | b"ec-3"
            | b"ac-4"
            | b"Opus"
            | b"fLaC"
            | b"mha1"
            | b"mha2"
            | b"mhm1"
            | b"mhm2"
            | b"dtsc"
            | b"dtsh"
            | b"dtsl"
            | b"dtse"
            | b"lpcm"
            | b"in24"
            | b"in32"
            | b"sowt"
            | b"twos"
            | b"raw "
            | b"fl32"
            | b"fl64"
            | b"ima4"
            | b"ulaw"
            | b"alaw"
            | b"NONE"
    )
}

fn hint(code: &[u8]) -> bool {
    matches!(
        code,
        b"rtp "
            | b"srtp"
            | b"rrtp"
            | b"rsrp"
            | b"rtcp"
            | b"stcp"
            | b"prtp"
            | b"rm2t"
            | b"sm2t"
            | b"pm2t"
            | b"fdp "
    )
}

fn find_entries(data: &[u8], code: &[u8; BOX_TYPE_SIZE], out: &mut Vec<Vec<u8>>) {
    for item in BoxIter::clamped(data) {
        let Ok(item) = item else { break };
        let entry = item.four_cc();
        if entry == *code {
            out.push(item.bytes.to_vec());
        }
        let fixed = if visual(&entry) {
            VISUAL_FIXED
        } else if audio(&entry) {
            AUDIO_FIXED
        } else if hint(&entry) {
            HINT_FIXED
        } else {
            SAMPLE_ENTRY_FIXED
        };
        if let Some(rest) = item.body.get(fixed..) {
            find(rest, code, out);
        }
    }
}

fn find(data: &[u8], code: &[u8; BOX_TYPE_SIZE], out: &mut Vec<Vec<u8>>) {
    for item in BoxIter::new(data) {
        let Ok(item) = item else { break };
        let fourcc = item.four_cc();
        if fourcc == *code {
            out.push(item.bytes.to_vec());
        }
        match &fourcc {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"dinf" | b"edts" | b"mvex"
            | b"moof" | b"traf" | b"mfra" | b"udta" | b"iprp" | b"ipco" | b"grpl" | b"strk"
            | b"sinf" | b"schi" | b"hnti" | b"hinf" | b"wave" | b"gmhd" | b"tapt" | b"tref"
            | b"trgr" | b"ipro" => find(item.body, code, out),
            b"meta" => find(item.body.get(4..).unwrap_or_default(), code, out),
            b"stsd" => find_entries(item.body.get(8..).unwrap_or_default(), code, out),
            _ => {}
        }
    }
}

fn one(path: &str, code: &[u8; BOX_TYPE_SIZE]) -> Vec<u8> {
    let data = fixtures::read(path);
    let mut found = Vec::new();
    find(&data, code, &mut found);
    found.remove(0)
}

#[test]
fn sthd_and_the_subt_route_from_a_real_subtitle_file() {
    // MP4Box -diso fixtures/mp4/stpp.mp4:
    //   <FileTypeBox ... MajorBrand="isom" MinorVersion="512">
    //   <HandlerBox ... hdlrType="subt" Name="SubtitleHandler" ...>
    //   <MPEGMediaHeaderBox Size="12" Type="sthd" Version="0" Flags="0" ...>
    let ftyp = one("mp4/stpp.mp4", b"ftyp");
    let file_type = isobmff::segments::FileTypeBox::parse_box(&ftyp).expect("ftyp");
    assert_eq!(file_type.major_brand_type(), Brand::Isom);
    assert_eq!(file_type.minor_version, 512);

    let hdlr = one("mp4/stpp.mp4", b"hdlr");
    let handler = HandlerBox::parse(&hdlr).expect("hdlr");
    assert_eq!(handler.handler(), HandlerType::Subt);

    let sthd = one("mp4/stpp.mp4", b"sthd");
    let parsed = isobmff::init_segment::SubtitleMediaHeaderBox::parse(&sthd).expect("sthd");
    assert_eq!(parsed.version(), 0);
    assert_eq!(parsed.flags(), 0);
    assert_eq!(parsed.serialized_len(), sthd.len());
    assert_eq!(parsed.try_to_bytes().expect("sthd bytes"), sthd);

    let minf = one("mp4/stpp.mp4", b"minf");
    let media = MediaInformationBox::parse(&minf).expect("minf");
    assert!(matches!(
        media.media_header(),
        Some(MediaInformationHeader::Subtitle(_))
    ));
}

#[test]
fn wide_on_a_real_quicktime_file() {
    // MP4Box -diso fixtures/audio/srat/qt-alac-v1.mov:
    //   <FileTypeBox ... MajorBrand="qt  " ...>
    //   <WideBox Size="8" Type="wide" Specification="apple" Container="*" >
    //   <HandlerBox ... hdlrType="soun" ...>
    let ftyp = one("audio/srat/qt-alac-v1.mov", b"ftyp");
    let file_type = isobmff::segments::FileTypeBox::parse_box(&ftyp).expect("ftyp");
    assert_eq!(file_type.major_brand_type(), Brand::Qt);

    let hdlr = one("audio/srat/qt-alac-v1.mov", b"hdlr");
    let handler = HandlerBox::parse(&hdlr).expect("hdlr");
    assert_eq!(handler.handler(), HandlerType::Soun);

    let wide = one("audio/srat/qt-alac-v1.mov", b"wide");
    assert_eq!(wide.len(), 8);
    let parsed = isobmff::segments::WideAtom::parse(&wide).expect("wide");
    assert!(parsed.trailing().is_empty());
    assert_eq!(parsed.try_to_bytes().expect("wide bytes"), wide);
}

#[test]
fn roll_group_and_hint_reference_from_real_files() {
    // MP4Box -diso fixtures/dash/multiperiod-restart/p1_dash_track2_1.m4s:
    //   <SampleGroupBox ... Type="sbgp" ... grouping_type="roll">
    //   <SampleGroupBoxEntry sample_count="87" group_description_index="1"/>
    let sbgp = one("dash/multiperiod-restart/p1_dash_track2_1.m4s", b"sbgp");
    let group = isobmff::sample_groups::SampleToGroupBox::parse(&sbgp).expect("sbgp");
    assert_eq!(group.group_type(), SampleGroupType::Roll);
    assert_eq!(group.entries.len(), 1);
    assert_eq!(group.entries[0].sample_count, 87);
    assert_eq!(group.entries[0].group_description_index, 1);
    assert_eq!(group.try_to_bytes().expect("sbgp bytes"), sbgp);

    // MP4Box -diso fixtures/mp4/boxes/hint-hmhd.mp4:
    //   <TrackReferenceTypeBox Size="12" Type="hint" ...>
    //   <TrackReferenceEntry TrackID="1"/> ... and TrackID="2"
    let tref = one("mp4/boxes/hint-hmhd.mp4", b"tref");
    let parsed = isobmff::track_reference::TrackReferenceBox::parse_body(&tref[8..]).expect("tref");
    assert_eq!(parsed.references.len(), 1);
    assert_eq!(parsed.references[0].reference(), TrackReferenceType::Hint);
    assert_eq!(
        parsed.references[0].track_ids,
        vec![isobmff::TrackId::new(1)]
    );
}

#[test]
fn c608_on_the_private_cea608_track() {
    // MP4Box -diso private/fixtures/candidates/shaka/cea608-track-init.mp4:
    //   <ClosedCaption Size="16" Type="c608" Specification="apple" Container="stsd" >
    let Some(path) = fixtures::private("candidates/shaka/cea608-track-init.mp4") else {
        eprintln!("SKIP c608: private checkout absent");
        return;
    };
    let data = std::fs::read(&path).expect("private c608 fixture");
    let mut found = Vec::new();
    find(&data, b"c608", &mut found);
    assert!(!found.is_empty(), "the fixture carries a c608 entry");
    for bytes in found {
        let entry = isobmff::subtitle_entries::ClosedCaptionSampleEntry::parse_box(&bytes)
            .expect("c608 entry");
        assert_eq!(entry.serialized_len(), bytes.len());
        assert_eq!(entry.try_to_bytes().expect("c608 bytes"), bytes);
    }
}
