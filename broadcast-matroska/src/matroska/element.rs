//! The Matroska element tree: one universal [`Element`] carrying every typed
//! master and leaf, and the segment-level document root.
//!
//! RFC 9559 §5 defines each element; this module types every master element so
//! a caller reaches a field through the element it belongs to, and keeps the
//! child order the file has. An id the crate does not name stays
//! [`Unknown`] with its bytes.

use alloc::vec::Vec;

use broadcast_core::{Parse, Serialize};

use crate::ebml::ElementHeader;
use crate::ebml::document::EbmlHeader;
use crate::ebml::global::{CRC32_ID, Crc32, VOID_ID, Void};
use crate::ebml::leaf::{
    BinaryElement, DateElement, FloatElement, SignedElement, TextElement, Unknown, UnsignedElement,
};
use crate::ebml::scalar::{Binary, Date, Float, Signed, Text, Unsigned};
use crate::error::{Error, Result};
use crate::limits::Limits;
use crate::matroska::ids;

/// Build the shared master-element struct and its parse/serialize impls.
macro_rules! master_elements {
    ($($ty:ident => $konst:ident),* $(,)?) => {
        $(
            #[doc = concat!("A master element (RFC 9559 §5, `", stringify!($konst), "`).")]
            #[derive(Debug, Clone, PartialEq)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize))]
            pub struct $ty<'a> {
                header: ElementHeader,
                children: Vec<Element<'a>>,
            }

            impl<'a> $ty<'a> {
                /// Build one from a parsed header and children.
                #[must_use]
                pub const fn from_parts(header: ElementHeader, children: Vec<Element<'a>>) -> Self {
                    Self { header, children }
                }

                /// The element header.
                #[must_use]
                pub const fn header(&self) -> ElementHeader {
                    self.header
                }

                /// The child elements, in file order.
                #[must_use]
                pub fn children(&self) -> &[Element<'a>] {
                    &self.children
                }

                /// The first child with `id`.
                #[must_use]
                pub fn find(&self, id: u32) -> Option<&Element<'a>> {
                    self.children
                        .iter()
                        .find(|element| element.header().id().value() == id)
                }

                /// Every child with `id`.
                pub fn find_all<'s>(
                    &'s self,
                    id: u32,
                ) -> impl Iterator<Item = &'s Element<'a>> + 's {
                    self.children
                        .iter()
                        .filter(move |element| element.header().id().value() == id)
                }
            }

            impl<'a> Parse<'a> for $ty<'a> {
                type Error = Error;

                fn parse(bytes: &'a [u8]) -> Result<Self> {
                    let (header, children, _) =
                        parse_master_children(bytes, ids::$konst, &Limits::new())?;
                    Ok(Self::from_parts(header, children))
                }
            }

            impl Serialize for $ty<'_> {
                type Error = Error;

                fn serialized_len(&self) -> usize {
                    master_serialized_len(self.header, &self.children)
                }

                fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                    write_master(self.header, &self.children, buf)
                }
            }
        )*
    };
}

master_elements! {
    Segment => SEGMENT,
    SeekHead => SEEK_HEAD,
    Seek => SEEK,
    Info => INFO,
    ChapterTranslate => CHAPTER_TRANSLATE,
    Cluster => CLUSTER,
    SilentTracks => SILENT_TRACKS,
    BlockGroup => BLOCK_GROUP,
    BlockAdditions => BLOCK_ADDITIONS,
    BlockMore => BLOCK_MORE,
    Slices => SLICES,
    TimeSlice => TIME_SLICE,
    ReferenceFrame => REFERENCE_FRAME,
    Tracks => TRACKS,
    TrackEntry => TRACK_ENTRY,
    BlockAdditionMapping => BLOCK_ADDITION_MAPPING,
    TrackTranslate => TRACK_TRANSLATE,
    Video => VIDEO,
    Colour => COLOUR,
    MasteringMetadata => MASTERING_METADATA,
    Projection => PROJECTION,
    Audio => AUDIO,
    TrackOperation => TRACK_OPERATION,
    TrackCombinePlanes => TRACK_COMBINE_PLANES,
    TrackPlane => TRACK_PLANE,
    TrackJoinBlocks => TRACK_JOIN_BLOCKS,
    ContentEncodings => CONTENT_ENCODINGS,
    ContentEncoding => CONTENT_ENCODING,
    ContentCompression => CONTENT_COMPRESSION,
    ContentEncryption => CONTENT_ENCRYPTION,
    ContentEncAesSettings => CONTENT_ENC_AES_SETTINGS,
    Cues => CUES,
    CuePoint => CUE_POINT,
    CueTrackPositions => CUE_TRACK_POSITIONS,
    CueReference => CUE_REFERENCE,
    Attachments => ATTACHMENTS,
    AttachedFile => ATTACHED_FILE,
    Chapters => CHAPTERS,
    EditionEntry => EDITION_ENTRY,
    ChapterAtom => CHAPTER_ATOM,
    ChapterDisplay => CHAPTER_DISPLAY,
    ChapProcess => CHAP_PROCESS,
    ChapProcessCommand => CHAP_PROCESS_COMMAND,
    Tags => TAGS,
    Tag => TAG,
    Targets => TARGETS,
    SimpleTag => SIMPLE_TAG,
}

/// One Matroska element: a typed master, a typed scalar leaf, a global element,
/// or an untyped [`Unknown`] (RFC 9559 §5).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Element<'a> {
    /// A `Segment` master element.
    Segment(Segment<'a>),
    /// A `SeekHead` master element.
    SeekHead(SeekHead<'a>),
    /// A `Seek` master element.
    Seek(Seek<'a>),
    /// A `Info` master element.
    Info(Info<'a>),
    /// A `ChapterTranslate` master element.
    ChapterTranslate(ChapterTranslate<'a>),
    /// A `Cluster` master element.
    Cluster(Cluster<'a>),
    /// A `SilentTracks` master element.
    SilentTracks(SilentTracks<'a>),
    /// A `BlockGroup` master element.
    BlockGroup(BlockGroup<'a>),
    /// A `BlockAdditions` master element.
    BlockAdditions(BlockAdditions<'a>),
    /// A `BlockMore` master element.
    BlockMore(BlockMore<'a>),
    /// A `Slices` master element.
    Slices(Slices<'a>),
    /// A `TimeSlice` master element.
    TimeSlice(TimeSlice<'a>),
    /// A `ReferenceFrame` master element.
    ReferenceFrame(ReferenceFrame<'a>),
    /// A `Tracks` master element.
    Tracks(Tracks<'a>),
    /// A `TrackEntry` master element.
    TrackEntry(TrackEntry<'a>),
    /// A `BlockAdditionMapping` master element.
    BlockAdditionMapping(BlockAdditionMapping<'a>),
    /// A `TrackTranslate` master element.
    TrackTranslate(TrackTranslate<'a>),
    /// A `Video` master element.
    Video(Video<'a>),
    /// A `Colour` master element.
    Colour(Colour<'a>),
    /// A `MasteringMetadata` master element.
    MasteringMetadata(MasteringMetadata<'a>),
    /// A `Projection` master element.
    Projection(Projection<'a>),
    /// A `Audio` master element.
    Audio(Audio<'a>),
    /// A `TrackOperation` master element.
    TrackOperation(TrackOperation<'a>),
    /// A `TrackCombinePlanes` master element.
    TrackCombinePlanes(TrackCombinePlanes<'a>),
    /// A `TrackPlane` master element.
    TrackPlane(TrackPlane<'a>),
    /// A `TrackJoinBlocks` master element.
    TrackJoinBlocks(TrackJoinBlocks<'a>),
    /// A `ContentEncodings` master element.
    ContentEncodings(ContentEncodings<'a>),
    /// A `ContentEncoding` master element.
    ContentEncoding(ContentEncoding<'a>),
    /// A `ContentCompression` master element.
    ContentCompression(ContentCompression<'a>),
    /// A `ContentEncryption` master element.
    ContentEncryption(ContentEncryption<'a>),
    /// A `ContentEncAesSettings` master element.
    ContentEncAesSettings(ContentEncAesSettings<'a>),
    /// A `Cues` master element.
    Cues(Cues<'a>),
    /// A `CuePoint` master element.
    CuePoint(CuePoint<'a>),
    /// A `CueTrackPositions` master element.
    CueTrackPositions(CueTrackPositions<'a>),
    /// A `CueReference` master element.
    CueReference(CueReference<'a>),
    /// A `Attachments` master element.
    Attachments(Attachments<'a>),
    /// A `AttachedFile` master element.
    AttachedFile(AttachedFile<'a>),
    /// A `Chapters` master element.
    Chapters(Chapters<'a>),
    /// A `EditionEntry` master element.
    EditionEntry(EditionEntry<'a>),
    /// A `ChapterAtom` master element.
    ChapterAtom(ChapterAtom<'a>),
    /// A `ChapterDisplay` master element.
    ChapterDisplay(ChapterDisplay<'a>),
    /// A `ChapProcess` master element.
    ChapProcess(ChapProcess<'a>),
    /// A `ChapProcessCommand` master element.
    ChapProcessCommand(ChapProcessCommand<'a>),
    /// A `Tags` master element.
    Tags(Tags<'a>),
    /// A `Tag` master element.
    Tag(Tag<'a>),
    /// A `Targets` master element.
    Targets(Targets<'a>),
    /// A `SimpleTag` master element.
    SimpleTag(SimpleTag<'a>),
    /// An unsigned integer payload (RFC 8794 §7.2).
    Unsigned(UnsignedElement),
    /// A signed integer payload (RFC 8794 §7.1).
    Signed(SignedElement),
    /// A float payload (RFC 8794 §7.3).
    Float(FloatElement),
    /// A string or UTF-8 payload (RFC 8794 §7.4, §7.5).
    Text(TextElement<'a>),
    /// A date payload (RFC 8794 §7.6).
    Date(DateElement),
    /// A binary payload (RFC 8794 §7.8).
    Binary(BinaryElement<'a>),
    /// The global `CRC-32` element (RFC 8794 §11.3.1).
    Crc32(Crc32),
    /// The global `Void` element (RFC 8794 §11.3.2).
    Void(Void<'a>),
    /// An element this crate does not name.
    Unknown(Unknown<'a>),
}

impl<'a> Element<'a> {
    /// The element header.
    #[must_use]
    pub fn header(&self) -> ElementHeader {
        match self {
            Self::Segment(e) => e.header(),
            Self::SeekHead(e) => e.header(),
            Self::Seek(e) => e.header(),
            Self::Info(e) => e.header(),
            Self::ChapterTranslate(e) => e.header(),
            Self::Cluster(e) => e.header(),
            Self::SilentTracks(e) => e.header(),
            Self::BlockGroup(e) => e.header(),
            Self::BlockAdditions(e) => e.header(),
            Self::BlockMore(e) => e.header(),
            Self::Slices(e) => e.header(),
            Self::TimeSlice(e) => e.header(),
            Self::ReferenceFrame(e) => e.header(),
            Self::Tracks(e) => e.header(),
            Self::TrackEntry(e) => e.header(),
            Self::BlockAdditionMapping(e) => e.header(),
            Self::TrackTranslate(e) => e.header(),
            Self::Video(e) => e.header(),
            Self::Colour(e) => e.header(),
            Self::MasteringMetadata(e) => e.header(),
            Self::Projection(e) => e.header(),
            Self::Audio(e) => e.header(),
            Self::TrackOperation(e) => e.header(),
            Self::TrackCombinePlanes(e) => e.header(),
            Self::TrackPlane(e) => e.header(),
            Self::TrackJoinBlocks(e) => e.header(),
            Self::ContentEncodings(e) => e.header(),
            Self::ContentEncoding(e) => e.header(),
            Self::ContentCompression(e) => e.header(),
            Self::ContentEncryption(e) => e.header(),
            Self::ContentEncAesSettings(e) => e.header(),
            Self::Cues(e) => e.header(),
            Self::CuePoint(e) => e.header(),
            Self::CueTrackPositions(e) => e.header(),
            Self::CueReference(e) => e.header(),
            Self::Attachments(e) => e.header(),
            Self::AttachedFile(e) => e.header(),
            Self::Chapters(e) => e.header(),
            Self::EditionEntry(e) => e.header(),
            Self::ChapterAtom(e) => e.header(),
            Self::ChapterDisplay(e) => e.header(),
            Self::ChapProcess(e) => e.header(),
            Self::ChapProcessCommand(e) => e.header(),
            Self::Tags(e) => e.header(),
            Self::Tag(e) => e.header(),
            Self::Targets(e) => e.header(),
            Self::SimpleTag(e) => e.header(),
            Self::Unsigned(e) => e.header(),
            Self::Signed(e) => e.header(),
            Self::Float(e) => e.header(),
            Self::Text(e) => e.header(),
            Self::Date(e) => e.header(),
            Self::Binary(e) => e.header(),
            Self::Crc32(e) => e.header(),
            Self::Void(e) => e.header(),
            Self::Unknown(e) => e.header(),
        }
    }

    /// The element's stored id (marker bits included).
    #[must_use]
    pub fn id(&self) -> u32 {
        self.header().id().value()
    }

    /// The element's declared payload length, or `None` for an unknown-size
    /// master.
    #[must_use]
    pub fn payload_len(&self) -> Option<u64> {
        self.header().payload_len()
    }

    /// The master's children, in file order.
    #[must_use]
    pub fn children(&self) -> Option<&[Element<'a>]> {
        match self {
            Self::Segment(e) => Some(e.children()),
            Self::SeekHead(e) => Some(e.children()),
            Self::Seek(e) => Some(e.children()),
            Self::Info(e) => Some(e.children()),
            Self::ChapterTranslate(e) => Some(e.children()),
            Self::Cluster(e) => Some(e.children()),
            Self::SilentTracks(e) => Some(e.children()),
            Self::BlockGroup(e) => Some(e.children()),
            Self::BlockAdditions(e) => Some(e.children()),
            Self::BlockMore(e) => Some(e.children()),
            Self::Slices(e) => Some(e.children()),
            Self::TimeSlice(e) => Some(e.children()),
            Self::ReferenceFrame(e) => Some(e.children()),
            Self::Tracks(e) => Some(e.children()),
            Self::TrackEntry(e) => Some(e.children()),
            Self::BlockAdditionMapping(e) => Some(e.children()),
            Self::TrackTranslate(e) => Some(e.children()),
            Self::Video(e) => Some(e.children()),
            Self::Colour(e) => Some(e.children()),
            Self::MasteringMetadata(e) => Some(e.children()),
            Self::Projection(e) => Some(e.children()),
            Self::Audio(e) => Some(e.children()),
            Self::TrackOperation(e) => Some(e.children()),
            Self::TrackCombinePlanes(e) => Some(e.children()),
            Self::TrackPlane(e) => Some(e.children()),
            Self::TrackJoinBlocks(e) => Some(e.children()),
            Self::ContentEncodings(e) => Some(e.children()),
            Self::ContentEncoding(e) => Some(e.children()),
            Self::ContentCompression(e) => Some(e.children()),
            Self::ContentEncryption(e) => Some(e.children()),
            Self::ContentEncAesSettings(e) => Some(e.children()),
            Self::Cues(e) => Some(e.children()),
            Self::CuePoint(e) => Some(e.children()),
            Self::CueTrackPositions(e) => Some(e.children()),
            Self::CueReference(e) => Some(e.children()),
            Self::Attachments(e) => Some(e.children()),
            Self::AttachedFile(e) => Some(e.children()),
            Self::Chapters(e) => Some(e.children()),
            Self::EditionEntry(e) => Some(e.children()),
            Self::ChapterAtom(e) => Some(e.children()),
            Self::ChapterDisplay(e) => Some(e.children()),
            Self::ChapProcess(e) => Some(e.children()),
            Self::ChapProcessCommand(e) => Some(e.children()),
            Self::Tags(e) => Some(e.children()),
            Self::Tag(e) => Some(e.children()),
            Self::Targets(e) => Some(e.children()),
            Self::SimpleTag(e) => Some(e.children()),
            _ => None,
        }
    }

    /// The first child with `id`, when this is a master.
    #[must_use]
    pub fn find(&self, id: u32) -> Option<&Element<'a>> {
        self.children()?
            .iter()
            .find(|element| element.header().id().value() == id)
    }

    /// Every child with `id`, when this is a master.
    pub fn find_all<'s>(&'s self, id: u32) -> impl Iterator<Item = &'s Element<'a>> + 's {
        self.children()
            .unwrap_or(&[])
            .iter()
            .filter(move |element| element.header().id().value() == id)
    }

    /// The unsigned integer payload.
    #[must_use]
    pub fn as_unsigned(&self) -> Option<u64> {
        match self {
            Self::Unsigned(e) => Some(e.value().value()),
            _ => None,
        }
    }

    /// The signed integer payload.
    #[must_use]
    pub fn as_signed(&self) -> Option<i64> {
        match self {
            Self::Signed(e) => Some(e.value().value()),
            _ => None,
        }
    }

    /// The float payload.
    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(e) => Some(e.value().value()),
            _ => None,
        }
    }

    /// The string or UTF-8 payload.
    #[must_use]
    pub fn as_text(&self) -> Option<Text<'a>> {
        match self {
            Self::Text(e) => Some(e.value()),
            _ => None,
        }
    }

    /// The date payload.
    #[must_use]
    pub fn as_date(&self) -> Option<Date> {
        match self {
            Self::Date(e) => Some(e.value()),
            _ => None,
        }
    }

    /// The binary payload.
    #[must_use]
    pub fn as_binary(&self) -> Option<Binary<'a>> {
        match self {
            Self::Binary(e) => Some(e.value()),
            _ => None,
        }
    }

    /// Parse one element from the front of `bytes` under `limits`, returning it
    /// and the octets it occupies.
    pub fn parse_at(bytes: &'a [u8], limits: &Limits) -> Result<(Self, usize)> {
        Self::parse_at_depth(bytes, limits, 0)
    }

    /// Parse one element at a given tree depth.
    fn parse_at_depth(bytes: &'a [u8], limits: &Limits, depth: u32) -> Result<(Self, usize)> {
        if depth > limits.max_depth() {
            return Err(Error::LimitExceeded {
                limit: "max_depth",
                value: u64::from(depth),
            });
        }
        let (header, header_len) = ElementHeader::parse(bytes)?;
        let id = header.id().value();
        let Some(payload_len) = header.payload_len() else {
            return parse_unknown_sized(bytes, header, header_len, limits, depth);
        };
        if payload_len > limits.max_element_bytes() {
            return Err(Error::LimitExceeded {
                limit: "max_element_bytes",
                value: payload_len,
            });
        }
        let payload_len = usize::try_from(payload_len).map_err(|_| Error::SizeOverflow)?;
        let end = header_len
            .checked_add(payload_len)
            .ok_or(Error::SizeOverflow)?;
        let payload = bytes.get(header_len..end).ok_or(Error::ElementOverruns {
            id,
            declared: payload_len as u64,
            available: bytes.len().saturating_sub(header_len),
        })?;
        let element = dispatch(header, payload, limits, depth)?;
        Ok((element, end))
    }
}

/// Parse an unknown-size `Segment` or `Cluster` (RFC 8794 §6.2, RFC 9559 §6).
fn parse_unknown_sized<'a>(
    bytes: &'a [u8],
    header: ElementHeader,
    header_len: usize,
    limits: &Limits,
    depth: u32,
) -> Result<(Element<'a>, usize)> {
    let id = header.id().value();
    let rest = bytes.get(header_len..).ok_or(Error::Truncated {
        what: "unknown-size element",
        need: header_len,
        have: bytes.len(),
    })?;
    if id == ids::SEGMENT {
        let (children, used) = parse_children_until(rest, limits, depth + 1, &[])?;
        return Ok((
            Element::Segment(Segment::from_parts(header, children)),
            header_len + used,
        ));
    }
    if id == ids::CLUSTER {
        let (children, used) =
            parse_children_until(rest, limits, depth + 1, ids::SEGMENT_CHILD_IDS)?;
        return Ok((
            Element::Cluster(Cluster::from_parts(header, children)),
            header_len + used,
        ));
    }
    Err(Error::UnknownSizeNotAllowed { id })
}

/// Parse a known-size master's children, checking its id.
fn parse_master_children<'a>(
    bytes: &'a [u8],
    expected_id: u32,
    limits: &Limits,
) -> Result<(ElementHeader, Vec<Element<'a>>, usize)> {
    let (header, header_len) = ElementHeader::parse(bytes)?;
    if header.id().value() != expected_id {
        return Err(Error::UnexpectedElement {
            expected: expected_id,
            found: header.id().value(),
        });
    }
    let stop_ids = if expected_id == ids::CLUSTER {
        ids::SEGMENT_CHILD_IDS
    } else {
        &[]
    };
    let Some(payload_len) = header.payload_len() else {
        if expected_id != ids::SEGMENT && expected_id != ids::CLUSTER {
            return Err(Error::UnknownSizeNotAllowed { id: expected_id });
        }
        let rest = bytes.get(header_len..).ok_or(Error::Truncated {
            what: "unknown-size master",
            need: header_len,
            have: bytes.len(),
        })?;
        let (children, used) = parse_children_until(rest, limits, 1, stop_ids)?;
        return Ok((header, children, header_len + used));
    };
    let payload_len = usize::try_from(payload_len).map_err(|_| Error::SizeOverflow)?;
    let end = header_len
        .checked_add(payload_len)
        .ok_or(Error::SizeOverflow)?;
    let payload = bytes.get(header_len..end).ok_or(Error::ElementOverruns {
        id: expected_id,
        declared: payload_len as u64,
        available: bytes.len().saturating_sub(header_len),
    })?;
    let (children, _) = parse_children_until(payload, limits, 1, &[])?;
    Ok((header, children, end))
}

/// Parse children until the payload ends or a stop id is seen.
fn parse_children_until<'a>(
    payload: &'a [u8],
    limits: &Limits,
    depth: u32,
    stop_ids: &[u32],
) -> Result<(Vec<Element<'a>>, usize)> {
    let mut children = Vec::new();
    let mut position = 0usize;
    while position < payload.len() {
        let rest = payload.get(position..).ok_or(Error::SizeOverflow)?;
        let (next_id, _) = crate::ebml::ElementId::parse(rest)?;
        if stop_ids.contains(&next_id.value()) {
            break;
        }
        let (element, used) = Element::parse_at_depth(rest, limits, depth)?;
        if used == 0 {
            return Err(Error::InvalidField(broadcast_core::error::InvalidField {
                field: "element",
                reason: "zero-length advance",
            }));
        }
        position = position.checked_add(used).ok_or(Error::SizeOverflow)?;
        children.push(element);
        if children.len() > limits.max_children() as usize {
            return Err(Error::LimitExceeded {
                limit: "max_children",
                value: children.len() as u64,
            });
        }
    }
    Ok((children, position))
}

/// Build the typed element for a known-size header and payload.
fn dispatch<'a>(
    header: ElementHeader,
    payload: &'a [u8],
    limits: &Limits,
    depth: u32,
) -> Result<Element<'a>> {
    let id = header.id().value();
    match id {
        ids::SEGMENT => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Segment(Segment::from_parts(header, children)))
        }
        ids::SEEK_HEAD => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::SeekHead(SeekHead::from_parts(header, children)))
        }
        ids::SEEK => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Seek(Seek::from_parts(header, children)))
        }
        ids::INFO => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Info(Info::from_parts(header, children)))
        }
        ids::CHAPTER_TRANSLATE => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ChapterTranslate(ChapterTranslate::from_parts(
                header, children,
            )))
        }
        ids::CLUSTER => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Cluster(Cluster::from_parts(header, children)))
        }
        ids::SILENT_TRACKS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::SilentTracks(SilentTracks::from_parts(
                header, children,
            )))
        }
        ids::BLOCK_GROUP => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::BlockGroup(BlockGroup::from_parts(
                header, children,
            )))
        }
        ids::BLOCK_ADDITIONS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::BlockAdditions(BlockAdditions::from_parts(
                header, children,
            )))
        }
        ids::BLOCK_MORE => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::BlockMore(BlockMore::from_parts(header, children)))
        }
        ids::SLICES => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Slices(Slices::from_parts(header, children)))
        }
        ids::TIME_SLICE => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::TimeSlice(TimeSlice::from_parts(header, children)))
        }
        ids::REFERENCE_FRAME => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ReferenceFrame(ReferenceFrame::from_parts(
                header, children,
            )))
        }
        ids::TRACKS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Tracks(Tracks::from_parts(header, children)))
        }
        ids::TRACK_ENTRY => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::TrackEntry(TrackEntry::from_parts(
                header, children,
            )))
        }
        ids::BLOCK_ADDITION_MAPPING => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::BlockAdditionMapping(
                BlockAdditionMapping::from_parts(header, children),
            ))
        }
        ids::TRACK_TRANSLATE => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::TrackTranslate(TrackTranslate::from_parts(
                header, children,
            )))
        }
        ids::VIDEO => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Video(Video::from_parts(header, children)))
        }
        ids::COLOUR => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Colour(Colour::from_parts(header, children)))
        }
        ids::MASTERING_METADATA => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::MasteringMetadata(MasteringMetadata::from_parts(
                header, children,
            )))
        }
        ids::PROJECTION => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Projection(Projection::from_parts(
                header, children,
            )))
        }
        ids::AUDIO => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Audio(Audio::from_parts(header, children)))
        }
        ids::TRACK_OPERATION => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::TrackOperation(TrackOperation::from_parts(
                header, children,
            )))
        }
        ids::TRACK_COMBINE_PLANES => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::TrackCombinePlanes(TrackCombinePlanes::from_parts(
                header, children,
            )))
        }
        ids::TRACK_PLANE => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::TrackPlane(TrackPlane::from_parts(
                header, children,
            )))
        }
        ids::TRACK_JOIN_BLOCKS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::TrackJoinBlocks(TrackJoinBlocks::from_parts(
                header, children,
            )))
        }
        ids::CONTENT_ENCODINGS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ContentEncodings(ContentEncodings::from_parts(
                header, children,
            )))
        }
        ids::CONTENT_ENCODING => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ContentEncoding(ContentEncoding::from_parts(
                header, children,
            )))
        }
        ids::CONTENT_COMPRESSION => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ContentCompression(ContentCompression::from_parts(
                header, children,
            )))
        }
        ids::CONTENT_ENCRYPTION => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ContentEncryption(ContentEncryption::from_parts(
                header, children,
            )))
        }
        ids::CONTENT_ENC_AES_SETTINGS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ContentEncAesSettings(
                ContentEncAesSettings::from_parts(header, children),
            ))
        }
        ids::CUES => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Cues(Cues::from_parts(header, children)))
        }
        ids::CUE_POINT => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::CuePoint(CuePoint::from_parts(header, children)))
        }
        ids::CUE_TRACK_POSITIONS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::CueTrackPositions(CueTrackPositions::from_parts(
                header, children,
            )))
        }
        ids::CUE_REFERENCE => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::CueReference(CueReference::from_parts(
                header, children,
            )))
        }
        ids::ATTACHMENTS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Attachments(Attachments::from_parts(
                header, children,
            )))
        }
        ids::ATTACHED_FILE => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::AttachedFile(AttachedFile::from_parts(
                header, children,
            )))
        }
        ids::CHAPTERS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Chapters(Chapters::from_parts(header, children)))
        }
        ids::EDITION_ENTRY => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::EditionEntry(EditionEntry::from_parts(
                header, children,
            )))
        }
        ids::CHAPTER_ATOM => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ChapterAtom(ChapterAtom::from_parts(
                header, children,
            )))
        }
        ids::CHAPTER_DISPLAY => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ChapterDisplay(ChapterDisplay::from_parts(
                header, children,
            )))
        }
        ids::CHAP_PROCESS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ChapProcess(ChapProcess::from_parts(
                header, children,
            )))
        }
        ids::CHAP_PROCESS_COMMAND => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::ChapProcessCommand(ChapProcessCommand::from_parts(
                header, children,
            )))
        }
        ids::TAGS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Tags(Tags::from_parts(header, children)))
        }
        ids::TAG => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Tag(Tag::from_parts(header, children)))
        }
        ids::TARGETS => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::Targets(Targets::from_parts(header, children)))
        }
        ids::SIMPLE_TAG => {
            let (children, _) = parse_children_until(payload, limits, depth + 1, &[])?;
            Ok(Element::SimpleTag(SimpleTag::from_parts(header, children)))
        }
        CRC32_ID => Ok(Element::Crc32(Crc32::from_payload(header, payload)?)),
        VOID_ID => Ok(Element::Void(Void::new(header, payload)?)),
        _ => match ids::kind(id) {
            Some(ids::ElementKind::Unsigned) => Ok(Element::Unsigned(UnsignedElement::new(
                header,
                Unsigned::parse(payload)?,
            ))),
            Some(ids::ElementKind::Signed) => Ok(Element::Signed(SignedElement::new(
                header,
                Signed::parse(payload)?,
            ))),
            Some(ids::ElementKind::Float) => Ok(Element::Float(FloatElement::new(
                header,
                Float::parse(payload)?,
            ))),
            Some(ids::ElementKind::String | ids::ElementKind::Utf8) => Ok(Element::Text(
                TextElement::new(header, Text::parse(payload)?),
            )),
            Some(ids::ElementKind::Date) => Ok(Element::Date(DateElement::new(
                header,
                Date::parse(payload)?,
            ))),
            Some(ids::ElementKind::Binary) => Ok(Element::Binary(BinaryElement::new(
                header,
                Binary::parse(payload)?,
            ))),
            Some(ids::ElementKind::Master) => {
                Err(Error::InvalidField(broadcast_core::error::InvalidField {
                    field: "element",
                    reason: "master element with no typed variant",
                }))
            }
            None => Ok(Element::Unknown(Unknown::new(header, payload))),
        },
    }
}

/// The serialized length of a master element.
fn master_serialized_len(header: ElementHeader, children: &[Element<'_>]) -> usize {
    let payload: usize = children.iter().map(Serialize::serialized_len).sum();
    let header_len = u64::try_from(payload)
        .ok()
        .and_then(|len| header.resized(len).ok())
        .map_or_else(|| header.serialized_len(), |header| header.serialized_len());
    header_len + payload
}

/// Write a master's header and children, returning the octets written.
fn write_master(header: ElementHeader, children: &[Element<'_>], buf: &mut [u8]) -> Result<usize> {
    let payload: usize = children.iter().map(Serialize::serialized_len).sum();
    let payload_len = u64::try_from(payload).map_err(|_| Error::SizeOverflow)?;
    let header = header.resized(payload_len)?;
    let mut used = header.write(buf)?;
    for child in children {
        let have = buf.len();
        let rest = buf.get_mut(used..).ok_or(Error::OutputBufferTooSmall(
            broadcast_core::error::OutputBufferTooSmall {
                need: used + child.serialized_len(),
                have,
            },
        ))?;
        used += child.serialize_into(rest)?;
    }
    Ok(used)
}

impl<'a> Parse<'a> for Element<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self::parse_at(bytes, &Limits::new())?.0)
    }
}

impl Serialize for Element<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Segment(e) => e.serialized_len(),
            Self::SeekHead(e) => e.serialized_len(),
            Self::Seek(e) => e.serialized_len(),
            Self::Info(e) => e.serialized_len(),
            Self::ChapterTranslate(e) => e.serialized_len(),
            Self::Cluster(e) => e.serialized_len(),
            Self::SilentTracks(e) => e.serialized_len(),
            Self::BlockGroup(e) => e.serialized_len(),
            Self::BlockAdditions(e) => e.serialized_len(),
            Self::BlockMore(e) => e.serialized_len(),
            Self::Slices(e) => e.serialized_len(),
            Self::TimeSlice(e) => e.serialized_len(),
            Self::ReferenceFrame(e) => e.serialized_len(),
            Self::Tracks(e) => e.serialized_len(),
            Self::TrackEntry(e) => e.serialized_len(),
            Self::BlockAdditionMapping(e) => e.serialized_len(),
            Self::TrackTranslate(e) => e.serialized_len(),
            Self::Video(e) => e.serialized_len(),
            Self::Colour(e) => e.serialized_len(),
            Self::MasteringMetadata(e) => e.serialized_len(),
            Self::Projection(e) => e.serialized_len(),
            Self::Audio(e) => e.serialized_len(),
            Self::TrackOperation(e) => e.serialized_len(),
            Self::TrackCombinePlanes(e) => e.serialized_len(),
            Self::TrackPlane(e) => e.serialized_len(),
            Self::TrackJoinBlocks(e) => e.serialized_len(),
            Self::ContentEncodings(e) => e.serialized_len(),
            Self::ContentEncoding(e) => e.serialized_len(),
            Self::ContentCompression(e) => e.serialized_len(),
            Self::ContentEncryption(e) => e.serialized_len(),
            Self::ContentEncAesSettings(e) => e.serialized_len(),
            Self::Cues(e) => e.serialized_len(),
            Self::CuePoint(e) => e.serialized_len(),
            Self::CueTrackPositions(e) => e.serialized_len(),
            Self::CueReference(e) => e.serialized_len(),
            Self::Attachments(e) => e.serialized_len(),
            Self::AttachedFile(e) => e.serialized_len(),
            Self::Chapters(e) => e.serialized_len(),
            Self::EditionEntry(e) => e.serialized_len(),
            Self::ChapterAtom(e) => e.serialized_len(),
            Self::ChapterDisplay(e) => e.serialized_len(),
            Self::ChapProcess(e) => e.serialized_len(),
            Self::ChapProcessCommand(e) => e.serialized_len(),
            Self::Tags(e) => e.serialized_len(),
            Self::Tag(e) => e.serialized_len(),
            Self::Targets(e) => e.serialized_len(),
            Self::SimpleTag(e) => e.serialized_len(),
            Self::Unsigned(e) => e.serialized_len(),
            Self::Signed(e) => e.serialized_len(),
            Self::Float(e) => e.serialized_len(),
            Self::Text(e) => e.serialized_len(),
            Self::Date(e) => e.serialized_len(),
            Self::Binary(e) => e.serialized_len(),
            Self::Crc32(e) => e.serialized_len(),
            Self::Void(e) => e.serialized_len(),
            Self::Unknown(e) => e.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Segment(e) => e.serialize_into(buf),
            Self::SeekHead(e) => e.serialize_into(buf),
            Self::Seek(e) => e.serialize_into(buf),
            Self::Info(e) => e.serialize_into(buf),
            Self::ChapterTranslate(e) => e.serialize_into(buf),
            Self::Cluster(e) => e.serialize_into(buf),
            Self::SilentTracks(e) => e.serialize_into(buf),
            Self::BlockGroup(e) => e.serialize_into(buf),
            Self::BlockAdditions(e) => e.serialize_into(buf),
            Self::BlockMore(e) => e.serialize_into(buf),
            Self::Slices(e) => e.serialize_into(buf),
            Self::TimeSlice(e) => e.serialize_into(buf),
            Self::ReferenceFrame(e) => e.serialize_into(buf),
            Self::Tracks(e) => e.serialize_into(buf),
            Self::TrackEntry(e) => e.serialize_into(buf),
            Self::BlockAdditionMapping(e) => e.serialize_into(buf),
            Self::TrackTranslate(e) => e.serialize_into(buf),
            Self::Video(e) => e.serialize_into(buf),
            Self::Colour(e) => e.serialize_into(buf),
            Self::MasteringMetadata(e) => e.serialize_into(buf),
            Self::Projection(e) => e.serialize_into(buf),
            Self::Audio(e) => e.serialize_into(buf),
            Self::TrackOperation(e) => e.serialize_into(buf),
            Self::TrackCombinePlanes(e) => e.serialize_into(buf),
            Self::TrackPlane(e) => e.serialize_into(buf),
            Self::TrackJoinBlocks(e) => e.serialize_into(buf),
            Self::ContentEncodings(e) => e.serialize_into(buf),
            Self::ContentEncoding(e) => e.serialize_into(buf),
            Self::ContentCompression(e) => e.serialize_into(buf),
            Self::ContentEncryption(e) => e.serialize_into(buf),
            Self::ContentEncAesSettings(e) => e.serialize_into(buf),
            Self::Cues(e) => e.serialize_into(buf),
            Self::CuePoint(e) => e.serialize_into(buf),
            Self::CueTrackPositions(e) => e.serialize_into(buf),
            Self::CueReference(e) => e.serialize_into(buf),
            Self::Attachments(e) => e.serialize_into(buf),
            Self::AttachedFile(e) => e.serialize_into(buf),
            Self::Chapters(e) => e.serialize_into(buf),
            Self::EditionEntry(e) => e.serialize_into(buf),
            Self::ChapterAtom(e) => e.serialize_into(buf),
            Self::ChapterDisplay(e) => e.serialize_into(buf),
            Self::ChapProcess(e) => e.serialize_into(buf),
            Self::ChapProcessCommand(e) => e.serialize_into(buf),
            Self::Tags(e) => e.serialize_into(buf),
            Self::Tag(e) => e.serialize_into(buf),
            Self::Targets(e) => e.serialize_into(buf),
            Self::SimpleTag(e) => e.serialize_into(buf),
            Self::Unsigned(e) => e.serialize_into(buf),
            Self::Signed(e) => e.serialize_into(buf),
            Self::Float(e) => e.serialize_into(buf),
            Self::Text(e) => e.serialize_into(buf),
            Self::Date(e) => e.serialize_into(buf),
            Self::Binary(e) => e.serialize_into(buf),
            Self::Crc32(e) => e.serialize_into(buf),
            Self::Void(e) => e.serialize_into(buf),
            Self::Unknown(e) => e.serialize_into(buf),
        }
    }
}

/// One EBML document: the header and the Segment after it (RFC 8794 §8).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Document<'a> {
    header: EbmlHeader<'a>,
    segment: Segment<'a>,
}

impl<'a> Document<'a> {
    /// Build one from a parsed header and segment.
    #[must_use]
    pub const fn from_parts(header: EbmlHeader<'a>, segment: Segment<'a>) -> Self {
        Self { header, segment }
    }

    /// The EBML header.
    #[must_use]
    pub const fn header(&self) -> &EbmlHeader<'a> {
        &self.header
    }

    /// The Segment.
    #[must_use]
    pub const fn segment(&self) -> &Segment<'a> {
        &self.segment
    }
}

impl<'a> Parse<'a> for Document<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, used) = crate::ebml::document::parse_document_start(bytes)?;
        let rest = bytes.get(used..).ok_or(Error::Truncated {
            what: "Segment",
            need: used,
            have: bytes.len(),
        })?;
        let segment = Segment::parse(rest)?;
        Ok(Self::from_parts(header, segment))
    }
}

impl Serialize for Document<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.header.serialized_len() + self.segment.serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let used = self.header.serialize_into(buf)?;
        let have = buf.len();
        let rest = buf.get_mut(used..).ok_or(Error::OutputBufferTooSmall(
            broadcast_core::error::OutputBufferTooSmall {
                need: used + self.segment.serialized_len(),
                have,
            },
        ))?;
        Ok(used + self.segment.serialize_into(rest)?)
    }
}
