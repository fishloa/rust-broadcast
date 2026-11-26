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
//! `tests/coverage.rs` pins every type not listed here under its issue tier.

use alloc::vec::Vec;

use broadcast_core::Parse;

use crate::error::Result;

/// One typed box: its registry four-CC, the Rust type that parses it, and the
/// proof that a minimal box of that type parses.
pub struct TypedBox {
    /// The registry four-CC.
    pub fourcc: [u8; 4],
    /// The four-CC as text.
    pub name: &'static str,
    /// The Rust type that carries the parse.
    pub rust_type: &'static str,
    /// Parse a minimal box of this type through the real parser.
    pub proves: fn() -> Result<()>,
}

/// Whether this crate types `fourcc`.
#[must_use]
pub fn is_typed(fourcc: &[u8; 4]) -> bool {
    TYPED_BOXES.iter().any(|row| row.fourcc == *fourcc)
}

/// A zero body long enough for any fixed-layout box; trailing bytes are not
/// read by a whole-box parser.
const ZERO: [u8; 128] = [0u8; 128];
/// A zero body with `timescale` (body offset 12) set to 1.
const TIMESCALE_AT_12: [u8; 128] = {
    let mut body = [0u8; 128];
    body[15] = 1;
    body
};
/// A zero body with the `sidx` `timescale` (body offset 8) set to 1.
const TIMESCALE_AT_8: [u8; 128] = {
    let mut body = [0u8; 128];
    body[11] = 1;
    body
};
/// A zero `stz2` body with `field_size` set to 8.
const STZ2_BODY: [u8; 128] = {
    let mut body = [0u8; 128];
    body[7] = 8;
    body
};

fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
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
    p_co64: b"co64", crate::init_segment::ChunkLargeOffsetBox, &ZERO;
    p_cslg: b"cslg", crate::timing::CompositionToDecodeBox, &ZERO;
    p_ctts: b"ctts", crate::timing::CompositionOffsetBox, &ZERO;
    p_dinf: b"dinf", crate::init_segment::DataInformationBox, &[];
    p_dref: b"dref", crate::init_segment::DataReferenceBox, &ZERO;
    p_elst: b"elst", crate::timing::EditListBox, &ZERO;
    p_frma: b"frma", crate::cenc::OriginalFormatBox, &ZERO;
    p_hdlr: b"hdlr", crate::init_segment::HandlerBox, &ZERO;
    p_keys: b"keys", crate::metadata::Keys, &ZERO[..8];
    p_mdhd: b"mdhd", crate::init_segment::MediaHeaderBox, &TIMESCALE_AT_12;
    p_mdia: b"mdia", crate::init_segment::MediaBox, &[];
    p_meta: b"meta", crate::metadata::Meta, &ZERO;
    p_mfro: b"mfro", crate::timing::MovieFragmentRandomAccessOffsetBox, &ZERO;
    p_minf: b"minf", crate::init_segment::MediaInformationBox, &[];
    p_mvhd: b"mvhd", crate::init_segment::MovieHeaderBox, &TIMESCALE_AT_12;
    p_prft: b"prft", crate::sample_groups::ProducerReferenceTimeBox, &ZERO;
    p_sbgp: b"sbgp", crate::sample_groups::SampleToGroupBox, &ZERO;
    p_schi: b"schi", crate::cenc::SchemeInformationBox, &[];
    p_sdtp: b"sdtp", crate::init_segment::SampleDependencyTypeBox, &ZERO;
    p_sgpd: b"sgpd", crate::sample_groups::SampleGroupDescriptionBox, &ZERO;
    p_sidx: b"sidx", crate::timing::SegmentIndexBox, &TIMESCALE_AT_8;
    p_smhd: b"smhd", crate::init_segment::SoundMediaHeaderBox, &ZERO;
    p_srat: b"srat", crate::init_segment::SamplingRateBox, &ZERO;
    p_stbl: b"stbl", crate::init_segment::SampleTableBox, &[];
    p_stco: b"stco", crate::init_segment::ChunkOffsetBox, &ZERO;
    p_stsc: b"stsc", crate::init_segment::SampleToChunkBox, &ZERO;
    p_stsd: b"stsd", crate::init_segment::SampleDescriptionBox, &ZERO;
    p_stss: b"stss", crate::init_segment::SyncSampleBox, &ZERO;
    p_stsz: b"stsz", crate::init_segment::SampleSizeBox, &ZERO;
    p_stts: b"stts", crate::timing::TimeToSampleBox, &ZERO;
    p_stz2: b"stz2", crate::init_segment::CompactSampleSizeBox, &STZ2_BODY;
    p_subs: b"subs", crate::sample_groups::SubSampleInformationBox, &ZERO;
    p_ssrt: b"ssrt", crate::sample_groups::SubSampleReferenceTableBox, &ZERO;
    p_tfra: b"tfra", crate::timing::TrackFragmentRandomAccessBox, &ZERO;
    p_tkhd: b"tkhd", crate::init_segment::TrackHeaderBox, &ZERO;
    p_trex: b"trex", crate::init_segment::TrackExtendsBox, &ZERO;
    p_udta: b"udta", crate::metadata::Udta, &[];
    p_vmhd: b"vmhd", crate::init_segment::VideoMediaHeaderBox, &ZERO;
    p_chnl: b"chnl", crate::audio_boxes::ChannelLayoutBox, &ZERO;
    p_dmix: b"dmix", crate::audio_boxes::DownMixInstructionsBox, &ZERO;
    p_ludt: b"ludt", crate::audio_boxes::LoudnessBox, &[];
    p_labl: b"labl", crate::audio_boxes::LabelBox, &ZERO;
    p_elng: b"elng", crate::track_desc::ExtendedLanguageBox, &ZERO;
    p_kind: b"kind", crate::track_desc::KindBox, &ZERO;
    p_trgr: b"trgr", crate::track_desc::TrackGroupBox, &[];
    p_tsel: b"tsel", crate::track_desc::TrackSelectionBox, &ZERO;
    p_stvi: b"stvi", crate::track_desc::StereoVideoBox, &ZERO;
    p_ssix: b"ssix", crate::index_boxes::SubsegmentIndexBox, &ZERO;
    p_leva: b"leva", crate::index_boxes::LevelAssignmentBox, &ZERO;
    p_pdin: b"pdin", crate::index_boxes::ProgressiveDownloadInfoBox, &ZERO;
    p_trep: b"trep", crate::index_boxes::TrackExtensionPropertiesBox, &ZERO;
    p_csgp: b"csgp", crate::sample_groups::CompactSampleToGroupBox, &ZERO;
    p_stdp: b"stdp", crate::sample_tables_extra::DegradationPriorityBox, &ZERO;
    p_padb: b"padb", crate::sample_tables_extra::PaddingBitsBox, &ZERO;
    p_stsg: b"stsg", crate::sample_tables_extra::SubTrackSampleGroupBox, &ZERO;
}

// Body-only visual extensions: their `Parse` takes the box payload, not the
// header.
fn p_mdat() -> Result<()> {
    crate::segments::MediaDataBox::parse_box(&boxed(*b"mdat", &ZERO)).map(|_| ())
}

fn p_saio() -> Result<()> {
    crate::cenc::SampleAuxInfoOffsetsBox::parse_box(&boxed(*b"saio", &ZERO)).map(|_| ())
}

fn p_saiz() -> Result<()> {
    crate::cenc::SampleAuxInfoSizesBox::parse_box(&boxed(*b"saiz", &ZERO)).map(|_| ())
}

fn p_schm() -> Result<()> {
    crate::cenc::SchemeTypeBox::parse_body(&ZERO, 0, 0).map(|_| ())
}

fn p_clli() -> Result<()> {
    crate::hdr_boxes::ContentLightLevel::parse(&ZERO[..4]).map(|_| ())
}

fn p_mdcv() -> Result<()> {
    crate::hdr_boxes::MasteringDisplayColourVolume::parse(&ZERO[..24]).map(|_| ())
}

fn p_pasp() -> Result<()> {
    crate::visual_ext::PixelAspectRatioBox::parse(&ZERO[..8]).map(|_| ())
}

fn p_clap() -> Result<()> {
    crate::visual_ext::CleanApertureBox::parse(&ZERO[..32]).map(|_| ())
}

fn p_colr() -> Result<()> {
    let mut body = [0u8; 11];
    body[..4].copy_from_slice(b"nclx");
    crate::visual_ext::ColourInformationBox::parse(&body).map(|_| ())
}

fn p_edts() -> Result<()> {
    let inner = boxed(*b"elst", &ZERO);
    crate::init_segment::EditBox::parse(&boxed(*b"edts", &inner)).map(|_| ())
}

fn p_trak() -> Result<()> {
    let inner = boxed(*b"tkhd", &ZERO);
    crate::init_segment::TrackBox::parse(&boxed(*b"trak", &inner)).map(|_| ())
}

fn p_moov() -> Result<()> {
    let inner = boxed(*b"mvhd", &TIMESCALE_AT_12);
    crate::init_segment::MovieBox::parse(&boxed(*b"moov", &inner)).map(|_| ())
}

fn p_tref() -> Result<()> {
    let inner = boxed(*b"hint", &ZERO);
    crate::track_reference::TrackReferenceBox::parse(&boxed(*b"tref", &inner)).map(|_| ())
}

fn p_mfra() -> Result<()> {
    let body = boxed(*b"mfro", &ZERO);
    crate::timing::MovieFragmentRandomAccessBox::parse(&boxed(*b"mfra", &body)).map(|_| ())
}

fn p_sinf() -> Result<()> {
    let mut body = boxed(*b"frma", &ZERO).to_vec();
    body.extend_from_slice(&boxed(*b"schm", &ZERO));
    crate::cenc::ProtectionSchemeInfoBox::parse(&boxed(*b"sinf", &body)).map(|_| ())
}

fn p_mfhd() -> Result<()> {
    crate::movie_fragment::MovieFragmentHeaderBox::parse_body(&ZERO).map(|_| ())
}

fn p_tfhd() -> Result<()> {
    crate::movie_fragment::TrackFragmentHeaderBox::parse_body(&ZERO).map(|_| ())
}

fn p_tfdt() -> Result<()> {
    crate::movie_fragment::TrackFragmentBaseMediaDecodeTimeBox::parse_body(&ZERO).map(|_| ())
}

fn p_trun() -> Result<()> {
    crate::movie_fragment::TrackFragmentRunBox::parse_body(&ZERO).map(|_| ())
}

fn p_moof() -> Result<()> {
    let mut body = boxed(*b"mfhd", &ZERO);
    body.extend_from_slice(&boxed(*b"traf", &boxed(*b"tfhd", &ZERO)));
    let mut budget = crate::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
    crate::movie_fragment::MovieFragmentBox::parse_body_budgeted(&body, &mut budget).map(|_| ())
}

fn p_traf() -> Result<()> {
    let body = boxed(*b"tfhd", &ZERO);
    let mut budget = crate::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
    crate::movie_fragment::TrackFragmentBox::parse_body_budgeted(&body, &mut budget).map(|_| ())
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
