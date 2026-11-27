//! A box audit over every typed box of the crate that a committed file
//! carries.
//!
//! The crate's rule is that anything it parses re-serializes byte-exact, and a
//! form it cannot represent is a typed error, never a rewrite. The sample-entry
//! audit exercises one family; this one walks every committed media fixture,
//! finds each occurrence of every four-CC in [`TYPED_BOXES`], and flips each
//! byte of the box body's fixed prefix one at a time, requiring
//! parse -> serialize byte identity or a typed error.
//!
//! Only the body is flipped, and only its first [`AUDIT_PREFIX`] bytes: the
//! 8-byte `size`/`type` header is structure rather than content
//! (ISO/IEC 14496-12:2026 §4.2), and a box's fixed syntax is at its start. The
//! same walk appends four bytes past each occurrence's last declared field: a
//! box either carries them or refuses them, and the boxes that as yet do
//! neither are pinned, so that list cannot grow (or shrink) silently. The
//! report states boxes x occurrences x bytes so the coverage is auditable, and
//! every typed box with no occurrence in a committed file is pinned below, so
//! that list cannot grow (or shrink) silently.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use broadcast_core::{Parse, Serialize};
use isobmff::Error;
use isobmff::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE};
use isobmff::coverage::TYPED_BOXES;
use test_fixtures as fixtures;

/// Bytes of each occurrence's fixed body prefix the audit flips.
const AUDIT_PREFIX: usize = 128;
/// A byte value unlikely to equal a field's default.
const NON_DEFAULT: u8 = 0xA5;
/// Depth cap for descending into a child region.
const MAX_DEPTH: u32 = 12;
/// Cap on the children one region may hold for the walk to accept it.
const MAX_CHILDREN: usize = 4096;
/// Media file extensions in the committed fixtures.
const MEDIA_EXTENSIONS: [&str; 5] = ["mp4", "m4s", "heic", "avif", "mov"];
/// How far into a container's body its first child may start (a FullBox
/// header, a count field, or a sample entry's fixed fields).
const SEARCH_WINDOW: usize = 96;
/// Bytes of a FullBox's version/flags before its own fields.
const FULLBOX_HEADER: usize = 4;

/// A parser that takes a whole box and writes either the whole box or just its
/// payload back, matching the type's own `Serialize`.
type Parser = fn(&[u8]) -> Result<Vec<u8>, Error>;

/// Whether a parser's output is the whole box or only its payload.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// `Serialize` writes `size`, `type` and the body.
    Whole,
    /// `Serialize` writes only the body (a box whose `Parse` is over the
    /// payload, e.g. `pasp`).
    Body,
}

/// One typed box and how its parser's output relates to the input.
#[derive(Clone, Copy)]
struct Typed {
    parser: Parser,
    shape: Shape,
}

fn whole<T: for<'a> Parse<'a, Error = Error> + Serialize<Error = Error>>(
    bytes: &[u8],
) -> Result<Vec<u8>, Error> {
    T::parse(bytes)?.try_to_bytes()
}

/// A type whose `Parse` and `Serialize` both take the payload after the header.
fn payload<T: for<'a> Parse<'a, Error = Error> + Serialize<Error = Error>>(
    bytes: &[u8],
) -> Result<Vec<u8>, Error> {
    let item = isobmff::box_types::parse_box(bytes)?.0;
    let parsed = T::parse(item.body)?;
    let mut out = vec![0u8; parsed.serialized_len()];
    let written = parsed.serialize_into(&mut out)?;
    out.truncate(written);
    Ok(out)
}

macro_rules! rows {
    ($( $fourcc:literal => $ty:ty, )*) => {
        fn whole_parser(fourcc: &[u8; BOX_TYPE_SIZE]) -> Option<Typed> {
            match fourcc {
                $( $fourcc => Some(Typed {
                    parser: whole::<$ty> as Parser,
                    shape: Shape::Whole,
                }), )*
                _ => None,
            }
        }
    };
}

macro_rules! payload_rows {
    ($( $fourcc:literal => $ty:ty, )*) => {
        fn payload_parser(fourcc: &[u8; BOX_TYPE_SIZE]) -> Option<Typed> {
            match fourcc {
                $( $fourcc => Some(Typed {
                    parser: payload::<$ty> as Parser,
                    shape: Shape::Body,
                }), )*
                _ => None,
            }
        }
    };
}

rows! {
    b"aedb" => isobmff::audio_elements::AudioElementDescriptionBox,
    b"aelm" => isobmff::audio_elements::AudioElementBox,
    b"aepp" => isobmff::audio_elements::AudioElementPosInteractivityPolarBox,
    b"aepr" => isobmff::audio_elements::AudioElementProminenceInteractivityBox,
    b"aesb" => isobmff::audio_elements::AudioElementSelectionBox,
    b"aesd" => isobmff::audio_elements::AudioElementSelectionDescriptionBox,
    b"ardi" => isobmff::audio_elements::AudioRenderingIndicationBox,
    b"assp" => isobmff::index_boxes::AlternativeStartupSequencePropertiesBox,
    b"brnd" => isobmff::item::BrandProperty,
    b"btrt" => isobmff::sample_entries::BitRateBox,
    b"bxml" => isobmff::meta_track::BinaryXmlBox,
    b"chnl" => isobmff::audio_boxes::ChannelLayoutBox,
    b"cinf" => isobmff::scheme::CompleteTrackInfoBox,
    b"co64" => isobmff::init_segment::ChunkLargeOffsetBox,
    b"colr" => isobmff::visual_ext::ColourInformationBox,
    b"cprt" => isobmff::meta_track::CopyrightBox,
    b"csch" => isobmff::scheme::CompatibleSchemeTypeBox,
    b"csgp" => isobmff::sample_groups::CompactSampleToGroupBox,
    b"cslg" => isobmff::timing::CompositionToDecodeBox,
    b"ctts" => isobmff::timing::CompositionOffsetBox,
    b"dimm" => isobmff::hint::DimmBox,
    b"dinf" => isobmff::init_segment::DataInformationBox,
    b"dmax" => isobmff::hint::DmaxBox,
    b"dmed" => isobmff::hint::DmedBox,
    b"dmix" => isobmff::audio_boxes::DownMixInstructionsBox,
    b"dref" => isobmff::init_segment::DataReferenceBox,
    b"drep" => isobmff::hint::DrepBox,
    b"edts" => isobmff::init_segment::EditBox,
    b"elng" => isobmff::track_desc::ExtendedLanguageBox,
    b"elst" => isobmff::timing::EditListBox,
    b"etyp" => isobmff::segments::ExtendedTypeBox,
    b"extr" => isobmff::file_delivery::ExtraDataBox,
    b"fdpa" => isobmff::file_delivery::FdPacketBox,
    b"feci" => isobmff::file_delivery::FecInformationBox,
    b"fecr" => isobmff::file_delivery::FecReservoirBox,
    b"fiin" => isobmff::file_delivery::FdItemInformationBox,
    b"fire" => isobmff::file_delivery::FileReservoirBox,
    b"fpar" => isobmff::file_delivery::FilePartitionBox,
    b"frma" => isobmff::cenc::OriginalFormatBox,
    b"gitn" => isobmff::file_delivery::GroupIdToNameBox,
    b"grpl" => isobmff::item::GroupsListBox,
    b"hdlr" => isobmff::init_segment::HandlerBox,
    b"hinf" => isobmff::hint::HintStatisticsBox,
    b"hmhd" => isobmff::init_segment::HintMediaHeaderBox,
    b"hnti" => isobmff::hint::HintInformationBox,
    b"idat" => isobmff::item::ItemDataBox,
    b"iinf" => isobmff::item::ItemInfoBox,
    b"iloc" => isobmff::item::ItemLocationBox,
    b"imda" => isobmff::segments::IdentifiedMediaDataBox,
    b"infe" => isobmff::item::ItemInfoEntry,
    b"ipma" => isobmff::item::ItemPropertyAssociationBox,
    b"ipro" => isobmff::item::ItemProtectionBox,
    b"iprp" => isobmff::item::ItemPropertiesBox,
    b"iref" => isobmff::item::ItemReferenceBox,
    b"istm" => isobmff::hint::InitialSampleTimeBox,
    b"keyd" => isobmff::meta_track::MetadataKeyDeclarationBox,
    b"keys" => isobmff::metadata::Keys,
    b"kind" => isobmff::track_desc::KindBox,
    b"labl" => isobmff::audio_boxes::LabelBox,
    b"leva" => isobmff::index_boxes::LevelAssignmentBox,
    b"loca" => isobmff::meta_track::MetadataLocaleBox,
    b"ludt" => isobmff::audio_boxes::LoudnessBox,
    b"maxr" => isobmff::hint::MaxrBox,
    b"mdhd" => isobmff::init_segment::MediaHeaderBox,
    b"mdia" => isobmff::init_segment::MediaBox,
    b"mehd" => isobmff::init_segment::MovieExtendsHeaderBox,
    b"meta" => isobmff::metadata::Meta,
    b"mfra" => isobmff::timing::MovieFragmentRandomAccessBox,
    b"mfro" => isobmff::timing::MovieFragmentRandomAccessOffsetBox,
    b"minf" => isobmff::init_segment::MediaInformationBox,
    b"moov" => isobmff::init_segment::MovieBox,
    b"mvhd" => isobmff::init_segment::MovieHeaderBox,
    b"nmhd" => isobmff::init_segment::NullMediaHeaderBox,
    b"npck" => isobmff::hint::NpckBox,
    b"nump" => isobmff::hint::NumpBox,
    b"otyp" => isobmff::segments::OriginalFileTypeBox,
    b"padb" => isobmff::sample_tables_extra::PaddingBitsBox,
    b"paen" => isobmff::file_delivery::PartitionEntryBox,
    b"payt" => isobmff::hint::PaytBox,
    b"pdin" => isobmff::index_boxes::ProgressiveDownloadInfoBox,
    b"pitm" => isobmff::item::PrimaryItemBox,
    b"pmax" => isobmff::hint::PmaxBox,
    b"prft" => isobmff::sample_groups::ProducerReferenceTimeBox,
    b"rcsr" => isobmff::hint::ReceivedCsrcBox,
    b"rinf" => isobmff::scheme::RestrictedSchemeInfoBox,
    b"rsot" => isobmff::movie_fragment::RedundantSampleOriginalTimingBox,
    b"rssr" => isobmff::hint::ReceivedSsrcBox,
    b"rtp " => isobmff::hint::RtpHintInformation,
    b"rtpx" => isobmff::hint::RtpHdrExtBox,
    b"sbgp" => isobmff::sample_groups::SampleToGroupBox,
    b"schi" => isobmff::cenc::SchemeInformationBox,
    b"scrb" => isobmff::scheme::ScrambleSchemeInfoBox,
    b"sdp " => isobmff::hint::SdpHintInformation,
    b"sdtp" => isobmff::init_segment::SampleDependencyTypeBox,
    b"segr" => isobmff::file_delivery::FdSessionGroupBox,
    b"setu" => isobmff::meta_track::MetadataSetupBox,
    b"sgpd" => isobmff::sample_groups::SampleGroupDescriptionBox,
    b"sidx" => isobmff::timing::SegmentIndexBox,
    b"sinf" => isobmff::cenc::ProtectionSchemeInfoBox,
    b"smhd" => isobmff::init_segment::SoundMediaHeaderBox,
    b"snro" => isobmff::hint::SequenceOffsetBox,
    b"spki" => isobmff::scheme::SamplePackingInformationBox,
    b"srat" => isobmff::init_segment::SamplingRateBox,
    b"srpp" => isobmff::hint::SrtpProcessBox,
    b"ssix" => isobmff::index_boxes::SubsegmentIndexBox,
    b"ssrt" => isobmff::sample_groups::SubSampleReferenceTableBox,
    b"stbl" => isobmff::init_segment::SampleTableBox,
    b"stco" => isobmff::init_segment::ChunkOffsetBox,
    b"stdp" => isobmff::sample_tables_extra::DegradationPriorityBox,
    b"strd" => isobmff::track_desc::SubTrackDefinitionBox,
    b"stri" => isobmff::track_desc::SubTrackInformationBox,
    b"strk" => isobmff::track_desc::SubTrackBox,
    b"stsc" => isobmff::init_segment::SampleToChunkBox,
    b"stsd" => isobmff::init_segment::SampleDescriptionBox,
    b"stsg" => isobmff::sample_tables_extra::SubTrackSampleGroupBox,
    b"stsh" => isobmff::sample_tables_extra::ShadowSyncSampleBox,
    b"stss" => isobmff::init_segment::SyncSampleBox,
    b"stsz" => isobmff::init_segment::SampleSizeBox,
    b"stts" => isobmff::timing::TimeToSampleBox,
    b"stvi" => isobmff::track_desc::StereoVideoBox,
    b"stz2" => isobmff::init_segment::CompactSampleSizeBox,
    b"subs" => isobmff::sample_groups::SubSampleInformationBox,
    b"tPAT" => isobmff::hint::PatBox,
    b"tPMT" => isobmff::hint::PmtBox,
    b"tfra" => isobmff::timing::TrackFragmentRandomAccessBox,
    b"tims" => isobmff::hint::TimescaleEntryBox,
    b"tkhd" => isobmff::init_segment::TrackHeaderBox,
    b"tmax" => isobmff::hint::TmaxBox,
    b"tmin" => isobmff::hint::TminBox,
    b"totl" => isobmff::hint::TotlBox,
    b"tpay" => isobmff::hint::TpayBox,
    b"tpyl" => isobmff::hint::TpylBox,
    b"trak" => isobmff::init_segment::TrackBox,
    b"tref" => isobmff::track_reference::TrackReferenceBox,
    b"trep" => isobmff::index_boxes::TrackExtensionPropertiesBox,
    b"trex" => isobmff::init_segment::TrackExtendsBox,
    b"trgr" => isobmff::track_desc::TrackGroupBox,
    b"trpy" => isobmff::hint::TrpyBox,
    b"tsel" => isobmff::track_desc::TrackSelectionBox,
    b"tsro" => isobmff::hint::TimeOffsetBox,
    b"tssy" => isobmff::hint::TimestampSynchronyBox,
    b"tsti" => isobmff::hint::TsTimingBox,
    b"tyco" => isobmff::segments::TypeCombinationBox,
    b"udta" => isobmff::metadata::Udta,
    b"uri " => isobmff::meta_track::UriBox,
    b"uriI" => isobmff::meta_track::UriInitBox,
    b"vmhd" => isobmff::init_segment::VideoMediaHeaderBox,
    b"vvhd" => isobmff::init_segment::VolumetricVisualMediaHeaderBox,
    b"xml " => isobmff::meta_track::XmlBox,
}

payload_rows! {
    b"amve" => isobmff::visual_ext::AmbientViewingEnvironmentBox,
    b"cclv" => isobmff::visual_ext::ContentColourVolumeBox,
    b"clap" => isobmff::visual_ext::CleanApertureBox,
    b"clli" => isobmff::hdr_boxes::ContentLightLevel,
    b"colr" => isobmff::visual_ext::ColourInformationBox,
    b"mdcv" => isobmff::hdr_boxes::MasteringDisplayColourVolume,
    b"pasp" => isobmff::visual_ext::PixelAspectRatioBox,
}

fn parse_mdat(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    isobmff::segments::MediaDataBox::parse_box(bytes)?.try_to_bytes()
}

fn parse_saio(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    isobmff::cenc::SampleAuxInfoOffsetsBox::parse_box(bytes)?.try_to_bytes()
}

fn parse_saiz(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    isobmff::cenc::SampleAuxInfoSizesBox::parse_box(bytes)?.try_to_bytes()
}

fn parse_fdsa(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let item = isobmff::box_types::parse_box(bytes)?.0;
    isobmff::file_delivery::FdSample::parse(item.body)?.try_to_bytes()
}

fn parse_ipco(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    isobmff::item::ItemPropertyContainerBox::parse_box(bytes)?.try_to_bytes()
}

fn moof_body<T: Serialize<Error = Error>>(
    bytes: &[u8],
    parse: fn(&[u8], &mut usize) -> Result<T, Error>,
) -> Result<Vec<u8>, Error> {
    let item = isobmff::box_types::parse_box(bytes)?.0;
    let mut budget = isobmff::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
    parse(item.body, &mut budget)?.try_to_bytes()
}

fn parse_moof(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    moof_body(
        bytes,
        isobmff::movie_fragment::MovieFragmentBox::parse_body_budgeted,
    )
}

fn parse_traf(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    moof_body(
        bytes,
        isobmff::movie_fragment::TrackFragmentBox::parse_body_budgeted,
    )
}

fn parse_mfhd(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    fragment_payload(
        bytes,
        isobmff::movie_fragment::MovieFragmentHeaderBox::parse_body,
    )
}

fn parse_tfhd(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    fragment_payload(
        bytes,
        isobmff::movie_fragment::TrackFragmentHeaderBox::parse_body,
    )
}

fn parse_tfdt(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    fragment_payload(
        bytes,
        isobmff::movie_fragment::TrackFragmentBaseMediaDecodeTimeBox::parse_body,
    )
}

fn parse_trun(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    fragment_payload(
        bytes,
        isobmff::movie_fragment::TrackFragmentRunBox::parse_body,
    )
}

fn fragment_payload<T: Serialize<Error = Error>>(
    bytes: &[u8],
    parse: fn(&[u8]) -> Result<T, Error>,
) -> Result<Vec<u8>, Error> {
    let item = isobmff::box_types::parse_box(bytes)?.0;
    parse(item.body)?.try_to_bytes()
}

fn parse_schm(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let item = isobmff::box_types::parse_box(bytes)?.0;
    let body = item.body;
    let header = body
        .get(..FULLBOX_HEADER)
        .ok_or(Error::InvalidInput("schm header"))?;
    let version = header[0];
    let flags = broadcast_core::wire::u24_at(header, 1);
    isobmff::cenc::SchemeTypeBox::parse_body(&body[FULLBOX_HEADER..], version, flags)?
        .try_to_bytes()
}

fn custom_parser(fourcc: &[u8; BOX_TYPE_SIZE]) -> Option<Typed> {
    let whole = |parser: Parser| {
        Some(Typed {
            parser,
            shape: Shape::Whole,
        })
    };
    match fourcc {
        b"fdsa" => Some(Typed {
            parser: parse_fdsa as Parser,
            shape: Shape::Body,
        }),
        b"ipco" => whole(parse_ipco),
        b"mdat" => whole(parse_mdat),
        b"mfhd" => whole(parse_mfhd),
        b"moof" => whole(parse_moof),
        b"saio" => whole(parse_saio),
        b"saiz" => whole(parse_saiz),
        b"schm" => whole(parse_schm),
        b"tfdt" => whole(parse_tfdt),
        b"tfhd" => whole(parse_tfhd),
        b"traf" => whole(parse_traf),
        b"trun" => whole(parse_trun),
        _ => None,
    }
}

/// The parser this audit uses for `fourcc`.
fn typed_parser(fourcc: &[u8; BOX_TYPE_SIZE]) -> Option<Typed> {
    whole_parser(fourcc)
        .or_else(|| payload_parser(fourcc))
        .or_else(|| custom_parser(fourcc))
}

/// One occurrence of a typed box in a committed fixture.
struct Occurrence {
    fixture: String,
    offset: u64,
    fourcc: [u8; BOX_TYPE_SIZE],
    typed: Typed,
    bytes: Vec<u8>,
    body_start: usize,
    /// How many bytes of the body belong to the box's own syntax: the offset
    /// its first child starts at, or the whole body for a leaf.
    fixed_len: usize,
}

/// One box header at `at` inside `end`: its total length, where its body
/// starts, and its four-CC (ISO/IEC 14496-12:2026 §4.2).
fn header(bytes: &[u8], at: usize, end: usize) -> Option<(usize, usize, [u8; BOX_TYPE_SIZE])> {
    if at.checked_add(BOX_HEADER_MIN_SIZE)? > end {
        return None;
    }
    let mut fourcc = [0u8; BOX_TYPE_SIZE];
    fourcc.copy_from_slice(bytes.get(at + 4..at + BOX_HEADER_MIN_SIZE)?);
    let size = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
    let (len, body_start) = match size {
        0 => (end.checked_sub(at)?, BOX_HEADER_MIN_SIZE),
        1 => {
            let wide = u64::from_be_bytes(
                bytes
                    .get(at + BOX_HEADER_MIN_SIZE..at + BOX_HEADER_MIN_SIZE + 8)?
                    .try_into()
                    .ok()?,
            );
            (usize::try_from(wide).ok()?, BOX_HEADER_MIN_SIZE + 8)
        }
        size => (usize::try_from(size).ok()?, BOX_HEADER_MIN_SIZE),
    };
    if len < body_start || at.checked_add(len)? > end {
        return None;
    }
    Some((len, body_start, fourcc))
}

/// Whether a box's body may hold children this crate types: a box it types
/// itself, or a sample entry (whose child region it reads through
/// `sample_entry_variant`). Anything else is payload, never a child region.
fn descendable(fourcc: &[u8; BOX_TYPE_SIZE], bytes: &[u8]) -> bool {
    typed_parser(fourcc).is_some() || isobmff::init_segment::sample_entry_variant(bytes).is_ok()
}

/// Whether `body` is exactly a sequence of well-formed child boxes, so the walk
/// may descend into it without a per-container table. Every child's four-CC
/// must look like one, which is what keeps a payload whose bytes happen to
/// frame another box from being walked as a container.
fn is_child_region(body: &[u8]) -> bool {
    let mut at = 0usize;
    let mut count = 0usize;
    while at < body.len() {
        let Some((len, _, fourcc)) = header(body, at, body.len()) else {
            return false;
        };
        if !plausible(&fourcc) {
            return false;
        }
        count += 1;
        if count > MAX_CHILDREN {
            return false;
        }
        at += len;
    }
    count > 0 && at == body.len()
}

/// A four-CC is printable ASCII with space padding (`uri `, `rtp `), or a
/// QuickTime `©` atom.
fn plausible(fourcc: &[u8; BOX_TYPE_SIZE]) -> bool {
    fourcc.iter().all(|byte| {
        byte.is_ascii_graphic() || *byte == FOURCC_PADDING || *byte == QUICKTIME_COPYRIGHT
    })
}

/// The `©` byte a QuickTime metadata atom starts with.
const QUICKTIME_COPYRIGHT: u8 = 0xA9;
/// The space a four-CC pads a short code with (`uri `, `xml `).
const FOURCC_PADDING: u8 = b' ';

fn collect(bytes: &[u8], base: u64, fixture: &str, depth: u32, out: &mut Vec<Occurrence>) {
    if depth > MAX_DEPTH {
        return;
    }
    let mut at = 0usize;
    while at < bytes.len() {
        let Some((len, body_start, fourcc)) = header(bytes, at, bytes.len()) else {
            return;
        };
        let box_bytes = &bytes[at..at + len];
        let body = &box_bytes[body_start..];
        let mut child_start = None;
        if descendable(&fourcc, box_bytes) {
            let window = SEARCH_WINDOW.min(body.len());
            for skip in 0..=window {
                let children = &body[skip..];
                if is_child_region(children) {
                    child_start = Some(skip);
                    collect(
                        children,
                        base + u64::try_from(at + body_start + skip).unwrap_or(u64::MAX),
                        fixture,
                        depth + 1,
                        out,
                    );
                    break;
                }
            }
        }
        if let Some(typed) = typed_parser(&fourcc) {
            out.push(Occurrence {
                fixture: fixture.to_string(),
                offset: base + u64::try_from(at).unwrap_or(u64::MAX),
                fourcc,
                typed,
                bytes: box_bytes.to_vec(),
                body_start,
                fixed_len: child_start.unwrap_or(body.len()),
            });
        }
        at += len;
    }
}

/// The bytes a parse of `bytes` must reproduce for this occurrence.
fn expected<'a>(occurrence: &Occurrence, bytes: &'a [u8]) -> &'a [u8] {
    match occurrence.typed.shape {
        Shape::Whole => bytes,
        Shape::Body => &bytes[occurrence.body_start..],
    }
}

/// Flip every audited byte of one occurrence's body.
fn audit(occurrence: &Occurrence, defects: &mut Vec<String>) -> usize {
    let parser = occurrence.typed.parser;
    let intact = parser(&occurrence.bytes);
    match intact {
        Ok(out) if out == expected(occurrence, &occurrence.bytes) => {}
        Ok(out) => defects.push(format!(
            "{}: {} at {} does not round-trip ({} bytes in, {} out)",
            occurrence.fixture,
            fourcc_text(&occurrence.fourcc),
            occurrence.offset,
            occurrence.bytes.len(),
            out.len()
        )),
        Err(err) => defects.push(format!(
            "{}: {} at {} does not parse: {err}",
            occurrence.fixture,
            fourcc_text(&occurrence.fourcc),
            occurrence.offset
        )),
    }
    let fixed = occurrence.fixed_len.min(AUDIT_PREFIX);
    let end = occurrence.body_start + fixed;
    let mut checked = 0usize;
    for at in occurrence.body_start..end {
        let mut mutated = occurrence.bytes.clone();
        mutated[at] ^= NON_DEFAULT;
        if let Ok(out) = parser(&mutated)
            && out != expected(occurrence, &mutated)
        {
            defects.push(format!(
                "{}: {} at {} body byte {}: rewrite differs ({} bytes in, {} out)",
                occurrence.fixture,
                fourcc_text(&occurrence.fourcc),
                occurrence.offset,
                at - occurrence.body_start,
                mutated.len(),
                out.len()
            ));
        }
        checked += 1;
    }
    checked
}

fn fourcc_text(fourcc: &[u8; BOX_TYPE_SIZE]) -> String {
    String::from_utf8_lossy(fourcc).into_owned()
}

/// Every committed media fixture, as (path relative to `fixtures/`, path).
fn media_fixtures() -> Vec<(String, PathBuf)> {
    let root = fixtures::root();
    let mut files = Vec::new();
    walk_files(&root, &mut files);
    files.sort();
    files
        .into_iter()
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| MEDIA_EXTENSIONS.contains(&ext))
        })
        .map(|path| {
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            (rel, path)
        })
        .collect()
}

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn committed_boxes_round_trip_or_error_under_every_body_byte() {
    let mut occurrences = Vec::new();
    for (rel, path) in media_fixtures() {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        collect(&data, 0, &rel, 0, &mut occurrences);
    }
    let mut boxes = BTreeSet::new();
    let mut defects = Vec::new();
    let mut checked = 0usize;
    for occurrence in &occurrences {
        boxes.insert(occurrence.fourcc);
        checked += audit(occurrence, &mut defects);
    }
    println!(
        "box audit: {} boxes x {} occurrences x {checked} bytes",
        boxes.len(),
        occurrences.len()
    );
    assert!(
        defects.is_empty(),
        "{} boxes x {} occurrences x {checked} bytes\n{}",
        boxes.len(),
        occurrences.len(),
        defects.join("\n")
    );
}

/// Bytes appended past a box's last declared field.
const TRAILER: [u8; 4] = [0xD1, 0x2C, 0x9B, 0x5E];

/// `occurrence` with four bytes appended to its body and its size field grown
/// to match, or `None` for a header form this rewrite would change.
fn with_trailer(occurrence: &Occurrence) -> Option<Vec<u8>> {
    let size = u32::from_be_bytes(occurrence.bytes.get(..4)?.try_into().ok()?);
    if size == 0 || size == 1 {
        return None;
    }
    let mut out = occurrence.bytes.clone();
    out.extend_from_slice(&TRAILER);
    let len = u32::try_from(out.len()).ok()?;
    out[..4].copy_from_slice(&len.to_be_bytes());
    Some(out)
}

#[test]
fn bytes_past_a_boxs_declared_fields_are_carried_or_refused() {
    let mut occurrences = Vec::new();
    for (rel, path) in media_fixtures() {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        collect(&data, 0, &rel, 0, &mut occurrences);
    }
    let mut checked = 0usize;
    let mut rewrites = BTreeSet::new();
    for occurrence in &occurrences {
        let Some(mutated) = with_trailer(occurrence) else {
            continue;
        };
        checked += 1;
        match (occurrence.typed.parser)(&mutated) {
            Err(_) => {}
            Ok(out) if out == expected(occurrence, &mutated) => {}
            Ok(_) => {
                rewrites.insert(fourcc_text(&occurrence.fourcc));
            }
        }
    }
    println!("trailing-byte audit: {checked} occurrences");
    let rewrites: Vec<&str> = rewrites.iter().map(String::as_str).collect();
    assert_eq!(
        rewrites, PINNED_REWRITES_PAST_FIELDS,
        "a typed box keeps bytes past its declared fields (or stopped keeping them)"
    );
}

/// Typed boxes that write back only the fields they declare, so four bytes
/// appended past them are dropped. The boxes a reader parses strictly owe the
/// carry instead; the rest are kept opaque by the reader that meets them, so a
/// rewrite of the file stays byte-exact. A box leaves this list when it carries
/// or refuses.
const PINNED_REWRITES_PAST_FIELDS: &[&str] = &[
    "clap", "ctts", "elng", "elst", "frma", "hmhd", "iinf", "iloc", "kind", "mdhd", "mehd", "mfhd",
    "mfro", "mvhd", "nmhd", "pasp", "pitm", "prft", "saio", "sbgp", "schi", "schm", "sgpd", "sinf",
    "smhd", "srat", "ssix", "stco", "stsc", "stss", "stts", "tfdt", "tkhd", "trex", "vmhd",
];

#[test]
fn every_typed_box_has_an_audit_parser() {
    let missing: Vec<&str> = TYPED_BOXES
        .iter()
        .filter(|row| typed_parser(&row.fourcc).is_none())
        .map(|row| row.name)
        .collect();
    assert!(missing.is_empty(), "no audit parser for {missing:?}");
}

#[test]
fn typed_boxes_without_a_committed_occurrence_are_the_pinned_list() {
    let mut occurrences = Vec::new();
    for (rel, path) in media_fixtures() {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        collect(&data, 0, &rel, 0, &mut occurrences);
    }
    let present: BTreeSet<[u8; BOX_TYPE_SIZE]> = occurrences
        .iter()
        .map(|occurrence| occurrence.fourcc)
        .collect();
    let missing: Vec<&str> = TYPED_BOXES
        .iter()
        .filter(|row| !present.contains(&row.fourcc))
        .map(|row| row.name)
        .collect();
    println!("typed boxes with no committed occurrence: {missing:?}");
    assert_eq!(missing, PINNED_NO_OCCURRENCE);
}

/// Typed boxes no committed fixture carries. A box belongs here until a real
/// file that uses it is committed; then it leaves this list.
const PINNED_NO_OCCURRENCE: &[&str] = &[
    "co64", "cslg", "keys", "mdcv", "stz2", "subs", "ssrt", "dmix", "ludt", "labl", "stvi", "leva",
    "pdin", "trep", "csgp", "stdp", "padb", "stsg", "brnd", "ipro", "uri ", "uriI", "xml ", "bxml",
    "keyd", "loca", "setu", "tyco", "imda", "stri", "strd", "vvhd", "etyp", "otyp", "strk", "csch",
    "spki", "rinf", "cinf", "scrb", "ardi", "aelm", "aedb", "aepp", "aepr", "aesb", "aesd", "assp",
    "amve", "cclv", "stsh", "rsot", "trpy", "nump", "tpyl", "totl", "npck", "tpay", "maxr", "dmed",
    "dimm", "drep", "tmin", "tmax", "pmax", "dmax", "payt", "tssy", "rssr", "srpp", "rtpx", "rcsr",
    "tPAT", "tPMT", "tsti", "istm", "hinf", "fiin", "paen", "fpar", "fecr", "fire", "segr", "gitn",
    "feci", "extr", "fdpa", "fdsa",
];
