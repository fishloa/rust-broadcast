//! Typed field accessors over the universal [`Element`](crate::matroska::Element).
//!
//! Each accessor reads the named child of RFC 9559 §5, so a caller never
//! re-implements the id table. The raw element stays reachable through
//! [`children`](crate::matroska::Element::children), which is what keeps an
//! untyped field round-tripping.

use crate::ebml::scalar::{Binary, Date, Text};
use broadcast_common::Parse;

use crate::error::Result;
use crate::matroska::block::Block;
use crate::matroska::element::{
    AttachedFile, Attachments, Audio, BlockAdditionMapping, BlockAdditions, BlockGroup, BlockMore,
    ChapProcess, ChapProcessCommand, ChapterAtom, ChapterDisplay, ChapterTranslate, Chapters,
    Cluster, Colour, ContentCompression, ContentEncAesSettings, ContentEncoding, ContentEncodings,
    ContentEncryption, CuePoint, CueReference, CueTrackPositions, Cues, EditionEntry, Info,
    MasteringMetadata, Projection, ReferenceFrame, Seek, SeekHead, Segment, SilentTracks,
    SimpleTag, Slices, Tag, Tags, Targets, TimeSlice, TrackCombinePlanes, TrackEntry,
    TrackJoinBlocks, TrackOperation, TrackPlane, TrackTranslate, Tracks, Video,
};
use crate::matroska::ids;
use crate::matroska::types::{
    AesSettingsCipherMode, ColourRange, ContentCompAlgo, ContentEncAlgo, DisplayUnit,
    ProjectionType, TrackType,
};

impl<'a> crate::matroska::Element<'a> {
    /// Borrow this element as a `Segment` master.
    #[must_use]
    pub fn as_segment(&self) -> Option<&Segment<'a>> {
        match self {
            Self::Segment(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `SeekHead` master.
    #[must_use]
    pub fn as_seek_head(&self) -> Option<&SeekHead<'a>> {
        match self {
            Self::SeekHead(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Seek` master.
    #[must_use]
    pub fn as_seek(&self) -> Option<&Seek<'a>> {
        match self {
            Self::Seek(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Info` master.
    #[must_use]
    pub fn as_info(&self) -> Option<&Info<'a>> {
        match self {
            Self::Info(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ChapterTranslate` master.
    #[must_use]
    pub fn as_chapter_translate(&self) -> Option<&ChapterTranslate<'a>> {
        match self {
            Self::ChapterTranslate(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Cluster` master.
    #[must_use]
    pub fn as_cluster(&self) -> Option<&Cluster<'a>> {
        match self {
            Self::Cluster(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `SilentTracks` master.
    #[must_use]
    pub fn as_silent_tracks(&self) -> Option<&SilentTracks<'a>> {
        match self {
            Self::SilentTracks(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `BlockGroup` master.
    #[must_use]
    pub fn as_block_group(&self) -> Option<&BlockGroup<'a>> {
        match self {
            Self::BlockGroup(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `BlockAdditions` master.
    #[must_use]
    pub fn as_block_additions(&self) -> Option<&BlockAdditions<'a>> {
        match self {
            Self::BlockAdditions(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `BlockMore` master.
    #[must_use]
    pub fn as_block_more(&self) -> Option<&BlockMore<'a>> {
        match self {
            Self::BlockMore(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Slices` master.
    #[must_use]
    pub fn as_slices(&self) -> Option<&Slices<'a>> {
        match self {
            Self::Slices(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `TimeSlice` master.
    #[must_use]
    pub fn as_time_slice(&self) -> Option<&TimeSlice<'a>> {
        match self {
            Self::TimeSlice(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ReferenceFrame` master.
    #[must_use]
    pub fn as_reference_frame(&self) -> Option<&ReferenceFrame<'a>> {
        match self {
            Self::ReferenceFrame(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Tracks` master.
    #[must_use]
    pub fn as_tracks(&self) -> Option<&Tracks<'a>> {
        match self {
            Self::Tracks(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `TrackEntry` master.
    #[must_use]
    pub fn as_track_entry(&self) -> Option<&TrackEntry<'a>> {
        match self {
            Self::TrackEntry(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `BlockAdditionMapping` master.
    #[must_use]
    pub fn as_block_addition_mapping(&self) -> Option<&BlockAdditionMapping<'a>> {
        match self {
            Self::BlockAdditionMapping(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `TrackTranslate` master.
    #[must_use]
    pub fn as_track_translate(&self) -> Option<&TrackTranslate<'a>> {
        match self {
            Self::TrackTranslate(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Video` master.
    #[must_use]
    pub fn as_video(&self) -> Option<&Video<'a>> {
        match self {
            Self::Video(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Colour` master.
    #[must_use]
    pub fn as_colour(&self) -> Option<&Colour<'a>> {
        match self {
            Self::Colour(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `MasteringMetadata` master.
    #[must_use]
    pub fn as_mastering_metadata(&self) -> Option<&MasteringMetadata<'a>> {
        match self {
            Self::MasteringMetadata(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Projection` master.
    #[must_use]
    pub fn as_projection(&self) -> Option<&Projection<'a>> {
        match self {
            Self::Projection(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Audio` master.
    #[must_use]
    pub fn as_audio(&self) -> Option<&Audio<'a>> {
        match self {
            Self::Audio(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `TrackOperation` master.
    #[must_use]
    pub fn as_track_operation(&self) -> Option<&TrackOperation<'a>> {
        match self {
            Self::TrackOperation(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `TrackCombinePlanes` master.
    #[must_use]
    pub fn as_track_combine_planes(&self) -> Option<&TrackCombinePlanes<'a>> {
        match self {
            Self::TrackCombinePlanes(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `TrackPlane` master.
    #[must_use]
    pub fn as_track_plane(&self) -> Option<&TrackPlane<'a>> {
        match self {
            Self::TrackPlane(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `TrackJoinBlocks` master.
    #[must_use]
    pub fn as_track_join_blocks(&self) -> Option<&TrackJoinBlocks<'a>> {
        match self {
            Self::TrackJoinBlocks(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ContentEncodings` master.
    #[must_use]
    pub fn as_content_encodings(&self) -> Option<&ContentEncodings<'a>> {
        match self {
            Self::ContentEncodings(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ContentEncoding` master.
    #[must_use]
    pub fn as_content_encoding(&self) -> Option<&ContentEncoding<'a>> {
        match self {
            Self::ContentEncoding(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ContentCompression` master.
    #[must_use]
    pub fn as_content_compression(&self) -> Option<&ContentCompression<'a>> {
        match self {
            Self::ContentCompression(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ContentEncryption` master.
    #[must_use]
    pub fn as_content_encryption(&self) -> Option<&ContentEncryption<'a>> {
        match self {
            Self::ContentEncryption(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ContentEncAesSettings` master.
    #[must_use]
    pub fn as_content_enc_aes_settings(&self) -> Option<&ContentEncAesSettings<'a>> {
        match self {
            Self::ContentEncAesSettings(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Cues` master.
    #[must_use]
    pub fn as_cues(&self) -> Option<&Cues<'a>> {
        match self {
            Self::Cues(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `CuePoint` master.
    #[must_use]
    pub fn as_cue_point(&self) -> Option<&CuePoint<'a>> {
        match self {
            Self::CuePoint(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `CueTrackPositions` master.
    #[must_use]
    pub fn as_cue_track_positions(&self) -> Option<&CueTrackPositions<'a>> {
        match self {
            Self::CueTrackPositions(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `CueReference` master.
    #[must_use]
    pub fn as_cue_reference(&self) -> Option<&CueReference<'a>> {
        match self {
            Self::CueReference(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Attachments` master.
    #[must_use]
    pub fn as_attachments(&self) -> Option<&Attachments<'a>> {
        match self {
            Self::Attachments(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `AttachedFile` master.
    #[must_use]
    pub fn as_attached_file(&self) -> Option<&AttachedFile<'a>> {
        match self {
            Self::AttachedFile(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Chapters` master.
    #[must_use]
    pub fn as_chapters(&self) -> Option<&Chapters<'a>> {
        match self {
            Self::Chapters(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `EditionEntry` master.
    #[must_use]
    pub fn as_edition_entry(&self) -> Option<&EditionEntry<'a>> {
        match self {
            Self::EditionEntry(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ChapterAtom` master.
    #[must_use]
    pub fn as_chapter_atom(&self) -> Option<&ChapterAtom<'a>> {
        match self {
            Self::ChapterAtom(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ChapterDisplay` master.
    #[must_use]
    pub fn as_chapter_display(&self) -> Option<&ChapterDisplay<'a>> {
        match self {
            Self::ChapterDisplay(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ChapProcess` master.
    #[must_use]
    pub fn as_chap_process(&self) -> Option<&ChapProcess<'a>> {
        match self {
            Self::ChapProcess(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `ChapProcessCommand` master.
    #[must_use]
    pub fn as_chap_process_command(&self) -> Option<&ChapProcessCommand<'a>> {
        match self {
            Self::ChapProcessCommand(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Tags` master.
    #[must_use]
    pub fn as_tags(&self) -> Option<&Tags<'a>> {
        match self {
            Self::Tags(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Tag` master.
    #[must_use]
    pub fn as_tag(&self) -> Option<&Tag<'a>> {
        match self {
            Self::Tag(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `Targets` master.
    #[must_use]
    pub fn as_targets(&self) -> Option<&Targets<'a>> {
        match self {
            Self::Targets(element) => Some(element),
            _ => None,
        }
    }

    /// Borrow this element as a `SimpleTag` master.
    #[must_use]
    pub fn as_simple_tag(&self) -> Option<&SimpleTag<'a>> {
        match self {
            Self::SimpleTag(element) => Some(element),
            _ => None,
        }
    }

    /// The first unsigned integer child with `id`.
    #[must_use]
    pub fn child_unsigned(&self, id: u32) -> Option<u64> {
        self.find(id)
            .and_then(crate::matroska::Element::as_unsigned)
    }

    /// The first float child with `id`.
    #[must_use]
    pub fn child_float(&self, id: u32) -> Option<f64> {
        self.find(id).and_then(crate::matroska::Element::as_float)
    }

    /// The first text child with `id`.
    #[must_use]
    pub fn child_text(&self, id: u32) -> Option<&'a str> {
        self.find(id)
            .and_then(crate::matroska::Element::as_text)
            .and_then(Text::as_str)
    }

    /// The first binary child with `id`.
    #[must_use]
    pub fn child_binary(&self, id: u32) -> Option<&'a [u8]> {
        self.find(id)
            .and_then(crate::matroska::Element::as_binary)
            .map(Binary::bytes)
    }

    /// The first date child with `id`.
    #[must_use]
    pub fn child_date(&self, id: u32) -> Option<Date> {
        self.find(id).and_then(crate::matroska::Element::as_date)
    }

    /// This element as a `SimpleBlock` or `Block` payload (RFC 9559 §12).
    ///
    /// `None` when the element is neither; `Some(Err(..))` when it is one but
    /// its payload is malformed.
    #[must_use]
    pub fn as_block(&self) -> Option<Result<Block<'a>>> {
        let id = self.id();
        let binary = self.as_binary()?;
        if id != ids::SIMPLE_BLOCK && id != ids::BLOCK {
            return None;
        }
        Some(Block::parse(binary.bytes()))
    }
}

/// The default `TimestampScale` in nanoseconds (RFC 9559 §5.1.2.9).
pub const DEFAULT_TIMESTAMP_SCALE: u64 = 1_000_000;

/// The default `FlagEnabled` (RFC 9559 §5.1.4.1.4).
pub const DEFAULT_FLAG_ENABLED: u64 = 1;
/// The default `FlagDefault` (RFC 9559 §5.1.4.1.5).
pub const DEFAULT_FLAG_DEFAULT: u64 = 1;
/// The default `FlagLacing` (RFC 9559 §5.1.4.1.12).
pub const DEFAULT_FLAG_LACING: u64 = 1;

use crate::matroska::Element as AnyElement;

macro_rules! child {
    ($self:expr, $id:expr, $get:ident) => {
        $self.find($id).and_then(AnyElement::$get)
    };
}

impl<'a> Segment<'a> {
    /// The `SeekHead`, when present.
    #[must_use]
    pub fn seek_head(&self) -> Option<&SeekHead<'a>> {
        self.find(ids::SEEK_HEAD).and_then(AnyElement::as_seek_head)
    }

    /// The `Info`, when present.
    #[must_use]
    pub fn info(&self) -> Option<&Info<'a>> {
        self.find(ids::INFO).and_then(AnyElement::as_info)
    }

    /// The `Tracks`, when present.
    #[must_use]
    pub fn tracks(&self) -> Option<&Tracks<'a>> {
        self.find(ids::TRACKS).and_then(AnyElement::as_tracks)
    }

    /// The `Cues`, when present.
    #[must_use]
    pub fn cues(&self) -> Option<&Cues<'a>> {
        self.find(ids::CUES).and_then(AnyElement::as_cues)
    }

    /// The `Tags`, when present.
    #[must_use]
    pub fn tags(&self) -> Option<&Tags<'a>> {
        self.find(ids::TAGS).and_then(AnyElement::as_tags)
    }

    /// The `Attachments`, when present.
    #[must_use]
    pub fn attachments(&self) -> Option<&Attachments<'a>> {
        self.find(ids::ATTACHMENTS)
            .and_then(AnyElement::as_attachments)
    }

    /// The `Chapters`, when present.
    #[must_use]
    pub fn chapters(&self) -> Option<&Chapters<'a>> {
        self.find(ids::CHAPTERS).and_then(AnyElement::as_chapters)
    }

    /// Every `Cluster`, in file order.
    pub fn clusters(&self) -> impl Iterator<Item = &Cluster<'a>> + '_ {
        self.children().iter().filter_map(AnyElement::as_cluster)
    }

    /// The `TimestampScale` in nanoseconds, or its default.
    #[must_use]
    pub fn timestamp_scale(&self) -> u64 {
        self.info()
            .map_or(DEFAULT_TIMESTAMP_SCALE, Info::timestamp_scale)
    }

    /// The `Duration` in Matroska Ticks, when present.
    #[must_use]
    pub fn duration(&self) -> Option<f64> {
        self.info().and_then(Info::duration)
    }
}

impl<'a> SeekHead<'a> {
    /// Every `Seek` entry.
    pub fn seeks(&self) -> impl Iterator<Item = &Seek<'a>> + '_ {
        self.children().iter().filter_map(AnyElement::as_seek)
    }
}

impl Seek<'_> {
    /// The `SeekID`.
    #[must_use]
    pub fn seek_id(&self) -> Option<&[u8]> {
        child!(self, ids::SEEK_ID, as_binary).map(Binary::bytes)
    }

    /// The `SeekPosition`.
    #[must_use]
    pub fn seek_position(&self) -> Option<u64> {
        child!(self, ids::SEEK_POSITION, as_unsigned)
    }
}

impl Info<'_> {
    /// The `TimestampScale` in nanoseconds, or its default.
    #[must_use]
    pub fn timestamp_scale(&self) -> u64 {
        child!(self, ids::TIMESTAMP_SCALE, as_unsigned).unwrap_or(DEFAULT_TIMESTAMP_SCALE)
    }

    /// The `Duration` in Matroska Ticks.
    #[must_use]
    pub fn duration(&self) -> Option<f64> {
        child!(self, ids::DURATION, as_float)
    }

    /// The `DateUTC`.
    #[must_use]
    pub fn date_utc(&self) -> Option<Date> {
        child!(self, ids::DATE_UTC, as_date)
    }

    /// The `Title`.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        child!(self, ids::TITLE, as_text).and_then(Text::as_str)
    }

    /// The `MuxingApp`.
    #[must_use]
    pub fn muxing_app(&self) -> Option<&str> {
        child!(self, ids::MUXING_APP, as_text).and_then(Text::as_str)
    }

    /// The `WritingApp`.
    #[must_use]
    pub fn writing_app(&self) -> Option<&str> {
        child!(self, ids::WRITING_APP, as_text).and_then(Text::as_str)
    }

    /// The `SegmentUUID`.
    #[must_use]
    pub fn segment_uuid(&self) -> Option<&[u8]> {
        child!(self, ids::SEGMENT_UUID, as_binary).map(Binary::bytes)
    }
}

impl<'a> Tracks<'a> {
    /// Every `TrackEntry`.
    pub fn entries(&self) -> impl Iterator<Item = &TrackEntry<'a>> + '_ {
        self.children()
            .iter()
            .filter_map(AnyElement::as_track_entry)
    }
}

impl<'a> TrackEntry<'a> {
    /// The `TrackNumber`.
    #[must_use]
    pub fn track_number(&self) -> Option<u64> {
        child!(self, ids::TRACK_NUMBER, as_unsigned)
    }

    /// The `TrackUID`.
    #[must_use]
    pub fn track_uid(&self) -> Option<u64> {
        child!(self, ids::TRACK_UID, as_unsigned)
    }

    /// The decoded `TrackType`.
    #[must_use]
    pub fn track_type(&self) -> Option<TrackType> {
        child!(self, ids::TRACK_TYPE, as_unsigned).and_then(TrackType::from_u64)
    }

    /// The `CodecID`.
    #[must_use]
    pub fn codec_id(&self) -> Option<&str> {
        child!(self, ids::CODEC_ID, as_text).and_then(Text::as_str)
    }

    /// The `CodecPrivate`.
    #[must_use]
    pub fn codec_private(&self) -> Option<&[u8]> {
        child!(self, ids::CODEC_PRIVATE, as_binary).map(Binary::bytes)
    }

    /// The `CodecDelay` in nanoseconds.
    #[must_use]
    pub fn codec_delay(&self) -> Option<u64> {
        child!(self, ids::CODEC_DELAY, as_unsigned)
    }

    /// The `SeekPreRoll` in nanoseconds.
    #[must_use]
    pub fn seek_pre_roll(&self) -> Option<u64> {
        child!(self, ids::SEEK_PRE_ROLL, as_unsigned)
    }

    /// The `DefaultDuration` in nanoseconds.
    #[must_use]
    pub fn default_duration(&self) -> Option<u64> {
        child!(self, ids::DEFAULT_DURATION, as_unsigned)
    }

    /// The `FlagEnabled` (`true` by default).
    #[must_use]
    pub fn flag_enabled(&self) -> bool {
        child!(self, ids::FLAG_ENABLED, as_unsigned).unwrap_or(DEFAULT_FLAG_ENABLED) != 0
    }

    /// The `FlagDefault` (`true` by default).
    #[must_use]
    pub fn flag_default(&self) -> bool {
        child!(self, ids::FLAG_DEFAULT, as_unsigned).unwrap_or(DEFAULT_FLAG_DEFAULT) != 0
    }

    /// The `FlagLacing` (`true` by default).
    #[must_use]
    pub fn flag_lacing(&self) -> bool {
        child!(self, ids::FLAG_LACING, as_unsigned).unwrap_or(DEFAULT_FLAG_LACING) != 0
    }

    /// The `Name`.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        child!(self, ids::NAME, as_text).and_then(Text::as_str)
    }

    /// The `Language`.
    #[must_use]
    pub fn language(&self) -> Option<&str> {
        child!(self, ids::LANGUAGE, as_text).and_then(Text::as_str)
    }

    /// The `Video`, when present.
    #[must_use]
    pub fn video(&self) -> Option<&Video<'a>> {
        child!(self, ids::VIDEO, as_video)
    }

    /// The `Audio`, when present.
    #[must_use]
    pub fn audio(&self) -> Option<&Audio<'a>> {
        child!(self, ids::AUDIO, as_audio)
    }

    /// The `ContentEncodings`, when present.
    #[must_use]
    pub fn content_encodings(&self) -> Option<&ContentEncodings<'a>> {
        child!(self, ids::CONTENT_ENCODINGS, as_content_encodings)
    }
}

impl Video<'_> {
    /// The `PixelWidth`.
    #[must_use]
    pub fn pixel_width(&self) -> Option<u64> {
        child!(self, ids::PIXEL_WIDTH, as_unsigned)
    }

    /// The `PixelHeight`.
    #[must_use]
    pub fn pixel_height(&self) -> Option<u64> {
        child!(self, ids::PIXEL_HEIGHT, as_unsigned)
    }

    /// The `DisplayWidth`.
    #[must_use]
    pub fn display_width(&self) -> Option<u64> {
        child!(self, ids::DISPLAY_WIDTH, as_unsigned)
    }

    /// The `DisplayHeight`.
    #[must_use]
    pub fn display_height(&self) -> Option<u64> {
        child!(self, ids::DISPLAY_HEIGHT, as_unsigned)
    }

    /// The decoded `DisplayUnit`.
    #[must_use]
    pub fn display_unit(&self) -> Option<DisplayUnit> {
        child!(self, ids::DISPLAY_UNIT, as_unsigned).and_then(DisplayUnit::from_u64)
    }

    /// The `Colour`, when present.
    #[must_use]
    pub fn colour(&self) -> Option<&Colour<'_>> {
        child!(self, ids::COLOUR, as_colour)
    }

    /// The `Projection`, when present.
    #[must_use]
    pub fn projection(&self) -> Option<&Projection<'_>> {
        child!(self, ids::PROJECTION, as_projection)
    }
}

impl Colour<'_> {
    /// The decoded `Range`.
    #[must_use]
    pub fn range(&self) -> Option<ColourRange> {
        child!(self, ids::RANGE, as_unsigned).and_then(ColourRange::from_u64)
    }

    /// The `MatrixCoefficients`.
    #[must_use]
    pub fn matrix_coefficients(&self) -> Option<u64> {
        child!(self, ids::MATRIX_COEFFICIENTS, as_unsigned)
    }

    /// The `Primaries`.
    #[must_use]
    pub fn primaries(&self) -> Option<u64> {
        child!(self, ids::PRIMARIES, as_unsigned)
    }

    /// The `TransferCharacteristics`.
    #[must_use]
    pub fn transfer_characteristics(&self) -> Option<u64> {
        child!(self, ids::TRANSFER_CHARACTERISTICS, as_unsigned)
    }

    /// The `BitsPerChannel`.
    #[must_use]
    pub fn bits_per_channel(&self) -> Option<u64> {
        child!(self, ids::BITS_PER_CHANNEL, as_unsigned)
    }

    /// The `MaxCLL`.
    #[must_use]
    pub fn max_cll(&self) -> Option<u64> {
        child!(self, ids::MAX_CLL, as_unsigned)
    }

    /// The `MaxFALL`.
    #[must_use]
    pub fn max_fall(&self) -> Option<u64> {
        child!(self, ids::MAX_FALL, as_unsigned)
    }

    /// The `MasteringMetadata`, when present.
    #[must_use]
    pub fn mastering_metadata(&self) -> Option<&MasteringMetadata<'_>> {
        child!(self, ids::MASTERING_METADATA, as_mastering_metadata)
    }
}

impl MasteringMetadata<'_> {
    /// The `LuminanceMax`.
    #[must_use]
    pub fn luminance_max(&self) -> Option<f64> {
        child!(self, ids::LUMINANCE_MAX, as_float)
    }

    /// The `LuminanceMin`.
    #[must_use]
    pub fn luminance_min(&self) -> Option<f64> {
        child!(self, ids::LUMINANCE_MIN, as_float)
    }
}

impl<'a> Projection<'a> {
    /// The decoded `ProjectionType`.
    #[must_use]
    pub fn projection_type(&self) -> Option<ProjectionType> {
        child!(self, ids::PROJECTION_TYPE, as_unsigned).and_then(ProjectionType::from_u64)
    }

    /// The `ProjectionPrivate`.
    #[must_use]
    pub fn projection_private(&self) -> Option<&'a [u8]> {
        child!(self, ids::PROJECTION_PRIVATE, as_binary).map(Binary::bytes)
    }

    /// The `ProjectionPoseYaw` in degrees.
    #[must_use]
    pub fn pose_yaw(&self) -> Option<f64> {
        child!(self, ids::PROJECTION_POSE_YAW, as_float)
    }

    /// The `ProjectionPosePitch` in degrees.
    #[must_use]
    pub fn pose_pitch(&self) -> Option<f64> {
        child!(self, ids::PROJECTION_POSE_PITCH, as_float)
    }

    /// The `ProjectionPoseRoll` in degrees.
    #[must_use]
    pub fn pose_roll(&self) -> Option<f64> {
        child!(self, ids::PROJECTION_POSE_ROLL, as_float)
    }
}

impl Audio<'_> {
    /// The `SamplingFrequency` in Hz.
    #[must_use]
    pub fn sampling_frequency(&self) -> Option<f64> {
        child!(self, ids::SAMPLING_FREQUENCY, as_float)
    }

    /// The `OutputSamplingFrequency` in Hz.
    #[must_use]
    pub fn output_sampling_frequency(&self) -> Option<f64> {
        child!(self, ids::OUTPUT_SAMPLING_FREQUENCY, as_float)
    }

    /// The `Channels`.
    #[must_use]
    pub fn channels(&self) -> Option<u64> {
        child!(self, ids::CHANNELS, as_unsigned)
    }

    /// The `BitDepth`.
    #[must_use]
    pub fn bit_depth(&self) -> Option<u64> {
        child!(self, ids::BIT_DEPTH, as_unsigned)
    }
}

impl<'a> ContentEncodings<'a> {
    /// Every `ContentEncoding`, in application order.
    pub fn encodings(&self) -> impl Iterator<Item = &ContentEncoding<'a>> + '_ {
        self.children()
            .iter()
            .filter_map(AnyElement::as_content_encoding)
    }
}

impl ContentEncoding<'_> {
    /// The `ContentEncodingOrder`.
    #[must_use]
    pub fn order(&self) -> Option<u64> {
        child!(self, ids::CONTENT_ENCODING_ORDER, as_unsigned)
    }

    /// The `ContentEncodingScope`.
    #[must_use]
    pub fn scope(&self) -> Option<u64> {
        child!(self, ids::CONTENT_ENCODING_SCOPE, as_unsigned)
    }

    /// The `ContentEncodingType`.
    #[must_use]
    pub fn encoding_type(&self) -> Option<u64> {
        child!(self, ids::CONTENT_ENCODING_TYPE, as_unsigned)
    }

    /// The `ContentCompression`, when present.
    #[must_use]
    pub fn compression(&self) -> Option<&ContentCompression<'_>> {
        child!(self, ids::CONTENT_COMPRESSION, as_content_compression)
    }

    /// The `ContentEncryption`, when present.
    #[must_use]
    pub fn encryption(&self) -> Option<&ContentEncryption<'_>> {
        child!(self, ids::CONTENT_ENCRYPTION, as_content_encryption)
    }
}

impl ContentCompression<'_> {
    /// The decoded `ContentCompAlgo`.
    #[must_use]
    pub fn algorithm(&self) -> Option<ContentCompAlgo> {
        child!(self, ids::CONTENT_COMP_ALGO, as_unsigned).and_then(ContentCompAlgo::from_u64)
    }

    /// The `ContentCompSettings`.
    #[must_use]
    pub fn settings(&self) -> Option<&[u8]> {
        child!(self, ids::CONTENT_COMP_SETTINGS, as_binary).map(Binary::bytes)
    }
}

impl ContentEncryption<'_> {
    /// The decoded `ContentEncAlgo`.
    #[must_use]
    pub fn algorithm(&self) -> Option<ContentEncAlgo> {
        child!(self, ids::CONTENT_ENC_ALGO, as_unsigned).and_then(ContentEncAlgo::from_u64)
    }

    /// The `ContentEncKeyID`.
    #[must_use]
    pub fn key_id(&self) -> Option<&[u8]> {
        child!(self, ids::CONTENT_ENC_KEY_ID, as_binary).map(Binary::bytes)
    }

    /// The `ContentEncAESSettings`, when present.
    #[must_use]
    pub fn aes_settings(&self) -> Option<&ContentEncAesSettings<'_>> {
        child!(
            self,
            ids::CONTENT_ENC_AES_SETTINGS,
            as_content_enc_aes_settings
        )
    }
}

impl ContentEncAesSettings<'_> {
    /// The decoded `AESSettingsCipherMode`.
    #[must_use]
    pub fn cipher_mode(&self) -> Option<AesSettingsCipherMode> {
        child!(self, ids::AES_SETTINGS_CIPHER_MODE, as_unsigned)
            .and_then(AesSettingsCipherMode::from_u64)
    }
}

impl<'a> Cluster<'a> {
    /// The Cluster `Timestamp` in Matroska Ticks.
    #[must_use]
    pub fn timestamp(&self) -> Option<u64> {
        child!(self, ids::TIMESTAMP, as_unsigned)
    }

    /// Every `SimpleBlock`, in file order.
    pub fn simple_blocks(&self) -> impl Iterator<Item = Result<Block<'a>>> + '_ {
        self.children()
            .iter()
            .filter(|element| element.id() == ids::SIMPLE_BLOCK)
            .filter_map(AnyElement::as_block)
    }

    /// Every `BlockGroup`, in file order.
    pub fn block_groups(&self) -> impl Iterator<Item = &BlockGroup<'a>> + '_ {
        self.children()
            .iter()
            .filter_map(AnyElement::as_block_group)
    }
}

impl<'a> BlockGroup<'a> {
    /// The `Block` payload.
    ///
    /// `None` when absent, `Some(Err(..))` when malformed.
    #[must_use]
    pub fn block(&self) -> Option<Result<Block<'a>>> {
        self.find(ids::BLOCK).and_then(AnyElement::as_block)
    }

    /// The `BlockDuration`.
    #[must_use]
    pub fn block_duration(&self) -> Option<u64> {
        child!(self, ids::BLOCK_DURATION, as_unsigned)
    }

    /// The `DiscardPadding` in nanoseconds.
    #[must_use]
    pub fn discard_padding(&self) -> Option<i64> {
        self.find(ids::DISCARD_PADDING)
            .and_then(AnyElement::as_signed)
    }

    /// The `ReferenceBlock` timestamps, relative to the Cluster `Timestamp`.
    pub fn reference_blocks(&self) -> impl Iterator<Item = i64> + '_ {
        self.children()
            .iter()
            .filter(|element| element.id() == ids::REFERENCE_BLOCK)
            .filter_map(AnyElement::as_signed)
    }
}

impl<'a> Cues<'a> {
    /// Every `CuePoint`.
    pub fn points(&self) -> impl Iterator<Item = &CuePoint<'a>> + '_ {
        self.children().iter().filter_map(AnyElement::as_cue_point)
    }
}

impl<'a> CuePoint<'a> {
    /// The `CueTime` in Matroska Ticks.
    #[must_use]
    pub fn time(&self) -> Option<u64> {
        child!(self, ids::CUE_TIME, as_unsigned)
    }

    /// Every `CueTrackPositions`.
    pub fn track_positions(&self) -> impl Iterator<Item = &CueTrackPositions<'a>> + '_ {
        self.children()
            .iter()
            .filter_map(AnyElement::as_cue_track_positions)
    }
}

impl CueTrackPositions<'_> {
    /// The `CueTrack`.
    #[must_use]
    pub fn track(&self) -> Option<u64> {
        child!(self, ids::CUE_TRACK, as_unsigned)
    }

    /// The `CueClusterPosition` relative to the Segment's data start.
    #[must_use]
    pub fn cluster_position(&self) -> Option<u64> {
        child!(self, ids::CUE_CLUSTER_POSITION, as_unsigned)
    }

    /// The `CueRelativePosition`.
    #[must_use]
    pub fn relative_position(&self) -> Option<u64> {
        child!(self, ids::CUE_RELATIVE_POSITION, as_unsigned)
    }

    /// The `CueDuration` in Matroska Ticks.
    #[must_use]
    pub fn duration(&self) -> Option<u64> {
        child!(self, ids::CUE_DURATION, as_unsigned)
    }
}

impl<'a> Attachments<'a> {
    /// Every `AttachedFile`.
    pub fn files(&self) -> impl Iterator<Item = &AttachedFile<'a>> + '_ {
        self.children()
            .iter()
            .filter_map(AnyElement::as_attached_file)
    }
}

impl<'a> AttachedFile<'a> {
    /// The `FileName`.
    #[must_use]
    pub fn name(&self) -> Option<&'a str> {
        child!(self, ids::FILE_NAME, as_text).and_then(Text::as_str)
    }

    /// The `FileDescription`.
    #[must_use]
    pub fn description(&self) -> Option<&'a str> {
        child!(self, ids::FILE_DESCRIPTION, as_text).and_then(Text::as_str)
    }

    /// The `FileMediaType`.
    #[must_use]
    pub fn media_type(&self) -> Option<&'a str> {
        child!(self, ids::FILE_MEDIA_TYPE, as_text).and_then(Text::as_str)
    }

    /// The `FileData`.
    #[must_use]
    pub fn data(&self) -> Option<&'a [u8]> {
        child!(self, ids::FILE_DATA, as_binary).map(Binary::bytes)
    }

    /// The `FileUID`.
    #[must_use]
    pub fn uid(&self) -> Option<u64> {
        child!(self, ids::FILE_UID, as_unsigned)
    }
}

impl<'a> Chapters<'a> {
    /// Every `EditionEntry`.
    pub fn editions(&self) -> impl Iterator<Item = &EditionEntry<'a>> + '_ {
        self.children()
            .iter()
            .filter_map(AnyElement::as_edition_entry)
    }
}

impl<'a> EditionEntry<'a> {
    /// The `EditionUID`.
    #[must_use]
    pub fn uid(&self) -> Option<u64> {
        child!(self, ids::EDITION_UID, as_unsigned)
    }

    /// Every `ChapterAtom`.
    pub fn atoms(&self) -> impl Iterator<Item = &ChapterAtom<'a>> + '_ {
        self.children()
            .iter()
            .filter_map(AnyElement::as_chapter_atom)
    }
}

impl<'a> ChapterAtom<'a> {
    /// The `ChapterUID`.
    #[must_use]
    pub fn uid(&self) -> Option<u64> {
        child!(self, ids::CHAPTER_UID, as_unsigned)
    }

    /// The `ChapterTimeStart` in nanoseconds.
    #[must_use]
    pub fn time_start(&self) -> Option<u64> {
        child!(self, ids::CHAPTER_TIME_START, as_unsigned)
    }

    /// The `ChapterTimeEnd` in nanoseconds.
    #[must_use]
    pub fn time_end(&self) -> Option<u64> {
        child!(self, ids::CHAPTER_TIME_END, as_unsigned)
    }

    /// Every `ChapterDisplay`.
    pub fn displays(&self) -> impl Iterator<Item = &ChapterDisplay<'a>> + '_ {
        self.children()
            .iter()
            .filter_map(AnyElement::as_chapter_display)
    }

    /// Every nested `ChapterAtom`.
    pub fn atoms(&self) -> impl Iterator<Item = &ChapterAtom<'a>> + '_ {
        self.children()
            .iter()
            .filter_map(AnyElement::as_chapter_atom)
    }
}

impl<'a> ChapterDisplay<'a> {
    /// The `ChapString`.
    #[must_use]
    pub fn string(&self) -> Option<&'a str> {
        child!(self, ids::CHAP_STRING, as_text).and_then(Text::as_str)
    }
}

impl<'a> Tags<'a> {
    /// Every `Tag`.
    pub fn tags(&self) -> impl Iterator<Item = &Tag<'a>> + '_ {
        self.children().iter().filter_map(AnyElement::as_tag)
    }
}

impl<'a> Tag<'a> {
    /// The `Targets`, when present.
    #[must_use]
    pub fn targets(&self) -> Option<&Targets<'a>> {
        child!(self, ids::TARGETS, as_targets)
    }

    /// Every `SimpleTag`.
    pub fn simple_tags(&self) -> impl Iterator<Item = &SimpleTag<'a>> + '_ {
        self.children().iter().filter_map(AnyElement::as_simple_tag)
    }
}

impl<'a> SimpleTag<'a> {
    /// The `TagName`.
    #[must_use]
    pub fn name(&self) -> Option<&'a str> {
        child!(self, ids::TAG_NAME, as_text).and_then(Text::as_str)
    }

    /// The `TagString`.
    #[must_use]
    pub fn string(&self) -> Option<&'a str> {
        child!(self, ids::TAG_STRING, as_text).and_then(Text::as_str)
    }

    /// The `TagBinary`.
    #[must_use]
    pub fn binary(&self) -> Option<&'a [u8]> {
        child!(self, ids::TAG_BINARY, as_binary).map(Binary::bytes)
    }

    /// Every nested `SimpleTag`.
    pub fn children_tags(&self) -> impl Iterator<Item = &SimpleTag<'a>> + '_ {
        self.children().iter().filter_map(AnyElement::as_simple_tag)
    }
}
