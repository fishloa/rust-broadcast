//! The crate's box-type coverage, as a fact the tests compare against the
//! ISO/IEC 14496-12:2026 registry.
//!
//! [`TYPED_BOXES`] is the one table of registry four-CCs this crate parses into
//! a typed structure. `crate::registry`'s `crate_types` answers from the same
//! table, so registration cannot drift from what is really modelled.
//!
//! Each row carries a `proves` function that parses a minimal box of that type
//! through the real parser, so a row for a type the crate does not parse fails
//! the coverage test. The private registry has 168 rows; the integration test
//! `tests/it/coverage.rs` pins every type not listed here under its issue tier.

use alloc::vec::Vec;

use broadcast_core::Parse;

use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE};
use crate::error::Result;

/// One typed box: its registry four-CC, the Rust type that parses it, and the
/// proof that a minimal box of that type parses.
pub struct TypedBox {
    /// The registry four-CC.
    pub fourcc: [u8; BOX_TYPE_SIZE],
    /// The four-CC as text.
    pub name: &'static str,
    /// The Rust type that carries the parse.
    pub rust_type: &'static str,
    /// Parse a minimal box of this type through the real parser.
    pub proves: fn() -> Result<()>,
}

/// Whether this crate types `fourcc`.
#[must_use]
pub fn is_typed(fourcc: &[u8; BOX_TYPE_SIZE]) -> bool {
    TYPED_BOXES.iter().any(|row| row.fourcc == *fourcc)
}

/// A zero body long enough for any fixed-layout box; trailing bytes are not
/// read by a whole-box parser.
const MINIMAL_BODY_LEN: usize = 256;
const MINIMAL_BODY: [u8; MINIMAL_BODY_LEN] = [0u8; MINIMAL_BODY_LEN];
/// A zero body with `timescale` (body offset 12) set to 1.
const MVHD_TIMESCALE_LOW_BYTE: usize = 12 + core::mem::size_of::<u32>() - 1;
const TIMESCALE_AT_12: [u8; MINIMAL_BODY_LEN] = {
    let mut body = [0u8; MINIMAL_BODY_LEN];
    body[MVHD_TIMESCALE_LOW_BYTE] = 1;
    body
};
/// A minimal version-0 `sidx` body: its fixed fields and an empty reference
/// table.
const SIDX_MIN_BODY_LEN: usize = 24;
/// Low byte of the `sidx` `timescale` (body offset 8).
const SIDX_TIMESCALE_LOW_BYTE: usize = 8 + core::mem::size_of::<u32>() - 1;
const SIDX_BODY: [u8; SIDX_MIN_BODY_LEN] = {
    let mut body = [0u8; SIDX_MIN_BODY_LEN];
    body[SIDX_TIMESCALE_LOW_BYTE] = 1;
    body
};
/// A zero `stz2` body with `field_size` set to 8.
const STZ2_FIELD_SIZE_OFFSET: usize = 7;
const STZ2_FIELD_SIZE_BITS: u8 = 8;
const STZ2_BODY: [u8; MINIMAL_BODY_LEN] = {
    let mut body = [0u8; MINIMAL_BODY_LEN];
    body[STZ2_FIELD_SIZE_OFFSET] = STZ2_FIELD_SIZE_BITS;
    body
};

fn boxed(fourcc: [u8; BOX_TYPE_SIZE], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(BOX_HEADER_MIN_SIZE + body.len());
    let size = broadcast_core::len::fit_u32(BOX_HEADER_MIN_SIZE + body.len(), "minimal box size")
        .unwrap_or(u32::MAX);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

macro_rules! leaf {
    ($( $fn_name:ident : $fourcc:literal , $ty:ty , $body:expr ; )*) => {
        $(
            fn $fn_name() -> Result<()> {
                let bytes = boxed(*$fourcc, $body);
                <$ty as Parse>::parse(&bytes).map(|_| ())
            }
        )*
    };
}

// Fixed-layout leaves read a generous zero body; containers get a minimal
// typed child.
leaf! {
    p_co64: b"co64", crate::init_segment::ChunkLargeOffsetBox, &MINIMAL_BODY;
    p_cslg: b"cslg", crate::timing::CompositionToDecodeBox, &MINIMAL_BODY;
    p_ctts: b"ctts", crate::timing::CompositionOffsetBox, &MINIMAL_BODY;
    p_dinf: b"dinf", crate::init_segment::DataInformationBox, &[];
    p_dref: b"dref", crate::init_segment::DataReferenceBox, &MINIMAL_BODY[..8];
    p_elst: b"elst", crate::timing::EditListBox, &MINIMAL_BODY;
    p_frma: b"frma", crate::cenc::OriginalFormatBox, &MINIMAL_BODY;
    p_hdlr: b"hdlr", crate::init_segment::HandlerBox, &MINIMAL_BODY;
    p_keys: b"keys", crate::metadata::Keys, &MINIMAL_BODY[..8];
    p_mdhd: b"mdhd", crate::init_segment::MediaHeaderBox, &TIMESCALE_AT_12;
    p_mdia: b"mdia", crate::init_segment::MediaBox, &[];
    p_meta: b"meta", crate::metadata::Meta, &MINIMAL_BODY;
    p_mfro: b"mfro", crate::timing::MovieFragmentRandomAccessOffsetBox, &MINIMAL_BODY;
    p_minf: b"minf", crate::init_segment::MediaInformationBox, &[];
    p_mvhd: b"mvhd", crate::init_segment::MovieHeaderBox, &TIMESCALE_AT_12;
    p_prft: b"prft", crate::sample_groups::ProducerReferenceTimeBox, &MINIMAL_BODY;
    p_sbgp: b"sbgp", crate::sample_groups::SampleToGroupBox, &MINIMAL_BODY;
    p_schi: b"schi", crate::cenc::SchemeInformationBox, &[];
    p_sdtp: b"sdtp", crate::init_segment::SampleDependencyTypeBox, &MINIMAL_BODY;
    p_sgpd: b"sgpd", crate::sample_groups::SampleGroupDescriptionBox, &MINIMAL_BODY;
    p_sidx: b"sidx", crate::timing::SegmentIndexBox, &SIDX_BODY;
    p_smhd: b"smhd", crate::init_segment::SoundMediaHeaderBox, &MINIMAL_BODY;
    p_srat: b"srat", crate::init_segment::SamplingRateBox, &MINIMAL_BODY;
    p_stbl: b"stbl", crate::init_segment::SampleTableBox, &[];
    p_stco: b"stco", crate::init_segment::ChunkOffsetBox, &MINIMAL_BODY;
    p_stsc: b"stsc", crate::init_segment::SampleToChunkBox, &MINIMAL_BODY;
    p_stsd: b"stsd", crate::init_segment::SampleDescriptionBox, &MINIMAL_BODY[..8];
    p_stss: b"stss", crate::init_segment::SyncSampleBox, &MINIMAL_BODY;
    p_stsz: b"stsz", crate::init_segment::SampleSizeBox, &MINIMAL_BODY[..12];
    p_stts: b"stts", crate::timing::TimeToSampleBox, &MINIMAL_BODY;
    p_stz2: b"stz2", crate::init_segment::CompactSampleSizeBox, &STZ2_BODY;
    p_subs: b"subs", crate::sample_groups::SubSampleInformationBox, &MINIMAL_BODY;
    p_ssrt: b"ssrt", crate::sample_groups::SubSampleReferenceTableBox, &MINIMAL_BODY;
    p_tfra: b"tfra", crate::timing::TrackFragmentRandomAccessBox, &MINIMAL_BODY[..16];
    p_tkhd: b"tkhd", crate::init_segment::TrackHeaderBox, &MINIMAL_BODY;
    p_trex: b"trex", crate::init_segment::TrackExtendsBox, &MINIMAL_BODY;
    p_udta: b"udta", crate::metadata::Udta, &[];
    p_vmhd: b"vmhd", crate::init_segment::VideoMediaHeaderBox, &MINIMAL_BODY;
    p_chnl: b"chnl", crate::audio_boxes::ChannelLayoutBox, &MINIMAL_BODY[..5];
    p_dmix: b"dmix", crate::audio_boxes::DownMixInstructionsBox, &MINIMAL_BODY;
    p_ludt: b"ludt", crate::audio_boxes::LoudnessBox, &[];
    p_labl: b"labl", crate::audio_boxes::LabelBox, &MINIMAL_BODY;
    p_elng: b"elng", crate::track_desc::ExtendedLanguageBox, &MINIMAL_BODY;
    p_kind: b"kind", crate::track_desc::KindBox, &MINIMAL_BODY;
    p_trgr: b"trgr", crate::track_desc::TrackGroupBox, &[];
    p_tsel: b"tsel", crate::track_desc::TrackSelectionBox, &MINIMAL_BODY;
    p_stvi: b"stvi", crate::track_desc::StereoVideoBox, &MINIMAL_BODY;
    p_ssix: b"ssix", crate::index_boxes::SubsegmentIndexBox, &MINIMAL_BODY;
    p_leva: b"leva", crate::index_boxes::LevelAssignmentBox, &MINIMAL_BODY;
    p_pdin: b"pdin", crate::index_boxes::ProgressiveDownloadInfoBox, &MINIMAL_BODY;
    p_trep: b"trep", crate::index_boxes::TrackExtensionPropertiesBox, &MINIMAL_BODY;
    p_csgp: b"csgp", crate::sample_groups::CompactSampleToGroupBox, &MINIMAL_BODY;
    p_stdp: b"stdp", crate::sample_tables_extra::DegradationPriorityBox, &MINIMAL_BODY;
    p_padb: b"padb", crate::sample_tables_extra::PaddingBitsBox, &MINIMAL_BODY;
    p_stsg: b"stsg", crate::sample_tables_extra::SubTrackSampleGroupBox, &MINIMAL_BODY;
    p_uri: b"uri ", crate::meta_track::UriBox, &MINIMAL_BODY;
    p_urii: b"uriI", crate::meta_track::UriInitBox, &MINIMAL_BODY;
    p_xml: b"xml ", crate::meta_track::XmlBox, &MINIMAL_BODY;
    p_bxml: b"bxml", crate::meta_track::BinaryXmlBox, &MINIMAL_BODY;
    p_keyd: b"keyd", crate::meta_track::MetadataKeyDeclarationBox, &MINIMAL_BODY;
    p_loca: b"loca", crate::meta_track::MetadataLocaleBox, &MINIMAL_BODY;
    p_setu: b"setu", crate::meta_track::MetadataSetupBox, &MINIMAL_BODY;
    p_cprt: b"cprt", crate::meta_track::CopyrightBox, &MINIMAL_BODY;
    p_nmhd: b"nmhd", crate::init_segment::NullMediaHeaderBox, &MINIMAL_BODY;
    p_hmhd: b"hmhd", crate::init_segment::HintMediaHeaderBox, &MINIMAL_BODY;
    p_tyco: b"tyco", crate::segments::TypeCombinationBox, &MINIMAL_BODY;
    p_imda: b"imda", crate::segments::IdentifiedMediaDataBox, &MINIMAL_BODY;
    p_stri: b"stri", crate::track_desc::SubTrackInformationBox, &MINIMAL_BODY;
    p_strd: b"strd", crate::track_desc::SubTrackDefinitionBox, &[];
    p_mehd: b"mehd", crate::init_segment::MovieExtendsHeaderBox, &MINIMAL_BODY;
    p_vvhd: b"vvhd", crate::init_segment::VolumetricVisualMediaHeaderBox, &MINIMAL_BODY;
    p_csch: b"csch", crate::scheme::CompatibleSchemeTypeBox, &MINIMAL_BODY;
    p_spki: b"spki", crate::scheme::SamplePackingInformationBox, &MINIMAL_BODY;
    p_ardi: b"ardi", crate::audio_elements::AudioRenderingIndicationBox, &MINIMAL_BODY;
    p_aelm: b"aelm", crate::audio_elements::AudioElementBox, &[];
    p_aedb: b"aedb", crate::audio_elements::AudioElementDescriptionBox, &MINIMAL_BODY;
    p_aepp: b"aepp", crate::audio_elements::AudioElementPosInteractivityPolarBox, &MINIMAL_BODY;
    p_aepr: b"aepr", crate::audio_elements::AudioElementProminenceInteractivityBox, &MINIMAL_BODY;
    p_aesb: b"aesb", crate::audio_elements::AudioElementSelectionBox, &[];
    p_aesd: b"aesd", crate::audio_elements::AudioElementSelectionDescriptionBox, &MINIMAL_BODY;
    p_assp: b"assp", crate::index_boxes::AlternativeStartupSequencePropertiesBox, &MINIMAL_BODY;
    p_brnd: b"brnd", crate::item::BrandProperty, &MINIMAL_BODY;
    p_grpl: b"grpl", crate::item::GroupsListBox, &[];
    p_idat: b"idat", crate::item::ItemDataBox, &MINIMAL_BODY;
    p_iinf: b"iinf", crate::item::ItemInfoBox, &MINIMAL_BODY;
    p_iloc: b"iloc", crate::item::ItemLocationBox, &MINIMAL_BODY;
    p_infe: b"infe", crate::item::ItemInfoEntry, &MINIMAL_BODY;
    p_ipma: b"ipma", crate::item::ItemPropertyAssociationBox, &MINIMAL_BODY[..8];
    p_ipro: b"ipro", crate::item::ItemProtectionBox, &MINIMAL_BODY;
    p_iref: b"iref", crate::item::ItemReferenceBox, &MINIMAL_BODY;
    p_pitm: b"pitm", crate::item::PrimaryItemBox, &MINIMAL_BODY;
}

// Body-only visual extensions: their `Parse` takes the box payload, not the
// header.
fn p_mdat() -> Result<()> {
    crate::segments::MediaDataBox::parse_box(&boxed(*b"mdat", &MINIMAL_BODY)).map(|_| ())
}

fn p_saio() -> Result<()> {
    crate::cenc::SampleAuxInfoOffsetsBox::parse_box(&boxed(*b"saio", &MINIMAL_BODY[..8]))
        .map(|_| ())
}

fn p_saiz() -> Result<()> {
    crate::cenc::SampleAuxInfoSizesBox::parse_box(&boxed(*b"saiz", &MINIMAL_BODY[..9])).map(|_| ())
}

fn p_schm() -> Result<()> {
    crate::cenc::SchemeTypeBox::parse_body(&MINIMAL_BODY, 0, 0).map(|_| ())
}

fn p_clli() -> Result<()> {
    crate::hdr_boxes::ContentLightLevel::parse(&MINIMAL_BODY[..4]).map(|_| ())
}

fn p_mdcv() -> Result<()> {
    crate::hdr_boxes::MasteringDisplayColourVolume::parse(&MINIMAL_BODY[..24]).map(|_| ())
}

fn p_pasp() -> Result<()> {
    crate::visual_ext::PixelAspectRatioBox::parse(&MINIMAL_BODY[..8]).map(|_| ())
}

fn p_clap() -> Result<()> {
    crate::visual_ext::CleanApertureBox::parse(&MINIMAL_BODY[..32]).map(|_| ())
}

fn p_colr() -> Result<()> {
    const COLR_NCLX_BODY_LEN: usize = 11;
    let mut body = [0u8; COLR_NCLX_BODY_LEN];
    body[..4].copy_from_slice(b"nclx");
    crate::visual_ext::ColourInformationBox::parse(&body).map(|_| ())
}

fn p_ipco() -> Result<()> {
    crate::item::ItemPropertyContainerBox::parse_box(&boxed(*b"ipco", &[])).map(|_| ())
}

fn p_iprp() -> Result<()> {
    let inner = boxed(*b"ipco", &[]);
    crate::item::ItemPropertiesBox::parse(&boxed(*b"iprp", &inner)).map(|_| ())
}

fn p_edts() -> Result<()> {
    let inner = boxed(*b"elst", &MINIMAL_BODY);
    crate::init_segment::EditBox::parse(&boxed(*b"edts", &inner)).map(|_| ())
}

fn p_trak() -> Result<()> {
    let inner = boxed(*b"tkhd", &MINIMAL_BODY);
    crate::init_segment::TrackBox::parse(&boxed(*b"trak", &inner)).map(|_| ())
}

fn p_moov() -> Result<()> {
    let inner = boxed(*b"mvhd", &TIMESCALE_AT_12);
    crate::init_segment::MovieBox::parse(&boxed(*b"moov", &inner)).map(|_| ())
}

fn p_tref() -> Result<()> {
    let inner = boxed(*b"hint", &MINIMAL_BODY);
    crate::track_reference::TrackReferenceBox::parse(&boxed(*b"tref", &inner)).map(|_| ())
}

fn p_mfra() -> Result<()> {
    let body = boxed(*b"mfro", &MINIMAL_BODY);
    crate::timing::MovieFragmentRandomAccessBox::parse(&boxed(*b"mfra", &body)).map(|_| ())
}

fn p_sinf() -> Result<()> {
    let mut body = boxed(*b"frma", &MINIMAL_BODY).to_vec();
    body.extend_from_slice(&boxed(*b"schm", &MINIMAL_BODY));
    crate::cenc::ProtectionSchemeInfoBox::parse(&boxed(*b"sinf", &body)).map(|_| ())
}

fn p_mfhd() -> Result<()> {
    crate::movie_fragment::MovieFragmentHeaderBox::parse_body(&MINIMAL_BODY[..8]).map(|_| ())
}

fn p_tfhd() -> Result<()> {
    crate::movie_fragment::TrackFragmentHeaderBox::parse_body(&MINIMAL_BODY[..8]).map(|_| ())
}

fn p_tfdt() -> Result<()> {
    crate::movie_fragment::TrackFragmentBaseMediaDecodeTimeBox::parse_body(&MINIMAL_BODY[..8])
        .map(|_| ())
}

fn p_trun() -> Result<()> {
    crate::movie_fragment::TrackFragmentRunBox::parse_body(&MINIMAL_BODY[..8]).map(|_| ())
}

fn p_moof() -> Result<()> {
    let mut body = boxed(*b"mfhd", &MINIMAL_BODY);
    body.extend_from_slice(&boxed(*b"traf", &boxed(*b"tfhd", &MINIMAL_BODY[..8])));
    let mut budget = crate::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
    crate::movie_fragment::MovieFragmentBox::parse_body_budgeted(&body, &mut budget).map(|_| ())
}

fn p_traf() -> Result<()> {
    let body = boxed(*b"tfhd", &MINIMAL_BODY[..8]);
    let mut budget = crate::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
    crate::movie_fragment::TrackFragmentBox::parse_body_budgeted(&body, &mut budget).map(|_| ())
}

fn p_amve() -> Result<()> {
    crate::visual_ext::AmbientViewingEnvironmentBox::parse(&MINIMAL_BODY[..8]).map(|_| ())
}

fn p_cclv() -> Result<()> {
    crate::visual_ext::ContentColourVolumeBox::parse(&MINIMAL_BODY).map(|_| ())
}

fn p_etyp() -> Result<()> {
    let body = boxed(*b"tyco", &MINIMAL_BODY);
    crate::segments::ExtendedTypeBox::parse(&boxed(*b"etyp", &body)).map(|_| ())
}

fn p_otyp() -> Result<()> {
    let mut body = boxed(*b"ftyp", &MINIMAL_BODY);
    body.extend_from_slice(&boxed(*b"etyp", &boxed(*b"tyco", &MINIMAL_BODY)));
    crate::segments::OriginalFileTypeBox::parse(&boxed(*b"otyp", &body)).map(|_| ())
}

/// `schm` fixed body: version/flags plus `scheme_type` and `scheme_version`.
const SCHM_FULL_BODY_LEN: usize = 12;

fn p_rinf() -> Result<()> {
    let body = boxed(*b"schm", &[0u8; SCHM_FULL_BODY_LEN]);
    crate::scheme::RestrictedSchemeInfoBox::parse(&boxed(*b"rinf", &body)).map(|_| ())
}

fn p_scrb() -> Result<()> {
    let body = boxed(*b"schm", &[0u8; SCHM_FULL_BODY_LEN]);
    crate::scheme::ScrambleSchemeInfoBox::parse(&boxed(*b"scrb", &body)).map(|_| ())
}

fn p_cinf() -> Result<()> {
    let body = boxed(*b"frma", b"mp4v");
    crate::scheme::CompleteTrackInfoBox::parse(&boxed(*b"cinf", &body)).map(|_| ())
}

fn p_strk() -> Result<()> {
    let body = boxed(*b"stri", &MINIMAL_BODY);
    crate::track_desc::SubTrackBox::parse(&boxed(*b"strk", &body)).map(|_| ())
}

macro_rules! typed {
    ($( $fn_name:ident : $fourcc:literal , $name:literal , $ty:literal ; )*) => {
        /// Every ISO/IEC 14496-12:2026 registry box type this crate parses.
        pub const TYPED_BOXES: &[TypedBox] = &[
            $(
                TypedBox {
                    fourcc: *$fourcc,
                    name: $name,
                    rust_type: $ty,
                    proves: $fn_name,
                },
            )*
        ];
    };
}

typed! {
        p_clli: b"clli", "clli", "ContentLightLevel";
        p_co64: b"co64", "co64", "ChunkLargeOffsetBox";
        p_cslg: b"cslg", "cslg", "CompositionToDecodeBox";
        p_ctts: b"ctts", "ctts", "CompositionOffsetBox";
        p_dinf: b"dinf", "dinf", "DataInformationBox";
        p_dref: b"dref", "dref", "DataReferenceBox";
        p_elst: b"elst", "elst", "EditListBox";
        p_frma: b"frma", "frma", "OriginalFormatBox";
        p_hdlr: b"hdlr", "hdlr", "HandlerBox";
        p_keys: b"keys", "keys", "Keys";
        p_mdat: b"mdat", "mdat", "MediaDataBox";
        p_mdcv: b"mdcv", "mdcv", "MasteringDisplayColourVolume";
        p_mdhd: b"mdhd", "mdhd", "MediaHeaderBox";
        p_mdia: b"mdia", "mdia", "MediaBox";
        p_meta: b"meta", "meta", "Meta";
        p_mfhd: b"mfhd", "mfhd", "MovieFragmentHeaderBox";
        p_mfro: b"mfro", "mfro", "MovieFragmentRandomAccessOffsetBox";
        p_minf: b"minf", "minf", "MediaInformationBox";
        p_mvhd: b"mvhd", "mvhd", "MovieHeaderBox";
        p_prft: b"prft", "prft", "ProducerReferenceTimeBox";
        p_saio: b"saio", "saio", "SampleAuxiliaryInformationOffsetsBox";
        p_saiz: b"saiz", "saiz", "SampleAuxiliaryInformationSizesBox";
        p_sbgp: b"sbgp", "sbgp", "SampleToGroupBox";
        p_schi: b"schi", "schi", "SchemeInformationBox";
        p_schm: b"schm", "schm", "SchemeTypeBox";
        p_sdtp: b"sdtp", "sdtp", "SampleDependencyTypeBox";
        p_sgpd: b"sgpd", "sgpd", "SampleGroupDescriptionBox";
        p_sidx: b"sidx", "sidx", "SegmentIndexBox";
        p_smhd: b"smhd", "smhd", "SoundMediaHeaderBox";
        p_srat: b"srat", "srat", "SamplingRateBox";
        p_stbl: b"stbl", "stbl", "SampleTableBox";
        p_stco: b"stco", "stco", "ChunkOffsetBox";
        p_stsc: b"stsc", "stsc", "SampleToChunkBox";
        p_stsd: b"stsd", "stsd", "SampleDescriptionBox";
        p_stss: b"stss", "stss", "SyncSampleBox";
        p_stsz: b"stsz", "stsz", "SampleSizeBox";
        p_stts: b"stts", "stts", "TimeToSampleBox";
        p_stz2: b"stz2", "stz2", "CompactSampleSizeBox";
        p_subs: b"subs", "subs", "SubSampleInformationBox";
        p_ssrt: b"ssrt", "ssrt", "SubSampleReferenceTableBox";
        p_tfdt: b"tfdt", "tfdt", "TrackFragmentBaseMediaDecodeTimeBox";
        p_tfhd: b"tfhd", "tfhd", "TrackFragmentHeaderBox";
        p_tfra: b"tfra", "tfra", "TrackFragmentRandomAccessBox";
        p_tkhd: b"tkhd", "tkhd", "TrackHeaderBox";
        p_trex: b"trex", "trex", "TrackExtendsBox";
        p_udta: b"udta", "udta", "Udta";
        p_vmhd: b"vmhd", "vmhd", "VideoMediaHeaderBox";
        p_chnl: b"chnl", "chnl", "ChannelLayoutBox";
        p_dmix: b"dmix", "dmix", "DownMixInstructionsBox";
        p_ludt: b"ludt", "ludt", "LoudnessBox";
        p_labl: b"labl", "labl", "LabelBox";
        p_elng: b"elng", "elng", "ExtendedLanguageBox";
        p_kind: b"kind", "kind", "KindBox";
        p_trgr: b"trgr", "trgr", "TrackGroupBox";
        p_tsel: b"tsel", "tsel", "TrackSelectionBox";
        p_stvi: b"stvi", "stvi", "StereoVideoBox";
        p_ssix: b"ssix", "ssix", "SubsegmentIndexBox";
        p_leva: b"leva", "leva", "LevelAssignmentBox";
        p_pdin: b"pdin", "pdin", "ProgressiveDownloadInfoBox";
        p_trep: b"trep", "trep", "TrackExtensionPropertiesBox";
        p_csgp: b"csgp", "csgp", "CompactSampleToGroupBox";
        p_stdp: b"stdp", "stdp", "DegradationPriorityBox";
        p_padb: b"padb", "padb", "PaddingBitsBox";
        p_stsg: b"stsg", "stsg", "SubTrackSampleGroupBox";
        p_brnd: b"brnd", "brnd", "BrandProperty";
        p_grpl: b"grpl", "grpl", "GroupsListBox";
        p_idat: b"idat", "idat", "ItemDataBox";
        p_iinf: b"iinf", "iinf", "ItemInfoBox";
        p_iloc: b"iloc", "iloc", "ItemLocationBox";
        p_infe: b"infe", "infe", "ItemInfoEntry";
        p_ipco: b"ipco", "ipco", "ItemPropertyContainerBox";
        p_ipma: b"ipma", "ipma", "ItemPropertyAssociationBox";
        p_ipro: b"ipro", "ipro", "ItemProtectionBox";
        p_iprp: b"iprp", "iprp", "ItemPropertiesBox";
        p_iref: b"iref", "iref", "ItemReferenceBox";
        p_pitm: b"pitm", "pitm", "PrimaryItemBox";
            p_pasp: b"pasp", "pasp", "PixelAspectRatioBox";
        p_clap: b"clap", "clap", "CleanApertureBox";
        p_colr: b"colr", "colr", "ColourInformationBox";
        p_edts: b"edts", "edts", "EditBox";
        p_trak: b"trak", "trak", "TrackBox";
        p_moov: b"moov", "moov", "MovieBox";
        p_tref: b"tref", "tref", "TrackReferenceBox";
        p_mfra: b"mfra", "mfra", "MovieFragmentRandomAccessBox";
        p_sinf: b"sinf", "sinf", "ProtectionSchemeInfoBox";
        p_moof: b"moof", "moof", "MovieFragmentBox";
        p_traf: b"traf", "traf", "TrackFragmentBox";
        p_trun: b"trun", "trun", "TrackFragmentRunBox";
        p_uri: b"uri ", "uri ", "UriBox";
        p_urii: b"uriI", "uriI", "UriInitBox";
        p_xml: b"xml ", "xml ", "XmlBox";
        p_bxml: b"bxml", "bxml", "BinaryXmlBox";
        p_keyd: b"keyd", "keyd", "MetadataKeyDeclarationBox";
        p_loca: b"loca", "loca", "MetadataLocaleBox";
        p_setu: b"setu", "setu", "MetadataSetupBox";
        p_cprt: b"cprt", "cprt", "CopyrightBox";
        p_nmhd: b"nmhd", "nmhd", "NullMediaHeaderBox";
        p_hmhd: b"hmhd", "hmhd", "HintMediaHeaderBox";
        p_tyco: b"tyco", "tyco", "TypeCombinationBox";
        p_imda: b"imda", "imda", "IdentifiedMediaDataBox";
        p_stri: b"stri", "stri", "SubTrackInformationBox";
        p_strd: b"strd", "strd", "SubTrackDefinitionBox";
        p_mehd: b"mehd", "mehd", "MovieExtendsHeaderBox";
        p_vvhd: b"vvhd", "vvhd", "VolumetricVisualMediaHeaderBox";
        p_etyp: b"etyp", "etyp", "ExtendedTypeBox";
        p_otyp: b"otyp", "otyp", "OriginalFileTypeBox";
        p_strk: b"strk", "strk", "SubTrackBox";
        p_csch: b"csch", "csch", "CompatibleSchemeTypeBox";
        p_spki: b"spki", "spki", "SamplePackingInformationBox";
        p_rinf: b"rinf", "rinf", "RestrictedSchemeInfoBox";
        p_cinf: b"cinf", "cinf", "CompleteTrackInfoBox";
        p_scrb: b"scrb", "scrb", "ScrambleSchemeInfoBox";
        p_ardi: b"ardi", "ardi", "AudioRenderingIndicationBox";
        p_aelm: b"aelm", "aelm", "AudioElementBox";
        p_aedb: b"aedb", "aedb", "AudioElementDescriptionBox";
        p_aepp: b"aepp", "aepp", "AudioElementPosInteractivityPolarBox";
        p_aepr: b"aepr", "aepr", "AudioElementProminenceInteractivityBox";
        p_aesb: b"aesb", "aesb", "AudioElementSelectionBox";
        p_aesd: b"aesd", "aesd", "AudioElementSelectionDescriptionBox";
        p_assp: b"assp", "assp", "AlternativeStartupSequencePropertiesBox";
        p_amve: b"amve", "amve", "AmbientViewingEnvironmentBox";
        p_cclv: b"cclv", "cclv", "ContentColourVolumeBox";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_proves_its_type_parses() {
        let mut failures = alloc::string::String::new();
        for row in TYPED_BOXES {
            if let Err(err) = (row.proves)() {
                let _ = core::fmt::Write::write_fmt(
                    &mut failures,
                    format_args!("\n{} ({}): {err:?}", row.name, row.rust_type),
                );
            }
        }
        assert!(failures.is_empty(), "{failures}");
    }

    #[test]
    fn names_are_unique_and_match_their_four_ccs() {
        for (index, row) in TYPED_BOXES.iter().enumerate() {
            assert_eq!(row.name.as_bytes(), &row.fourcc);
            for other in &TYPED_BOXES[index + 1..] {
                assert_ne!(row.fourcc, other.fourcc, "duplicate {}", row.name);
            }
        }
    }
}
