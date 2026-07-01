//! `transmux` — ISOBMFF box layer (ISO/IEC 14496-12:2015 §4.2).
//!
//! Parses and serialises ISO Base Media File Format box headers: [`BoxHeader`] with
//! optional 64-bit `largesize` and `uuid` extended type, [`FullBoxHeader`] with
//! `version` + `flags`, and a generic size-driven box walker ([`BoxIter`]) that
//! skips unknown box types by advancing by `size`.
//!
//! # Quick start
//!
//! ```rust
//! use transmux::{box_iter, parse_box, BoxHeader, BoxType};
//! use broadcast_common::Parse;
//!
//! // Parse a simple ftyp box header.
//! let bytes = [0, 0, 0, 12, b'f', b't', b'y', b'p', 0, 0, 0, 0];
//! let header = BoxHeader::parse(&bytes).unwrap();
//! assert_eq!(header.box_type.to_string(), "ftyp");
//! ```
//!
//! # Spec citations
//!
//! - **Box / FullBox**: ISO/IEC 14496-12:2015 §4.2 (fulltext L1254–L1304).
//! - **Box order**: §6.2.3 (recommendations; only `ftyp` before variable-length and
//!   `moof` sequence_number are mandatory).
//! - **Skip-unknown rule**: §4.2 L1294 — unknown type → ignore + skip via size.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | yes     | `std::error::Error` impls |
//! | `serde` | yes     | `serde::Serialize` for box types |

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod aac_asc;
pub mod avc_config;
pub mod box_types;
pub mod error;
pub mod hevc_config;
pub mod init_segment;
pub mod movie_fragment;
pub mod mp4esds;
pub mod nalu_types;
pub mod sample_entries;
pub mod segments;
pub mod timing;

pub use aac_asc::{
    build_adts_header, parse_adts_header, AdtsHeader, AudioObjectType, AudioSpecificConfig,
    ChannelConfiguration, SamplingFrequencyIndex,
};
pub use avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
pub use box_types::{box_iter, parse_box, BoxHeader, BoxIter, BoxRef, BoxType, FullBoxHeader};
pub use error::{Error, Result};
pub use hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
pub use init_segment::{
    ChunkOffsetBox, DataEntryUrlBox, DataInformationBox, DataReferenceBox, EditBox, HandlerBox,
    MediaBox, MediaHeaderBox, MediaInformationBox, MovieBox, MovieExtendsBox, MovieHeaderBox,
    Mp4aSampleEntry, OpaqueBox, SampleDescriptionBox, SampleEntryVariant, SampleSizeBox,
    SampleTableBox, SampleToChunkBox, SoundMediaHeaderBox, StblChild, StscEntry, TrackBox,
    TrackExtendsBox, TrackHeaderBox, VideoMediaHeaderBox,
};
pub use movie_fragment::{
    MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentRunBox, TrunSample,
};
pub use mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType,
};
pub use nalu_types::{AvcPps, AvcSps, AvcSpsExt, HevcNalArray, HevcNalUnit};
pub use sample_entries::{AVCSampleEntry, HEVCSampleEntry};
pub use segments::{FileTypeBox, MediaDataBox, SegmentTypeBox};
pub use timing::{
    CompositionOffsetBox, CompositionToDecodeBox, CttsEntry, EditListBox, EditListEntry,
    SegmentIndexBox, SidxReference, SttsEntry, TimeToSampleBox,
};
