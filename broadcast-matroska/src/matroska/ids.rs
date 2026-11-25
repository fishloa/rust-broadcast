//! Every Matroska element definition: its ID, EBML type and parent path.
//!
//! RFC 9559 §5 (element definitions) and §27.1 (the "Matroska Element IDs"
//! registry), transcribed at
//! `private/specs-verified/ietf/specs/rfc9559_matroska.txt`. Appendix A's
//! deprecated elements are included so a legacy file still walks.
//!
//! One [`ElementDef`] table is the single source of truth: the named constants
//! are generated from the same rows, so an id cannot drift from its name, type
//! or parent. `tests/it/ids_drift.rs` pins the table to the private
//! transcription at runtime.

use broadcast_common::impl_spec_display;

/// An element's EBML type (RFC 8794 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ElementKind {
    /// A master element holding child elements.
    Master,
    /// An unsigned integer payload.
    Unsigned,
    /// A signed integer payload.
    Signed,
    /// A four- or eight-octet IEEE 754 float payload.
    Float,
    /// A printable-ASCII string payload.
    String,
    /// A UTF-8 string payload.
    Utf8,
    /// An eight-octet date payload.
    Date,
    /// A binary payload this crate does not interpret.
    Binary,
}

impl ElementKind {
    /// The EBML type's name as RFC 8794 §7 spells it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Unsigned => "uinteger",
            Self::Signed => "integer",
            Self::Float => "float",
            Self::String => "string",
            Self::Utf8 => "utf-8",
            Self::Date => "date",
            Self::Binary => "binary",
        }
    }
}

impl_spec_display!(ElementKind);

/// One row of the element table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementDef {
    /// The element's name as RFC 9559 writes it.
    pub name: &'static str,
    /// The stored Element ID, marker bits included.
    pub id: u32,
    /// The EBML type of its payload.
    pub kind: ElementKind,
    /// The parent's path without the leading backslash, or `""` at the root.
    pub parent: &'static str,
}

/// ChapterDisplay (RFC 9559 §5.1.7.1.4.9).
pub const CHAPTER_DISPLAY: u32 = 0x80;
/// TrackType (RFC 9559 §5.1.4.1.3).
pub const TRACK_TYPE: u32 = 0x83;
/// ChapString (RFC 9559 §5.1.7.1.4.10).
pub const CHAP_STRING: u32 = 0x85;
/// CodecID (RFC 9559 §5.1.4.1.21).
pub const CODEC_ID: u32 = 0x86;
/// FlagDefault (RFC 9559 §5.1.4.1.5).
pub const FLAG_DEFAULT: u32 = 0x88;
/// Slices, deprecated (RFC 9559 Appendix A).
pub const SLICES: u32 = 0x8E;
/// ChapterTimeStart (RFC 9559 §5.1.7.1.4.3).
pub const CHAPTER_TIME_START: u32 = 0x91;
/// ChapterTimeEnd (RFC 9559 §5.1.7.1.4.4).
pub const CHAPTER_TIME_END: u32 = 0x92;
/// CueRefTime (RFC 9559 §5.1.5.1.2.8).
pub const CUE_REF_TIME: u32 = 0x96;
/// CueRefCluster, deprecated (RFC 9559 Appendix A).
pub const CUE_REF_CLUSTER: u32 = 0x97;
/// ChapterFlagHidden (RFC 9559 §5.1.7.1.4.5).
pub const CHAPTER_FLAG_HIDDEN: u32 = 0x98;
/// FlagInterlaced (RFC 9559 §5.1.4.1.28.1).
pub const FLAG_INTERLACED: u32 = 0x9A;
/// BlockDuration (RFC 9559 §5.1.3.5.3).
pub const BLOCK_DURATION: u32 = 0x9B;
/// FlagLacing (RFC 9559 §5.1.4.1.12).
pub const FLAG_LACING: u32 = 0x9C;
/// FieldOrder (RFC 9559 §5.1.4.1.28.2).
pub const FIELD_ORDER: u32 = 0x9D;
/// Channels (RFC 9559 §5.1.4.1.29.3).
pub const CHANNELS: u32 = 0x9F;
/// BlockGroup (RFC 9559 §5.1.3.5).
pub const BLOCK_GROUP: u32 = 0xA0;
/// Block (RFC 9559 §5.1.3.5.1).
pub const BLOCK: u32 = 0xA1;
/// BlockVirtual, deprecated (RFC 9559 Appendix A).
pub const BLOCK_VIRTUAL: u32 = 0xA2;
/// SimpleBlock (RFC 9559 §5.1.3.4).
pub const SIMPLE_BLOCK: u32 = 0xA3;
/// CodecState (RFC 9559 §5.1.3.5.6).
pub const CODEC_STATE: u32 = 0xA4;
/// BlockAdditional (RFC 9559 §5.1.3.5.2.2).
pub const BLOCK_ADDITIONAL: u32 = 0xA5;
/// BlockMore (RFC 9559 §5.1.3.5.2.1).
pub const BLOCK_MORE: u32 = 0xA6;
/// Position (RFC 9559 §5.1.3.2).
pub const POSITION: u32 = 0xA7;
/// CodecDecodeAll, deprecated (RFC 9559 Appendix A).
pub const CODEC_DECODE_ALL: u32 = 0xAA;
/// PrevSize (RFC 9559 §5.1.3.3).
pub const PREV_SIZE: u32 = 0xAB;
/// TrackEntry (RFC 9559 §5.1.4.1).
pub const TRACK_ENTRY: u32 = 0xAE;
/// EncryptedBlock, deprecated (RFC 9559 Appendix A).
pub const ENCRYPTED_BLOCK: u32 = 0xAF;
/// PixelWidth (RFC 9559 §5.1.4.1.28.6).
pub const PIXEL_WIDTH: u32 = 0xB0;
/// CueDuration (RFC 9559 §5.1.5.1.2.4).
pub const CUE_DURATION: u32 = 0xB2;
/// CueTime (RFC 9559 §5.1.5.1.1).
pub const CUE_TIME: u32 = 0xB3;
/// SamplingFrequency (RFC 9559 §5.1.4.1.29.1).
pub const SAMPLING_FREQUENCY: u32 = 0xB5;
/// ChapterAtom (RFC 9559 §5.1.7.1.4).
pub const CHAPTER_ATOM: u32 = 0xB6;
/// CueTrackPositions (RFC 9559 §5.1.5.1.2).
pub const CUE_TRACK_POSITIONS: u32 = 0xB7;
/// FlagEnabled (RFC 9559 §5.1.4.1.4).
pub const FLAG_ENABLED: u32 = 0xB9;
/// PixelHeight (RFC 9559 §5.1.4.1.28.7).
pub const PIXEL_HEIGHT: u32 = 0xBA;
/// CuePoint (RFC 9559 §5.1.5.1).
pub const CUE_POINT: u32 = 0xBB;
/// TrickTrackUID, deprecated (RFC 9559 Appendix A).
pub const TRICK_TRACK_UID: u32 = 0xC0;
/// TrickTrackSegmentUID, deprecated (RFC 9559 Appendix A).
pub const TRICK_TRACK_SEGMENT_UID: u32 = 0xC1;
/// TrickMasterTrackSegmentUID, deprecated (RFC 9559 Appendix A).
pub const TRICK_MASTER_TRACK_SEGMENT_UID: u32 = 0xC4;
/// TrickTrackFlag, deprecated (RFC 9559 Appendix A).
pub const TRICK_TRACK_FLAG: u32 = 0xC6;
/// TrickMasterTrackUID, deprecated (RFC 9559 Appendix A).
pub const TRICK_MASTER_TRACK_UID: u32 = 0xC7;
/// ReferenceFrame, deprecated (RFC 9559 Appendix A).
pub const REFERENCE_FRAME: u32 = 0xC8;
/// ReferenceOffset, deprecated (RFC 9559 Appendix A).
pub const REFERENCE_OFFSET: u32 = 0xC9;
/// ReferenceTimestamp, deprecated (RFC 9559 Appendix A).
pub const REFERENCE_TIMESTAMP: u32 = 0xCA;
/// BlockAdditionID, deprecated (RFC 9559 Appendix A).
pub const BLOCK_ADDITION_ID: u32 = 0xCB;
/// LaceNumber, deprecated (RFC 9559 Appendix A).
pub const LACE_NUMBER: u32 = 0xCC;
/// FrameNumber, deprecated (RFC 9559 Appendix A).
pub const FRAME_NUMBER: u32 = 0xCD;
/// Delay, deprecated (RFC 9559 Appendix A).
pub const DELAY: u32 = 0xCE;
/// SliceDuration, deprecated (RFC 9559 Appendix A).
pub const SLICE_DURATION: u32 = 0xCF;
/// TrackNumber (RFC 9559 §5.1.4.1.1).
pub const TRACK_NUMBER: u32 = 0xD7;
/// CueReference (RFC 9559 §5.1.5.1.2.7).
pub const CUE_REFERENCE: u32 = 0xDB;
/// Video (RFC 9559 §5.1.4.1.28).
pub const VIDEO: u32 = 0xE0;
/// Audio (RFC 9559 §5.1.4.1.29).
pub const AUDIO: u32 = 0xE1;
/// TrackOperation (RFC 9559 §5.1.4.1.30).
pub const TRACK_OPERATION: u32 = 0xE2;
/// TrackCombinePlanes (RFC 9559 §5.1.4.1.30.1).
pub const TRACK_COMBINE_PLANES: u32 = 0xE3;
/// TrackPlane (RFC 9559 §5.1.4.1.30.2).
pub const TRACK_PLANE: u32 = 0xE4;
/// TrackPlaneUID (RFC 9559 §5.1.4.1.30.3).
pub const TRACK_PLANE_UID: u32 = 0xE5;
/// TrackPlaneType (RFC 9559 §5.1.4.1.30.4).
pub const TRACK_PLANE_TYPE: u32 = 0xE6;
/// Timestamp (RFC 9559 §5.1.3.1).
pub const TIMESTAMP: u32 = 0xE7;
/// TimeSlice, deprecated (RFC 9559 Appendix A).
pub const TIME_SLICE: u32 = 0xE8;
/// TrackJoinBlocks (RFC 9559 §5.1.4.1.30.5).
pub const TRACK_JOIN_BLOCKS: u32 = 0xE9;
/// CueCodecState (RFC 9559 §5.1.5.1.2.6).
pub const CUE_CODEC_STATE: u32 = 0xEA;
/// CueRefCodecState, deprecated (RFC 9559 Appendix A).
pub const CUE_REF_CODEC_STATE: u32 = 0xEB;
/// TrackJoinUID (RFC 9559 §5.1.4.1.30.6).
pub const TRACK_JOIN_UID: u32 = 0xED;
/// BlockAddID (RFC 9559 §5.1.3.5.2.3).
pub const BLOCK_ADD_ID: u32 = 0xEE;
/// CueRelativePosition (RFC 9559 §5.1.5.1.2.3).
pub const CUE_RELATIVE_POSITION: u32 = 0xF0;
/// CueClusterPosition (RFC 9559 §5.1.5.1.2.2).
pub const CUE_CLUSTER_POSITION: u32 = 0xF1;
/// CueTrack (RFC 9559 §5.1.5.1.2.1).
pub const CUE_TRACK: u32 = 0xF7;
/// ReferencePriority (RFC 9559 §5.1.3.5.4).
pub const REFERENCE_PRIORITY: u32 = 0xFA;
/// ReferenceBlock (RFC 9559 §5.1.3.5.5).
pub const REFERENCE_BLOCK: u32 = 0xFB;
/// ReferenceVirtual, deprecated (RFC 9559 Appendix A).
pub const REFERENCE_VIRTUAL: u32 = 0xFD;
/// BlockAddIDName (RFC 9559 §5.1.4.1.17.2).
pub const BLOCK_ADD_ID_NAME: u32 = 0x41A4;
/// BlockAdditionMapping (RFC 9559 §5.1.4.1.17).
pub const BLOCK_ADDITION_MAPPING: u32 = 0x41E4;
/// BlockAddIDType (RFC 9559 §5.1.4.1.17.3).
pub const BLOCK_ADD_ID_TYPE: u32 = 0x41E7;
/// BlockAddIDExtraData (RFC 9559 §5.1.4.1.17.4).
pub const BLOCK_ADD_ID_EXTRA_DATA: u32 = 0x41ED;
/// BlockAddIDValue (RFC 9559 §5.1.4.1.17.1).
pub const BLOCK_ADD_ID_VALUE: u32 = 0x41F0;
/// ContentCompAlgo (RFC 9559 §5.1.4.1.31.6).
pub const CONTENT_COMP_ALGO: u32 = 0x4254;
/// ContentCompSettings (RFC 9559 §5.1.4.1.31.7).
pub const CONTENT_COMP_SETTINGS: u32 = 0x4255;
/// ChapLanguage (RFC 9559 §5.1.7.1.4.11).
pub const CHAP_LANGUAGE: u32 = 0x437C;
/// ChapLanguageBCP47 (RFC 9559 §5.1.7.1.4.12).
pub const CHAP_LANGUAGE_BCP47: u32 = 0x437D;
/// ChapCountry (RFC 9559 §5.1.7.1.4.13).
pub const CHAP_COUNTRY: u32 = 0x437E;
/// SegmentFamily (RFC 9559 §5.1.2.7).
pub const SEGMENT_FAMILY: u32 = 0x4444;
/// DateUTC (RFC 9559 §5.1.2.11).
pub const DATE_UTC: u32 = 0x4461;
/// TagLanguage (RFC 9559 §5.1.8.1.2.2).
pub const TAG_LANGUAGE: u32 = 0x447A;
/// TagLanguageBCP47 (RFC 9559 §5.1.8.1.2.3).
pub const TAG_LANGUAGE_BCP47: u32 = 0x447B;
/// TagDefault (RFC 9559 §5.1.8.1.2.4).
pub const TAG_DEFAULT: u32 = 0x4484;
/// TagBinary (RFC 9559 §5.1.8.1.2.6).
pub const TAG_BINARY: u32 = 0x4485;
/// TagString (RFC 9559 §5.1.8.1.2.5).
pub const TAG_STRING: u32 = 0x4487;
/// Duration (RFC 9559 §5.1.2.10).
pub const DURATION: u32 = 0x4489;
/// TagDefaultBogus, deprecated (RFC 9559 Appendix A).
pub const TAG_DEFAULT_BOGUS: u32 = 0x44B4;
/// ChapProcessPrivate (RFC 9559 §5.1.7.1.4.16).
pub const CHAP_PROCESS_PRIVATE: u32 = 0x450D;
/// TagName (RFC 9559 §5.1.8.1.2.1).
pub const TAG_NAME: u32 = 0x45A3;
/// EditionEntry (RFC 9559 §5.1.7.1).
pub const EDITION_ENTRY: u32 = 0x45B9;
/// EditionUID (RFC 9559 §5.1.7.1.1).
pub const EDITION_UID: u32 = 0x45BC;
/// EditionFlagDefault (RFC 9559 §5.1.7.1.2).
pub const EDITION_FLAG_DEFAULT: u32 = 0x45DB;
/// EditionFlagOrdered (RFC 9559 §5.1.7.1.3).
pub const EDITION_FLAG_ORDERED: u32 = 0x45DD;
/// FileData (RFC 9559 §5.1.6.1.4).
pub const FILE_DATA: u32 = 0x465C;
/// FileMediaType (RFC 9559 §5.1.6.1.3).
pub const FILE_MEDIA_TYPE: u32 = 0x4660;
/// FileUsedStartTime, deprecated (RFC 9559 Appendix A).
pub const FILE_USED_START_TIME: u32 = 0x4661;
/// FileUsedEndTime, deprecated (RFC 9559 Appendix A).
pub const FILE_USED_END_TIME: u32 = 0x4662;
/// FileName (RFC 9559 §5.1.6.1.2).
pub const FILE_NAME: u32 = 0x466E;
/// FileReferral, deprecated (RFC 9559 Appendix A).
pub const FILE_REFERRAL: u32 = 0x4675;
/// FileDescription (RFC 9559 §5.1.6.1.1).
pub const FILE_DESCRIPTION: u32 = 0x467E;
/// FileUID (RFC 9559 §5.1.6.1.5).
pub const FILE_UID: u32 = 0x46AE;
/// ContentEncAlgo (RFC 9559 §5.1.4.1.31.9).
pub const CONTENT_ENC_ALGO: u32 = 0x47E1;
/// ContentEncKeyID (RFC 9559 §5.1.4.1.31.10).
pub const CONTENT_ENC_KEY_ID: u32 = 0x47E2;
/// ContentSignature, deprecated (RFC 9559 Appendix A).
pub const CONTENT_SIGNATURE: u32 = 0x47E3;
/// ContentSigKeyID, deprecated (RFC 9559 Appendix A).
pub const CONTENT_SIG_KEY_ID: u32 = 0x47E4;
/// ContentSigAlgo, deprecated (RFC 9559 Appendix A).
pub const CONTENT_SIG_ALGO: u32 = 0x47E5;
/// ContentSigHashAlgo, deprecated (RFC 9559 Appendix A).
pub const CONTENT_SIG_HASH_ALGO: u32 = 0x47E6;
/// ContentEncAESSettings (RFC 9559 §5.1.4.1.31.11).
pub const CONTENT_ENC_AES_SETTINGS: u32 = 0x47E7;
/// AESSettingsCipherMode (RFC 9559 §5.1.4.1.31.12).
pub const AES_SETTINGS_CIPHER_MODE: u32 = 0x47E8;
/// MuxingApp (RFC 9559 §5.1.2.13).
pub const MUXING_APP: u32 = 0x4D80;
/// Seek (RFC 9559 §5.1.1.1).
pub const SEEK: u32 = 0x4DBB;
/// ContentEncodingOrder (RFC 9559 §5.1.4.1.31.2).
pub const CONTENT_ENCODING_ORDER: u32 = 0x5031;
/// ContentEncodingScope (RFC 9559 §5.1.4.1.31.3).
pub const CONTENT_ENCODING_SCOPE: u32 = 0x5032;
/// ContentEncodingType (RFC 9559 §5.1.4.1.31.4).
pub const CONTENT_ENCODING_TYPE: u32 = 0x5033;
/// ContentCompression (RFC 9559 §5.1.4.1.31.5).
pub const CONTENT_COMPRESSION: u32 = 0x5034;
/// ContentEncryption (RFC 9559 §5.1.4.1.31.8).
pub const CONTENT_ENCRYPTION: u32 = 0x5035;
/// CueRefNumber, deprecated (RFC 9559 Appendix A).
pub const CUE_REF_NUMBER: u32 = 0x535F;
/// Name (RFC 9559 §5.1.4.1.18).
pub const NAME: u32 = 0x536E;
/// CueBlockNumber (RFC 9559 §5.1.5.1.2.5).
pub const CUE_BLOCK_NUMBER: u32 = 0x5378;
/// TrackOffset, deprecated (RFC 9559 Appendix A).
pub const TRACK_OFFSET: u32 = 0x537F;
/// SeekID (RFC 9559 §5.1.1.1.1).
pub const SEEK_ID: u32 = 0x53AB;
/// SeekPosition (RFC 9559 §5.1.1.1.2).
pub const SEEK_POSITION: u32 = 0x53AC;
/// StereoMode (RFC 9559 §5.1.4.1.28.3).
pub const STEREO_MODE: u32 = 0x53B8;
/// OldStereoMode (RFC 9559 §5.1.4.1.28.5).
pub const OLD_STEREO_MODE: u32 = 0x53B9;
/// AlphaMode (RFC 9559 §5.1.4.1.28.4).
pub const ALPHA_MODE: u32 = 0x53C0;
/// PixelCropBottom (RFC 9559 §5.1.4.1.28.8).
pub const PIXEL_CROP_BOTTOM: u32 = 0x54AA;
/// DisplayWidth (RFC 9559 §5.1.4.1.28.12).
pub const DISPLAY_WIDTH: u32 = 0x54B0;
/// DisplayUnit (RFC 9559 §5.1.4.1.28.14).
pub const DISPLAY_UNIT: u32 = 0x54B2;
/// AspectRatioType, deprecated (RFC 9559 Appendix A).
pub const ASPECT_RATIO_TYPE: u32 = 0x54B3;
/// DisplayHeight (RFC 9559 §5.1.4.1.28.13).
pub const DISPLAY_HEIGHT: u32 = 0x54BA;
/// PixelCropTop (RFC 9559 §5.1.4.1.28.9).
pub const PIXEL_CROP_TOP: u32 = 0x54BB;
/// PixelCropLeft (RFC 9559 §5.1.4.1.28.10).
pub const PIXEL_CROP_LEFT: u32 = 0x54CC;
/// PixelCropRight (RFC 9559 §5.1.4.1.28.11).
pub const PIXEL_CROP_RIGHT: u32 = 0x54DD;
/// FlagForced (RFC 9559 §5.1.4.1.6).
pub const FLAG_FORCED: u32 = 0x55AA;
/// FlagHearingImpaired (RFC 9559 §5.1.4.1.7).
pub const FLAG_HEARING_IMPAIRED: u32 = 0x55AB;
/// FlagVisualImpaired (RFC 9559 §5.1.4.1.8).
pub const FLAG_VISUAL_IMPAIRED: u32 = 0x55AC;
/// FlagTextDescriptions (RFC 9559 §5.1.4.1.9).
pub const FLAG_TEXT_DESCRIPTIONS: u32 = 0x55AD;
/// FlagOriginal (RFC 9559 §5.1.4.1.10).
pub const FLAG_ORIGINAL: u32 = 0x55AE;
/// FlagCommentary (RFC 9559 §5.1.4.1.11).
pub const FLAG_COMMENTARY: u32 = 0x55AF;
/// Colour (RFC 9559 §5.1.4.1.28.16).
pub const COLOUR: u32 = 0x55B0;
/// MatrixCoefficients (RFC 9559 §5.1.4.1.28.17).
pub const MATRIX_COEFFICIENTS: u32 = 0x55B1;
/// BitsPerChannel (RFC 9559 §5.1.4.1.28.18).
pub const BITS_PER_CHANNEL: u32 = 0x55B2;
/// ChromaSubsamplingHorz (RFC 9559 §5.1.4.1.28.19).
pub const CHROMA_SUBSAMPLING_HORZ: u32 = 0x55B3;
/// ChromaSubsamplingVert (RFC 9559 §5.1.4.1.28.20).
pub const CHROMA_SUBSAMPLING_VERT: u32 = 0x55B4;
/// CbSubsamplingHorz (RFC 9559 §5.1.4.1.28.21).
pub const CB_SUBSAMPLING_HORZ: u32 = 0x55B5;
/// CbSubsamplingVert (RFC 9559 §5.1.4.1.28.22).
pub const CB_SUBSAMPLING_VERT: u32 = 0x55B6;
/// ChromaSitingHorz (RFC 9559 §5.1.4.1.28.23).
pub const CHROMA_SITING_HORZ: u32 = 0x55B7;
/// ChromaSitingVert (RFC 9559 §5.1.4.1.28.24).
pub const CHROMA_SITING_VERT: u32 = 0x55B8;
/// Range (RFC 9559 §5.1.4.1.28.25).
pub const RANGE: u32 = 0x55B9;
/// TransferCharacteristics (RFC 9559 §5.1.4.1.28.26).
pub const TRANSFER_CHARACTERISTICS: u32 = 0x55BA;
/// Primaries (RFC 9559 §5.1.4.1.28.27).
pub const PRIMARIES: u32 = 0x55BB;
/// MaxCLL (RFC 9559 §5.1.4.1.28.28).
pub const MAX_CLL: u32 = 0x55BC;
/// MaxFALL (RFC 9559 §5.1.4.1.28.29).
pub const MAX_FALL: u32 = 0x55BD;
/// MasteringMetadata (RFC 9559 §5.1.4.1.28.30).
pub const MASTERING_METADATA: u32 = 0x55D0;
/// PrimaryRChromaticityX (RFC 9559 §5.1.4.1.28.31).
pub const PRIMARY_R_CHROMATICITY_X: u32 = 0x55D1;
/// PrimaryRChromaticityY (RFC 9559 §5.1.4.1.28.32).
pub const PRIMARY_R_CHROMATICITY_Y: u32 = 0x55D2;
/// PrimaryGChromaticityX (RFC 9559 §5.1.4.1.28.33).
pub const PRIMARY_G_CHROMATICITY_X: u32 = 0x55D3;
/// PrimaryGChromaticityY (RFC 9559 §5.1.4.1.28.34).
pub const PRIMARY_G_CHROMATICITY_Y: u32 = 0x55D4;
/// PrimaryBChromaticityX (RFC 9559 §5.1.4.1.28.35).
pub const PRIMARY_B_CHROMATICITY_X: u32 = 0x55D5;
/// PrimaryBChromaticityY (RFC 9559 §5.1.4.1.28.36).
pub const PRIMARY_B_CHROMATICITY_Y: u32 = 0x55D6;
/// WhitePointChromaticityX (RFC 9559 §5.1.4.1.28.37).
pub const WHITE_POINT_CHROMATICITY_X: u32 = 0x55D7;
/// WhitePointChromaticityY (RFC 9559 §5.1.4.1.28.38).
pub const WHITE_POINT_CHROMATICITY_Y: u32 = 0x55D8;
/// LuminanceMax (RFC 9559 §5.1.4.1.28.39).
pub const LUMINANCE_MAX: u32 = 0x55D9;
/// LuminanceMin (RFC 9559 §5.1.4.1.28.40).
pub const LUMINANCE_MIN: u32 = 0x55DA;
/// MaxBlockAdditionID (RFC 9559 §5.1.4.1.16).
pub const MAX_BLOCK_ADDITION_ID: u32 = 0x55EE;
/// ChapterStringUID (RFC 9559 §5.1.7.1.4.2).
pub const CHAPTER_STRING_UID: u32 = 0x5654;
/// CodecDelay (RFC 9559 §5.1.4.1.25).
pub const CODEC_DELAY: u32 = 0x56AA;
/// SeekPreRoll (RFC 9559 §5.1.4.1.26).
pub const SEEK_PRE_ROLL: u32 = 0x56BB;
/// WritingApp (RFC 9559 §5.1.2.14).
pub const WRITING_APP: u32 = 0x5741;
/// SilentTracks, deprecated (RFC 9559 Appendix A).
pub const SILENT_TRACKS: u32 = 0x5854;
/// SilentTrackNumber, deprecated (RFC 9559 Appendix A).
pub const SILENT_TRACK_NUMBER: u32 = 0x58D7;
/// AttachedFile (RFC 9559 §5.1.6.1).
pub const ATTACHED_FILE: u32 = 0x61A7;
/// ContentEncoding (RFC 9559 §5.1.4.1.31.1).
pub const CONTENT_ENCODING: u32 = 0x6240;
/// BitDepth (RFC 9559 §5.1.4.1.29.4).
pub const BIT_DEPTH: u32 = 0x6264;
/// CodecPrivate (RFC 9559 §5.1.4.1.22).
pub const CODEC_PRIVATE: u32 = 0x63A2;
/// Targets (RFC 9559 §5.1.8.1.1).
pub const TARGETS: u32 = 0x63C0;
/// ChapterPhysicalEquiv (RFC 9559 §5.1.7.1.4.8).
pub const CHAPTER_PHYSICAL_EQUIV: u32 = 0x63C3;
/// TagChapterUID (RFC 9559 §5.1.8.1.1.5).
pub const TAG_CHAPTER_UID: u32 = 0x63C4;
/// TagTrackUID (RFC 9559 §5.1.8.1.1.3).
pub const TAG_TRACK_UID: u32 = 0x63C5;
/// TagAttachmentUID (RFC 9559 §5.1.8.1.1.6).
pub const TAG_ATTACHMENT_UID: u32 = 0x63C6;
/// TagEditionUID (RFC 9559 §5.1.8.1.1.4).
pub const TAG_EDITION_UID: u32 = 0x63C9;
/// TargetType (RFC 9559 §5.1.8.1.1.2).
pub const TARGET_TYPE: u32 = 0x63CA;
/// TrackTranslate (RFC 9559 §5.1.4.1.27).
pub const TRACK_TRANSLATE: u32 = 0x6624;
/// TrackTranslateTrackID (RFC 9559 §5.1.4.1.27.1).
pub const TRACK_TRANSLATE_TRACK_ID: u32 = 0x66A5;
/// TrackTranslateCodec (RFC 9559 §5.1.4.1.27.2).
pub const TRACK_TRANSLATE_CODEC: u32 = 0x66BF;
/// TrackTranslateEditionUID (RFC 9559 §5.1.4.1.27.3).
pub const TRACK_TRANSLATE_EDITION_UID: u32 = 0x66FC;
/// SimpleTag (RFC 9559 §5.1.8.1.2).
pub const SIMPLE_TAG: u32 = 0x67C8;
/// TargetTypeValue (RFC 9559 §5.1.8.1.1.1).
pub const TARGET_TYPE_VALUE: u32 = 0x68CA;
/// ChapProcessCommand (RFC 9559 §5.1.7.1.4.17).
pub const CHAP_PROCESS_COMMAND: u32 = 0x6911;
/// ChapProcessTime (RFC 9559 §5.1.7.1.4.18).
pub const CHAP_PROCESS_TIME: u32 = 0x6922;
/// ChapterTranslate (RFC 9559 §5.1.2.8).
pub const CHAPTER_TRANSLATE: u32 = 0x6924;
/// ChapProcessData (RFC 9559 §5.1.7.1.4.19).
pub const CHAP_PROCESS_DATA: u32 = 0x6933;
/// ChapProcess (RFC 9559 §5.1.7.1.4.14).
pub const CHAP_PROCESS: u32 = 0x6944;
/// ChapProcessCodecID (RFC 9559 §5.1.7.1.4.15).
pub const CHAP_PROCESS_CODEC_ID: u32 = 0x6955;
/// ChapterTranslateID (RFC 9559 §5.1.2.8.1).
pub const CHAPTER_TRANSLATE_ID: u32 = 0x69A5;
/// ChapterTranslateCodec (RFC 9559 §5.1.2.8.2).
pub const CHAPTER_TRANSLATE_CODEC: u32 = 0x69BF;
/// ChapterTranslateEditionUID (RFC 9559 §5.1.2.8.3).
pub const CHAPTER_TRANSLATE_EDITION_UID: u32 = 0x69FC;
/// ContentEncodings (RFC 9559 §5.1.4.1.31).
pub const CONTENT_ENCODINGS: u32 = 0x6D80;
/// MinCache, deprecated (RFC 9559 Appendix A).
pub const MIN_CACHE: u32 = 0x6DE7;
/// MaxCache, deprecated (RFC 9559 Appendix A).
pub const MAX_CACHE: u32 = 0x6DF8;
/// ChapterSegmentUUID (RFC 9559 §5.1.7.1.4.6).
pub const CHAPTER_SEGMENT_UUID: u32 = 0x6E67;
/// ChapterSegmentEditionUID (RFC 9559 §5.1.7.1.4.7).
pub const CHAPTER_SEGMENT_EDITION_UID: u32 = 0x6EBC;
/// TrackOverlay, deprecated (RFC 9559 Appendix A).
pub const TRACK_OVERLAY: u32 = 0x6FAB;
/// Tag (RFC 9559 §5.1.8.1).
pub const TAG: u32 = 0x7373;
/// SegmentFilename (RFC 9559 §5.1.2.2).
pub const SEGMENT_FILENAME: u32 = 0x7384;
/// SegmentUUID (RFC 9559 §5.1.2.1).
pub const SEGMENT_UUID: u32 = 0x73A4;
/// ChapterUID (RFC 9559 §5.1.7.1.4.1).
pub const CHAPTER_UID: u32 = 0x73C4;
/// TrackUID (RFC 9559 §5.1.4.1.2).
pub const TRACK_UID: u32 = 0x73C5;
/// AttachmentLink (RFC 9559 §5.1.4.1.24).
pub const ATTACHMENT_LINK: u32 = 0x7446;
/// BlockAdditions (RFC 9559 §5.1.3.5.2).
pub const BLOCK_ADDITIONS: u32 = 0x75A1;
/// DiscardPadding (RFC 9559 §5.1.3.5.7).
pub const DISCARD_PADDING: u32 = 0x75A2;
/// Projection (RFC 9559 §5.1.4.1.28.41).
pub const PROJECTION: u32 = 0x7670;
/// ProjectionType (RFC 9559 §5.1.4.1.28.42).
pub const PROJECTION_TYPE: u32 = 0x7671;
/// ProjectionPrivate (RFC 9559 §5.1.4.1.28.43).
pub const PROJECTION_PRIVATE: u32 = 0x7672;
/// ProjectionPoseYaw (RFC 9559 §5.1.4.1.28.44).
pub const PROJECTION_POSE_YAW: u32 = 0x7673;
/// ProjectionPosePitch (RFC 9559 §5.1.4.1.28.45).
pub const PROJECTION_POSE_PITCH: u32 = 0x7674;
/// ProjectionPoseRoll (RFC 9559 §5.1.4.1.28.46).
pub const PROJECTION_POSE_ROLL: u32 = 0x7675;
/// OutputSamplingFrequency (RFC 9559 §5.1.4.1.29.2).
pub const OUTPUT_SAMPLING_FREQUENCY: u32 = 0x78B5;
/// Title (RFC 9559 §5.1.2.12).
pub const TITLE: u32 = 0x7BA9;
/// ChannelPositions, deprecated (RFC 9559 Appendix A).
pub const CHANNEL_POSITIONS: u32 = 0x7D7B;
/// Language (RFC 9559 §5.1.4.1.19).
pub const LANGUAGE: u32 = 0x22B59C;
/// LanguageBCP47 (RFC 9559 §5.1.4.1.20).
pub const LANGUAGE_BCP47: u32 = 0x22B59D;
/// TrackTimestampScale (RFC 9559 §5.1.4.1.15).
pub const TRACK_TIMESTAMP_SCALE: u32 = 0x23314F;
/// DefaultDecodedFieldDuration (RFC 9559 §5.1.4.1.14).
pub const DEFAULT_DECODED_FIELD_DURATION: u32 = 0x234E7A;
/// FrameRate, deprecated (RFC 9559 Appendix A).
pub const FRAME_RATE: u32 = 0x2383E3;
/// DefaultDuration (RFC 9559 §5.1.4.1.13).
pub const DEFAULT_DURATION: u32 = 0x23E383;
/// CodecName (RFC 9559 §5.1.4.1.23).
pub const CODEC_NAME: u32 = 0x258688;
/// CodecDownloadURL, deprecated (RFC 9559 Appendix A).
pub const CODEC_DOWNLOAD_URL: u32 = 0x26B240;
/// TimestampScale (RFC 9559 §5.1.2.9).
pub const TIMESTAMP_SCALE: u32 = 0x2AD7B1;
/// UncompressedFourCC (RFC 9559 §5.1.4.1.28.15).
pub const UNCOMPRESSED_FOURCC: u32 = 0x2EB524;
/// GammaValue, deprecated (RFC 9559 Appendix A).
pub const GAMMA_VALUE: u32 = 0x2FB523;
/// CodecSettings, deprecated (RFC 9559 Appendix A).
pub const CODEC_SETTINGS: u32 = 0x3A9697;
/// CodecInfoURL, deprecated (RFC 9559 Appendix A).
pub const CODEC_INFO_URL: u32 = 0x3B4040;
/// PrevFilename (RFC 9559 §5.1.2.4).
pub const PREV_FILENAME: u32 = 0x3C83AB;
/// PrevUUID (RFC 9559 §5.1.2.3).
pub const PREV_UUID: u32 = 0x3CB923;
/// NextFilename (RFC 9559 §5.1.2.6).
pub const NEXT_FILENAME: u32 = 0x3E83BB;
/// NextUUID (RFC 9559 §5.1.2.5).
pub const NEXT_UUID: u32 = 0x3EB923;
/// Chapters (RFC 9559 §5.1.7).
pub const CHAPTERS: u32 = 0x1043A770;
/// SeekHead (RFC 9559 §5.1.1).
pub const SEEK_HEAD: u32 = 0x114D9B74;
/// Tags (RFC 9559 §5.1.8).
pub const TAGS: u32 = 0x1254C367;
/// Info (RFC 9559 §5.1.2).
pub const INFO: u32 = 0x1549A966;
/// Tracks (RFC 9559 §5.1.4).
pub const TRACKS: u32 = 0x1654AE6B;
/// Segment (RFC 9559 §5.1).
pub const SEGMENT: u32 = 0x18538067;
/// Attachments (RFC 9559 §5.1.6).
pub const ATTACHMENTS: u32 = 0x1941A469;
/// Cues (RFC 9559 §5.1.5).
pub const CUES: u32 = 0x1C53BB6B;
/// Cluster (RFC 9559 §5.1.3).
pub const CLUSTER: u32 = 0x1F43B675;

/// Every element this crate names, in id order.
pub const ALL: &[ElementDef] = &[
    ElementDef {
        name: "ChapterDisplay",
        id: CHAPTER_DISPLAY,
        kind: ElementKind::Master,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "TrackType",
        id: TRACK_TYPE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "ChapString",
        id: CHAP_STRING,
        kind: ElementKind::Utf8,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom\ChapterDisplay",
    },
    ElementDef {
        name: "CodecID",
        id: CODEC_ID,
        kind: ElementKind::String,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "FlagDefault",
        id: FLAG_DEFAULT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "Slices",
        id: SLICES,
        kind: ElementKind::Master,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "ChapterTimeStart",
        id: CHAPTER_TIME_START,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "ChapterTimeEnd",
        id: CHAPTER_TIME_END,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "CueRefTime",
        id: CUE_REF_TIME,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions\CueReference",
    },
    ElementDef {
        name: "CueRefCluster",
        id: CUE_REF_CLUSTER,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions\CueReference",
    },
    ElementDef {
        name: "ChapterFlagHidden",
        id: CHAPTER_FLAG_HIDDEN,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "FlagInterlaced",
        id: FLAG_INTERLACED,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "BlockDuration",
        id: BLOCK_DURATION,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "FlagLacing",
        id: FLAG_LACING,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "FieldOrder",
        id: FIELD_ORDER,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "Channels",
        id: CHANNELS,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Audio",
    },
    ElementDef {
        name: "BlockGroup",
        id: BLOCK_GROUP,
        kind: ElementKind::Master,
        parent: r"Segment\Cluster",
    },
    ElementDef {
        name: "Block",
        id: BLOCK,
        kind: ElementKind::Binary,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "BlockVirtual",
        id: BLOCK_VIRTUAL,
        kind: ElementKind::Binary,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "SimpleBlock",
        id: SIMPLE_BLOCK,
        kind: ElementKind::Binary,
        parent: r"Segment\Cluster",
    },
    ElementDef {
        name: "CodecState",
        id: CODEC_STATE,
        kind: ElementKind::Binary,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "BlockAdditional",
        id: BLOCK_ADDITIONAL,
        kind: ElementKind::Binary,
        parent: r"Segment\Cluster\BlockGroup\BlockAdditions\BlockMore",
    },
    ElementDef {
        name: "BlockMore",
        id: BLOCK_MORE,
        kind: ElementKind::Master,
        parent: r"Segment\Cluster\BlockGroup\BlockAdditions",
    },
    ElementDef {
        name: "Position",
        id: POSITION,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster",
    },
    ElementDef {
        name: "CodecDecodeAll",
        id: CODEC_DECODE_ALL,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "PrevSize",
        id: PREV_SIZE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster",
    },
    ElementDef {
        name: "TrackEntry",
        id: TRACK_ENTRY,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks",
    },
    ElementDef {
        name: "EncryptedBlock",
        id: ENCRYPTED_BLOCK,
        kind: ElementKind::Binary,
        parent: r"Segment\Cluster",
    },
    ElementDef {
        name: "PixelWidth",
        id: PIXEL_WIDTH,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "CueDuration",
        id: CUE_DURATION,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions",
    },
    ElementDef {
        name: "CueTime",
        id: CUE_TIME,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint",
    },
    ElementDef {
        name: "SamplingFrequency",
        id: SAMPLING_FREQUENCY,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Audio",
    },
    ElementDef {
        name: "ChapterAtom",
        id: CHAPTER_ATOM,
        kind: ElementKind::Master,
        parent: r"Segment\Chapters\EditionEntry",
    },
    ElementDef {
        name: "CueTrackPositions",
        id: CUE_TRACK_POSITIONS,
        kind: ElementKind::Master,
        parent: r"Segment\Cues\CuePoint",
    },
    ElementDef {
        name: "FlagEnabled",
        id: FLAG_ENABLED,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "PixelHeight",
        id: PIXEL_HEIGHT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "CuePoint",
        id: CUE_POINT,
        kind: ElementKind::Master,
        parent: r"Segment\Cues",
    },
    ElementDef {
        name: "TrickTrackUID",
        id: TRICK_TRACK_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "TrickTrackSegmentUID",
        id: TRICK_TRACK_SEGMENT_UID,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "TrickMasterTrackSegmentUID",
        id: TRICK_MASTER_TRACK_SEGMENT_UID,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "TrickTrackFlag",
        id: TRICK_TRACK_FLAG,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "TrickMasterTrackUID",
        id: TRICK_MASTER_TRACK_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "ReferenceFrame",
        id: REFERENCE_FRAME,
        kind: ElementKind::Master,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "ReferenceOffset",
        id: REFERENCE_OFFSET,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup\ReferenceFrame",
    },
    ElementDef {
        name: "ReferenceTimestamp",
        id: REFERENCE_TIMESTAMP,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup\ReferenceFrame",
    },
    ElementDef {
        name: "BlockAdditionID",
        id: BLOCK_ADDITION_ID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup\Slices\TimeSlice",
    },
    ElementDef {
        name: "LaceNumber",
        id: LACE_NUMBER,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup\Slices\TimeSlice",
    },
    ElementDef {
        name: "FrameNumber",
        id: FRAME_NUMBER,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup\Slices\TimeSlice",
    },
    ElementDef {
        name: "Delay",
        id: DELAY,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup\Slices\TimeSlice",
    },
    ElementDef {
        name: "SliceDuration",
        id: SLICE_DURATION,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup\Slices\TimeSlice",
    },
    ElementDef {
        name: "TrackNumber",
        id: TRACK_NUMBER,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "CueReference",
        id: CUE_REFERENCE,
        kind: ElementKind::Master,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions",
    },
    ElementDef {
        name: "Video",
        id: VIDEO,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "Audio",
        id: AUDIO,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "TrackOperation",
        id: TRACK_OPERATION,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "TrackCombinePlanes",
        id: TRACK_COMBINE_PLANES,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\TrackOperation",
    },
    ElementDef {
        name: "TrackPlane",
        id: TRACK_PLANE,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\TrackOperation\TrackCombinePlanes",
    },
    ElementDef {
        name: "TrackPlaneUID",
        id: TRACK_PLANE_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\TrackOperation\TrackCombinePlanes\TrackPlane",
    },
    ElementDef {
        name: "TrackPlaneType",
        id: TRACK_PLANE_TYPE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\TrackOperation\TrackCombinePlanes\TrackPlane",
    },
    ElementDef {
        name: "Timestamp",
        id: TIMESTAMP,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster",
    },
    ElementDef {
        name: "TimeSlice",
        id: TIME_SLICE,
        kind: ElementKind::Master,
        parent: r"Segment\Cluster\BlockGroup\Slices",
    },
    ElementDef {
        name: "TrackJoinBlocks",
        id: TRACK_JOIN_BLOCKS,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\TrackOperation",
    },
    ElementDef {
        name: "CueCodecState",
        id: CUE_CODEC_STATE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions",
    },
    ElementDef {
        name: "CueRefCodecState",
        id: CUE_REF_CODEC_STATE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions\CueReference",
    },
    ElementDef {
        name: "TrackJoinUID",
        id: TRACK_JOIN_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\TrackOperation\TrackJoinBlocks",
    },
    ElementDef {
        name: "BlockAddID",
        id: BLOCK_ADD_ID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup\BlockAdditions\BlockMore",
    },
    ElementDef {
        name: "CueRelativePosition",
        id: CUE_RELATIVE_POSITION,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions",
    },
    ElementDef {
        name: "CueClusterPosition",
        id: CUE_CLUSTER_POSITION,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions",
    },
    ElementDef {
        name: "CueTrack",
        id: CUE_TRACK,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions",
    },
    ElementDef {
        name: "ReferencePriority",
        id: REFERENCE_PRIORITY,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "ReferenceBlock",
        id: REFERENCE_BLOCK,
        kind: ElementKind::Signed,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "ReferenceVirtual",
        id: REFERENCE_VIRTUAL,
        kind: ElementKind::Signed,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "BlockAddIDName",
        id: BLOCK_ADD_ID_NAME,
        kind: ElementKind::String,
        parent: r"Segment\Tracks\TrackEntry\BlockAdditionMapping",
    },
    ElementDef {
        name: "BlockAdditionMapping",
        id: BLOCK_ADDITION_MAPPING,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "BlockAddIDType",
        id: BLOCK_ADD_ID_TYPE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\BlockAdditionMapping",
    },
    ElementDef {
        name: "BlockAddIDExtraData",
        id: BLOCK_ADD_ID_EXTRA_DATA,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry\BlockAdditionMapping",
    },
    ElementDef {
        name: "BlockAddIDValue",
        id: BLOCK_ADD_ID_VALUE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\BlockAdditionMapping",
    },
    ElementDef {
        name: "ContentCompAlgo",
        id: CONTENT_COMP_ALGO,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentCompression",
    },
    ElementDef {
        name: "ContentCompSettings",
        id: CONTENT_COMP_SETTINGS,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentCompression",
    },
    ElementDef {
        name: "ChapLanguage",
        id: CHAP_LANGUAGE,
        kind: ElementKind::String,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom\ChapterDisplay",
    },
    ElementDef {
        name: "ChapLanguageBCP47",
        id: CHAP_LANGUAGE_BCP47,
        kind: ElementKind::String,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom\ChapterDisplay",
    },
    ElementDef {
        name: "ChapCountry",
        id: CHAP_COUNTRY,
        kind: ElementKind::String,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom\ChapterDisplay",
    },
    ElementDef {
        name: "SegmentFamily",
        id: SEGMENT_FAMILY,
        kind: ElementKind::Binary,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "DateUTC",
        id: DATE_UTC,
        kind: ElementKind::Date,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "TagLanguage",
        id: TAG_LANGUAGE,
        kind: ElementKind::String,
        parent: r"Segment\Tags\Tag\+SimpleTag",
    },
    ElementDef {
        name: "TagLanguageBCP47",
        id: TAG_LANGUAGE_BCP47,
        kind: ElementKind::String,
        parent: r"Segment\Tags\Tag\+SimpleTag",
    },
    ElementDef {
        name: "TagDefault",
        id: TAG_DEFAULT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tags\Tag\+SimpleTag",
    },
    ElementDef {
        name: "TagBinary",
        id: TAG_BINARY,
        kind: ElementKind::Binary,
        parent: r"Segment\Tags\Tag\+SimpleTag",
    },
    ElementDef {
        name: "TagString",
        id: TAG_STRING,
        kind: ElementKind::Utf8,
        parent: r"Segment\Tags\Tag\+SimpleTag",
    },
    ElementDef {
        name: "Duration",
        id: DURATION,
        kind: ElementKind::Float,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "TagDefaultBogus",
        id: TAG_DEFAULT_BOGUS,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tags\Tag\+SimpleTag",
    },
    ElementDef {
        name: "ChapProcessPrivate",
        id: CHAP_PROCESS_PRIVATE,
        kind: ElementKind::Binary,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom\ChapProcess",
    },
    ElementDef {
        name: "TagName",
        id: TAG_NAME,
        kind: ElementKind::Utf8,
        parent: r"Segment\Tags\Tag\+SimpleTag",
    },
    ElementDef {
        name: "EditionEntry",
        id: EDITION_ENTRY,
        kind: ElementKind::Master,
        parent: r"Segment\Chapters",
    },
    ElementDef {
        name: "EditionUID",
        id: EDITION_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry",
    },
    ElementDef {
        name: "EditionFlagDefault",
        id: EDITION_FLAG_DEFAULT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry",
    },
    ElementDef {
        name: "EditionFlagOrdered",
        id: EDITION_FLAG_ORDERED,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry",
    },
    ElementDef {
        name: "FileData",
        id: FILE_DATA,
        kind: ElementKind::Binary,
        parent: r"Segment\Attachments\AttachedFile",
    },
    ElementDef {
        name: "FileMediaType",
        id: FILE_MEDIA_TYPE,
        kind: ElementKind::String,
        parent: r"Segment\Attachments\AttachedFile",
    },
    ElementDef {
        name: "FileUsedStartTime",
        id: FILE_USED_START_TIME,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Attachments\AttachedFile",
    },
    ElementDef {
        name: "FileUsedEndTime",
        id: FILE_USED_END_TIME,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Attachments\AttachedFile",
    },
    ElementDef {
        name: "FileName",
        id: FILE_NAME,
        kind: ElementKind::Utf8,
        parent: r"Segment\Attachments\AttachedFile",
    },
    ElementDef {
        name: "FileReferral",
        id: FILE_REFERRAL,
        kind: ElementKind::Binary,
        parent: r"Segment\Attachments\AttachedFile",
    },
    ElementDef {
        name: "FileDescription",
        id: FILE_DESCRIPTION,
        kind: ElementKind::Utf8,
        parent: r"Segment\Attachments\AttachedFile",
    },
    ElementDef {
        name: "FileUID",
        id: FILE_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Attachments\AttachedFile",
    },
    ElementDef {
        name: "ContentEncAlgo",
        id: CONTENT_ENC_ALGO,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentEncryption",
    },
    ElementDef {
        name: "ContentEncKeyID",
        id: CONTENT_ENC_KEY_ID,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentEncryption",
    },
    ElementDef {
        name: "ContentSignature",
        id: CONTENT_SIGNATURE,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentEncryption",
    },
    ElementDef {
        name: "ContentSigKeyID",
        id: CONTENT_SIG_KEY_ID,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentEncryption",
    },
    ElementDef {
        name: "ContentSigAlgo",
        id: CONTENT_SIG_ALGO,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentEncryption",
    },
    ElementDef {
        name: "ContentSigHashAlgo",
        id: CONTENT_SIG_HASH_ALGO,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentEncryption",
    },
    ElementDef {
        name: "ContentEncAESSettings",
        id: CONTENT_ENC_AES_SETTINGS,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentEncryption",
    },
    ElementDef {
        name: "AESSettingsCipherMode",
        id: AES_SETTINGS_CIPHER_MODE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding\ContentEncryption\ContentEncAESSettings",
    },
    ElementDef {
        name: "MuxingApp",
        id: MUXING_APP,
        kind: ElementKind::Utf8,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "Seek",
        id: SEEK,
        kind: ElementKind::Master,
        parent: r"Segment\SeekHead",
    },
    ElementDef {
        name: "ContentEncodingOrder",
        id: CONTENT_ENCODING_ORDER,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding",
    },
    ElementDef {
        name: "ContentEncodingScope",
        id: CONTENT_ENCODING_SCOPE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding",
    },
    ElementDef {
        name: "ContentEncodingType",
        id: CONTENT_ENCODING_TYPE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding",
    },
    ElementDef {
        name: "ContentCompression",
        id: CONTENT_COMPRESSION,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding",
    },
    ElementDef {
        name: "ContentEncryption",
        id: CONTENT_ENCRYPTION,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings\ContentEncoding",
    },
    ElementDef {
        name: "CueRefNumber",
        id: CUE_REF_NUMBER,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions\CueReference",
    },
    ElementDef {
        name: "Name",
        id: NAME,
        kind: ElementKind::Utf8,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "CueBlockNumber",
        id: CUE_BLOCK_NUMBER,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cues\CuePoint\CueTrackPositions",
    },
    ElementDef {
        name: "TrackOffset",
        id: TRACK_OFFSET,
        kind: ElementKind::Signed,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "SeekID",
        id: SEEK_ID,
        kind: ElementKind::Binary,
        parent: r"Segment\SeekHead\Seek",
    },
    ElementDef {
        name: "SeekPosition",
        id: SEEK_POSITION,
        kind: ElementKind::Unsigned,
        parent: r"Segment\SeekHead\Seek",
    },
    ElementDef {
        name: "StereoMode",
        id: STEREO_MODE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "OldStereoMode",
        id: OLD_STEREO_MODE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "AlphaMode",
        id: ALPHA_MODE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "PixelCropBottom",
        id: PIXEL_CROP_BOTTOM,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "DisplayWidth",
        id: DISPLAY_WIDTH,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "DisplayUnit",
        id: DISPLAY_UNIT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "AspectRatioType",
        id: ASPECT_RATIO_TYPE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "DisplayHeight",
        id: DISPLAY_HEIGHT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "PixelCropTop",
        id: PIXEL_CROP_TOP,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "PixelCropLeft",
        id: PIXEL_CROP_LEFT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "PixelCropRight",
        id: PIXEL_CROP_RIGHT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "FlagForced",
        id: FLAG_FORCED,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "FlagHearingImpaired",
        id: FLAG_HEARING_IMPAIRED,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "FlagVisualImpaired",
        id: FLAG_VISUAL_IMPAIRED,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "FlagTextDescriptions",
        id: FLAG_TEXT_DESCRIPTIONS,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "FlagOriginal",
        id: FLAG_ORIGINAL,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "FlagCommentary",
        id: FLAG_COMMENTARY,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "Colour",
        id: COLOUR,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "MatrixCoefficients",
        id: MATRIX_COEFFICIENTS,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "BitsPerChannel",
        id: BITS_PER_CHANNEL,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "ChromaSubsamplingHorz",
        id: CHROMA_SUBSAMPLING_HORZ,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "ChromaSubsamplingVert",
        id: CHROMA_SUBSAMPLING_VERT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "CbSubsamplingHorz",
        id: CB_SUBSAMPLING_HORZ,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "CbSubsamplingVert",
        id: CB_SUBSAMPLING_VERT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "ChromaSitingHorz",
        id: CHROMA_SITING_HORZ,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "ChromaSitingVert",
        id: CHROMA_SITING_VERT,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "Range",
        id: RANGE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "TransferCharacteristics",
        id: TRANSFER_CHARACTERISTICS,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "Primaries",
        id: PRIMARIES,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "MaxCLL",
        id: MAX_CLL,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "MaxFALL",
        id: MAX_FALL,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "MasteringMetadata",
        id: MASTERING_METADATA,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour",
    },
    ElementDef {
        name: "PrimaryRChromaticityX",
        id: PRIMARY_R_CHROMATICITY_X,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "PrimaryRChromaticityY",
        id: PRIMARY_R_CHROMATICITY_Y,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "PrimaryGChromaticityX",
        id: PRIMARY_G_CHROMATICITY_X,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "PrimaryGChromaticityY",
        id: PRIMARY_G_CHROMATICITY_Y,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "PrimaryBChromaticityX",
        id: PRIMARY_B_CHROMATICITY_X,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "PrimaryBChromaticityY",
        id: PRIMARY_B_CHROMATICITY_Y,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "WhitePointChromaticityX",
        id: WHITE_POINT_CHROMATICITY_X,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "WhitePointChromaticityY",
        id: WHITE_POINT_CHROMATICITY_Y,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "LuminanceMax",
        id: LUMINANCE_MAX,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "LuminanceMin",
        id: LUMINANCE_MIN,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Colour\MasteringMetadata",
    },
    ElementDef {
        name: "MaxBlockAdditionID",
        id: MAX_BLOCK_ADDITION_ID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "ChapterStringUID",
        id: CHAPTER_STRING_UID,
        kind: ElementKind::Utf8,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "CodecDelay",
        id: CODEC_DELAY,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "SeekPreRoll",
        id: SEEK_PRE_ROLL,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "WritingApp",
        id: WRITING_APP,
        kind: ElementKind::Utf8,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "SilentTracks",
        id: SILENT_TRACKS,
        kind: ElementKind::Master,
        parent: r"Segment\Cluster",
    },
    ElementDef {
        name: "SilentTrackNumber",
        id: SILENT_TRACK_NUMBER,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Cluster\SilentTracks",
    },
    ElementDef {
        name: "AttachedFile",
        id: ATTACHED_FILE,
        kind: ElementKind::Master,
        parent: r"Segment\Attachments",
    },
    ElementDef {
        name: "ContentEncoding",
        id: CONTENT_ENCODING,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\ContentEncodings",
    },
    ElementDef {
        name: "BitDepth",
        id: BIT_DEPTH,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Audio",
    },
    ElementDef {
        name: "CodecPrivate",
        id: CODEC_PRIVATE,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "Targets",
        id: TARGETS,
        kind: ElementKind::Master,
        parent: r"Segment\Tags\Tag",
    },
    ElementDef {
        name: "ChapterPhysicalEquiv",
        id: CHAPTER_PHYSICAL_EQUIV,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "TagChapterUID",
        id: TAG_CHAPTER_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tags\Tag\Targets",
    },
    ElementDef {
        name: "TagTrackUID",
        id: TAG_TRACK_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tags\Tag\Targets",
    },
    ElementDef {
        name: "TagAttachmentUID",
        id: TAG_ATTACHMENT_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tags\Tag\Targets",
    },
    ElementDef {
        name: "TagEditionUID",
        id: TAG_EDITION_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tags\Tag\Targets",
    },
    ElementDef {
        name: "TargetType",
        id: TARGET_TYPE,
        kind: ElementKind::String,
        parent: r"Segment\Tags\Tag\Targets",
    },
    ElementDef {
        name: "TrackTranslate",
        id: TRACK_TRANSLATE,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "TrackTranslateTrackID",
        id: TRACK_TRANSLATE_TRACK_ID,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry\TrackTranslate",
    },
    ElementDef {
        name: "TrackTranslateCodec",
        id: TRACK_TRANSLATE_CODEC,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\TrackTranslate",
    },
    ElementDef {
        name: "TrackTranslateEditionUID",
        id: TRACK_TRANSLATE_EDITION_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\TrackTranslate",
    },
    ElementDef {
        name: "SimpleTag",
        id: SIMPLE_TAG,
        kind: ElementKind::Master,
        parent: r"Segment\Tags\Tag",
    },
    ElementDef {
        name: "TargetTypeValue",
        id: TARGET_TYPE_VALUE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tags\Tag\Targets",
    },
    ElementDef {
        name: "ChapProcessCommand",
        id: CHAP_PROCESS_COMMAND,
        kind: ElementKind::Master,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom\ChapProcess",
    },
    ElementDef {
        name: "ChapProcessTime",
        id: CHAP_PROCESS_TIME,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom\ChapProcess\ChapProcessCommand",
    },
    ElementDef {
        name: "ChapterTranslate",
        id: CHAPTER_TRANSLATE,
        kind: ElementKind::Master,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "ChapProcessData",
        id: CHAP_PROCESS_DATA,
        kind: ElementKind::Binary,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom\ChapProcess\ChapProcessCommand",
    },
    ElementDef {
        name: "ChapProcess",
        id: CHAP_PROCESS,
        kind: ElementKind::Master,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "ChapProcessCodecID",
        id: CHAP_PROCESS_CODEC_ID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom\ChapProcess",
    },
    ElementDef {
        name: "ChapterTranslateID",
        id: CHAPTER_TRANSLATE_ID,
        kind: ElementKind::Binary,
        parent: r"Segment\Info\ChapterTranslate",
    },
    ElementDef {
        name: "ChapterTranslateCodec",
        id: CHAPTER_TRANSLATE_CODEC,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Info\ChapterTranslate",
    },
    ElementDef {
        name: "ChapterTranslateEditionUID",
        id: CHAPTER_TRANSLATE_EDITION_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Info\ChapterTranslate",
    },
    ElementDef {
        name: "ContentEncodings",
        id: CONTENT_ENCODINGS,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "MinCache",
        id: MIN_CACHE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "MaxCache",
        id: MAX_CACHE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "ChapterSegmentUUID",
        id: CHAPTER_SEGMENT_UUID,
        kind: ElementKind::Binary,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "ChapterSegmentEditionUID",
        id: CHAPTER_SEGMENT_EDITION_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "TrackOverlay",
        id: TRACK_OVERLAY,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "Tag",
        id: TAG,
        kind: ElementKind::Master,
        parent: r"Segment\Tags",
    },
    ElementDef {
        name: "SegmentFilename",
        id: SEGMENT_FILENAME,
        kind: ElementKind::Utf8,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "SegmentUUID",
        id: SEGMENT_UUID,
        kind: ElementKind::Binary,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "ChapterUID",
        id: CHAPTER_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Chapters\EditionEntry\+ChapterAtom",
    },
    ElementDef {
        name: "TrackUID",
        id: TRACK_UID,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "AttachmentLink",
        id: ATTACHMENT_LINK,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "BlockAdditions",
        id: BLOCK_ADDITIONS,
        kind: ElementKind::Master,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "DiscardPadding",
        id: DISCARD_PADDING,
        kind: ElementKind::Signed,
        parent: r"Segment\Cluster\BlockGroup",
    },
    ElementDef {
        name: "Projection",
        id: PROJECTION,
        kind: ElementKind::Master,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "ProjectionType",
        id: PROJECTION_TYPE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry\Video\Projection",
    },
    ElementDef {
        name: "ProjectionPrivate",
        id: PROJECTION_PRIVATE,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry\Video\Projection",
    },
    ElementDef {
        name: "ProjectionPoseYaw",
        id: PROJECTION_POSE_YAW,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Projection",
    },
    ElementDef {
        name: "ProjectionPosePitch",
        id: PROJECTION_POSE_PITCH,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Projection",
    },
    ElementDef {
        name: "ProjectionPoseRoll",
        id: PROJECTION_POSE_ROLL,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video\Projection",
    },
    ElementDef {
        name: "OutputSamplingFrequency",
        id: OUTPUT_SAMPLING_FREQUENCY,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Audio",
    },
    ElementDef {
        name: "Title",
        id: TITLE,
        kind: ElementKind::Utf8,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "ChannelPositions",
        id: CHANNEL_POSITIONS,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry\Audio",
    },
    ElementDef {
        name: "Language",
        id: LANGUAGE,
        kind: ElementKind::String,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "LanguageBCP47",
        id: LANGUAGE_BCP47,
        kind: ElementKind::String,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "TrackTimestampScale",
        id: TRACK_TIMESTAMP_SCALE,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "DefaultDecodedFieldDuration",
        id: DEFAULT_DECODED_FIELD_DURATION,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "FrameRate",
        id: FRAME_RATE,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "DefaultDuration",
        id: DEFAULT_DURATION,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "CodecName",
        id: CODEC_NAME,
        kind: ElementKind::Utf8,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "CodecDownloadURL",
        id: CODEC_DOWNLOAD_URL,
        kind: ElementKind::String,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "TimestampScale",
        id: TIMESTAMP_SCALE,
        kind: ElementKind::Unsigned,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "UncompressedFourCC",
        id: UNCOMPRESSED_FOURCC,
        kind: ElementKind::Binary,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "GammaValue",
        id: GAMMA_VALUE,
        kind: ElementKind::Float,
        parent: r"Segment\Tracks\TrackEntry\Video",
    },
    ElementDef {
        name: "CodecSettings",
        id: CODEC_SETTINGS,
        kind: ElementKind::Utf8,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "CodecInfoURL",
        id: CODEC_INFO_URL,
        kind: ElementKind::String,
        parent: r"Segment\Tracks\TrackEntry",
    },
    ElementDef {
        name: "PrevFilename",
        id: PREV_FILENAME,
        kind: ElementKind::Utf8,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "PrevUUID",
        id: PREV_UUID,
        kind: ElementKind::Binary,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "NextFilename",
        id: NEXT_FILENAME,
        kind: ElementKind::Utf8,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "NextUUID",
        id: NEXT_UUID,
        kind: ElementKind::Binary,
        parent: r"Segment\Info",
    },
    ElementDef {
        name: "Chapters",
        id: CHAPTERS,
        kind: ElementKind::Master,
        parent: r"Segment",
    },
    ElementDef {
        name: "SeekHead",
        id: SEEK_HEAD,
        kind: ElementKind::Master,
        parent: r"Segment",
    },
    ElementDef {
        name: "Tags",
        id: TAGS,
        kind: ElementKind::Master,
        parent: r"Segment",
    },
    ElementDef {
        name: "Info",
        id: INFO,
        kind: ElementKind::Master,
        parent: r"Segment",
    },
    ElementDef {
        name: "Tracks",
        id: TRACKS,
        kind: ElementKind::Master,
        parent: r"Segment",
    },
    ElementDef {
        name: "Segment",
        id: SEGMENT,
        kind: ElementKind::Master,
        parent: r"",
    },
    ElementDef {
        name: "Attachments",
        id: ATTACHMENTS,
        kind: ElementKind::Master,
        parent: r"Segment",
    },
    ElementDef {
        name: "Cues",
        id: CUES,
        kind: ElementKind::Master,
        parent: r"Segment",
    },
    ElementDef {
        name: "Cluster",
        id: CLUSTER,
        kind: ElementKind::Master,
        parent: r"Segment",
    },
];

/// The definition of `id`, if this crate names it.
#[must_use]
pub fn lookup(id: u32) -> Option<&'static ElementDef> {
    ALL.iter().find(|def| def.id == id)
}

/// The EBML type of `id`, if this crate names it.
#[must_use]
pub fn kind(id: u32) -> Option<ElementKind> {
    lookup(id).map(|def| def.kind)
}

/// Whether `id` is a master element.
#[must_use]
pub fn is_master(id: u32) -> bool {
    matches!(kind(id), Some(ElementKind::Master))
}

/// The Segment's direct children, excluding the global elements. An
/// unknown-size `Cluster` ends at the first of these (RFC 8794 §6.2).
pub const SEGMENT_CHILD_IDS: &[u32] = &[
    SEEK_HEAD,
    INFO,
    CLUSTER,
    TRACKS,
    CUES,
    ATTACHMENTS,
    CHAPTERS,
    TAGS,
];
