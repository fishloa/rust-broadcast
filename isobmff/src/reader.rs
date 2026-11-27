//! The sans-IO MP4 reader: it asks the caller for structure bytes and yields
//! every track, then one sample descriptor at a time.
//!
//! The reader owns the bytes of one `moov` or one `moof` at a time and nothing
//! else. Sample data never passes through it: a [`SampleRef`] says where the
//! bytes are, and the caller fetches that range itself. A progressive file
//! whose `moov` follows a multi-gigabyte `mdat` is reached by skipping the
//! media box by its declared size, so the walk costs only box headers; over a
//! forward-only source that file is a typed
//! [`Error::MoovAfterMdatForwardOnly`].
//!
//! A fragmented file (a `moov` with `mvex`) is walked one `moof` at a time:
//! the fragment's box is read under [`Limits::max_fragment_bytes`], its `trex`
//! defaults applied, and the following `mdat` payload ranges learned from their
//! headers. [`Step::Fragment`] followed by one [`Step::Sample`] per sample is
//! emitted as soon as every run of the fragment's `traf`s resolves inside the
//! payloads located so far, before the walk asks for anything past that media,
//! so a forward-only source still has the sample bytes ahead of it. A run that
//! addresses a later `mdat` keeps the fragment pending until that box is
//! located. Fragment sample data is addressed by [`crate::frag_offsets`] with
//! those ranges, never with a slice of the file, so no byte inside an `mdat`
//! payload is ever requested.
//!
//! Driving is [`Reader::step`] / [`Reader::supply`]: each `step` returns either
//! a [`Step::Need`] range to supply or a typed step, and `supply` accepts
//! exactly the outstanding range. See the streaming-first design §4.2 and §4.4
//! (`docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md`)
//! and ISO/IEC 14496-12:2026 §4.2, §8.2-§8.7
//! (`private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`).

use alloc::collections::VecDeque;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::cmp::Ordering;

use broadcast_core::Parse;
use broadcast_core::timescale::Timescale;

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, BoxHeader, BoxIter, FULLBOX_EXTRA_SIZE,
    parse_box,
};
use crate::cenc::{SampleEncryptionBox, TrackEncryptionBox, sinf_of_sample_entry};
use crate::error::{Error, Result};
use crate::frag_offsets::{MdatRange, sample_ranges_in_run, sample_ranges_in_run_until};
use crate::init_segment::{
    MediaBox, MovieBox, SampleEntryVariant, StblChild, TrackBox, TrackExtendsBox,
    sample_entry_variant,
};
use crate::limits::Limits;
use crate::metadata::MovieMetadata;
use crate::movie_fragment::{
    MAX_ZERO_WIDTH_TRUN_SAMPLES, MovieFragmentBox, SAMPLE_FLAG_IS_NON_SYNC, TrackFragmentBox,
    TrafChild, apply_trex_defaults,
};
use crate::plan::{BoxPosition, SamplePlan, TopLevelBox, sample_description, sample_table};
use crate::sample_groups::{GROUPING_TYPE_SEIG, SampleGroupDescriptionBox, SampleToGroupBox};
use crate::sample_table::{CursorState, SampleRef};
use crate::segments::FileTypeBox;
use crate::timing::{
    EditListEntry, MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox,
    SegmentIndexBox,
};
use crate::track_id::TrackId;
use crate::track_reference::TrackReferenceTypeBox;
use crate::writer::SampleAux;

/// Largest bytes a top-level box header can occupy: 32-bit `size`, 32-bit
/// `type`, a 64-bit `largesize` and a 16-byte `uuid` usertype
/// (ISO/IEC 14496-12:2026 §4.2).
const MAX_BOX_HEADER_BYTES: u64 = 32;

/// A box header occupies at least `size` + `type` (§4.2).
const MIN_BOX_HEADER_BYTES: u64 = BOX_HEADER_MIN_SIZE as u64;

/// A sample description starts with a FullBox header then a 32-bit entry count
/// (ISO/IEC 14496-12:2026 §8.5.2).
const STSD_ENTRY_COUNT_OFFSET: usize = FULLBOX_EXTRA_SIZE;

/// Depth at which the `moov` walk starts counting: the movie box is one.
const MOOV_DEPTH: u32 = 1;
/// Depth of a `trak` inside `moov`.
const TRAK_DEPTH: u32 = MOOV_DEPTH + 1;
/// Depth of a `mdia` inside `trak`.
const MDIA_DEPTH: u32 = TRAK_DEPTH + 1;
/// Depth of a `minf` inside `mdia`.
const MINF_DEPTH: u32 = MDIA_DEPTH + 1;
/// Depth of a `stbl` inside `minf`.
const STBL_DEPTH: u32 = MINF_DEPTH + 1;
/// Depth of a `stsd` inside `stbl`.
const STSD_DEPTH: u32 = STBL_DEPTH + 1;

/// Bytes of a `mfro` box, header included: the trailer a reader looks at to
/// find a trailing `mfra` (ISO/IEC 14496-12:2026 §8.8.11).
const MFRO_BOX_LEN: u64 = 16;

/// Indexes the reader keeps from the walk: one per track of a segment, for the
/// `sidx` that precedes each track's fragments.
const MAX_REMEMBERED_INDEXES: usize = 16;

/// A half-open byte range in the input file: `[offset, offset + len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ByteRange {
    /// First byte of the range.
    pub offset: u64,
    /// Number of bytes in the range.
    pub len: u64,
}

impl ByteRange {
    /// One past the last byte of the range.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.offset.saturating_add(self.len)
    }
}

/// One identified media data box (`imda`, §8.1.4) the walk has passed.
///
/// A `DataEntryImdaBox` (`imdt`) or `DataEntrySeqNumImdaBox` (`snim`) data
/// reference addresses this box by [`identifier`](Self::identifier); the
/// media-data offsets are relative to the first byte of
/// [`payload`](Self::payload) (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct IdentifiedMediaData {
    identifier: u32,
    payload: ByteRange,
}

impl IdentifiedMediaData {
    /// `imda_identifier`.
    #[must_use]
    pub const fn identifier(&self) -> u32 {
        self.identifier
    }

    /// The payload's byte range: where media-data offset 0 points.
    #[must_use]
    pub const fn payload(&self) -> ByteRange {
        self.payload
    }
}

/// What a sample entry's `data_reference_index` points at (§8.7.2.3, §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataReferenceBase {
    /// The data is in this file, addressed by absolute offsets (`url ` with
    /// the self-contained flag, or no data reference).
    SameFile,
    /// The data is in the `imda` with this identifier (`imdt`).
    Identified(u32),
    /// The data is in the `imda` whose identifier is the fragment's sequence
    /// number (`snim`).
    SequenceNumber,
}

/// The `data_reference_index` at offset 14 of a sample entry's bytes
/// (SampleEntry `reserved[6]` then the 16-bit index, §8.5.2.2).
fn entry_data_reference_index(body: &[u8]) -> u16 {
    const DATA_REFERENCE_INDEX_OFFSET: usize = core::mem::size_of::<u32>() * 2 + 6;
    body.get(DATA_REFERENCE_INDEX_OFFSET..DATA_REFERENCE_INDEX_OFFSET + core::mem::size_of::<u16>())
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        .unwrap_or(1)
}

/// The offsets a track's stsd entries are addressed relative to.
fn track_data_references(
    trak: &crate::init_segment::TrackBox,
    entries: &[SampleEntryInfo],
) -> Vec<DataReferenceBase> {
    let dref = trak
        .mdia
        .as_ref()
        .and_then(|mdia| mdia.minf.as_ref())
        .and_then(|minf| minf.dinf.as_ref())
        .and_then(|dinf| dinf.dref.as_ref());
    entries
        .iter()
        .map(|entry| {
            let index = usize::from(entry_data_reference_index(&entry.body));
            let Some(dref) = dref else {
                return DataReferenceBase::SameFile;
            };
            match index.checked_sub(1).and_then(|i| dref.entries.get(i)) {
                Some(crate::init_segment::DataEntry::Imda(box_)) => {
                    DataReferenceBase::Identified(box_.imda_ref_identifier)
                }
                Some(crate::init_segment::DataEntry::SeqNumImda(_)) => {
                    DataReferenceBase::SequenceNumber
                }
                _ => DataReferenceBase::SameFile,
            }
        })
        .collect()
}

/// The byte offset a sample referenced through `base` starts at, or `None`
/// when the identified media data box it names has not been walked yet.
fn data_reference_bias(
    bases: &[DataReferenceBase],
    imda: &[IdentifiedMediaData],
    description_index: u32,
) -> Option<u64> {
    let index = usize::try_from(description_index.checked_sub(1)?).ok()?;
    match bases
        .get(index)
        .copied()
        .unwrap_or(DataReferenceBase::SameFile)
    {
        DataReferenceBase::SameFile | DataReferenceBase::SequenceNumber => Some(0),
        DataReferenceBase::Identified(identifier) => imda
            .iter()
            .find(|m| m.identifier == identifier)
            .map(|m| m.payload.offset),
    }
}

/// The order [`Reader::step`] yields [`Step::Sample`] steps in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SampleOrder {
    /// Merge tracks by decode time, ties broken by `moov` track order.
    #[default]
    DecodeTime,
    /// Ascending file offset, smallest next offset among the track heads.
    File,
}

/// Whether the caller's byte source can seek backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Source {
    /// Every requested range can be served, including a backward seek.
    #[default]
    Seekable,
    /// Ranges must not move backwards.
    ForwardOnly,
}

/// How strictly the reader treats known real-world faults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Mode {
    /// Any violation is an error.
    #[default]
    Strict,
    /// The faults listed by the design §4.7 are reported as warnings.
    Permissive,
}

/// Configuration for [`Reader::new`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ReaderConfig {
    limits: Limits,
    total_len: Option<u64>,
    order: SampleOrder,
    source: Source,
    mode: Mode,
    registry: Option<crate::registry::BoxRegistry>,
}

impl ReaderConfig {
    /// Set the total input length, which lets a box of size 0 run to the end.
    #[must_use]
    pub fn with_total_len(mut self, total_len: Option<u64>) -> Self {
        self.total_len = total_len;
        self
    }

    /// Set the sample delivery order.
    #[must_use]
    pub fn with_order(mut self, order: SampleOrder) -> Self {
        self.order = order;
        self
    }

    /// Set whether the source can seek backwards.
    #[must_use]
    pub fn with_source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    /// Set the fault policy.
    #[must_use]
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// Set the hostile-input caps.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The hostile-input caps.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The total input length, when the caller knows it.
    #[must_use]
    pub fn total_len(&self) -> Option<u64> {
        self.total_len
    }

    /// The sample delivery order.
    #[must_use]
    pub fn order(&self) -> SampleOrder {
        self.order
    }

    /// Whether the source can seek backwards.
    #[must_use]
    pub fn source(&self) -> Source {
        self.source
    }

    /// The fault policy.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Attach a [`BoxRegistry`](crate::registry::BoxRegistry) of private box
    /// types, so a registered `stsd` entry comes back typed in
    /// [`SampleEntryInfo::registered`].
    #[must_use]
    pub fn with_registry(mut self, registry: crate::registry::BoxRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// The attached private box registry, when one was set.
    #[must_use]
    pub fn registry(&self) -> Option<&crate::registry::BoxRegistry> {
        self.registry.as_ref()
    }
}

/// One sample description (`stsd` entry, ISO/IEC 14496-12:2026 §8.5.2).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SampleEntryInfo {
    /// The entry's four-CC.
    pub fourcc: [u8; BOX_TYPE_SIZE],
    /// The full sample-entry box bytes, header included, copied from `moov`.
    pub body: Vec<u8>,
    /// The typed parse, or `None` when the entry is an unknown four-CC.
    pub typed: Option<SampleEntryVariant>,
    /// Why a known four-CC's typed parse failed, when it did. `typed` is
    /// `None` in that case and this carries the parser's own message, so a
    /// demuxer can name the real cause (issue #952).
    pub error: Option<alloc::string::String>,
    /// The [`ReaderConfig`]'s private-box-registry parse of this entry, when a
    /// registration matched its four-CC; call
    /// [`RegisteredEntry::downcast_ref`](crate::registry::RegisteredEntry::downcast_ref)
    /// for the concrete type.
    pub registered: Option<crate::registry::RegisteredEntry>,
}

/// A timed-metadata sample entry, as the configured entry variant.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum TimedMetadataEntry<'a> {
    /// `metx` — [`XmlMetaDataSampleEntry`](crate::meta_track::XmlMetaDataSampleEntry).
    Xml(&'a crate::meta_track::XmlMetaDataSampleEntry),
    /// `mett` or `stxt` — [`TextMetaDataSampleEntry`](crate::meta_track::TextMetaDataSampleEntry).
    Text(&'a crate::meta_track::TextMetaDataSampleEntry),
    /// `urim` — [`UriMetaDataSampleEntry`](crate::meta_track::UriMetaDataSampleEntry).
    Uri(&'a crate::meta_track::UriMetaDataSampleEntry),
    /// `mebx` — [`BoxedMetaDataSampleEntry`](crate::meta_track::BoxedMetaDataSampleEntry).
    Boxed(&'a crate::meta_track::BoxedMetaDataSampleEntry),
}

impl TimedMetadataEntry<'_> {
    /// The entry's four-CC.
    #[must_use]
    pub fn fourcc(&self) -> [u8; BOX_TYPE_SIZE] {
        match self {
            Self::Xml(_) => crate::meta_track::METX,
            Self::Text(entry) => entry.codec_type(),
            Self::Uri(_) => crate::meta_track::URIM,
            Self::Boxed(_) => crate::meta_track::MEBX,
        }
    }

    /// The metadata form's URI (`uri `), for a `urim` entry.
    #[must_use]
    pub fn uri(&self) -> Option<&crate::meta_track::UriBox> {
        match self {
            Self::Uri(entry) => Some(entry.uri()),
            _ => None,
        }
    }

    /// The URI initialization data (`uriI`), for a `urim` entry that carries
    /// it.
    #[must_use]
    pub fn uri_init(&self) -> Option<&crate::meta_track::UriInitBox> {
        match self {
            Self::Uri(entry) => entry.uri_init(),
            _ => None,
        }
    }

    /// The key table (`keys`), for a `mebx` entry.
    #[must_use]
    pub fn key_table(&self) -> Option<&crate::meta_track::MetadataKeyTableBox> {
        match self {
            Self::Boxed(entry) => Some(entry.key_table()),
            _ => None,
        }
    }

    /// The XML namespaces (`utf8list`), for a `metx` entry.
    #[must_use]
    pub fn namespace(&self) -> Option<&[u8]> {
        match self {
            Self::Xml(entry) => Some(entry.namespace()),
            _ => None,
        }
    }

    /// The MIME format, for a `mett`/`stxt` entry.
    #[must_use]
    pub fn mime_format(&self) -> Option<&[u8]> {
        match self {
            Self::Text(entry) => Some(entry.mime_format()),
            _ => None,
        }
    }
}

impl SampleEntryInfo {
    /// The timed-metadata entry this sample description is, when it is one
    /// (§12.3.3, §12.5.3, §12.8.4.1).
    #[must_use]
    pub fn timed_metadata(&self) -> Option<TimedMetadataEntry<'_>> {
        match self.typed.as_ref()? {
            crate::init_segment::SampleEntryVariant::Metx(entry) => {
                Some(TimedMetadataEntry::Xml(entry))
            }
            crate::init_segment::SampleEntryVariant::Text(entry) => {
                Some(TimedMetadataEntry::Text(entry))
            }
            crate::init_segment::SampleEntryVariant::Urim(entry) => {
                Some(TimedMetadataEntry::Uri(entry))
            }
            crate::init_segment::SampleEntryVariant::Mebx(entry) => {
                Some(TimedMetadataEntry::Boxed(entry))
            }
            _ => None,
        }
    }

    /// The hint sample entry this description is, when it is one (§9.1.2,
    /// §9.2.3, §9.3.3, §9.4.1–9.4.4).
    #[must_use]
    pub fn hint(&self) -> Option<&crate::hint::HintSampleEntry> {
        match self.typed.as_ref()? {
            crate::init_segment::SampleEntryVariant::Hint(entry) => Some(entry),
            _ => None,
        }
    }

    /// The bit-rate declaration (`btrt`, §8.5.2) this sample description
    /// carries, when present.
    #[must_use]
    pub fn bit_rate(&self) -> Option<crate::sample_entries::BitRateBox> {
        self.entry_body(b"btrt")
            .and_then(|body| crate::sample_entries::BitRateBox::parse_body(body).ok())
    }

    /// The restriction scheme (`rinf`, §13.5.3) this sample description
    /// carries, when it carries one — either because the entry's four-CC is a
    /// restricted code (`resv`, `resa`, …) or because an advisory `rinf` sits
    /// inside an untransformed entry.
    #[must_use]
    pub fn restricted_scheme(&self) -> Option<crate::scheme::RestrictedSchemeInfoBox> {
        self.entry_child(b"rinf")
            .and_then(|bytes| crate::scheme::RestrictedSchemeInfoBox::parse(bytes).ok())
    }

    /// The scrambling scheme (`scrb`, §13.4.8) this sample description
    /// carries, when it carries one.
    #[must_use]
    pub fn scramble_scheme(&self) -> Option<crate::scheme::ScrambleSchemeInfoBox> {
        self.entry_child(b"scrb")
            .and_then(|bytes| crate::scheme::ScrambleSchemeInfoBox::parse(bytes).ok())
    }

    /// The complete-track information (`cinf`, §13.6.3) an incomplete track's
    /// sample description carries, when it carries one.
    #[must_use]
    pub fn complete_track_info(&self) -> Option<crate::scheme::CompleteTrackInfoBox> {
        self.entry_child(b"cinf")
            .and_then(|bytes| crate::scheme::CompleteTrackInfoBox::parse(bytes).ok())
    }

    /// The ambient viewing environment (`amve`, §12.1.9) this visual sample
    /// description carries, when it carries one.
    #[must_use]
    pub fn ambient_viewing_environment(
        &self,
    ) -> Option<crate::visual_ext::AmbientViewingEnvironmentBox> {
        self.entry_body(b"amve")
            .and_then(|body| crate::visual_ext::AmbientViewingEnvironmentBox::parse(body).ok())
    }

    /// The content colour volume (`cclv`, §12.1.8) this visual sample
    /// description carries, when it carries one.
    #[must_use]
    pub fn content_colour_volume(&self) -> Option<crate::visual_ext::ContentColourVolumeBox> {
        self.entry_body(b"cclv")
            .and_then(|body| crate::visual_ext::ContentColourVolumeBox::parse(body).ok())
    }

    /// One child box of the entry's payload, by four-CC.
    fn entry_body(&self, fourcc: &[u8; BOX_TYPE_SIZE]) -> Option<&[u8]> {
        self.entry_child(fourcc)
            .and_then(|bytes| parse_box(bytes).ok())
            .map(|(item, _)| item.body)
    }

    /// One child box of the entry, by four-CC, from the entry's own bytes.
    ///
    /// The children start after the SampleEntry fixed part, whose length
    /// depends on the entry's class (§8.5.2.2, §12.1.3, §12.2.3): a visual
    /// entry's 78 bytes, an audio entry's 28, and 8 for the metadata, text and
    /// subtitle classes. A restricted (`resv`/`resa`/...) or incomplete
    /// (`icpv`/`icpa`) entry keeps its replaced class's fixed part
    /// (§13.5.2 Table 14, §13.6.2), so the four-CC decides when no typed form
    /// names the class.
    fn entry_child(&self, fourcc: &[u8; BOX_TYPE_SIZE]) -> Option<&[u8]> {
        let body = parse_box(&self.body).ok()?.0.body;
        let fixed = self.entry_fixed_len();
        let region = body.get(fixed..)?;
        crate::box_types::BoxIter::new(region)
            .filter_map(|child| child.ok())
            .find(|child| child.four_cc() == *fourcc)
            .map(|child| child.bytes)
    }

    /// The SampleEntry fixed-part length this entry uses, in bytes.
    fn entry_fixed_len(&self) -> usize {
        use crate::init_segment::SampleEntryVariant as V;
        const SAMPLE_ENTRY_BASE: usize = 8;
        const AUDIO_FIXED: usize = 28;
        const VISUAL_FIXED: usize = 78;
        match self.typed.as_ref() {
            Some(V::Avc1(_) | V::Hevc1(_) | V::Vvc(_) | V::Mp4v(_) | V::Av01(_) | V::Vp09(_)) => {
                VISUAL_FIXED
            }
            Some(V::Mp4a(_) | V::Ac3(_) | V::Ec3(_) | V::Ac4(_) | V::Opus(_) | V::Flac(_))
            | Some(V::Mha(_) | V::Dts(_)) => AUDIO_FIXED,
            Some(_) => SAMPLE_ENTRY_BASE,
            None => match &self.fourcc {
                b"resv" | b"icpv" => VISUAL_FIXED,
                b"resa" | b"icpa" => AUDIO_FIXED,
                _ => SAMPLE_ENTRY_BASE,
            },
        }
    }

    /// The channel layout (`chnl`, §12.2.4) this audio entry carries, when
    /// present.
    #[must_use]
    pub fn channel_layout(&self) -> Option<crate::audio_boxes::ChannelLayoutBox> {
        self.typed.as_ref()?.channel_layout()
    }

    /// The downmix instructions (`dmix`, §12.2.5) this audio entry carries,
    /// parsed against the entry's base channel count.
    #[must_use]
    pub fn downmix_instructions(
        &self,
        base_channel_count: u8,
    ) -> Option<crate::audio_boxes::DownMixInstructionsBox> {
        self.typed
            .as_ref()?
            .downmix_instructions(base_channel_count)
    }
}

/// Every fact the reader surfaces about one track, including tracks the media
/// IR has no mapping for.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TrackInfo {
    /// The `tkhd.track_ID` (§8.3.2).
    pub track_id: TrackId,
    /// The `hdlr.handler_type` (§8.4.3).
    pub handler: [u8; BOX_TYPE_SIZE],
    /// The `mdhd.timescale` (§8.4.2).
    pub timescale: Timescale,
    /// The `mdhd.duration`, in the track timescale (§8.4.2).
    pub duration: u64,
    /// The full track header fields (§8.3.2).
    pub header: crate::init_segment::TrackHeaderBox,
    /// The `tref` references (§8.3.3), preserving wire order.
    pub references: Vec<TrackReferenceTypeBox>,
    /// The `elst` entries, unapplied (§8.6.6).
    pub edits: Vec<EditListEntry>,
    /// The `stsd` entries in wire order; `description_index` on a sample is
    /// one-based into this list.
    pub entries: Vec<SampleEntryInfo>,
    /// The QuickTime track aperture dimensions (`tapt`), when present.
    pub tapt: Option<crate::quicktime::TrackApertureModeDimensionsBox>,
    /// The extended language tag (`elng`, §8.4.6), when present.
    pub extended_language: Option<crate::track_desc::ExtendedLanguageBox>,
    /// The track groups (`trgr`, §8.3.4), when present.
    pub track_groups: Option<crate::track_desc::TrackGroupBox>,
    /// The track's kind labels (`kind`, §8.10.4), in wire order.
    pub kinds: Vec<crate::track_desc::KindBox>,
    /// The track's labels (`labl`, §8.10.5), in wire order.
    pub labels: Vec<crate::audio_boxes::LabelBox>,
    /// The track's audio loudness declaration (`ludt`, §12.2.7), when present.
    pub loudness: Option<crate::audio_boxes::LoudnessBox>,
    /// The track selection (`tsel`, §8.10.3), when present.
    pub track_selection: Option<crate::track_desc::TrackSelectionBox>,
    /// The media information header (`vmhd`/`smhd`/`nmhd`/`hmhd`/`vvhd`,
    /// §8.4.5), when the crate models it.
    pub media_header: Option<crate::init_segment::MediaInformationHeader>,
    /// The track's sub-tracks (`strk`, §8.13.3), in wire order.
    pub subtracks: Vec<crate::track_desc::SubTrackBox>,
    /// The samples a decoder must decode before its output is correct, from a
    /// `prol` or a negative `roll` sample-group distance (§10.1). This is a
    /// decoder pre-roll requirement, not a gapless trim.
    pub pre_roll: Option<u32>,
    /// The movie's or the track's iTunes gapless facts ([`crate::metadata::Itunsmpb`]),
    /// when the file states them. A track-level item wins over a movie-level
    /// one; a movie-level item applies only when the file has one audio track.
    pub itunsmpb: Option<crate::metadata::Itunsmpb>,
    /// The hint information (`hnti`, §9.1.4), when the track's `udta` carries
    /// one.
    pub hint_information: Option<crate::hint::HintInformationBox>,
    /// The hint statistics (`hinf`, §9.1.5), when the track's `udta` carries
    /// one.
    pub hint_statistics: Option<crate::hint::HintStatisticsBox>,
}

impl TrackInfo {
    /// Whether this is an image sequence track: handler type `pict`
    /// (ISO/IEC 23008-12:2025 7.1, 7.2.2).
    #[must_use]
    pub const fn is_image_sequence(&self) -> bool {
        matches!(&self.handler, b"pict")
    }

    /// The first sample entry's Coding Constraints box (`ccst`, 7.2.3), when
    /// the entry carries one.
    #[must_use]
    pub fn coding_constraints(&self) -> Option<crate::image_sequence::CodingConstraintsBox> {
        self.entries
            .iter()
            .find_map(|entry| find_entry_child(&entry.body, &crate::image_sequence::CCST))
            .and_then(|bytes| crate::image_sequence::CodingConstraintsBox::parse(&bytes).ok())
    }

    /// The first sample entry's Auxiliary Type Info box (`auxi`, 7.5.3).
    #[must_use]
    pub fn auxiliary_type_info(&self) -> Option<crate::image_sequence::AuxiliaryTypeInfoBox> {
        self.entries
            .iter()
            .find_map(|entry| find_entry_child(&entry.body, &crate::image_sequence::AUXI))
            .and_then(|bytes| crate::image_sequence::AuxiliaryTypeInfoBox::parse(&bytes).ok())
    }

    /// The reader's track view for an already-parsed `trak`.
    ///
    /// It carries no movie metadata, so [`itunsmpb`](Self::itunsmpb) is always
    /// `None`; use [`Reader`] or [`MovieBox`] for
    /// the file's item list.
    ///
    /// A caller that holds the whole `moov` (the whole-input demuxers) needs
    /// the same per-track facts [`Reader`] surfaces in [`Step::Tracks`]. This
    /// is that view, built from the typed box tree: each `stsd` entry is
    /// serialised back to its own box bytes so [`SampleEntryInfo::body`] is
    /// the bytes the reader would have copied. `handler` is zero when the
    /// `trak` carries no `hdlr`, which the whole-input demuxers never read.
    pub fn from_trak(trak: &TrackBox) -> Result<Self> {
        use broadcast_core::{Parse, Serialize};
        let mdia = trak
            .mdia
            .as_ref()
            .ok_or(Error::UnexpectedBox { expected: "mdia" })?;
        let mdhd = mdia
            .mdhd
            .as_ref()
            .ok_or(Error::UnexpectedBox { expected: "mdhd" })?;
        let minf = mdia
            .minf
            .as_ref()
            .ok_or(Error::UnexpectedBox { expected: "minf" })?;
        let stbl = minf
            .stbl
            .as_ref()
            .ok_or(Error::UnexpectedBox { expected: "stbl" })?;
        let stsd = stbl
            .children
            .iter()
            .find_map(|child| match child {
                StblChild::Stsd(stsd) => Some(Ok(stsd.clone())),
                StblChild::Opaque(bytes)
                    if bytes.len() >= BOX_HEADER_MIN_SIZE
                        && &bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] == b"stsd" =>
                {
                    Some(crate::init_segment::SampleDescriptionBox::parse(
                        bytes.as_slice(),
                    ))
                }
                _ => None,
            })
            .ok_or(Error::UnexpectedBox { expected: "stsd" })??;
        let mut entries = Vec::with_capacity(stsd.entries.len());
        for entry in &stsd.entries {
            let mut body = alloc::vec![0u8; entry.serialized_len()];
            let len = entry.serialize_into(&mut body)?;
            body.truncate(len);
            let mut fourcc = [0u8; BOX_TYPE_SIZE];
            if body.len() >= BOX_HEADER_MIN_SIZE {
                fourcc.copy_from_slice(&body[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE]);
            }
            entries.push(SampleEntryInfo {
                fourcc,
                body,
                typed: match entry {
                    SampleEntryVariant::Unknown(_) => None,
                    known => Some(known.clone()),
                },
                error: None,
                registered: None,
            });
        }
        let references = match trak.track_references()? {
            Some(reference) => reference.references,
            None => Vec::new(),
        };
        let edits = match trak.edts.as_ref().and_then(|edts| edts.elst.as_ref()) {
            Some(elst) => elst.entries.clone(),
            None => Vec::new(),
        };
        let tapt = trak
            .opaque
            .iter()
            .find(|b| b.box_type == crate::quicktime::aperture::TAPT)
            .and_then(|b| {
                crate::quicktime::TrackApertureModeDimensionsBox::parse_body(&b.data).ok()
            });
        let desc = track_description(trak);
        Ok(Self {
            track_id: trak.tkhd.track_id,
            handler: mdia
                .hdlr
                .as_ref()
                .map_or([0u8; BOX_TYPE_SIZE], |hdlr| hdlr.handler_type),
            timescale: mdhd.timescale,
            duration: mdhd.duration,
            header: trak.tkhd.clone(),
            references,
            edits,
            entries,
            tapt,
            extended_language: desc.extended_language,
            track_groups: desc.track_groups,
            kinds: desc.kinds,
            labels: desc.labels,
            loudness: desc.loudness,
            track_selection: desc.track_selection,
            media_header: desc.media_header,
            subtracks: desc.subtracks,
            pre_roll: track_pre_roll(trak),
            itunsmpb: None,
            hint_information: desc.hint_information,
            hint_statistics: desc.hint_statistics,
        })
    }
}

/// One fragment (`moof`) the reader is about to hand out samples for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FragmentInfo {
    /// The `mfhd.sequence_number` (§8.8.2).
    pub sequence_number: u32,
    /// The `moof` box's own byte range, header included.
    pub moof: ByteRange,
    /// The tracks this fragment carries samples for, in `traf` order.
    pub tracks: Vec<TrackId>,
    /// How this fragment states itself: its `tfhd`/`trun` field choices, run
    /// splits, child order and the boxes between it and the previous
    /// fragment. The writer reproduces it byte for byte.
    pub layout: crate::frag_layout::FragmentLayout,
    /// The subsegment index (`ssix`, §8.14.4) seen before this fragment,
    /// beside the `sidx`.
    pub subsegment_index: Option<crate::index_boxes::SubsegmentIndexBox>,
    /// The movie's level assignment (`leva`, §8.8.13), when `mvex` carries
    /// one.
    pub level_assignment: Option<crate::index_boxes::LevelAssignmentBox>,
}

/// A fault [`Mode::Permissive`] tolerated, reported once as [`Step::Warning`].
///
/// Each variant names the box or sample table row that carried the fault and
/// the absolute file offset it starts at. A warning never invents a field
/// value: the reader keeps what the file states and drops only what it cannot
/// address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// A box inside `moov` declared a size past its parent; it was clamped to
    /// the parent's end.
    ClampedBox {
        /// The four-CC of the clamped box.
        box_type: [u8; BOX_TYPE_SIZE],
        /// Absolute offset of the box.
        offset: u64,
    },
    /// A known sample-entry four-CC did not parse; the entry was kept raw, with
    /// no typed form.
    UnparseableSampleEntry {
        /// The original four-CC.
        fourcc: [u8; BOX_TYPE_SIZE],
        /// Absolute offset of the entry.
        offset: u64,
    },
    /// An `mp4a` sound entry's `wave` atom carried no `esds`.
    WaveWithoutEsds {
        /// Absolute offset of the `wave` atom.
        offset: u64,
    },
    /// The movie header timescale was zero; it was treated as 1 so the walk can
    /// continue. Durations in the movie timescale are unknown.
    ZeroMovieTimescale {
        /// Absolute offset of the `mvhd` box.
        offset: u64,
    },
    /// The final `mdat` was truncated; samples fully inside the input were
    /// emitted.
    TruncatedMdat {
        /// Absolute offset of the first range or box past the input end.
        offset: u64,
    },
    /// A final fragment (`moof` or its `mdat`) was truncated.
    TruncatedFragment {
        /// Absolute offset of the truncated fragment box.
        offset: u64,
    },
    /// A `moof` in the middle of the file was malformed; the walk continued
    /// with the next fragment.
    MalformedFragment {
        /// Absolute offset of the malformed `moof`.
        offset: u64,
    },
    /// A box the reader typed carried bytes past its last declared field.
    /// They are kept verbatim and written back with the box; nothing about the
    /// table is lost.
    TrailingBytes {
        /// The four-CC of the box.
        box_type: [u8; BOX_TYPE_SIZE],
        /// Absolute offset of the box.
        offset: u64,
        /// How many bytes past its last field the box carried.
        carried: u64,
    },
    /// A box that is not needed to locate samples was in a form the crate
    /// cannot reproduce. It is kept opaque in its own position, byte-exact,
    /// and reading continues.
    UnrepresentableBox {
        /// The four-CC of the box.
        box_type: [u8; BOX_TYPE_SIZE],
        /// Absolute offset of the box.
        offset: u64,
        /// The class of fault the typed parse reported.
        reason: crate::error::OpaqueReason,
    },
}

impl Warning {
    /// The warning's stable label.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::ClampedBox { .. } => "clamped-box",
            Self::UnparseableSampleEntry { .. } => "unparseable-sample-entry",
            Self::WaveWithoutEsds { .. } => "wave-without-esds",
            Self::ZeroMovieTimescale { .. } => "zero-movie-timescale",
            Self::TruncatedMdat { .. } => "truncated-mdat",
            Self::TruncatedFragment { .. } => "truncated-fragment",
            Self::MalformedFragment { .. } => "malformed-fragment",
            Self::TrailingBytes { .. } => "trailing-bytes",
            Self::UnrepresentableBox { .. } => "unrepresentable-box",
        }
    }
}

broadcast_core::impl_spec_display!(Warning);

impl Warning {
    /// The absolute file offset the fault starts at.
    #[must_use]
    pub const fn position(&self) -> u64 {
        match self {
            Self::ClampedBox { offset, .. }
            | Self::UnparseableSampleEntry { offset, .. }
            | Self::WaveWithoutEsds { offset }
            | Self::ZeroMovieTimescale { offset }
            | Self::TruncatedMdat { offset }
            | Self::TruncatedFragment { offset }
            | Self::MalformedFragment { offset }
            | Self::TrailingBytes { offset, .. }
            | Self::UnrepresentableBox { offset, .. } => *offset,
        }
    }
}

/// One step of the reader state machine.
#[derive(Debug)]
#[non_exhaustive]
pub enum Step<'a> {
    /// Supply exactly this range with [`Reader::supply`].
    Need(ByteRange),
    /// Every track of the movie, in `moov` order. Emitted once.
    Tracks(&'a [TrackInfo]),
    /// The movie's and each track's typed metadata. Emitted once, after
    /// [`Step::Tracks`], only when the file carries `udta` or `meta`.
    Metadata(&'a MovieMetadata),
    /// A fragment's samples follow.
    Fragment(FragmentInfo),
    /// One sample descriptor, in the configured order.
    Sample(SampleRef),
    /// A fault the configured [`Mode::Permissive`] tolerated.
    Warning(Warning),
    /// No more steps.
    Done,
}

/// The outstanding byte demand.
#[derive(Debug, Clone, Copy)]
enum Pending {
    Header(ByteRange),
    Ftyp(ByteRange),
    Moov(ByteRange),
    Moof(ByteRange),
    Index(ByteRange),
    Subsegment(ByteRange),
    MfraTrailer(ByteRange),
    Mfra(ByteRange),
    MfraForm(ByteRange),
    TopLevel(ByteRange),
    Imda { head: ByteRange, payload_end: u64 },
}

impl Pending {
    fn range(&self) -> ByteRange {
        match self {
            Self::Header(range)
            | Self::Ftyp(range)
            | Self::Moov(range)
            | Self::Moof(range)
            | Self::Index(range)
            | Self::Subsegment(range)
            | Self::MfraTrailer(range)
            | Self::Mfra(range)
            | Self::MfraForm(range)
            | Self::TopLevel(range) => *range,
            Self::Imda { head, .. } => *head,
        }
    }
}

/// One track's cursor and the `moov` track index it was built from.
#[derive(Debug, Clone)]
struct CursorSlot {
    track_index: usize,
    state: CursorState,
}

/// A raw `stsd` entry taken from the supplied `moov` bytes.
struct RawEntry {
    fourcc: [u8; BOX_TYPE_SIZE],
    body: Vec<u8>,
    offset: u64,
}

/// One protected track's `tenc` defaults, resolved from the `sinf` of its
/// sample entry (ISO/IEC 23001-7 §12.2).
#[derive(Debug, Clone)]
struct TrackCrypto {
    tenc: TrackEncryptionBox,
}

/// The `moof` being read, with the `mdat` payload ranges its runs may address.
#[derive(Debug)]
struct FragmentState {
    offset: u64,
    range: ByteRange,
    moof: Option<MovieFragmentBox>,
    mdats: Vec<MdatRange>,
    before: Vec<crate::box_support::OpaqueBox>,
    layout: Option<crate::frag_layout::FragmentLayout>,
}

/// A `sidx` found in the walk, with the byte offset its `first_offset` is
/// measured from (the first byte after the box, §8.14.3).
#[derive(Debug)]
struct SegmentIndex {
    index: SegmentIndexBox,
    anchor: u64,
}

/// Where a fragmented seek is in its byte-driven search for an index.
#[derive(Debug, Clone, Copy)]
enum SeekPhase {
    /// Walking headers for a `sidx` that precedes the fragments.
    Scanning,
    /// Waiting for the trailing `mfro` bytes at the end of the file.
    Trailer,
    /// Waiting for the `mfra` box the `mfro` pointed at.
    Mfra,
}

/// A seek that needs index bytes before it can move the walk.
#[derive(Debug, Clone, Copy)]
struct SeekRequest {
    track: TrackId,
    decode_time: u64,
    phase: SeekPhase,
}

/// One owned step of a resolved fragment, so handing it out does not keep the
/// reader borrowed.
#[derive(Debug)]
enum FragmentStep {
    Fragment(FragmentInfo),
    Sample(SampleRef),
}

/// One fragment resolved against the `mdat` payloads located so far, before
/// the reader's per-track cursors are advanced.
#[derive(Debug)]
struct ResolvedFragment {
    info: FragmentInfo,
    samples: Vec<SampleRef>,
    truncated: bool,
    /// Per track: the `moov` track index, the decode time after the fragment
    /// and the sample count after it.
    cursors: Vec<(usize, u64, u32)>,
}

/// What the walk does with a fragment whose runs do not all resolve inside the
/// `mdat` payloads located so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unresolved {
    /// A `mdat` later in the file may still hold the data: leave the fragment
    /// pending and keep walking.
    Keep,
    /// Nothing more can arrive for it, so the resolver's own error is the
    /// caller's.
    Fail,
}

/// A fragment whose samples are resolved and waiting to be handed out.
#[derive(Debug)]
struct ReadyFragment {
    info: FragmentInfo,
    samples: Vec<SampleRef>,
    next: usize,
    emitted: bool,
}

/// The sans-IO MP4 reader. See the [module docs](self).
///
/// # What is structural, and what degrades
///
/// A box is **structural** when the samples cannot be located without it. Such
/// a box is read strictly: a declared count or size that runs past the box, a
/// FullBox version the crate does not model, or a header a size cannot hold is
/// a typed error. Bytes a structural box carries past its last declared field
/// are not an error: they are kept verbatim and written back, and reported in
/// [`Mode::Permissive`] as [`Warning::TrailingBytes`].
///
/// | Where | Structural (strict) | Degrades to raw carriage |
/// |---|---|---|
/// | top level | `ftyp`, `moov`, `moof`, `mdat`, `sidx`, `ssix`, `mfra` | any other box |
/// | `moov` | `mvhd`, `trak`, `mvex` | `udta`, `meta` and their typed atoms |
/// | `trak` | `tkhd`, `mdia` | `edts`/`elst`, `trgr`, `udta`/`meta` atoms |
/// | `mdia` | `mdhd`, `hdlr`, `minf` | `elng` |
/// | `minf` | `dinf`/`dref`, `stbl` | `vmhd`, `smhd`, `nmhd`, `hmhd`, `vvhd`, `gmhd` |
/// | `stbl` | `stsd`, `stts`, `ctts`, `stsc`, `stsz`/`stz2`, `stco`/`co64`, `stss`, `sdtp`, `stdp`, `padb` | a known four-CC whose parse fails (kept as `StblChild::Opaque`) |
/// | `moof` | `mfhd`, `tfhd`, `tfdt`, `trun` | a `traf` child the crate does not type |
/// | sample entry | the fixed entry fields and the codec config box | every other child, typed on demand |
///
/// A degraded box keeps its bytes, its four-CC and its position, so a rewrite
/// stays byte-exact; in [`Mode::Permissive`] it is reported as
/// [`Warning::UnrepresentableBox`] naming the box, its offset and the kind of
/// fault. A sample entry whose known four-CC fails additionally keeps the
/// parser's message in [`SampleEntryInfo::error`](crate::reader::SampleEntryInfo)
/// and is reported as [`Warning::UnparseableSampleEntry`].
#[derive(Debug)]
pub struct Reader {
    config: ReaderConfig,
    pending: Option<Pending>,
    pos: u64,
    highest_requested: u64,
    saw_mdat: bool,
    movie: Option<MovieBox>,
    brand: Option<FileTypeBox>,
    extended_type: Option<crate::segments::ExtendedTypeBox>,
    original_type: Option<crate::segments::OriginalFileTypeBox>,
    identified_media: Vec<IdentifiedMediaData>,
    sample_references: Vec<Vec<DataReferenceBase>>,
    warnings: Vec<Warning>,
    deferred_warning: Option<Warning>,
    mdat_truncation_reported: bool,
    tracks: Vec<TrackInfo>,
    cursors: Vec<CursorSlot>,
    emitted_tracks: bool,
    metadata: Option<MovieMetadata>,
    emitted_metadata: bool,
    done: bool,
    fragmented: bool,
    trex: Vec<TrackExtendsBox>,
    crypto: Vec<Option<TrackCrypto>>,
    progressive_iv_sizes: Vec<Option<u8>>,
    decode_times: Vec<u64>,
    sample_counts: Vec<u32>,
    fragments: VecDeque<FragmentState>,
    top_level: Vec<TopLevelBox>,
    ready: Option<ReadyFragment>,
    last_fragment: Option<(u64, Vec<SampleRef>)>,
    open_box: Option<ByteRange>,
    indexes: Vec<SegmentIndex>,
    index: crate::frag_layout::FragmentIndex,
    between: Vec<crate::box_support::OpaqueBox>,
    subsegment_index: Option<crate::index_boxes::SubsegmentIndexBox>,
    level_assignment: Option<crate::index_boxes::LevelAssignmentBox>,
    seek: Option<SeekRequest>,
    #[cfg(feature = "scan-counters")]
    bytes_requested: u64,
}

impl Reader {
    /// Create a reader. It issues its first [`Step::Need`] from the first
    /// [`Reader::step`].
    #[must_use]
    pub fn new(config: ReaderConfig) -> Self {
        Self {
            config,
            pending: None,
            pos: 0,
            highest_requested: 0,
            saw_mdat: false,
            movie: None,
            brand: None,
            extended_type: None,
            original_type: None,
            identified_media: Vec::new(),
            sample_references: Vec::new(),
            warnings: Vec::new(),
            deferred_warning: None,
            mdat_truncation_reported: false,
            tracks: Vec::new(),
            cursors: Vec::new(),
            emitted_tracks: false,
            metadata: None,
            emitted_metadata: false,
            done: false,
            fragmented: false,
            trex: Vec::new(),
            crypto: Vec::new(),
            progressive_iv_sizes: Vec::new(),
            decode_times: Vec::new(),
            sample_counts: Vec::new(),
            fragments: VecDeque::new(),
            top_level: Vec::new(),
            ready: None,
            last_fragment: None,
            open_box: None,
            indexes: Vec::new(),
            index: crate::frag_layout::FragmentIndex::default(),
            between: Vec::new(),
            subsegment_index: None,
            level_assignment: None,
            seek: None,
            #[cfg(feature = "scan-counters")]
            bytes_requested: 0,
        }
    }

    /// Supply the outstanding [`Step::Need`] range.
    ///
    /// `offset` must equal the requested offset and `bytes` may not be longer
    /// than the requested range; a shorter supply means the input ended there.
    pub fn supply(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        let Some(pending) = self.pending.take() else {
            return Err(Error::InvalidInput("supply without an outstanding need"));
        };
        let expected = pending.range();
        let got = ByteRange {
            offset,
            len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        };
        if offset != expected.offset || got.len > expected.len {
            self.pending = Some(pending);
            return Err(Error::SupplyMismatch { expected, got });
        }
        #[cfg(feature = "scan-counters")]
        {
            self.bytes_requested = self.bytes_requested.saturating_add(got.len);
        }
        match pending {
            Pending::Header(range) => self.accept_header(range, bytes),
            Pending::Ftyp(_) => self.accept_ftyp(bytes),
            Pending::Moov(range) => self.accept_moov(range, bytes),
            Pending::Moof(_) => self.accept_moof(bytes),
            Pending::Index(range) => self.accept_index(range, bytes),
            Pending::Subsegment(range) => self.accept_subsegment(range, bytes),
            Pending::MfraTrailer(_) => self.accept_mfra_trailer(bytes),
            Pending::Mfra(range) => self.accept_mfra(range, bytes),
            Pending::MfraForm(range) => self.accept_mfra_form(range, bytes),
            Pending::TopLevel(_) => self.accept_top_level(bytes),
            Pending::Imda { head, payload_end } => {
                self.accept_imda(head.offset, payload_end, bytes)
            }
        }
    }

    /// Bytes the reader has asked the caller to supply, under the
    /// `scan-counters` feature.
    #[cfg(feature = "scan-counters")]
    #[must_use]
    pub fn bytes_requested(&self) -> u64 {
        self.bytes_requested
    }

    /// Seek to the last sync sample at or before `decode_time`, expressed in
    /// `track`'s timescale.
    ///
    /// A progressive file seeks every track's cursor through its `stss`
    /// (§8.6.2). A fragmented file positions the walk on the fragment holding
    /// the time, using a `sidx` seen before the fragments (§8.14.3) or, failing
    /// that, the trailing `mfra` reached through its `mfro` (§8.8.9-§8.8.11);
    /// a file with neither is [`Error::NoIndex`]. When index bytes must be read
    /// first, this returns with a [`Step::Need`] outstanding and the next
    /// material step is the [`Step::Fragment`] containing the time.
    pub fn seek_to(&mut self, track: TrackId, decode_time: u64) -> Result<()> {
        if self.movie.is_none() {
            return Err(Error::InvalidInput("seek before moov is parsed"));
        }
        let named = self
            .tracks
            .iter()
            .position(|info| info.track_id == track)
            .ok_or(Error::InvalidInput("unknown track"))?;
        if self.fragmented {
            return self.seek_fragmented(track, decode_time);
        }
        let movie = self
            .movie
            .as_ref()
            .ok_or(Error::InvalidInput("seek before moov is parsed"))?;
        let from_timescale = self.tracks[named].timescale;
        for slot in &mut self.cursors {
            let info = self
                .tracks
                .get(slot.track_index)
                .ok_or(Error::InvalidInput("unknown track"))?;
            let target = rescale_floor(decode_time, from_timescale, info.timescale);
            let trak = movie
                .tracks
                .get(slot.track_index)
                .ok_or(Error::UnexpectedBox { expected: "trak" })?;
            let stbl = sample_table(trak).ok_or(Error::NoIndex {
                track: info.track_id,
            })?;
            slot.state.seek_sync(stbl, target)?;
        }
        self.done = false;
        Ok(())
    }

    /// Declare that the bytes supplied so far end one self-contained unit of
    /// sequential input: a media segment.
    ///
    /// `unit_end` is the absolute offset one past the unit's last byte, the
    /// caller's running sequential position. A top-level box whose declared end
    /// lies past it is dropped. In [`Mode::Permissive`] the cut fragment's
    /// samples wholly inside the unit stay available and a
    /// [`Warning::TruncatedFragment`] follows them; in [`Mode::Strict`] the
    /// truncation is the typed error. The `moov` and its track state, the
    /// `trex` defaults and the running decode time are kept, and the top-level
    /// walk resumes at `unit_end`, so the next segment is read from its first
    /// byte. A unit that ended on a box boundary produces no step.
    pub fn end_of_unit(&mut self, unit_end: u64) -> Result<()> {
        let overrun = self.open_box.is_some_and(|range| range.end() > unit_end);
        let samples_overrun =
            self.ready
                .as_ref()
                .is_some_and(|ready| ready.samples.iter().any(|sample| extent(sample) > unit_end))
                || self.last_fragment.as_ref().is_some_and(|(_, samples)| {
                    samples.iter().any(|sample| extent(sample) > unit_end)
                });
        let truncated = overrun || samples_overrun;
        if truncated && !self.tolerated() {
            let (start, end) = match self.open_box.filter(|range| range.end() > unit_end) {
                Some(range) => (range.offset, range.end()),
                None => (unit_end, unit_end),
            };
            return Err(Error::BufferTooShort(
                broadcast_core::error::BufferTooShort {
                    need: usize::try_from(end.saturating_sub(start)).unwrap_or(usize::MAX),
                    have: usize::try_from(unit_end.saturating_sub(start)).unwrap_or(usize::MAX),
                    what: "top-level box body",
                },
            ));
        }
        if let Some(ready) = self.ready.take() {
            self.retime_after_cut(&ready.samples, unit_end);
            let emitted = ready.next;
            let samples: Vec<SampleRef> = ready
                .samples
                .into_iter()
                .enumerate()
                .filter(|(index, sample)| *index < emitted || extent(sample) <= unit_end)
                .map(|(_, sample)| sample)
                .collect();
            let kept = samples.len();
            if !ready.emitted || emitted < kept {
                self.ready = Some(ReadyFragment {
                    info: ready.info,
                    samples,
                    next: emitted.min(kept),
                    emitted: ready.emitted,
                });
            } else {
                self.last_fragment = Some((ready.info.moof.offset, samples));
            }
        } else if let Some((offset, samples)) = self.last_fragment.take() {
            self.retime_after_cut(&samples, unit_end);
            self.last_fragment = Some((offset, samples));
        }
        if truncated {
            self.deferred_warning = Some(Warning::TruncatedFragment {
                offset: self
                    .fragments
                    .front()
                    .map(|fragment| fragment.offset)
                    .or_else(|| self.ready.as_ref().map(|ready| ready.info.moof.offset))
                    .or_else(|| self.last_fragment.as_ref().map(|(offset, _)| *offset))
                    .unwrap_or(unit_end),
            });
        }
        self.fragments.clear();
        self.open_box = None;
        self.last_fragment = None;
        self.between.clear();
        self.pending = None;
        self.pos = unit_end;
        self.highest_requested = unit_end;
        self.done = false;
        Ok(())
    }

    /// Recompute each track's running decode time and sample count from the
    /// samples that survive a cut, so a following fragment without `tfdt`
    /// continues from the last kept sample rather than from a dropped one.
    fn retime_after_cut(&mut self, samples: &[SampleRef], unit_end: u64) {
        let mut base: Vec<(usize, u64, u32)> = Vec::new();
        let mut advanced: Vec<(usize, u64, u32)> = Vec::new();
        for sample in samples {
            let Some(index) = self
                .tracks
                .iter()
                .position(|info| info.track_id == sample.track)
            else {
                continue;
            };
            match base.iter_mut().find(|slot| slot.0 == index) {
                Some(slot) => {
                    slot.1 = slot.1.min(sample.decode_time);
                    slot.2 = slot.2.min(sample.index);
                }
                None => base.push((index, sample.decode_time, sample.index)),
            }
            if extent(sample) <= unit_end {
                let decode_time = sample
                    .decode_time
                    .saturating_add(u64::from(sample.duration));
                let count = sample.index.saturating_add(1);
                match advanced.iter_mut().find(|slot| slot.0 == index) {
                    Some(slot) => {
                        slot.1 = decode_time;
                        slot.2 = count;
                    }
                    None => advanced.push((index, decode_time, count)),
                }
            }
        }
        for (index, decode_time, count) in base {
            self.decode_times[index] = decode_time;
            self.sample_counts[index] = count;
        }
        for (index, decode_time, count) in advanced {
            self.decode_times[index] = decode_time;
            self.sample_counts[index] = count;
        }
    }

    /// The file's extended type declarations (`etyp`, §5.3.2), once the walk
    /// has passed one.
    #[must_use]
    pub fn extended_types(&self) -> Option<&crate::segments::ExtendedTypeBox> {
        self.extended_type.as_ref()
    }

    /// The original file type (`otyp`, §8.16.5), once the walk has passed one.
    #[must_use]
    pub fn original_type(&self) -> Option<&crate::segments::OriginalFileTypeBox> {
        self.original_type.as_ref()
    }

    /// Every identified media data box (`imda`, §8.1.4) the walk has passed,
    /// with its identifier and payload byte range.
    #[must_use]
    pub fn identified_media(&self) -> &[IdentifiedMediaData] {
        &self.identified_media
    }

    /// The movie's declared whole duration (`mehd.fragment_duration`,
    /// §8.8.2), in the movie timescale, once `moov` is parsed.
    #[must_use]
    pub fn fragment_duration(&self) -> Option<u64> {
        self.movie
            .as_ref()?
            .mvex
            .as_ref()?
            .mehd
            .as_ref()
            .map(crate::init_segment::MovieExtendsHeaderBox::fragment_duration)
    }

    /// The file's `ftyp` brand box, when the file carries one.
    ///
    /// A QuickTime `.mov` whose major brand is `qt  ` and a brandless file both
    /// read completely; this is how a caller tells them from an ISO file.
    #[must_use]
    pub fn brand(&self) -> Option<&FileTypeBox> {
        self.brand.as_ref()
    }

    /// The progressive sample plan, once `moov` is parsed.
    #[must_use]
    pub fn plan(&self) -> Option<SamplePlan<'_>> {
        self.movie.as_ref().map(|movie| {
            SamplePlan::new(
                movie,
                &self.tracks,
                self.config.total_len,
                &self.top_level,
                &self.progressive_iv_sizes,
            )
        })
    }

    /// The movie timescale (`mvhd.timescale`), once `moov` is parsed.
    #[must_use]
    pub fn movie_timescale(&self) -> Option<Timescale> {
        self.movie.as_ref().map(|movie| movie.mvhd.timescale)
    }

    /// The forms of the index boxes around the fragments: the `sidx` that
    /// precedes them and the `mfra` that follows, as the source states them.
    ///
    /// The walk reaches both, so this is complete once [`Step::Done`] arrives.
    /// A source with neither carries the default (no index).
    #[must_use]
    pub fn fragment_index(&self) -> &crate::frag_layout::FragmentIndex {
        &self.index
    }

    /// Whether the movie is fragmented (`moov` carries `mvex`), once `moov` is
    /// parsed. `None` before the track table is known.
    #[must_use]
    pub fn is_fragmented(&self) -> Option<bool> {
        self.movie.as_ref().map(|_| self.fragmented)
    }
}

impl Reader {
    fn warn(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// Turn the tolerant walk's notes into the warnings a permissive read
    /// reports.
    fn report_notes(&mut self, notes: Vec<crate::box_support::Note>) {
        if !self.tolerated() {
            return;
        }
        for note in notes {
            self.warn(match note {
                crate::box_support::Note::Trailing {
                    box_type,
                    offset,
                    carried,
                } => Warning::TrailingBytes {
                    box_type,
                    offset,
                    carried: u64::try_from(carried).unwrap_or(u64::MAX),
                },
                crate::box_support::Note::Unrepresentable {
                    box_type,
                    offset,
                    reason,
                } => Warning::UnrepresentableBox {
                    box_type,
                    offset,
                    reason,
                },
            });
        }
    }

    /// Report a truncated final `mdat` once per file: the box header and a
    /// sample that runs past the input describe the same fault.
    fn warn_truncated_mdat(&mut self, offset: u64) {
        if self.mdat_truncation_reported {
            return;
        }
        self.mdat_truncation_reported = true;
        self.warn(Warning::TruncatedMdat { offset });
    }

    fn finish_step(&mut self) -> Result<Step<'_>> {
        for carried in core::mem::take(&mut self.between) {
            self.top_level.push(TopLevelBox {
                position: BoxPosition::AfterMdat,
                data: carried,
            });
        }
        self.done = true;
        if let Some(warning) = self.next_warning() {
            return Ok(Step::Warning(warning));
        }
        Ok(Step::Done)
    }

    fn take_warning(&mut self) -> Option<Warning> {
        if self.warnings.is_empty() {
            return None;
        }
        Some(self.warnings.remove(0))
    }

    fn next_warning(&mut self) -> Option<Warning> {
        if self.ready.is_some() {
            return None;
        }
        self.deferred_warning.take().or_else(|| self.take_warning())
    }

    fn tolerated(&self) -> bool {
        self.config.mode == Mode::Permissive
    }

    fn seek_fragmented(&mut self, track: TrackId, decode_time: u64) -> Result<()> {
        if let Some(offset) = self.indexed_moof(track, decode_time) {
            return self.reposition(offset);
        }
        // A `sidx` that precedes the fragments has not necessarily been walked
        // past yet; scan for one before falling back to the trailing index.
        self.seek = Some(SeekRequest {
            track,
            decode_time,
            phase: SeekPhase::Scanning,
        });
        self.done = false;
        Ok(())
    }

    /// The `moof` offset an already-known index names for `track` at
    /// `decode_time`, when one of them covers the track.
    fn indexed_moof(&self, track: TrackId, decode_time: u64) -> Option<u64> {
        for index in &self.indexes {
            if index.index.reference_id != track.value() {
                continue;
            }
            if let Some(offset) = moof_in_sidx(index, decode_time) {
                return Some(offset);
            }
        }
        None
    }

    fn seek_phase_scanning(&self) -> bool {
        self.seek
            .is_some_and(|request| matches!(request.phase, SeekPhase::Scanning))
    }

    /// Fall back to the trailing `mfra`: needs the file length and a source
    /// that can read backwards.
    fn seek_via_mfra(&mut self, request: SeekRequest) -> Result<()> {
        let total = self.config.total_len.ok_or(Error::NoIndex {
            track: request.track,
        })?;
        if self.config.source != Source::Seekable || total < MFRO_BOX_LEN {
            return Err(Error::NoIndex {
                track: request.track,
            });
        }
        let range = ByteRange {
            offset: total - MFRO_BOX_LEN,
            len: MFRO_BOX_LEN,
        };
        self.pending = Some(Pending::MfraTrailer(range));
        self.seek = Some(SeekRequest {
            phase: SeekPhase::Trailer,
            ..request
        });
        Ok(())
    }

    /// Move the walk to `offset`, dropping whatever was pending.
    fn reposition(&mut self, offset: u64) -> Result<()> {
        if self.config.source == Source::ForwardOnly && offset < self.pos {
            return Err(Error::InvalidInput(
                "a backward seek needs a seekable source",
            ));
        }
        self.pos = offset;
        self.highest_requested = offset;
        self.pending = None;
        self.fragments.clear();
        self.ready = None;
        self.last_fragment = None;
        self.deferred_warning = None;
        // Progressive samples of a hybrid file are not re-read; the seek
        // positions the fragment walk.
        self.cursors.clear();
        self.seek = None;
        self.done = false;
        Ok(())
    }

    fn accept_subsegment(&mut self, _range: ByteRange, bytes: &[u8]) -> Result<()> {
        self.subsegment_index = Some(crate::index_boxes::SubsegmentIndexBox::parse(bytes)?);
        Ok(())
    }

    fn accept_index(&mut self, range: ByteRange, bytes: &[u8]) -> Result<()> {
        let index = SegmentIndexBox::parse(bytes)?;
        if self.tolerated() && !index.trailing().is_empty() {
            self.warn(Warning::TrailingBytes {
                box_type: *b"sidx",
                offset: range.offset,
                carried: u64::try_from(index.trailing().len()).unwrap_or(u64::MAX),
            });
        }
        if self.index.sidx.is_none() {
            self.index.sidx = Some(crate::frag_layout::SegmentIndexForm {
                version: index.version,
                flags: index.flags,
                reference_id: TrackId::new(index.reference_id),
                timescale: index.timescale,
                earliest_presentation_time: index.earliest_presentation_time,
                reference_type: index.references.first().map_or(0, |r| r.reference_type),
            });
        }
        if self.indexes.len() < MAX_REMEMBERED_INDEXES {
            self.indexes.push(SegmentIndex {
                index,
                anchor: range.end(),
            });
        }
        if self.seek_phase_scanning() {
            let Some(request) = self.seek else {
                return Ok(());
            };
            if let Some(offset) = self.indexed_moof(request.track, request.decode_time) {
                return self.reposition(offset);
            }
            // Another track's index (or none of them reaches the time): keep
            // scanning.
            self.seek = Some(request);
        }
        Ok(())
    }

    fn accept_mfra_trailer(&mut self, bytes: &[u8]) -> Result<()> {
        let Some(request) = self.seek.take() else {
            return Err(Error::InvalidInput("mfra trailer with no seek pending"));
        };
        let total = self.config.total_len.ok_or(Error::NoIndex {
            track: request.track,
        })?;
        let Ok(offset_box) = MovieFragmentRandomAccessOffsetBox::parse(bytes) else {
            return Err(Error::NoIndex {
                track: request.track,
            });
        };
        let size = u64::from(offset_box.parent_size);
        if size < MFRO_BOX_LEN || size > total {
            return Err(Error::NoIndex {
                track: request.track,
            });
        }
        let range = ByteRange {
            offset: total - size,
            len: size,
        };
        self.request(range.offset, range.len)?;
        self.pending = Some(Pending::Mfra(range));
        self.seek = Some(SeekRequest {
            phase: SeekPhase::Mfra,
            ..request
        });
        Ok(())
    }

    /// Record the trailing `mfra`'s form so a rewrite can regenerate it from
    /// its own positions. A box this crate cannot read states no form, exactly
    /// as it was ignored before.
    fn accept_mfra_form(&mut self, range: ByteRange, bytes: &[u8]) -> Result<()> {
        let mut notes = Vec::new();
        let parsed = {
            let mut sink =
                crate::box_support::Notes::new(bytes, range.offset, self.tolerated(), &mut notes);
            crate::box_types::parse_box(bytes).and_then(|(header, _)| {
                MovieFragmentRandomAccessBox::parse_body_notes(header.body, Some(&mut sink))
            })
        };
        let Ok(mfra) = parsed else {
            return Ok(());
        };
        self.report_notes(notes);
        let mut tracks = Vec::with_capacity(mfra.tfra.len());
        for tfra in &mfra.tfra {
            tracks.push(crate::frag_layout::MfraTrackForm {
                track_id: tfra.track_id,
                version: tfra.version,
                lengths: crate::frag_layout::TfraLengthSizes {
                    traf: tfra.length_size_of_traf_num,
                    trun: tfra.length_size_of_trun_num,
                    sample: tfra.length_size_of_sample_num,
                },
            });
        }
        self.index.mfra = Some(crate::frag_layout::MfraForm { tracks });
        Ok(())
    }

    fn accept_mfra(&mut self, range: ByteRange, bytes: &[u8]) -> Result<()> {
        let Some(request) = self.seek.take() else {
            return Err(Error::InvalidInput("mfra with no seek pending"));
        };
        let mut notes = Vec::new();
        let mfra = {
            let mut sink =
                crate::box_support::Notes::new(bytes, range.offset, self.tolerated(), &mut notes);
            crate::box_types::parse_box(bytes).and_then(|(header, _)| {
                MovieFragmentRandomAccessBox::parse_body_notes(header.body, Some(&mut sink))
            })
        }?;
        self.report_notes(notes);
        let offset = mfra
            .tfra
            .iter()
            .find(|tfra| tfra.track_id == request.track)
            .and_then(|tfra| moof_in_tfra(tfra, request.decode_time))
            .ok_or(Error::NoIndex {
                track: request.track,
            })?;
        self.reposition(offset)
    }
}

/// Whether a top-level box is carried for a rewrite. A trailing `mfra` is not:
/// its `tfra` entries address the source's fragments, so carrying it into a
/// rewritten file would leave a stale index (the writer does not rebuild one).
/// A `styp`/`emsg` belongs to the fragment run it precedes and is emitted by
/// the caller that writes that run.
fn carried_top_level(fourcc: &[u8; BOX_TYPE_SIZE]) -> bool {
    fourcc != b"mfra" && fourcc != b"styp" && fourcc != b"emsg"
}

/// Whether an opaque `stsd` holds an entry with a body, which means a known
/// four-CC failed to parse. A header-only entry is a caller's raw sample entry
/// and stays legal in both modes.
fn opaque_stsd_has_body(stsd: &[u8]) -> bool {
    let Some(body) = stsd.get(BOX_HEADER_MIN_SIZE..) else {
        return false;
    };
    if body.len() < FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() {
        return false;
    }
    let count = broadcast_core::wire::u32_at(body, FULLBOX_EXTRA_SIZE);
    let mut at = FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
    for _ in 0..count {
        let Some(header) = body.get(at..at + BOX_HEADER_MIN_SIZE) else {
            return false;
        };
        let size = usize::try_from(broadcast_core::wire::u32_at(header, 0)).unwrap_or(0);
        if size > BOX_HEADER_MIN_SIZE {
            return true;
        }
        if size < BOX_HEADER_MIN_SIZE {
            return false;
        }
        at = at.saturating_add(size);
    }
    false
}

/// Whether a fragment parse failure is one the permissive mode tolerates.
///
/// A `LimitExceeded` is never tolerated: the caps bind both modes.
fn tolerable_fragment_error(error: &Error) -> bool {
    matches!(error, Error::BufferTooShort(_) | Error::InvalidInput(_))
}

/// The `moof` a `sidx` names for `decode_time` (§8.14.3).
///
/// Reference offsets start at the box's `first_offset` past the byte following
/// the `sidx`, and each later reference follows the previous one's
/// `referenced_size`. A nested index (`reference_type == 1`) is not a `moof`, so
/// it cannot be positioned on and yields `None`.
fn moof_in_sidx(index: &SegmentIndex, decode_time: u64) -> Option<u64> {
    let mut start = index.index.earliest_presentation_time;
    let mut offset = index.anchor.checked_add(index.index.first_offset)?;
    let mut last = None;
    for reference in &index.index.references {
        if reference.reference_type != 0 {
            return None;
        }
        let end = start.saturating_add(u64::from(reference.subsegment_duration));
        last = Some(offset);
        if decode_time < end {
            return Some(offset);
        }
        start = end;
        offset = offset.checked_add(u64::from(reference.referenced_size))?;
    }
    last
}

/// The `moof` a `tfra` names for `decode_time`: the last listed sync sample at
/// or before it, or the first when the time precedes every entry (§8.8.10).
fn moof_in_tfra(
    tfra: &crate::timing::TrackFragmentRandomAccessBox,
    decode_time: u64,
) -> Option<u64> {
    let mut chosen = None;
    for entry in &tfra.entries {
        if entry.time <= decode_time {
            chosen = Some(entry.moof_offset);
        } else {
            break;
        }
    }
    chosen.or_else(|| tfra.entries.first().map(|entry| entry.moof_offset))
}

impl Reader {
    /// The next step.
    ///
    /// Named `step`, not `next`: each step borrows the reader (the
    /// [`Step::Tracks`] arm hands out its own track table), which
    /// [`Iterator::next`]'s signature cannot express.
    pub fn step(&mut self) -> Result<Step<'_>> {
        if let Some(warning) = self.next_warning() {
            return Ok(Step::Warning(warning));
        }
        if let Some(pending) = self.pending {
            return Ok(Step::Need(pending.range()));
        }
        if self.done {
            return Ok(Step::Done);
        }
        if self.seek_phase_scanning() {
            return self.walk_step();
        }
        // A queued fragment whose `mdat` has arrived resolves one at a time,
        // in file order, before the walk reads the next header: a forward-only
        // source has not yet passed a payload it can still address.
        if self.ready.is_none()
            && self
                .fragments
                .front()
                .is_some_and(|fragment| fragment.moof.is_some() && !fragment.mdats.is_empty())
        {
            self.finish_fragment(Unresolved::Keep)?;
        }
        if self.ready.is_some()
            && let Some(step) = self.next_fragment_step()
        {
            return Ok(match step {
                FragmentStep::Fragment(info) => Step::Fragment(info),
                FragmentStep::Sample(sample) => Step::Sample(sample),
            });
        }
        if self.movie.is_some() {
            if !self.emitted_tracks {
                self.emitted_tracks = true;
                return Ok(Step::Tracks(&self.tracks));
            }
            if !self.emitted_metadata {
                self.emitted_metadata = true;
                if self.metadata.is_some() {
                    return self.metadata_step();
                }
            }
            if let Some(sample) = self.next_sample()? {
                return Ok(Step::Sample(sample));
            }
            if !self.fragmented && self.config.source == Source::ForwardOnly {
                // A forward-only caller has already streamed past the sample
                // data, so the boxes after it cannot be re-read; finish here
                // rather than demand a range behind the source's position.
                return self.finish_step();
            }
        }
        self.walk_step()
    }

    fn metadata_step(&self) -> Result<Step<'_>> {
        match self.metadata.as_ref() {
            Some(metadata) => Ok(Step::Metadata(metadata)),
            None => Err(Error::InvalidInput("metadata step without metadata")),
        }
    }

    /// Read the next top-level box header, or finish at the end of the input.
    fn walk_step(&mut self) -> Result<Step<'_>> {
        match self.next_header_range()? {
            Some(range) => {
                self.pending = Some(Pending::Header(range));
                Ok(Step::Need(range))
            }
            None => {
                if let Some(request) = self
                    .seek
                    .filter(|request| matches!(request.phase, SeekPhase::Scanning))
                {
                    self.seek_via_mfra(request)?;
                    if let Some(pending) = self.pending {
                        return Ok(Step::Need(pending.range()));
                    }
                }
                while !self.fragments.is_empty() {
                    self.finish_fragment(Unresolved::Fail)?;
                    if let Some(step) = self.next_fragment_step() {
                        return Ok(match step {
                            FragmentStep::Fragment(info) => Step::Fragment(info),
                            FragmentStep::Sample(sample) => Step::Sample(sample),
                        });
                    }
                }
                self.finish_step()
            }
        }
    }
}

impl Reader {
    fn next_header_range(&mut self) -> Result<Option<ByteRange>> {
        let len = match self.config.total_len {
            Some(total) if self.pos >= total => return Ok(None),
            Some(total) => (total - self.pos).min(MIN_BOX_HEADER_BYTES),
            None => MIN_BOX_HEADER_BYTES,
        };
        self.request(self.pos, len).map(Some)
    }

    fn request(&mut self, offset: u64, len: u64) -> Result<ByteRange> {
        if self.config.source == Source::ForwardOnly && offset < self.highest_requested {
            return Err(Error::MoovAfterMdatForwardOnly);
        }
        let range = ByteRange { offset, len };
        self.highest_requested = self.highest_requested.max(offset);
        Ok(range)
    }

    fn accept_header(&mut self, range: ByteRange, bytes: &[u8]) -> Result<()> {
        let offset = range.offset;
        if bytes.len() < BOX_HEADER_MIN_SIZE {
            if bytes.is_empty() && self.at_end(offset) {
                self.done = true;
                return Ok(());
            }
            return Err(Error::BufferTooShort(
                broadcast_core::error::BufferTooShort {
                    need: BOX_HEADER_MIN_SIZE,
                    have: bytes.len(),
                    what: "top-level box header",
                },
            ));
        }
        let raw_size = broadcast_core::wire::u32_at(bytes, 0);
        let uuid = &bytes[4..8] == b"uuid";
        let largesize = raw_size == crate::box_types::SIZE_INDICATES_LARGESIZE;
        let required = MIN_BOX_HEADER_BYTES
            .saturating_add(if largesize {
                crate::box_types::LARGESIZE_SIZE as u64
            } else {
                0
            })
            .saturating_add(if uuid {
                crate::box_types::UUID_TYPE_SIZE as u64
            } else {
                0
            })
            .min(MAX_BOX_HEADER_BYTES);
        let supplied = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if supplied < required && range.len < required {
            let len = match self.config.total_len {
                Some(total) => total.saturating_sub(offset).min(required),
                None => required,
            };
            if len > range.len {
                self.pending = Some(Pending::Header(ByteRange { offset, len }));
                return Ok(());
            }
        }
        let header = BoxHeader::parse(bytes)?;
        let mut size = if header.size == 0 {
            let total = self.config.total_len.ok_or(Error::InvalidInput(
                "a size-0 box needs the total input length",
            ))?;
            total.checked_sub(offset).ok_or(Error::InvalidInput(
                "a size-0 box starts past the input end",
            ))?
        } else {
            header.size
        };
        let mut end = offset
            .checked_add(size)
            .ok_or(Error::InvalidInput("top-level box end overflow"))?;
        if let Some(total) = self.config.total_len
            && end > total
        {
            let truncatable = header.box_type.is(b"mdat") || header.box_type.is(b"moof");
            if self.tolerated() && truncatable {
                end = total;
                size = end.saturating_sub(offset);
                if header.box_type.is(b"mdat") {
                    if !self.fragments.is_empty() {
                        self.warn(Warning::TruncatedFragment { offset });
                    } else {
                        self.warn_truncated_mdat(offset);
                    }
                } else {
                    self.warn(Warning::TruncatedFragment { offset });
                    self.done = true;
                }
            } else {
                return Err(Error::BufferTooShort(
                    broadcast_core::error::BufferTooShort {
                        need: usize::try_from(size).unwrap_or(usize::MAX),
                        have: usize::try_from(total.saturating_sub(offset)).unwrap_or(usize::MAX),
                        what: "top-level box body",
                    },
                ));
            }
        }
        self.open_box = Some(ByteRange { offset, len: size });
        if header.box_type.is(b"ftyp") {
            if size > self.config.limits.max_moov_bytes() {
                return Err(Error::LimitExceeded {
                    what: "ftyp size",
                    limit: self.config.limits.max_moov_bytes(),
                });
            }
            let range = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pending = Some(Pending::Ftyp(range));
            self.pos = end;
            return Ok(());
        }
        if header.box_type.is(b"moov") {
            if self.config.source == Source::ForwardOnly && self.saw_mdat {
                return Err(Error::MoovAfterMdatForwardOnly);
            }
            if size > self.config.limits.max_moov_bytes() {
                return Err(Error::LimitExceeded {
                    what: "moov size",
                    limit: self.config.limits.max_moov_bytes(),
                });
            }
            let range = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pending = Some(Pending::Moov(range));
            self.pos = end;
            return Ok(());
        }
        if header.box_type.is(b"ssix") {
            if size > self.config.limits.max_moov_bytes() {
                return Err(Error::LimitExceeded {
                    what: "subsegment index size",
                    limit: self.config.limits.max_moov_bytes(),
                });
            }
            let index = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pending = Some(Pending::Subsegment(index));
            self.pos = end;
            return Ok(());
        }
        if header.box_type.is(b"sidx") {
            if size > self.config.limits.max_moov_bytes() {
                return Err(Error::LimitExceeded {
                    what: "index size",
                    limit: self.config.limits.max_moov_bytes(),
                });
            }
            let index = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pending = Some(Pending::Index(index));
            self.pos = end;
            return Ok(());
        }
        if header.box_type.is(b"moof") {
            if let Some(request) = self
                .seek
                .filter(|request| matches!(request.phase, SeekPhase::Scanning))
            {
                // No `sidx` precedes the fragments: nothing is consumed here,
                // so the walk reads this header again once it has a position.
                self.pos = offset;
                return self.seek_via_mfra(request);
            }
            // A `moof` ends only a queued fragment whose `mdat` has already
            // arrived; a run of `moof`s ahead of their shared `mdat` (the CMF2
            // shape, one `moof` per track, §8.8.4) stays queued.
            while self
                .fragments
                .front()
                .is_some_and(|fragment| !fragment.mdats.is_empty())
            {
                self.finish_fragment(Unresolved::Fail)?;
            }
            let limit = self.config.limits.max_fragment_bytes();
            let held: u64 = self.fragments.iter().fold(0u64, |total, fragment| {
                total.saturating_add(fragment.range.len)
            });
            if held.saturating_add(size) > limit {
                return Err(Error::LimitExceeded {
                    what: "fragment size",
                    limit,
                });
            }
            let fragment = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pending = Some(Pending::Moof(fragment));
            self.fragments.push_back(FragmentState {
                offset,
                range: fragment,
                moof: None,
                mdats: Vec::new(),
                before: core::mem::take(&mut self.between),
                layout: None,
            });
            self.pos = end;
            return Ok(());
        }
        if header.box_type.is(b"mfra") && size <= self.config.limits.max_metadata_item_bytes() {
            let range = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pos = end;
            self.pending = Some(Pending::MfraForm(range));
            return Ok(());
        }
        if header.box_type.is(b"mdat") {
            self.saw_mdat = true;
            let payload = offset.saturating_add(header.header_size() as u64);
            for fragment in &mut self.fragments {
                fragment.mdats.push(MdatRange::new(payload, end));
            }
        } else if header.box_type.is(&crate::segments::IMDA) {
            self.saw_mdat = true;
            let payload = offset.saturating_add(
                (header.header_size() + crate::segments::IMDA_IDENTIFIER_LEN) as u64,
            );
            for fragment in &mut self.fragments {
                fragment.mdats.push(MdatRange::new(payload, end));
            }
            let head = ByteRange {
                offset,
                len: ((header.header_size() + crate::segments::IMDA_IDENTIFIER_LEN) as u64)
                    .min(size),
            };
            self.request(offset, head.len)?;
            self.pos = end;
            self.pending = Some(Pending::Imda {
                head,
                payload_end: end,
            });
            return Ok(());
        } else if self
            .fragments
            .front()
            .is_some_and(|fragment| !fragment.mdats.is_empty())
            && !header.box_type.is(b"free")
        {
            // A box other than `mdat`/`free` after a fragment's media ends that
            // fragment; nothing is consumed here, so the walk reads this header
            // again as the next step's first box.
            self.finish_fragment(Unresolved::Fail)?;
            self.pos = offset;
            return Ok(());
        }
        self.pos = end;
        if !header.box_type.is(b"mdat")
            && size <= self.config.limits.max_metadata_item_bytes()
            && carried_top_level(&header.box_type.0)
        {
            let range = ByteRange { offset, len: size };
            self.pending = Some(Pending::TopLevel(range));
        }
        Ok(())
    }

    /// Where a top-level box the writer does not model sits, relative to the
    /// boxes the writer emits.
    fn top_level_position(&self) -> BoxPosition {
        if self.saw_mdat {
            BoxPosition::AfterMdat
        } else if self.movie.is_some() {
            BoxPosition::AfterMoov
        } else {
            BoxPosition::AfterFtyp
        }
    }

    /// Keep one top-level box outside `ftyp`/`moov`/`mdat` so a rewrite can
    /// reproduce it in place.
    fn accept_imda(&mut self, head_offset: u64, payload_end: u64, bytes: &[u8]) -> Result<()> {
        let (identifier, payload_offset) =
            crate::segments::IdentifiedMediaDataBox::parse_identification(bytes)?;
        let start = head_offset.saturating_add(payload_offset as u64);
        self.identified_media.push(IdentifiedMediaData {
            identifier,
            payload: ByteRange {
                offset: start,
                len: payload_end.saturating_sub(start),
            },
        });
        Ok(())
    }

    fn accept_top_level(&mut self, bytes: &[u8]) -> Result<()> {
        let (item, _) = parse_box(bytes)?;
        match &item.four_cc() {
            b"etyp" => {
                self.extended_type = Some(crate::segments::ExtendedTypeBox::parse(bytes)?);
            }
            b"otyp" => {
                self.original_type = Some(crate::segments::OriginalFileTypeBox::parse(bytes)?);
            }
            _ => {}
        }
        let carried = OpaqueBox {
            box_type: item.four_cc(),
            data: item.payload().to_vec(),
            to_end: item.to_end(),
            largesize: item.has_largesize(),
        };
        if self.saw_mdat {
            self.between.push(carried);
        } else {
            let position = self.top_level_position();
            self.top_level.push(TopLevelBox {
                position,
                data: carried,
            });
        }
        Ok(())
    }

    fn accept_ftyp(&mut self, bytes: &[u8]) -> Result<()> {
        self.brand = Some(FileTypeBox::parse_box(bytes)?);
        Ok(())
    }

    fn accept_moof(&mut self, bytes: &[u8]) -> Result<()> {
        let tolerate = self.tolerated();
        let mut notes = Vec::new();
        let offset = match self.fragments.back() {
            Some(fragment) => fragment.range.offset,
            None => {
                return Err(Error::InvalidInput(
                    "moof supplied with no fragment pending",
                ));
            }
        };
        let parsed = {
            let mut sink = crate::box_support::Notes::new(bytes, offset, tolerate, &mut notes);
            parse_box(bytes).and_then(|(header, _)| {
                let mut zero_width_budget = MAX_ZERO_WIDTH_TRUN_SAMPLES;
                MovieFragmentBox::parse_body_budgeted_notes(
                    header.body,
                    &mut zero_width_budget,
                    Some(&mut sink),
                )
            })
        };
        self.report_notes(core::mem::take(&mut notes));
        match parsed {
            Ok(mut moof) => {
                let Some(fragment) = self.fragments.back_mut() else {
                    return Err(Error::InvalidInput(
                        "moof supplied with no fragment pending",
                    ));
                };
                // The layout is the moof's *wire* state: the `trex` defaults
                // below are filled into the `tfhd` so the samples resolve, and
                // are not fields the source wrote.
                let mut layout = crate::frag_layout::FragmentLayout::from_moof(&moof, offset)?;
                layout.before = core::mem::take(&mut fragment.before);
                fragment.layout = Some(layout);
                apply_trex_defaults(&mut moof, &self.trex);
                fragment.moof = Some(moof);
                Ok(())
            }
            Err(err) if tolerate && tolerable_fragment_error(&err) => {
                self.fragments.pop_back();
                let truncated = self.config.total_len.is_some_and(|total| self.pos >= total);
                if !self.done {
                    self.warn(if truncated {
                        Warning::TruncatedFragment { offset }
                    } else {
                        Warning::MalformedFragment { offset }
                    });
                }
                if truncated {
                    self.done = true;
                }
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    fn accept_moov(&mut self, range: ByteRange, bytes: &[u8]) -> Result<()> {
        let mut owned = Vec::new();
        let source: &[u8] = if self.tolerated() {
            owned.extend_from_slice(bytes);
            sanitize_moov(&mut owned, range.offset, &mut self.warnings);
            &owned
        } else {
            bytes
        };
        let mut notes = Vec::new();
        let movie = {
            let mut sink =
                crate::box_support::Notes::new(source, range.offset, self.tolerated(), &mut notes);
            MovieBox::parse_notes(source, Some(&mut sink), &self.config.limits)
        }?;
        self.report_notes(core::mem::take(&mut notes));
        let mut tracks = Vec::with_capacity(movie.tracks.len());
        let mut cursors = Vec::new();
        let mut crypto = Vec::with_capacity(movie.tracks.len());
        let mut progressive_iv_sizes = Vec::with_capacity(movie.tracks.len());
        let mut decode_times = Vec::with_capacity(movie.tracks.len());
        let mut sample_counts = Vec::with_capacity(movie.tracks.len());
        for (index, trak) in movie.tracks.iter().enumerate() {
            let raw = raw_stsd_entries(source, index, &self.config.limits, range.offset)?;
            self.check_opaque_stsd(trak, &raw)?;
            self.check_wave_esds(trak, &raw)?;
            let track_crypto = track_crypto(trak, &raw)?;
            let info = build_track_info(trak, raw, self.config.registry.as_ref())?;
            let mut declared_samples = 0u32;
            if let Some(stbl) = sample_table(trak) {
                let iv_size = track_crypto
                    .as_ref()
                    .map(|crypto| crypto.tenc.default_per_sample_iv_size);
                let state = CursorState::new_with_aux(
                    stbl,
                    info.track_id,
                    &self.config.limits,
                    self.config.total_len,
                    iv_size,
                )?;
                declared_samples = state.remaining();
                cursors.push(CursorSlot {
                    track_index: index,
                    state,
                });
            }
            progressive_iv_sizes.push(
                track_crypto
                    .as_ref()
                    .map(|crypto| crypto.tenc.default_per_sample_iv_size),
            );
            crypto.push(track_crypto);
            decode_times.push(0);
            sample_counts.push(declared_samples);
            tracks.push(info);
        }
        self.fragmented = movie.mvex.is_some();
        self.trex = movie
            .mvex
            .as_ref()
            .map(|mvex| mvex.trex.clone())
            .unwrap_or_default();
        self.level_assignment = movie.mvex.as_ref().and_then(|mvex| mvex.leva.clone());
        let mut movie_metadata = match self.movie_metadata(&movie) {
            Ok(metadata) => metadata,
            // The parse-time notes already named the atom that would not
            // parse; losing the metadata view is all that costs.
            Err(_) if self.tolerated() => None,
            Err(err) => return Err(err),
        };
        if let Some(metadata) = movie_metadata.take() {
            self.metadata = Some(MovieMetadata {
                movie: metadata,
                tracks: Vec::new(),
            });
        }
        for trak in &movie.tracks {
            let metadata = match trak.metadata(&self.config.limits) {
                Ok(metadata) => metadata,
                Err(_) if self.tolerated() => None,
                Err(err) => return Err(err),
            };
            if let Some(metadata) = metadata {
                self.metadata
                    .get_or_insert_with(MovieMetadata::default)
                    .tracks
                    .push((trak.tkhd.track_id, metadata));
            }
        }
        if let Some(metadata) = self.metadata.as_ref() {
            apply_itunsmpb(&mut tracks, metadata);
        }
        self.emitted_metadata = false;
        self.sample_references = movie
            .tracks
            .iter()
            .zip(&tracks)
            .map(|(trak, info)| track_data_references(trak, &info.entries))
            .collect();
        self.movie = Some(movie);
        self.tracks = tracks;
        self.cursors = cursors;
        self.crypto = crypto;
        self.progressive_iv_sizes = progressive_iv_sizes;
        self.decode_times = decode_times;
        self.sample_counts = sample_counts;
        self.emitted_tracks = false;
        Ok(())
    }

    fn movie_metadata(&self, movie: &MovieBox) -> Result<Option<crate::metadata::Metadata>> {
        movie.metadata(&self.config.limits)
    }

    /// Resolve the pending fragment's samples into [`Reader::ready`].
    ///
    /// A fragment with no `moof` yet (its body was never supplied because the
    /// input ended inside it) leaves [`Reader::ready`] empty. When a run does
    /// not resolve inside the `mdat` payloads located so far, `unresolved`
    /// decides whether that is the end of the fragment ([`Unresolved::Fail`])
    /// or a fragment still waiting for a later `mdat` ([`Unresolved::Keep`]).
    fn finish_fragment(&mut self, unresolved: Unresolved) -> Result<()> {
        let Some(state) = self.fragments.pop_front() else {
            return Ok(());
        };
        let Some(moof) = state.moof.as_ref() else {
            return Ok(());
        };
        let at_end = self.config.total_len.is_some_and(|total| self.pos >= total);
        let partial = unresolved == Unresolved::Fail && at_end && self.tolerated();
        let resolved = match self.resolve_fragment(&state, moof, partial) {
            Ok(resolved) => resolved,
            Err(err) => {
                if unresolved == Unresolved::Keep {
                    self.fragments.push_front(state);
                    return Ok(());
                }
                if self.tolerated() && tolerable_fragment_error(&err) {
                    let truncated = self.config.total_len.is_some_and(|total| self.pos >= total);
                    let already = self.warnings.iter().any(|warning| {
                        matches!(warning, Warning::TruncatedFragment { offset } if *offset == state.offset)
                    });
                    if !already {
                        self.warn(if truncated {
                            Warning::TruncatedFragment {
                                offset: state.offset,
                            }
                        } else {
                            Warning::MalformedFragment {
                                offset: state.offset,
                            }
                        });
                    }
                    if truncated {
                        self.done = true;
                    }
                    return Ok(());
                }
                return Err(err);
            }
        };
        if resolved.truncated {
            self.deferred_warning = Some(Warning::TruncatedFragment {
                offset: state.offset,
            });
        }
        for (index, decode_time, count) in resolved.cursors {
            self.decode_times[index] = decode_time;
            self.sample_counts[index] = count;
        }
        self.last_fragment = None;
        self.ready = Some(ReadyFragment {
            info: resolved.info,
            samples: resolved.samples,
            next: 0,
            emitted: false,
        });
        Ok(())
    }

    /// One fragment's samples, addressed through [`sample_ranges_in_run`]
    /// against the `mdat` payloads located so far, in the configured order.
    ///
    /// A run whose data a later `mdat` may hold is the resolver's own
    /// `InvalidInput`, which [`Reader::finish_fragment`] decides whether to
    /// treat as "not yet".
    fn resolve_fragment(
        &self,
        state: &FragmentState,
        moof: &MovieFragmentBox,
        partial: bool,
    ) -> Result<ResolvedFragment> {
        // The fragment's own media: a sample must live inside one of these
        // payloads, so non-overlapping samples cannot total more than they do.
        let budget = state.mdats.iter().fold(0u64, |total, range| {
            total.saturating_add(range.end().saturating_sub(range.start()))
        });
        let mut copied_bytes = 0u64;
        let mut rows: Vec<(usize, SampleRef)> = Vec::new();
        let mut truncated = false;
        let mut present: Vec<TrackId> = Vec::new();
        let mut cursors: Vec<(usize, u64, u32)> = Vec::new();
        for traf in &moof.traf {
            let track_id = traf.tfhd.track_id;
            let Some(index) = self
                .tracks
                .iter()
                .position(|info| info.track_id == track_id)
            else {
                continue;
            };
            present.push(track_id);
            let base = match &traf.tfdt {
                Some(tfdt) => tfdt.base_media_decode_time(),
                None => cursors
                    .iter()
                    .find(|slot| slot.0 == index)
                    .map_or(self.decode_times[index], |slot| slot.1),
            };
            let (ranges, cut) = if partial {
                sample_ranges_in_run_until(
                    state.offset,
                    moof,
                    track_id,
                    &state.mdats,
                    budget,
                    &mut copied_bytes,
                )?
            } else {
                (
                    sample_ranges_in_run(
                        state.offset,
                        moof,
                        track_id,
                        &state.mdats,
                        budget,
                        &mut copied_bytes,
                    )?,
                    false,
                )
            };
            if cut {
                truncated = true;
            }
            let auxes = fragment_aux(traf, self.crypto[index].as_ref(), ranges.len())?;
            let description_index = traf
                .tfhd
                .sample_description_index
                .or_else(|| {
                    self.trex
                        .iter()
                        .find(|trex| trex.track_id == track_id)
                        .map(|trex| trex.default_sample_description_index)
                })
                .unwrap_or(DEFAULT_DESCRIPTION_INDEX);
            let mut decode_time = base;
            let mut count = cursors
                .iter()
                .find(|slot| slot.0 == index)
                .map_or(self.sample_counts[index], |slot| slot.2);
            for (range, aux) in ranges.into_iter().zip(auxes) {
                let composition_offset = i32::try_from(range.composition_offset).map_err(|_| {
                    Error::InvalidInput("sample composition offset leaves the signed 32-bit range")
                })?;
                let sample = SampleRef {
                    track: track_id,
                    index: count,
                    decode_time,
                    composition_offset,
                    duration: range.duration,
                    sync: range.flags & SAMPLE_FLAG_IS_NON_SYNC == 0,
                    offset: range.start,
                    size: u32::try_from(range.end.saturating_sub(range.start))
                        .map_err(|_| Error::InvalidInput("sample size leaves the 32-bit range"))?,
                    description_index,
                    aux,
                };
                count = count.saturating_add(1);
                rows.push((index, sample));
                decode_time = decode_time.saturating_add(u64::from(range.duration));
            }
            match cursors.iter_mut().find(|slot| slot.0 == index) {
                Some(slot) => {
                    slot.1 = decode_time;
                    slot.2 = count;
                }
                None => cursors.push((index, decode_time, count)),
            }
            if truncated {
                break;
            }
        }
        let order = self.config.order;
        let tracks = &self.tracks;
        rows.sort_by(|(left_index, left), (right_index, right)| {
            compare_samples(
                order,
                left,
                tracks[*left_index].timescale,
                *left_index,
                right,
                tracks[*right_index].timescale,
                *right_index,
            )
        });
        Ok(ResolvedFragment {
            info: FragmentInfo {
                sequence_number: moof.mfhd.sequence_number,
                moof: state.range,
                tracks: present,
                layout: state.layout.clone().ok_or(Error::InvalidInput(
                    "a resolved fragment carries no recorded layout",
                ))?,
                subsegment_index: self.subsegment_index.clone(),
                level_assignment: self.level_assignment.clone(),
            },
            samples: rows.into_iter().map(|(_, sample)| sample).collect(),
            truncated,
            cursors,
        })
    }

    /// The next step of a resolved fragment, clearing it once its samples are
    /// exhausted.
    fn next_fragment_step(&mut self) -> Option<FragmentStep> {
        {
            let ready = self.ready.as_mut()?;
            if !ready.emitted {
                ready.emitted = true;
                return Some(FragmentStep::Fragment(ready.info.clone()));
            }
            if let Some(sample) = ready.samples.get(ready.next).cloned() {
                ready.next += 1;
                return Some(FragmentStep::Sample(sample));
            }
        }
        let ready = self.ready.take()?;
        self.last_fragment = Some((ready.info.moof.offset, ready.samples));
        None
    }

    fn at_end(&self, offset: u64) -> bool {
        self.config.total_len.is_none_or(|total| offset >= total)
    }

    fn next_sample(&mut self) -> Result<Option<SampleRef>> {
        let movie = self
            .movie
            .as_ref()
            .ok_or(Error::InvalidInput("samples before moov is parsed"))?;
        let order = self.config.order;
        let mut best: Option<(usize, usize, Timescale, SampleRef)> = None;
        for (slot_index, slot) in self.cursors.iter_mut().enumerate() {
            let info = self
                .tracks
                .get(slot.track_index)
                .ok_or(Error::InvalidInput("unknown track"))?;
            let trak = movie
                .tracks
                .get(slot.track_index)
                .ok_or(Error::UnexpectedBox { expected: "trak" })?;
            let stbl = sample_table(trak).ok_or(Error::UnexpectedBox { expected: "stbl" })?;
            let Some(sample) = slot.state.peek(stbl)? else {
                continue;
            };
            let Some(base) = data_reference_bias(
                self.sample_references
                    .get(slot.track_index)
                    .map_or(&[], alloc::vec::Vec::as_slice),
                &self.identified_media,
                sample.description_index,
            ) else {
                // The identified media data box this entry addresses has not
                // been walked yet; a later `Step::Need` pass will find it.
                continue;
            };
            let mut sample = sample;
            sample.offset = sample.offset.saturating_add(base);
            let replace = match &best {
                None => true,
                Some((_, best_index, best_timescale, best_sample)) => {
                    compare_samples(
                        order,
                        &sample,
                        info.timescale,
                        slot.track_index,
                        best_sample,
                        *best_timescale,
                        *best_index,
                    ) == Ordering::Less
                }
            };
            if replace {
                best = Some((slot_index, slot.track_index, info.timescale, sample));
            }
        }
        let Some((slot_index, track_index, _, sample)) = best else {
            return Ok(None);
        };
        let slot = self
            .cursors
            .get_mut(slot_index)
            .ok_or(Error::InvalidInput("unknown cursor"))?;
        let trak = movie
            .tracks
            .get(track_index)
            .ok_or(Error::UnexpectedBox { expected: "trak" })?;
        let stbl = sample_table(trak).ok_or(Error::UnexpectedBox { expected: "stbl" })?;
        if let Err(err) = slot.state.advance(stbl) {
            if self.tolerated() && matches!(err, Error::BufferTooShort(_)) {
                self.warn_truncated_mdat(sample.offset);
                return Ok(None);
            }
            return Err(err);
        }
        Ok(Some(sample))
    }
}

/// Total order over two heads: the configured primary key, ties broken by
/// `moov` track order.
fn compare_samples(
    order: SampleOrder,
    sample: &SampleRef,
    timescale: Timescale,
    track_index: usize,
    other: &SampleRef,
    other_timescale: Timescale,
    other_track_index: usize,
) -> Ordering {
    let primary = match order {
        SampleOrder::DecodeTime => {
            let left = u128::from(sample.decode_time) * u128::from(other_timescale.get());
            let right = u128::from(other.decode_time) * u128::from(timescale.get());
            left.cmp(&right)
        }
        SampleOrder::File => sample.offset.cmp(&other.offset),
    };
    primary.then_with(|| track_index.cmp(&other_track_index))
}

/// One past the sample's last byte.
fn extent(sample: &SampleRef) -> u64 {
    sample.offset.saturating_add(u64::from(sample.size))
}

/// One-based `stsd` entry a sample uses when neither its `tfhd` nor its
/// `trex` names one (ISO/IEC 14496-12:2026 §8.8.7.1).
const DEFAULT_DESCRIPTION_INDEX: u32 = 1;

/// Resolve a track's CENC defaults from the `sinf` of its sample entries.
///
/// A `seig` sample group would override them per run of samples (key rotation),
/// which this crate does not implement; a protected track that carries one is a
/// typed [`Error::UnsupportedFeature`] rather than samples decrypted with the
/// wrong `tenc`.
fn track_crypto(trak: &TrackBox, raw: &[RawEntry]) -> Result<Option<TrackCrypto>> {
    let mut tenc = None;
    for entry in raw {
        let Some(sinf) = sinf_of_sample_entry(&entry.body) else {
            continue;
        };
        let parsed = crate::cenc::ProtectionSchemeInfoBox::parse(sinf)?;
        if let Some(found) = parsed
            .scheme_info
            .as_ref()
            .and_then(|info| info.tenc.clone())
        {
            tenc = Some(found);
            break;
        }
    }
    let Some(tenc) = tenc else {
        return Ok(None);
    };
    if stbl_has_seig_(trak)? {
        return Err(Error::UnsupportedFeature(
            "seig sample-group key rotation not implemented",
        ));
    }
    Ok(Some(TrackCrypto { tenc }))
}

/// Whether a track's `stbl` carries a `seig` sample group (ISO/IEC 23001-7
/// §12.5): key rotation this crate does not implement.
fn stbl_has_seig_(trak: &TrackBox) -> Result<bool> {
    let Some(stbl) = sample_table(trak) else {
        return Ok(false);
    };
    for child in &stbl.children {
        if let StblChild::Opaque(bytes) = child
            && let Ok(sgpd) = SampleGroupDescriptionBox::parse(bytes)
            && sgpd.grouping_type == GROUPING_TYPE_SEIG
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether one `traf` carries a `seig` sample group, the per-fragment form of
/// the same key rotation.
fn traf_has_seig(traf: &TrackFragmentBox) -> Result<bool> {
    for child in &traf.order {
        if let TrafChild::Opaque(opaque) = child
            && opaque.box_type == *b"sgpd"
            && let Ok(sgpd) = SampleGroupDescriptionBox::parse_body(&opaque.data)
            && sgpd.grouping_type == GROUPING_TYPE_SEIG
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One `traf`'s per-sample CENC auxiliary information, from its `senc` box
/// (ISO/IEC 23001-7 §12.3), one entry per sample of the track fragment.
///
/// The per-sample IV size is the track's `tenc.default_per_sample_iv_size`
/// (§12.2), the same rule the decrypt path uses. A constant-IV track has no
/// `senc` and yields no per-sample data, and an unprotected track yields none
/// either.
fn fragment_aux(
    traf: &TrackFragmentBox,
    crypto: Option<&TrackCrypto>,
    count: usize,
) -> Result<Vec<Option<SampleAux>>> {
    let Some(crypto) = crypto else {
        return Ok(core::iter::repeat_with(|| None).take(count).collect());
    };
    if traf_has_seig(traf)? {
        return Err(Error::UnsupportedFeature(
            "seig sample-group key rotation not implemented",
        ));
    }
    let senc = traf.order.iter().find_map(|child| match child {
        TrafChild::Opaque(opaque) if opaque.box_type == crate::cenc::SENC => Some(opaque),
        _ => None,
    });
    let Some(senc) = senc else {
        return Ok(core::iter::repeat_with(|| None).take(count).collect());
    };
    let data = &senc.data;
    if data.len() < FULLBOX_EXTRA_SIZE {
        return Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE,
                have: data.len(),
                what: "senc header",
            },
        ));
    }
    let version = data[0];
    let flags = broadcast_core::wire::u32_at(data, 0) & crate::box_types::FULLBOX_FLAGS_MASK;
    let parsed = SampleEncryptionBox::parse_body(
        &data[FULLBOX_EXTRA_SIZE..],
        version,
        flags,
        crypto.tenc.default_per_sample_iv_size,
    )?;
    if parsed.entries.len() < count {
        return Err(Error::InvalidInput(
            "senc has fewer entries than the fragment's samples",
        ));
    }
    Ok(parsed
        .entries
        .into_iter()
        .take(count)
        .map(|entry| {
            Some(SampleAux {
                initialization_vector: entry.initialization_vector,
                subsamples: entry.subsamples,
            })
        })
        .collect())
}

/// The track-description facts that live in `mdia`, `trak` and the track's
/// `udta`, gathered once so both track-info builders agree.
#[derive(Default)]
struct TrackDescription {
    extended_language: Option<crate::track_desc::ExtendedLanguageBox>,
    track_groups: Option<crate::track_desc::TrackGroupBox>,
    kinds: Vec<crate::track_desc::KindBox>,
    labels: Vec<crate::audio_boxes::LabelBox>,
    loudness: Option<crate::audio_boxes::LoudnessBox>,
    track_selection: Option<crate::track_desc::TrackSelectionBox>,
    media_header: Option<crate::init_segment::MediaInformationHeader>,
    subtracks: Vec<crate::track_desc::SubTrackBox>,
    hint_information: Option<crate::hint::HintInformationBox>,
    hint_statistics: Option<crate::hint::HintStatisticsBox>,
}

/// The track's decoder pre-roll from its `roll`/`prol` sample group (§10.1).
///
/// A description only applies when the sibling `sbgp` maps at least one sample
/// to it (a description index is one-based; zero means no group).
fn track_pre_roll(trak: &TrackBox) -> Option<u32> {
    let stbl = sample_table(trak)?;
    let mut group: Option<(u32, u32)> = None;
    for child in &stbl.children {
        let StblChild::Opaque(bytes) = child else {
            continue;
        };
        if bytes.len() < BOX_HEADER_MIN_SIZE
            || &bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] != b"sgpd"
        {
            continue;
        }
        if let Ok(sgpd) = SampleGroupDescriptionBox::parse(bytes.as_slice())
            && let Some(samples) = sgpd
                .entries
                .first()
                .and_then(|entry| entry.pre_roll_samples())
        {
            group = Some((sgpd.grouping_type, samples));
            break;
        }
    }
    let (grouping_type, samples) = group?;
    for child in &stbl.children {
        let StblChild::Opaque(bytes) = child else {
            continue;
        };
        if bytes.len() < BOX_HEADER_MIN_SIZE
            || &bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] != b"sbgp"
        {
            continue;
        }
        if let Ok(sbgp) = SampleToGroupBox::parse(bytes.as_slice())
            && sbgp.grouping_type == grouping_type
            && sbgp
                .entries
                .iter()
                .any(|entry| entry.group_description_index > 0)
        {
            return Some(samples);
        }
    }
    None
}

/// Attach each track's iTunes gapless facts.
///
/// A per-track item always applies to its own track. A movie-level item is
/// only unambiguous when the file carries one audio track; with several, it is
/// left unapplied rather than guessed at.
fn apply_itunsmpb(tracks: &mut [TrackInfo], metadata: &crate::metadata::MovieMetadata) {
    let movie_item = metadata.movie.itunsmpb();
    let audio: Vec<TrackId> = tracks
        .iter()
        .filter(|track| &track.handler == b"soun")
        .map(|track| track.track_id)
        .collect();
    for track in tracks.iter_mut() {
        let own = metadata
            .tracks
            .iter()
            .find(|(id, _)| *id == track.track_id)
            .and_then(|(_, metadata)| metadata.itunsmpb());
        let from_movie = if audio.len() == 1 && audio[0] == track.track_id {
            movie_item.clone()
        } else {
            None
        };
        track.itunsmpb = own.or(from_movie);
    }
}

fn track_description(trak: &TrackBox) -> TrackDescription {
    use crate::metadata::UdtaChild;
    let mut out = TrackDescription {
        extended_language: trak.mdia.as_ref().and_then(|mdia| mdia.elng.clone()),
        media_header: trak
            .mdia
            .as_ref()
            .and_then(|mdia| mdia.minf.as_ref())
            .and_then(|minf| minf.media_header()),
        track_groups: trak.trgr.clone(),
        ..TrackDescription::default()
    };
    let udta = trak
        .opaque
        .iter()
        .find(|child| child.box_type == crate::metadata::udta::UDTA)
        .and_then(|child| crate::metadata::Udta::parse_body(&child.data, &Limits::default()).ok());
    if let Some(udta) = udta {
        for child in udta.children {
            match child {
                UdtaChild::Kind(kind) => out.kinds.push(kind),
                UdtaChild::Label(label) => out.labels.push(label),
                UdtaChild::Loudness(loudness) => out.loudness = Some(loudness),
                UdtaChild::TrackSelection(selection) => out.track_selection = Some(selection),
                UdtaChild::SubTrack(subtrack) => out.subtracks.push(subtrack),
                UdtaChild::HintInformation(hint) => out.hint_information = Some(*hint),
                UdtaChild::HintStatistics(statistics) => out.hint_statistics = Some(*statistics),
                _ => {}
            }
        }
    }
    out
}

fn build_track_info(
    trak: &TrackBox,
    raw: Vec<RawEntry>,
    registry: Option<&crate::registry::BoxRegistry>,
) -> Result<TrackInfo> {
    let mdia: &MediaBox = trak
        .mdia
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "mdia" })?;
    let mdhd = mdia
        .mdhd
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "mdhd" })?;
    let hdlr = mdia
        .hdlr
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "hdlr" })?;
    let stsd = mdia
        .minf
        .as_ref()
        .and_then(|minf| minf.stbl.as_ref())
        .and_then(sample_description);
    let mut entries = Vec::with_capacity(raw.len());
    for (index, raw) in raw.into_iter().enumerate() {
        let typed = stsd
            .and_then(|box_| box_.entries.get(index))
            .cloned()
            .and_then(|entry| match entry {
                SampleEntryVariant::Unknown(_) => None,
                known => Some(known),
            });
        // A known four-CC whose `stsd` body failed to parse is kept raw: parse
        // the entry again to recover both its typed form and, when it still
        // fails, the parser's own message for the abandon reason.
        let mut error = None;
        let typed = match typed {
            Some(typed) => Some(typed),
            None => match sample_entry_variant(&raw.body) {
                Ok(SampleEntryVariant::Unknown(_)) => None,
                Ok(known) => Some(known),
                Err(err) => {
                    error = Some(err.to_string());
                    None
                }
            },
        };
        let registered = match registry {
            Some(registry) => registry.parse(Some(STSD), &raw.body)?,
            None => None,
        };
        entries.push(SampleEntryInfo {
            fourcc: raw.fourcc,
            body: raw.body,
            typed,
            error,
            registered,
        });
    }
    let references = match trak.track_references()? {
        Some(reference) => reference.references,
        None => Vec::new(),
    };
    let edits = match trak.edts.as_ref().and_then(|edts| edts.elst.as_ref()) {
        Some(elst) => elst.entries.clone(),
        None => Vec::new(),
    };
    let tapt = trak
        .opaque
        .iter()
        .find(|b| b.box_type == crate::quicktime::aperture::TAPT)
        .and_then(|b| crate::quicktime::TrackApertureModeDimensionsBox::parse_body(&b.data).ok());
    let desc = track_description(trak);
    Ok(TrackInfo {
        track_id: trak.tkhd.track_id,
        handler: hdlr.handler_type,
        timescale: mdhd.timescale,
        duration: mdhd.duration,
        header: trak.tkhd.clone(),
        references,
        edits,
        entries,
        tapt,
        extended_language: desc.extended_language,
        track_groups: desc.track_groups,
        kinds: desc.kinds,
        labels: desc.labels,
        loudness: desc.loudness,
        track_selection: desc.track_selection,
        media_header: desc.media_header,
        subtracks: desc.subtracks,
        pre_roll: track_pre_roll(trak),
        itunsmpb: None,
        hint_information: desc.hint_information,
        hint_statistics: desc.hint_statistics,
    })
}

fn raw_stsd_entries(
    moov: &[u8],
    track_index: usize,
    limits: &Limits,
    base_offset: u64,
) -> Result<Vec<RawEntry>> {
    let movie = parse_box(moov)?.0;
    let Some(trak) = nth_child(movie.body, b"trak", track_index, TRAK_DEPTH, limits)? else {
        return Ok(Vec::new());
    };
    let trak_body = parse_box(trak)?.0.body;
    let mdia = child(trak_body, b"mdia", MDIA_DEPTH, limits)?
        .ok_or(Error::UnexpectedBox { expected: "mdia" })?;
    let mdia_body = parse_box(mdia)?.0.body;
    let minf = child(mdia_body, b"minf", MINF_DEPTH, limits)?
        .ok_or(Error::UnexpectedBox { expected: "minf" })?;
    let minf_body = parse_box(minf)?.0.body;
    let stbl = child(minf_body, b"stbl", STBL_DEPTH, limits)?
        .ok_or(Error::UnexpectedBox { expected: "stbl" })?;
    let stbl_body = parse_box(stbl)?.0.body;
    let stsd = child(stbl_body, b"stsd", STSD_DEPTH, limits)?
        .ok_or(Error::UnexpectedBox { expected: "stsd" })?;
    let stsd_body = parse_box(stsd)?.0.body;
    let count = usize::try_from(broadcast_core::wire::be_u32(
        stsd_body,
        STSD_ENTRY_COUNT_OFFSET,
        "stsd entry_count",
    )?)
    .unwrap_or(usize::MAX);
    let entries_start = STSD_ENTRY_COUNT_OFFSET + core::mem::size_of::<u32>();
    let entries = stsd_body.get(entries_start..).unwrap_or(&[]);
    let base = moov.as_ptr() as usize;
    let mut out = Vec::new();
    for item in BoxIter::clamped(entries).take(count) {
        let Ok(item) = item else {
            break;
        };
        let offset = base_offset
            .saturating_add(u64::try_from(item.bytes.as_ptr() as usize - base).unwrap_or(u64::MAX));
        out.push(RawEntry {
            fourcc: item.four_cc(),
            body: item.bytes.to_vec(),
            offset,
        });
    }
    Ok(out)
}

fn child<'a>(
    body: &'a [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    depth: u32,
    limits: &Limits,
) -> Result<Option<&'a [u8]>> {
    check_depth(depth, limits)?;
    for item in BoxIter::new(body) {
        let item = item?;
        if item.header.box_type.is(fourcc) {
            return Ok(Some(item.bytes));
        }
    }
    Ok(None)
}

fn nth_child<'a>(
    body: &'a [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    index: usize,
    depth: u32,
    limits: &Limits,
) -> Result<Option<&'a [u8]>> {
    check_depth(depth, limits)?;
    let mut seen = 0usize;
    for item in BoxIter::new(body) {
        let item = item?;
        if item.header.box_type.is(fourcc) {
            if seen == index {
                return Ok(Some(item.bytes));
            }
            seen += 1;
        }
    }
    Ok(None)
}

fn check_depth(depth: u32, limits: &Limits) -> Result<()> {
    if depth > limits.max_depth() {
        return Err(Error::LimitExceeded {
            what: "box tree depth",
            limit: u64::from(limits.max_depth()),
        });
    }
    Ok(())
}

fn rescale_floor(value: u64, from: Timescale, to: Timescale) -> u64 {
    if from == to {
        return value;
    }
    let scaled = u128::from(value) * u128::from(to.get()) / u128::from(from.get());
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

/// Four-CC of the sample description box.
const STSD: [u8; BOX_TYPE_SIZE] = *b"stsd";

impl Reader {
    fn check_opaque_stsd(&mut self, trak: &TrackBox, raw: &[RawEntry]) -> Result<()> {
        let Some(stbl) = sample_table(trak) else {
            return Ok(());
        };
        let failed = stbl.children.iter().any(|child| match child {
            StblChild::Opaque(bytes)
                if bytes.len() >= BOX_HEADER_MIN_SIZE
                    && bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] == STSD =>
            {
                opaque_stsd_has_body(bytes)
            }
            _ => false,
        });
        if !failed {
            return Ok(());
        }
        if self.tolerated() {
            let (fourcc, offset) = raw.first().map_or(([0u8; BOX_TYPE_SIZE], 0), |entry| {
                (entry.fourcc, entry.offset)
            });
            self.warn(Warning::UnparseableSampleEntry { fourcc, offset });
            Ok(())
        } else {
            Err(Error::InvalidInput(
                "a known sample-entry four-CC did not parse",
            ))
        }
    }

    fn check_wave_esds(&mut self, trak: &TrackBox, raw: &[RawEntry]) -> Result<()> {
        let Some(stsd) = trak
            .mdia
            .as_ref()
            .and_then(|mdia| mdia.minf.as_ref())
            .and_then(|minf| minf.stbl.as_ref())
            .and_then(sample_description)
        else {
            return Ok(());
        };
        for (index, entry) in stsd.entries.iter().enumerate() {
            let SampleEntryVariant::Mp4a(mp4a) = entry else {
                continue;
            };
            let Some(wave) = mp4a.wave() else {
                continue;
            };
            if wave.esds.is_some() {
                continue;
            }
            let offset = raw.get(index).map_or(0, |entry| entry.offset);
            if self.tolerated() {
                self.warn(Warning::WaveWithoutEsds { offset });
            } else {
                return Err(Error::UnexpectedBox {
                    expected: "esds in an mp4a wave atom",
                });
            }
        }
        Ok(())
    }
}

/// Deepest container nesting the moov sanitizer descends.
const SANITIZE_MAX_DEPTH: u32 = 16;

/// Offset of the byte after a `meta` box's FullBox header.
const META_BODY_START: usize = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE;
/// Body offset of the `mvhd` timescale in the version 0 form.
const MVHD_V0_TIMESCALE_BODY_OFFSET: usize = FULLBOX_EXTRA_SIZE + 8;
/// Body offset of the `mvhd` timescale in the version 1 form.
const MVHD_V1_TIMESCALE_BODY_OFFSET: usize = FULLBOX_EXTRA_SIZE + 16;

/// Container four-CCs the permissive `moov` sanitizer descends.
const SANITIZE_CONTAINERS: &[[u8; BOX_TYPE_SIZE]] = &[
    *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"dinf", *b"edts", *b"udta", *b"mvex",
];

/// The `meta` box that carries a four-byte FullBox header before its children.
const META: [u8; BOX_TYPE_SIZE] = *b"meta";

/// Repair the faults a permissive `moov` walk tolerates.
fn sanitize_moov(data: &mut [u8], base: u64, warnings: &mut Vec<Warning>) {
    let body_start = crate::box_types::BoxHeader::parse(data)
        .map(|header| header.header_size())
        .unwrap_or(BOX_HEADER_MIN_SIZE);
    clamp_children(data, body_start, data.len(), base, warnings, 0);
    patch_zero_mvhd(data, body_start, base, warnings);
}

fn clamp_children(
    data: &mut [u8],
    start: usize,
    end: usize,
    base: u64,
    warnings: &mut Vec<Warning>,
    depth: u32,
) {
    if depth > SANITIZE_MAX_DEPTH {
        return;
    }
    let mut at = start;
    while at + BOX_HEADER_MIN_SIZE <= end {
        let size = broadcast_core::wire::u32_at(data, at);
        if size == 0 {
            return;
        }
        if size == 1 {
            return;
        }
        let mut size = usize::try_from(size).unwrap_or(usize::MAX);
        if size < BOX_HEADER_MIN_SIZE {
            return;
        }
        let mut fourcc = [0u8; BOX_TYPE_SIZE];
        fourcc.copy_from_slice(&data[at + BOX_TYPE_OFFSET..at + BOX_HEADER_MIN_SIZE]);
        if at.saturating_add(size) > end {
            let clamped = end.saturating_sub(at);
            let Ok(clamped32) = u32::try_from(clamped) else {
                return;
            };
            data[at..at + BOX_TYPE_OFFSET].copy_from_slice(&clamped32.to_be_bytes());
            warnings.push(Warning::ClampedBox {
                box_type: fourcc,
                offset: base.saturating_add(u64::try_from(at).unwrap_or(u64::MAX)),
            });
            size = clamped;
        }
        let child_body = if fourcc == META {
            at + META_BODY_START
        } else {
            at + BOX_HEADER_MIN_SIZE
        };
        if SANITIZE_CONTAINERS.contains(&fourcc) || fourcc == META {
            clamp_children(data, child_body, at + size, base, warnings, depth + 1);
        }
        at = at.saturating_add(size);
    }
}

fn patch_zero_mvhd(data: &mut [u8], start: usize, base: u64, warnings: &mut Vec<Warning>) {
    let mut found = None;
    let mut at = start;
    while at + BOX_HEADER_MIN_SIZE <= data.len() {
        let size = usize::try_from(broadcast_core::wire::u32_at(data, at)).unwrap_or(usize::MAX);
        if size < BOX_HEADER_MIN_SIZE {
            return;
        }
        if data[at + BOX_TYPE_OFFSET..at + BOX_HEADER_MIN_SIZE] == *b"mvhd" {
            found = Some((at, size));
            break;
        }
        let next = at.saturating_add(size);
        if next <= at || next > data.len() {
            return;
        }
        at = next;
    }
    let Some((at, size)) = found else {
        return;
    };
    let body = at + BOX_HEADER_MIN_SIZE;
    if body + FULLBOX_EXTRA_SIZE > data.len() || body + FULLBOX_EXTRA_SIZE > at + size {
        return;
    }
    let version = data[body];
    let timescale_at = if version == 1 {
        body + MVHD_V1_TIMESCALE_BODY_OFFSET
    } else {
        body + MVHD_V0_TIMESCALE_BODY_OFFSET
    };
    if timescale_at + core::mem::size_of::<u32>() > data.len() {
        return;
    }
    if broadcast_core::wire::u32_at(data, timescale_at) != 0 {
        return;
    }
    data[timescale_at..timescale_at + core::mem::size_of::<u32>()]
        .copy_from_slice(&1u32.to_be_bytes());
    warnings.push(Warning::ZeroMovieTimescale {
        offset: base.saturating_add(u64::try_from(at).unwrap_or(u64::MAX)),
    });
}

/// Locate a child box of a visual sample entry by four-CC. The visual sample
/// entry header is `visual_sample_entry_header_len()` bytes after the box
/// header (ISO/IEC 14496-12:2026 §12.1.3).
fn find_entry_child(entry: &[u8], fourcc: &[u8; BOX_TYPE_SIZE]) -> Option<Vec<u8>> {
    let start =
        BOX_HEADER_MIN_SIZE + crate::sample_entries::VisualSampleEntryFields::serialized_len();
    let mut cursor = start;
    while cursor + BOX_HEADER_MIN_SIZE <= entry.len() {
        let size = usize::try_from(broadcast_core::wire::u32_at(entry, cursor)).ok()?;
        if size < BOX_HEADER_MIN_SIZE || cursor + size > entry.len() {
            return None;
        }
        let typ = &entry[cursor + BOX_TYPE_OFFSET..cursor + BOX_HEADER_MIN_SIZE];
        if typ == fourcc {
            return Some(entry[cursor..cursor + size].to_vec());
        }
        cursor += size;
    }
    None
}

#[cfg(test)]
mod image_sequence_entry_tests {
    use super::find_entry_child;
    use broadcast_core::{Parse, Serialize};

    #[test]
    fn every_clause_7_sample_entry_child_is_found() {
        let constraints = crate::image_sequence::CodingConstraintsBox::new(true, true, 2);
        let auxiliary = crate::image_sequence::AuxiliaryTypeInfoBox::new(b"urn:example:aux");
        let mut entry = alloc::vec![0u8; crate::sample_entries::VisualSampleEntryFields::serialized_len()
            + crate::box_types::BOX_HEADER_MIN_SIZE];
        entry.extend_from_slice(&constraints.try_to_bytes().expect("ccst"));
        entry.extend_from_slice(&auxiliary.try_to_bytes().expect("auxi"));
        let ccst = find_entry_child(&entry, &crate::image_sequence::CCST).expect("ccst");
        assert_eq!(
            crate::image_sequence::CodingConstraintsBox::parse(&ccst).expect("parse"),
            constraints
        );
        let auxi = find_entry_child(&entry, &crate::image_sequence::AUXI).expect("auxi");
        assert_eq!(
            crate::image_sequence::AuxiliaryTypeInfoBox::parse(&auxi).expect("parse"),
            auxiliary
        );
        assert!(find_entry_child(&entry, b"ispe").is_none());
    }
}
