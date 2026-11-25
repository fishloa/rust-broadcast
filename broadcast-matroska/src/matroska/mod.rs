//! The Matroska element model (RFC 9559): ids, typed elements, blocks.
//!
//! The entry points are [`Document`] (the EBML header and the Segment) and
//! [`Segment`]; every other element is reached through [`Element`]'s typed
//! accessors in [`accessors`].

pub mod accessors;
pub mod block;
pub mod element;
pub mod ids;
pub mod types;

pub use accessors::DEFAULT_TIMESTAMP_SCALE;
pub use block::{Block, BlockHeader, LaceHeader, Lacing};
pub use element::{
    AttachedFile, Attachments, Audio, BlockAdditionMapping, BlockAdditions, BlockGroup, BlockMore,
    ChapProcess, ChapProcessCommand, ChapterAtom, ChapterDisplay, ChapterTranslate, Chapters,
    Cluster, Colour, ContentCompression, ContentEncAesSettings, ContentEncoding, ContentEncodings,
    ContentEncryption, CuePoint, CueReference, CueTrackPositions, Cues, Document, EditionEntry,
    Element, Info, MasteringMetadata, Projection, ReferenceFrame, Seek, SeekHead, Segment,
    SilentTracks, SimpleTag, Slices, Tag, Tags, Targets, TimeSlice, TrackCombinePlanes, TrackEntry,
    TrackJoinBlocks, TrackOperation, TrackPlane, TrackTranslate, Tracks, Video,
};
pub use types::{
    AesSettingsCipherMode, ColourRange, ContentCompAlgo, ContentEncAlgo, DisplayUnit, FieldOrder,
    FlagInterlaced, LacingKind, ProjectionType, StereoMode, TrackType,
};
