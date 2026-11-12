//! FLV (Flash Video) container spoke — Adobe Flash Video File Format
//! Specification **v10.1, Annex E**; see `private/specs-verified/mpeg/transmux/codec/flv.md`.
//!
//! FLV is a hub spoke like [`TsDemux`](crate::TsDemux) / [`WebmDemux`](crate::WebmDemux):
//! [`FlvDemux`] ([`Unpackage`]) parses an FLV byte stream into the neutral
//! [`Media`] IR, and [`FlvMux`] ([`Package`]) serialises a [`Media`] back into
//! FLV. transmux carries the FLV mainstream — **H.264/AVC video + AAC audio**
//! (Annex E §E.4.3.2 / §E.4.2.2) — reusing the existing
//! [`CodecConfig::Avc`] / [`CodecConfig::Aac`]; no new codec variant.
//!
//! **Unlike the TS demux** (which carries every other `stream_type` as an
//! opaque [`CodecConfig::Data`] track so nothing is dropped), a video tag
//! whose `CodecID` isn't AVC, or an audio tag whose `SoundFormat` isn't AAC,
//! is **silently skipped** here — no track, no error, no other signal that
//! data was discarded (see the `continue` sites in
//! [`FlvDemux::unpackage`](Unpackage::unpackage)).
//!
//! # Layout (Adobe FLV v10.1 Annex E)
//!
//! - **Header** (§E.2, 9 bytes) + first `PreviousTagSize0` (4 bytes, = 0):
//!   `"FLV"` signature, version, `TypeFlags` (bit0 audio, bit2 video), and the
//!   `DataOffset`. All fields are **big-endian**.
//! - **Tags** (§E.4.1): repeated `[Tag][PreviousTagSize]`. A tag is an 11-byte
//!   header (`TagType` UI8, `DataSize` UI24, `Timestamp` UI24 ms +
//!   `TimestampExtended` UI8 high byte, `StreamID` UI24 = 0) then `DataSize`
//!   body bytes; the trailing `PreviousTagSize` UI32 is the whole tag size
//!   (11 + `DataSize`).
//! - **Video tag** (§E.4.3): `VideoTagHeader` = `FrameType` (`UB[4]`) +
//!   `CodecID` (`UB[4]`). For `CodecID == 7` (AVC) the body is an
//!   **AVCVIDEOPACKET** (§E.4.3.2): `AVCPacketType` UI8 (0 = sequence header /
//!   `avcC`, 1 = NALU, 2 = end of sequence) + `CompositionTime` SI24 +
//!   payload. Tag `Timestamp` is the **DTS**; `PTS = DTS + CompositionTime`.
//! - **Audio tag** (§E.4.2): `AudioTagHeader` = `SoundFormat` (`UB[4]`) +
//!   `SoundRate` (`UB[2]`) + `SoundSize` (`UB[1]`) + `SoundType` (`UB[1]`). For
//!   `SoundFormat == 10` (AAC) the body is **AACAUDIODATA** (§E.4.2.2):
//!   `AACPacketType` UI8 (0 = `AudioSpecificConfig`, 1 = raw AAC frame) +
//!   payload.
//! - **Script data** (§E.4.1, `TagType == 18`): `onMetaData`; informational,
//!   parsed leniently (skipped) on demux and emitted minimally on mux.
//!
//! `no_std` + `alloc`.

use crate::TrackId;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use broadcast_common::error::BufferTooShort;
use broadcast_common::len::need_in;
use core::fmt;
use core::marker::PhantomData;
use rtmp_runtime::RtmpError;
use rtmp_runtime::amf0::{Amf0Value, amf0_values};
use rtmp_runtime::flv::FlvHeader;
use rtmp_runtime::flv::FlvTag as RtmpFlvTag;
use rtmp_runtime::flv::TagType;

use broadcast_common::timescale::{Rounding, rescale_i64};
use broadcast_common::{Package, Parse, Serialize, Unpackage};

use crate::aac_asc::AudioSpecificConfig;
use crate::annexb::{NAL_LENGTH_SIZE, NAL_LENGTH_SIZE_MINUS_ONE, normalise_nal_length_size};
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::ir::InputDegradation;
use crate::media::{Media, Track};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};

// ---------------------------------------------------------------------------
// Spec constants (Adobe FLV v10.1 Annex E) — no magic numbers outside tests.
// ---------------------------------------------------------------------------

/// FLV signature `"FLV"` (§E.2), from the shared FLV codec.
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738) to
/// validate the header incrementally without duplicating the constant.
pub(crate) const FLV_SIGNATURE: [u8; 3] = rtmp_runtime::flv::FLV_SIGNATURE;
/// FLV header length in bytes (§E.2), from the shared FLV codec.
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const FLV_HEADER_LEN: usize = rtmp_runtime::flv::FLV_HEADER_LEN;
/// Size of a tag header before its body (§E.4.1), from the shared FLV codec.
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const TAG_HEADER_LEN: usize = rtmp_runtime::flv::FLV_TAG_HEADER_LEN;
/// Size of the `PreviousTagSize` trailer after every tag (§E.4.1), from the
/// shared FLV codec.
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const PREV_TAG_SIZE_LEN: usize = rtmp_runtime::flv::FLV_PREV_TAG_SIZE_LEN;
/// A generous upper bound on a conformant `DataOffset` (§E.2): the header
/// itself is always [`FLV_HEADER_LEN`] (9) bytes; this crate's own
/// [`FlvMux`] never emits a `DataOffset` other than 9. A value far beyond
/// this is not a "larger but still real" header, it is either a corrupt
/// stream or a malicious `DataOffset` crafted to grow
/// [`crate::flv_stream::StreamingFlvDemux`]'s pre-header buffer without
/// bound while it waits for a "header" that never completes (#738 T11a
/// review, Important — remote OOM/DoS). Rejected via
/// [`FlvError::HeaderTooLarge`] before any buffering.
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const MAX_FLV_HEADER_LEN: usize = 1024;

/// What `rtmp_runtime::flv::FlvHeader::parse` names a `DataOffset` it rejects
/// (§E.2 — the field is the header length, so it can never be below
/// [`FLV_HEADER_LEN`]). Matched so the boxed error carries the real cause
/// rather than a bad-signature report for a stream whose signature is `FLV`.
pub(crate) const FLV_DATA_OFFSET: &str = "FLV header DataOffset";

/// `TagType` values (§E.4.1).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) mod tag_type {
    /// Audio tag (`AudioTagHeader` + AACAUDIODATA).
    pub const AUDIO: u8 = 8;
    /// Video tag (`VideoTagHeader` + AVCVIDEOPACKET).
    pub const VIDEO: u8 = 9;
    /// Script-data tag (`onMetaData`); informational.
    pub const SCRIPT: u8 = 18;
}

/// `CodecID` for AVC/H.264 in a `VideoTagHeader` (§E.4.3).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const CODEC_ID_AVC: u8 = 7;
/// `FrameType` for a keyframe / seekable frame (§E.4.3).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const FRAME_TYPE_KEYFRAME: u8 = 1;
/// `FrameType` for an inter frame (non-seekable) (§E.4.3).
const FRAME_TYPE_INTER: u8 = 2;

/// `AVCPacketType` values (§E.4.3.2).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) mod avc_packet_type {
    /// AVC sequence header — the `AVCDecoderConfigurationRecord` (`avcC`).
    pub const SEQUENCE_HEADER: u8 = 0;
    /// One or more length-prefixed NAL units.
    pub const NALU: u8 = 1;
    /// End of sequence (empty body).
    pub const END_OF_SEQUENCE: u8 = 2;
}

/// `SoundFormat` for AAC in an `AudioTagHeader` (§E.4.2).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const SOUND_FORMAT_AAC: u8 = 10;
/// `SoundRate` code 3 = 44 kHz — always used for AAC (real rate is in the ASC) (§E.4.2).
const SOUND_RATE_44K: u8 = 3;
/// `SoundSize` code 1 = 16-bit samples (§E.4.2).
const SOUND_SIZE_16BIT: u8 = 1;
/// `SoundType` code 1 = stereo (§E.4.2).
const SOUND_TYPE_STEREO: u8 = 1;
/// `SoundType` code 0 = mono (§E.4.2).
const SOUND_TYPE_MONO: u8 = 0;

/// `AACPacketType` values (§E.4.2.2).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) mod aac_packet_type {
    /// AAC sequence header — the `AudioSpecificConfig`.
    pub const SEQUENCE_HEADER: u8 = 0;
    /// One raw AAC access unit.
    pub const RAW: u8 = 1;
}

/// The IR timescale FLV uses: milliseconds (FLV tag timestamps are in ms, §E.4.1),
/// so sample durations / composition offsets round-trip losslessly.
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const FLV_TIMESCALE: u32 = 1000;

/// Smallest `CompositionTime` representable as the signed 24-bit field
/// `SI24` of an `AVCVIDEOPACKET` (§E.4.3.2).
const SI24_MIN: i32 = -(1 << 23);
/// Largest `CompositionTime` representable as the signed 24-bit field
/// `SI24` of an `AVCVIDEOPACKET` (§E.4.3.2).
const SI24_MAX: i32 = (1 << 23) - 1;

// `esds` construction constants (mirroring `ts_demux`).
/// MPEG-4 Audio object type indication for AAC (ISO/IEC 14496-1 §7.2.6.6 Table 5).
const OTI_MPEG4_AUDIO: u8 = 0x40;
/// `streamType` = AudioStream (ISO/IEC 14496-1 §7.2.6.6.2 Table 6).
const STREAM_TYPE_AUDIO: u8 = 0x05;
/// `ES_ID` assigned to the single audio elementary stream.
const ESDS_AUDIO_ES_ID: u16 = 1;
/// `SLConfigDescriptor.predefined = 2` (MP4 default; ISO/IEC 14496-1 §7.3.2.3).
/// Audio sample size in bits carried in the sample entry (typically 16).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const AUDIO_SAMPLE_SIZE_BITS: u16 = 16;

/// `CodecConfig::Aac.channel_count` for an `AudioSpecificConfig` whose
/// `channelConfiguration` does not determine a channel count.
///
/// ISO/IEC 14496-3 Table 1.19 gives a count for configurations 1..=7
/// (configuration 7 is **8** channels, 7.1). Configuration 0 means the mapping
/// is carried in-band by a `program_config_element` in the raw data stream —
/// this crate decodes no PCE, and FLV's AAC sequence header cannot contain one
/// (a PCE is a raw_data_block element, not part of the ASC) — and 8..=15 are
/// reserved. `0` marks "not derived", matching this crate's existing
/// placeholder convention (`ts_demux`'s `MPEGH_CHANNEL_COUNT_UNSPECIFIED`), and
/// is never a channel count a real stream has. It is *not* fabricated: writing
/// the raw configuration index as a count was the bug (7.1 reported as 7, a
/// PCE-signalled stream as 0 channels), and writing some other number would be
/// a second fabrication.
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const AAC_CHANNEL_COUNT_UNKNOWN: u16 = 0;

/// The channel count [`CodecConfig::Aac`] should carry for `config`.
///
/// [`AAC_CHANNEL_COUNT_UNKNOWN`] when Table 1.19 does not determine one.
pub(crate) fn aac_channel_count(config: &AudioSpecificConfig) -> u16 {
    config
        .channel_configuration
        .channel_count()
        .unwrap_or(AAC_CHANNEL_COUNT_UNKNOWN)
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors specific to FLV framing (Adobe FLV v10.1 Annex E).
// No longer `Eq` (only `PartialEq`): `FlvError::Codec` wraps this crate's
// own `Error`, which is no longer `Eq` since gaining `Error::HlsAttrValue`
// (issue #1140 T12 — wraps `broadcast_hls::Error`, which carries a
// non-`Eq` `f64`).
#[derive(Debug, PartialEq, Clone)]
#[non_exhaustive]
pub enum FlvError {
    /// The 3-byte signature was not `"FLV"` (§E.2).
    BadSignature([u8; 3]),
    /// A tag's declared `DataSize` ran past the end of the buffer (§E.4.1).
    TagOverrun {
        /// Byte offset of the tag header.
        offset: usize,
        /// Bytes the tag needed.
        need: usize,
        /// Bytes actually available.
        have: usize,
    },
    /// The stream carried no supported track (no AVC video and no AAC audio).
    NoSupportedTrack,
    /// The FLV header's `DataOffset` (§E.2) exceeded this crate's maximum
    /// accepted header size — rejected before buffering to bound
    /// [`StreamingFlvDemux`](crate::flv_stream::StreamingFlvDemux)'s
    /// pre-header buffer (#738 T11a review, Important).
    HeaderTooLarge {
        /// The declared `DataOffset`.
        declared: u32,
        /// The maximum this crate accepts.
        max: usize,
    },
    /// A [`Media`] track used a codec FLV cannot carry (only AVC + AAC).
    UnsupportedCodec {
        /// The codec name.
        codec: &'static str,
    },
    /// The signature is `"FLV"` but the rest of the header was rejected
    /// (§E.2) — currently a `DataOffset` below the 9-byte header itself.
    Header(RtmpError),
    /// A tag's trailing `PreviousTagSize` did not match `11 + DataSize`
    /// (§E.4.1), or another structural tag-header error.
    Tag(RtmpError),
    /// Underlying parse/serialize error from a reused codec-config builder.
    Codec(Error),
}

impl fmt::Display for FlvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlvError::BadSignature(sig) => {
                write!(f, "bad FLV signature: {sig:02X?} (expected \"FLV\")")
            }
            FlvError::TagOverrun { offset, need, have } => write!(
                f,
                "FLV tag at offset {offset} overruns buffer: need {need}, have {have}"
            ),
            FlvError::NoSupportedTrack => {
                write!(
                    f,
                    "FLV carried no supported track (need AVC video or AAC audio)"
                )
            }
            FlvError::HeaderTooLarge { declared, max } => write!(
                f,
                "FLV header DataOffset {declared} exceeds the maximum accepted {max} bytes"
            ),
            FlvError::UnsupportedCodec { codec } => {
                write!(
                    f,
                    "codec {codec} has no FLV carriage in this crate (only AVC + AAC)"
                )
            }
            FlvError::Codec(e) => write!(f, "FLV codec config: {e}"),
            FlvError::Header(e) => write!(f, "FLV header: {e}"),
            FlvError::Tag(e) => write!(f, "FLV tag: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for FlvError {}

impl From<Error> for FlvError {
    fn from(e: Error) -> Self {
        FlvError::Codec(e)
    }
}

// ---------------------------------------------------------------------------
// FlvDemux — Unpackage<Input = &[u8]>
// ---------------------------------------------------------------------------

/// Demux an FLV byte stream into a [`Media`] (Adobe FLV v10.1 Annex E).
///
/// Parses the header then walks the tag loop: an AVC sequence-header tag
/// (`AVCPacketType == 0`) yields the [`CodecConfig::Avc`] (dimensions decoded
/// from the SPS inside the `avcC`); an AAC sequence-header tag
/// (`AACPacketType == 0`) yields the [`CodecConfig::Aac`] (ASC → `esds`).
/// Type-1 tags become [`Sample`]s: video DTS = tag timestamp, PTS = DTS +
/// `CompositionTime`; audio frames are the raw AAC AUs. Script (`onMetaData`)
/// tags are skipped leniently.
///
/// The `'a` parameter ties the demuxer to the byte-slice lifetime it consumes
/// via [`Unpackage::Input`]; construct one per call with [`FlvDemux::new`].
#[derive(Debug, Default, Clone)]
pub struct FlvDemux<'a> {
    _marker: PhantomData<&'a [u8]>,
    /// Whether the most recent [`unpackage`](Unpackage::unpackage) stopped on a
    /// short final tag. See [`FlvDemux::last_walk_was_truncated`].
    truncated_tail: bool,
}

impl FlvDemux<'_> {
    /// Create a new demuxer.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
            truncated_tail: false,
        }
    }

    /// Whether the most recent [`unpackage`](Unpackage::unpackage) call ended on
    /// a **truncated final tag**: the last tag's declared `DataSize` ran past
    /// the end of the buffer, so that tag was discarded and everything before it
    /// returned. Always `false` before the first call.
    ///
    /// This exists so a truncated capture is not indistinguishable from a
    /// complete one. The returned [`Media`] is otherwise identical — a recorded
    /// or captured live FLV routinely ends mid-tag, and refusing the whole file
    /// over it would discard all the good media — but the caller can now tell,
    /// and decide whether to warn, trim, or re-fetch. A tag that is complete
    /// except for its trailing `PreviousTagSize` is *not* reported here: that
    /// field is redundant (§E.4.1), so such a tag is kept and the walk is clean.
    pub fn last_walk_was_truncated(&self) -> bool {
        self.truncated_tail
    }
}

/// One parsed FLV tag (header fields + body slice).
struct FlvTag<'a> {
    /// Byte offset of the tag header in the input (for degradation reports).
    offset: usize,
    /// The tag's `TagType` (§E.4.1), parsed by the shared FLV codec.
    tag_type: TagType,
    timestamp: u32,
    body: &'a [u8],
}

/// The tag walk of an FLV stream: the complete tags plus whether the stream
/// ended on a short tail (see [`iter_tags`]).
struct TagWalk<'a> {
    tags: Vec<FlvTag<'a>>,
    /// `true` when the walk stopped because the last tag declares more bytes
    /// than the buffer holds — i.e. the file ends mid-tag. The tags collected
    /// before that point are complete and usable; the caller may want to say so
    /// (see [`FlvDemux::last_walk_was_truncated`]).
    truncated_tail: bool,
    /// Offsets of spans whose tag framing was damaged and skipped (audit T16,
    /// #1144); the walk resynchronised on the next plausible tag.
    damaged: Vec<usize>,
}

/// Iterate the tags of an FLV stream after its 9-byte header, keeping what is
/// readable across damage (§E.4.1).
///
/// The walk itself is `rtmp_runtime::flv::tags_resync`, the workspace's one FLV
/// tag walker: a span whose framing is damaged is skipped to the next
/// consistent tag chain and reported in [`TagWalk::damaged`], and a final tag
/// whose body overruns the buffer stops the walk as the truncated tail
/// ([`TagWalk::truncated_tail`]) rather than discarding the good prefix. Two
/// shapes are *not* truncation: a final tag whose body is complete but whose
/// 4-byte `PreviousTagSize` is missing (§E.4.1 makes that field redundant) is
/// kept and the walk ends cleanly, and a header too short to read at the very
/// end is simply the end of the walk.
///
/// A stream whose very first tag is already incomplete has nothing to return at
/// all, so that alone is [`FlvError::TagOverrun`] — the caller gets a diagnosis
/// rather than a bare `NoSupportedTrack`.
fn iter_tags(input: &[u8]) -> Result<TagWalk<'_>> {
    need_in(input, FLV_HEADER_LEN + PREV_TAG_SIZE_LEN, "FLV header")?;
    let header = FlvHeader::parse(input).map_err(|e| match e.clone() {
        RtmpError::Malformed {
            what: FLV_DATA_OFFSET,
        } => Error::from(FlvErrorAsError(FlvError::Header(e))),
        RtmpError::Malformed { what } => Error::InvalidInput(what),
        RtmpError::BufferTooShort(w) => Error::from(FlvErrorAsError(FlvError::TagOverrun {
            offset: 0,
            need: w.need,
            have: w.have,
        })),
        _ => Error::from(FlvErrorAsError(FlvError::BadSignature([
            input[0], input[1], input[2],
        ]))),
    })?;
    let overrun = |need| {
        Error::from(FlvErrorAsError(FlvError::TagOverrun {
            offset: header.first_tag_offset(),
            need,
            have: input.len(),
        }))
    };
    let walk = rtmp_runtime::flv::tags_resync(input, &header).map_err(|e| match e {
        RtmpError::BufferTooShort(w) => overrun(w.need),
        _ => overrun(input.len().saturating_add(1)),
    })?;
    let tags = walk
        .tags
        .into_iter()
        .map(|(offset, tag)| FlvTag {
            offset,
            tag_type: tag.tag_type,
            timestamp: tag.timestamp,
            body: tag.body,
        })
        .collect::<Vec<_>>();
    match walk.truncated_tail {
        Some((offset, need, have)) if tags.is_empty() => {
            Err(Error::from(FlvErrorAsError(FlvError::TagOverrun {
                offset,
                need,
                have,
            })))
        }
        tail => Ok(TagWalk {
            tags,
            truncated_tail: tail.is_some(),
            damaged: walk.damaged,
        }),
    }
}

// A tiny bridge so `iter_tags` can surface FLV-specific overrun through the
// crate `Error` used by the reused config helpers. `TagOverrun` maps onto the
// structured `BufferTooShort` shape.
struct FlvErrorAsError(FlvError);
impl From<FlvErrorAsError> for Error {
    fn from(w: FlvErrorAsError) -> Self {
        match w.0 {
            FlvError::TagOverrun { need, have, .. } => Error::BufferTooShort(BufferTooShort {
                need,
                have,
                what: "FLV tag body",
            }),
            other => Error::InvalidInput(match other {
                FlvError::BadSignature(_) => "FLV bad signature",
                FlvError::NoSupportedTrack => "FLV no supported track",
                FlvError::UnsupportedCodec { .. } => "FLV unsupported codec",
                FlvError::HeaderTooLarge { .. } => "FLV header DataOffset too large",
                FlvError::Header(_) => "FLV header DataOffset below the header",
                FlvError::Tag(_) => "FLV tag parse error",
                _ => "FLV error",
            }),
        }
    }
}

/// Golden for the tag walk: what it yields — the tag count, the damaged-span
/// offsets, the truncated-tail flag and every tag's `(offset, type, timestamp,
/// body length)` — for every committed FLV fixture and for the damage and
/// truncation shapes the FLV tests build.
///
/// It lives beside the walker because the walk and its result are crate-private.
/// The record was taken from the walker before its logic moved into
/// `rtmp_runtime::flv::tags_resync`; the move had to reproduce it byte for byte,
/// so it is never re-blessed. `REGENERATE_FLV_WALK_GOLDEN=1` rewrites it.
#[cfg(test)]
mod walk_golden {
    use std::env;
    use std::fmt::Write as _;
    use std::fs;
    use std::path::PathBuf;

    use super::{FlvTag, iter_tags};

    /// Environment variable that rewrites the golden instead of comparing it.
    const REGENERATE_ENV: &str = "REGENERATE_FLV_WALK_GOLDEN";
    /// The variable CI sets; a rewrite is refused there, so a CI run can never
    /// replace the record it is meant to check.
    const CI_ENV: &str = "CI";
    /// The golden record (public fixtures tier, `docs/FIXTURES.md` layout).
    const GOLDEN: &str = "golden/flv/tag-walk.golden";
    /// FLV header (9 bytes) and its `PreviousTagSize0` (4 bytes).
    const FLV_HEADER_PLUS_PREV: usize = 13;
    /// FLV tag header (§E.4.1).
    const TAG_HEADER_LEN: usize = 11;
    /// The one-byte `PreviousTagSize` cut the truncation shapes use.
    const CUT_TAIL: usize = 40;

    const FIXTURES: &[&str] = &["flv/av.flv", "flv/aac-7-1.flv", "flv/oversize-dims.flv"];

    /// `(offset, tag_type, data_size)` of every tag, walked strictly by `DataSize`.
    fn tags(bytes: &[u8]) -> Vec<(usize, u8, usize)> {
        let mut out = Vec::new();
        let mut off = FLV_HEADER_PLUS_PREV;
        while off + TAG_HEADER_LEN <= bytes.len() {
            let size =
                u32::from_be_bytes([0, bytes[off + 1], bytes[off + 2], bytes[off + 3]]) as usize;
            out.push((off, bytes[off], size));
            off += TAG_HEADER_LEN + size + 4;
        }
        out
    }

    /// The first mid-stream video tag whose body has more than 20 bytes, when
    /// the fixture carries one.
    fn mid_video_tag(bytes: &[u8]) -> Option<(usize, usize)> {
        let all = tags(bytes);
        all.iter()
            .copied()
            .skip(all.len() / 2)
            .find(|&(_, tag_type, size)| tag_type == 9 && size > 20)
            .map(|(off, _, size)| (off, size))
    }

    /// `(name, bytes)` for every input the walker must keep reproducing.
    fn inputs() -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        for rel in FIXTURES {
            let bytes = test_fixtures::read(rel);
            let name = rel.replace('/', "-");
            out.push((name.clone(), bytes.clone()));
            out.push((
                format!("{name}-cut-tail"),
                bytes[..bytes.len() - CUT_TAIL].to_vec(),
            ));
            let (last, last_size) = tags(&bytes)
                .last()
                .copied()
                .map(|(off, _, size)| (off, size))
                .expect("fixture carries tags");
            let last_body_ends = last + last_size;
            out.push((
                format!("{name}-cut-last-tag-body"),
                bytes[..last_body_ends.saturating_sub(1)].to_vec(),
            ));
            out.push((
                format!("{name}-cut-previous-tag-size"),
                bytes[..bytes.len() - 4].to_vec(),
            ));
            out.push((
                format!("{name}-header-only"),
                bytes[..FLV_HEADER_PLUS_PREV].to_vec(),
            ));
            out.push((
                format!("{name}-cut-before-first-tag-body"),
                bytes[..FLV_HEADER_PLUS_PREV + TAG_HEADER_LEN + 5].to_vec(),
            ));
            if let Some((victim, size)) = mid_video_tag(&bytes) {
                let mut inflated = bytes.clone();
                let bad = u32::try_from(size + 5).expect("DataSize fits");
                inflated[victim + 1..victim + 4].copy_from_slice(&bad.to_be_bytes()[1..]);
                out.push((format!("{name}-inflated-datasize"), inflated));

                let mut garbled = bytes.clone();
                garbled[victim..victim + TAG_HEADER_LEN].fill(0xFF);
                out.push((format!("{name}-garbled-tag-header"), garbled));
            }
        }
        out
    }

    /// One record per input: the tag count, the damaged offsets, the
    /// truncated-tail flag and every tag, in the walker's own order.
    fn render(name: &str, bytes: &[u8]) -> String {
        match iter_tags(bytes) {
            Ok(walk) => {
                let (tags, damaged, truncated_tail) =
                    (walk.tags, walk.damaged, walk.truncated_tail);
                let mut line = format!(
                    "{name} tags={} damaged={damaged:?} truncated_tail={truncated_tail}",
                    tags.len()
                );
                for tag in &tags {
                    let FlvTag {
                        offset,
                        tag_type,
                        timestamp,
                        body,
                    } = tag;
                    write!(
                        line,
                        "\n  offset={offset} type={} timestamp={timestamp} body_len={}",
                        tag_type.to_byte(),
                        body.len()
                    )
                    .expect("write to String");
                }
                line.push('\n');
                line
            }
            Err(e) => format!("{name} error={e}\n"),
        }
    }

    fn golden_path() -> PathBuf {
        test_fixtures::path(GOLDEN)
    }

    #[test]
    fn the_tag_walk_reproduces_its_golden() {
        let mut actual = String::new();
        for (name, bytes) in &inputs() {
            actual.push_str(&render(name, bytes));
        }
        let path = golden_path();
        if env::var_os(REGENERATE_ENV).is_some() {
            assert!(
                env::var_os(CI_ENV).is_none(),
                "{REGENERATE_ENV} rewrites a committed golden and is refused under {CI_ENV}"
            );
            fs::write(&path, &actual).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
            return;
        }
        let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        assert_eq!(
            actual, expected,
            "the tag walk changed; the golden was recorded from the walker before the move and is never re-blessed"
        );
    }

    /// The record covers damage and truncation, not only clean fixtures: a walk
    /// that stopped noticing either would still match a record of fixtures only.
    #[test]
    fn the_record_exercises_damage_truncation_and_refusal() {
        let text = fs::read_to_string(golden_path()).expect("the golden is committed");
        assert!(
            text.lines().any(|line| {
                line.split_once("damaged=[")
                    .and_then(|(_, rest)| rest.split_once(']'))
                    .is_some_and(|(spans, _)| !spans.is_empty())
            }),
            "a damaged span is recorded"
        );
        assert!(
            text.contains("truncated_tail=true"),
            "a truncated tail is recorded"
        );
        assert!(
            text.contains(" error="),
            "an unrecoverable input is recorded"
        );
    }
}

impl<'a> Unpackage for FlvDemux<'a> {
    type Input = &'a [u8];
    type Media = Media;
    type Error = FlvError;

    fn unpackage(&mut self, input: &'a [u8]) -> core::result::Result<Media, FlvError> {
        // Reset first: a caller that reuses one demuxer for two streams must
        // never see the previous stream's truncation reported against this one.
        self.truncated_tail = false;
        if input.len() < FLV_HEADER_LEN {
            return Err(FlvError::Codec(Error::BufferTooShort(BufferTooShort {
                need: FLV_HEADER_LEN,
                have: input.len(),
                what: "FLV header",
            })));
        }
        if input[0..3] != FLV_SIGNATURE {
            return Err(FlvError::BadSignature([input[0], input[1], input[2]]));
        }

        let walk = iter_tags(input).map_err(FlvError::Codec)?;
        self.truncated_tail = walk.truncated_tail;
        let tags = walk.tags;
        let mut degraded: Vec<InputDegradation> = walk
            .damaged
            .iter()
            .map(|&o| InputDegradation::MalformedUnit { position: o as u64 })
            .collect();

        // Video track state.
        let mut avc_config: Option<AVCConfigurationBox> = None;
        // The NAL length-prefix size the *source* `avcC` declared (the emitted
        // record's field is rewritten to the canonical 4, so the original must
        // be kept separately for the sample-rewrite below).
        let mut avc_source_length_size: Option<usize> = None;
        let mut video_samples: Vec<Sample> = Vec::new();
        let mut last_video_dts: Option<u32> = None;
        // Audio track state.
        let mut aac_esds: Option<EsdsBox> = None;
        let mut aac_channels: u16 = 0;
        let mut aac_rate: u32 = 0;
        let mut audio_samples: Vec<Sample> = Vec::new();
        let mut last_audio_dts: Option<u32> = None;

        for tag in &tags {
            match tag.tag_type {
                TagType::Video => {
                    if tag.body.len() < 2 {
                        continue;
                    }
                    let frame_type = tag.body[0] >> 4;
                    let codec_id = tag.body[0] & 0x0F;
                    if codec_id != CODEC_ID_AVC {
                        continue; // non-AVC video is out of scope
                    }
                    let avc_packet_type = tag.body[1];
                    // AVCVIDEOPACKET: AVCPacketType(1) + CompositionTime(SI24=3) + Data.
                    if tag.body.len() < 5 {
                        continue;
                    }
                    let composition_time = read_si24(&tag.body[2..5]);
                    let data = &tag.body[5..];
                    match avc_packet_type {
                        avc_packet_type::SEQUENCE_HEADER
                            if avc_config.is_none() && !data.is_empty() =>
                        {
                            // A corrupt sequence header is skipped and reported; a
                            // later intact one can still configure the track.
                            let Some(mut record) = AVCDecoderConfigurationRecord::parse(data)
                                .ok()
                                .filter(|r| r.require_sps().is_ok())
                            else {
                                degraded.push(InputDegradation::MalformedUnit {
                                    position: tag.offset as u64,
                                });
                                continue;
                            };
                            // The NALU samples are normalised to 4-byte NAL
                            // prefixes below, so the emitted `avcC` must declare
                            // that size, not the source's (r04-W43).
                            avc_source_length_size =
                                Some(usize::from(record.length_size_minus_one) + 1);
                            record.length_size_minus_one = NAL_LENGTH_SIZE_MINUS_ONE;
                            avc_config = Some(AVCConfigurationBox::new(record));
                        }
                        avc_packet_type::NALU => {
                            let dts = tag.timestamp;
                            // Absolute dts/pts (media plane step 2c): the FLV
                            // tag timestamp is already an absolute wire clock
                            // (milliseconds, matching `FLV_TIMESCALE`), unlike
                            // the five demuxers this step fixes that used to
                            // leave the anchor at 0 — FLV's `CompositionTime`
                            // (§E.4.3.2) folds directly into `pts`.
                            let dts_abs = dts as i64;
                            let pts_abs = dts_abs + composition_time as i64;
                            // The NALU payload's NAL prefixes use the
                            // `lengthSizeMinusOne` the AVC sequence header
                            // declared (§E.4.3.1 / ISO/IEC 14496-15:2017 §5.3.3) —
                            // 1, 2 or 4 bytes. The rest of the pipeline is
                            // fixed at 4, so rewrite them (r04-W43).
                            let length_size = avc_source_length_size.unwrap_or(NAL_LENGTH_SIZE);
                            let Ok(data) = normalise_nal_length_size(data, length_size) else {
                                // Corrupt NAL framing in this tag: drop the
                                // sample, report it, keep going.
                                degraded.push(InputDegradation::MalformedUnit {
                                    position: tag.offset as u64,
                                });
                                continue;
                            };
                            // Only a good sample advances the duration clock.
                            let duration = delta_duration(&mut last_video_dts, dts);
                            video_samples.push(Sample {
                                data: data.into(),
                                dts: Some(dts_abs),
                                pts: Some(pts_abs),
                                duration: Some(duration),
                                flags: crate::ir::SampleFlags::new(
                                    frame_type == FRAME_TYPE_KEYFRAME,
                                ),
                                provenance: None,
                                display: None,
                            });
                        }
                        avc_packet_type::END_OF_SEQUENCE => {}
                        _ => {}
                    }
                }
                TagType::Audio => {
                    if tag.body.is_empty() {
                        continue;
                    }
                    let sound_format = tag.body[0] >> 4;
                    if sound_format != SOUND_FORMAT_AAC {
                        continue; // non-AAC audio is out of scope
                    }
                    // AACAUDIODATA: AACPacketType(1) + Data.
                    if tag.body.len() < 2 {
                        continue;
                    }
                    let aac_pkt_type = tag.body[1];
                    let data = &tag.body[2..];
                    match aac_pkt_type {
                        // Some muxers emit a spurious empty AAC sequence
                        // header; keep looking until a non-empty ASC arrives.
                        aac_packet_type::SEQUENCE_HEADER
                            if aac_esds.is_none() && !data.is_empty() =>
                        {
                            let Ok(asc) = AudioSpecificConfig::parse(data) else {
                                degraded.push(InputDegradation::MalformedUnit {
                                    position: tag.offset as u64,
                                });
                                continue;
                            };
                            aac_channels = aac_channel_count(&asc);
                            aac_rate = asc_rate_hz(&asc);
                            aac_esds = Some(build_aac_esds(data.to_vec()));
                        }
                        aac_packet_type::RAW => {
                            let dts = tag.timestamp;
                            let duration = delta_duration(&mut last_audio_dts, dts);
                            let dts_abs = dts as i64;
                            audio_samples.push(Sample {
                                data: data.to_vec().into(),
                                dts: Some(dts_abs),
                                pts: Some(dts_abs),
                                duration: Some(duration),
                                flags: crate::ir::SampleFlags::SYNC,
                                provenance: None,
                                display: None,
                            });
                        }
                        _ => {}
                    }
                }
                TagType::Script => { /* onMetaData — informational, skipped */ }
                _ => { /* unknown tag type — skipped leniently */ }
            }
        }

        // Backfill the final sample's duration from the previous delta (no next
        // tag to measure against): reuse the second-to-last delta.
        backfill_last_duration(&mut video_samples);
        backfill_last_duration(&mut audio_samples);

        let mut tracks: Vec<Track> = Vec::new();
        let mut track_id = TrackId::new(1);
        if let Some(config) = avc_config
            && !video_samples.is_empty()
        {
            let (width, height) = avc_dimensions(&config)?;
            let anchor = anchor_of(&video_samples);
            tracks.push(Track::new_at(
                TrackSpec::new(
                    track_id,
                    FLV_TIMESCALE,
                    CodecConfig::Avc {
                        config,
                        width,
                        height,
                    },
                ),
                video_samples,
                anchor,
            ));
            track_id = track_id.next();
        }
        if let Some(esds) = aac_esds
            && !audio_samples.is_empty()
        {
            let anchor = anchor_of(&audio_samples);
            tracks.push(Track::new_at(
                TrackSpec::new(
                    track_id,
                    FLV_TIMESCALE,
                    CodecConfig::Aac {
                        esds,
                        channel_count: aac_channels,
                        sample_rate: aac_rate,
                        sample_size: AUDIO_SAMPLE_SIZE_BITS,
                    },
                ),
                audio_samples,
                anchor,
            ));
        }

        if tracks.is_empty() {
            return Err(FlvError::NoSupportedTrack);
        }
        let mut media = Media::new(tracks, FLV_TIMESCALE);
        media.degraded = degraded;
        Ok(media)
    }
}

/// Read a signed 24-bit big-endian integer (FLV `CompositionTime`, §E.4.3.2).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738) so
/// the two demuxers agree byte-for-byte on `CompositionTime` decoding.
pub(crate) fn read_si24(b: &[u8]) -> i32 {
    let raw = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
    // Sign-extend from 24 bits.
    if raw & 0x0080_0000 != 0 {
        (raw | 0xFF00_0000) as i32
    } else {
        raw as i32
    }
}

/// Compute a sample's duration as the delta from the previous DTS, updating the
/// running previous-DTS. The first sample gets a provisional 0 (backfilled).
fn delta_duration(prev: &mut Option<u32>, dts: u32) -> u32 {
    let dur = match *prev {
        Some(p) => dts.saturating_sub(p),
        None => 0,
    };
    *prev = Some(dts);
    dur
}

/// The delta scheme leaves sample 0 with duration 0 and the last sample with no
/// forward delta. Shift durations so each sample carries the *forward* delta
/// (dts[i+1]-dts[i]); the last sample repeats the previous forward delta.
fn backfill_last_duration(samples: &mut [Sample]) {
    let n = samples.len();
    if n == 0 {
        return;
    }
    // `duration[i]` currently holds dts[i]-dts[i-1] (0 for i==0). Rebuild as the
    // forward delta dts[i+1]-dts[i] by shifting left by one.
    for i in 0..n.saturating_sub(1) {
        samples[i].duration = samples[i + 1].duration;
    }
    if n >= 2 {
        // Last sample: reuse the previous forward delta as a best estimate.
        samples[n - 1].duration = samples[n - 2].duration;
    }
}

/// The AVC track's `Track::start_decode_time` anchor: the first video
/// sample's absolute dts (media plane step 2c) — kept in lockstep with
/// `samples[0].dts` per the crate-wide invariant.
fn anchor_of(samples: &[Sample]) -> u64 {
    samples
        .first()
        .and_then(|s| s.dts)
        .map(|d| d.max(0) as u64)
        .unwrap_or(0)
}

/// Build an AAC `esds` from a raw `AudioSpecificConfig` byte slice (mirrors the
/// `ts_demux` AAC path).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) fn build_aac_esds(asc_bytes: Vec<u8>) -> EsdsBox {
    EsdsBox::new(ESDescriptor::new(
        ESDS_AUDIO_ES_ID,
        0,
        Some(DecoderConfigDescriptor::new(
            OTI_MPEG4_AUDIO,
            STREAM_TYPE_AUDIO,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(asc_bytes)),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ))
}

/// Sampling rate in Hz from a parsed ASC: the explicit escape rate if present,
/// else the `samplingFrequencyIndex` table (ISO/IEC 14496-3 §1.6.3.4 Table
/// 1.10), via the crate's single copy of it
/// ([`SamplingFrequencyIndex::table_hz`]). Returns 0 when the index determines
/// no rate (reserved, or the escape form with no explicit value).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) fn asc_rate_hz(asc: &AudioSpecificConfig) -> u32 {
    asc.effective_sampling_frequency().unwrap_or(0)
}

/// Coded dimensions from an AVC sequence header's `avcC` (the SPS it carries),
/// as the `u16` pair the IR's [`CodecConfig::Avc`] stores.
///
/// A missing or undecodable SPS yields `(0, 0)` — the pre-existing "unknown"
/// sentinel, and the same one [`crate::ts_demux`] uses when a probe cannot
/// resolve dimensions. A dimension that *is* decoded but does not fit `u16`
/// (a corrupt or hostile SPS; the field itself is unbounded in the grammar) is
/// an error: the previous `as u16` silently truncated it (70 000 → 4 464), and
/// a container that misdescribes its own coded size is worse than no
/// dimensions at all.
pub(crate) fn avc_dimensions(
    config: &AVCConfigurationBox,
) -> core::result::Result<(u16, u16), FlvError> {
    let Some(info) = config
        .config
        .sps
        .first()
        .and_then(|sps| crate::sps::decode_avc_sps(&sps.0).ok())
    else {
        return Ok((0, 0));
    };
    for (value, field) in [(info.width, "width"), (info.height, "height")] {
        if value > u16::MAX as u32 {
            return Err(FlvError::Codec(Error::InvalidInput(match field {
                "height" => "FLV avcC height does not fit 16 bits",
                _ => "FLV avcC width does not fit 16 bits",
            })));
        }
    }
    Ok((info.width as u16, info.height as u16))
}

// ---------------------------------------------------------------------------
// FlvMux — Package<Output = Vec<u8>>
// ---------------------------------------------------------------------------

/// Mux a [`Media`] into an FLV byte stream (Adobe FLV v10.1 Annex E).
///
/// Emits the header (`TypeFlags` from the track kinds), a minimal `onMetaData`
/// script tag (duration / width / height / codec ids), the AVC sequence-header
/// tag (`avcC`) and AAC sequence-header tag (ASC), then the interleaved A/V
/// type-1 tags ordered by DTS, each followed by its `PreviousTagSize`. Only
/// [`CodecConfig::Avc`] video and [`CodecConfig::Aac`] audio are carried; any
/// other codec is rejected with [`FlvError::UnsupportedCodec`].
#[derive(Debug, Default, Clone)]
pub struct FlvMux;

impl FlvMux {
    /// Create a new FLV muxer.
    pub fn new() -> Self {
        Self
    }
}

/// A tag ready to serialise: its type, timestamp (ms) and fully-built body.
/// The `FieldOverflow` behind a shared-codec serialize error: the only failure
/// a tag or header serializer can have is a field that did not fit.
fn field_overflow(error: RtmpError) -> broadcast_common::len::FieldOverflow {
    match error {
        RtmpError::FieldOverflow(overflow) => overflow,
        _ => broadcast_common::len::FieldOverflow {
            field: "FLV tag",
            value: 0,
            max: 0,
        },
    }
}

struct OutTag {
    tag_type: TagType,
    timestamp: u32,
    body: Vec<u8>,
}

impl OutTag {
    /// Append this tag's framing to `out` through the shared FLV codec.
    fn write_into(&self, out: &mut Vec<u8>) -> core::result::Result<(), FlvError> {
        let tag = RtmpFlvTag::new(self.tag_type, self.timestamp, &self.body);
        let bytes = tag
            .try_to_bytes()
            .map_err(|e| Error::FieldOverflow(field_overflow(e)))?;
        out.extend_from_slice(&bytes);
        Ok(())
    }
}

/// Locate `media`'s (at most one) AVC video track and (at most one) AAC
/// audio track — shared by [`FlvMux::package`] and the RTMP-payload builders
/// [`flv_sequence_header_payloads`]/[`flv_frame_payloads`] (issue #934).
/// Errors if any *other* track carries a codec FLV cannot represent, or if
/// neither an AVC nor an AAC track is present at all.
fn locate_av_tracks(
    media: &Media,
) -> core::result::Result<(Option<&Track>, Option<&Track>), FlvError> {
    let mut video: Option<&Track> = None;
    let mut audio: Option<&Track> = None;
    for t in &media.tracks {
        match &t.spec.config {
            CodecConfig::Avc { .. } if video.is_none() => video = Some(t),
            CodecConfig::Aac { .. } if audio.is_none() => audio = Some(t),
            CodecConfig::Avc { .. } | CodecConfig::Aac { .. } => {}
            other => {
                return Err(FlvError::UnsupportedCodec {
                    codec: other.name(),
                });
            }
        }
    }
    if video.is_none() && audio.is_none() {
        return Err(FlvError::NoSupportedTrack);
    }
    Ok((video, audio))
}

/// Build the AVC sequence-header (`avcC`) tag body for `vt`, or `None` if
/// `vt`'s config isn't [`CodecConfig::Avc`] (defensive; callers only pass a
/// track [`locate_av_tracks`] already classified as video).
fn video_sequence_header_body(vt: &Track) -> core::result::Result<Option<Vec<u8>>, FlvError> {
    let CodecConfig::Avc { config, .. } = &vt.spec.config else {
        return Ok(None);
    };
    let mut avcc = vec![0u8; config.config.serialized_len()];
    let n = config
        .config
        .serialize_into(&mut avcc)
        .map_err(FlvError::Codec)?;
    avcc.truncate(n);
    let mut body = Vec::with_capacity(5 + avcc.len());
    body.push((FRAME_TYPE_KEYFRAME << 4) | CODEC_ID_AVC);
    body.push(avc_packet_type::SEQUENCE_HEADER);
    body.extend_from_slice(&[0, 0, 0]); // CompositionTime = 0
    body.extend_from_slice(&avcc);
    Ok(Some(body))
}

/// This audio track's `SoundType` (mono/stereo, §E.4.2) from its channel
/// count. Defensive default (stereo) if `at` isn't [`CodecConfig::Aac`].
fn audio_sound_type(at: &Track) -> u8 {
    match &at.spec.config {
        CodecConfig::Aac { channel_count, .. } if *channel_count <= 1 => SOUND_TYPE_MONO,
        _ => SOUND_TYPE_STEREO,
    }
}

/// Build the AAC sequence-header (`AudioSpecificConfig`) tag body for `at`,
/// or `None` if `at`'s config isn't [`CodecConfig::Aac`] (defensive; callers
/// only pass a track [`locate_av_tracks`] already classified as audio).
fn audio_sequence_header_body(at: &Track) -> core::result::Result<Option<Vec<u8>>, FlvError> {
    let CodecConfig::Aac { esds, .. } = &at.spec.config else {
        return Ok(None);
    };
    let asc = esds_asc_bytes(esds)?;
    let mut body = Vec::with_capacity(2 + asc.len());
    body.push(audio_tag_header_byte(audio_sound_type(at)));
    body.push(aac_packet_type::SEQUENCE_HEADER);
    body.extend_from_slice(&asc);
    Ok(Some(body))
}

/// Build one AVC `VideoTagHeader`+`AVCVIDEOPACKET` frame body (§E.4.3):
/// `FrameType`/`CodecID` nibble + `AVCPacketType::NALU` + `CompositionTime`
/// (`SI24`, already in FLV's millisecond clock) + the coded NAL data.
fn video_frame_body(is_sync: bool, composition_time_ms: i32, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(5 + data.len());
    let ft = if is_sync {
        FRAME_TYPE_KEYFRAME
    } else {
        FRAME_TYPE_INTER
    };
    body.push((ft << 4) | CODEC_ID_AVC);
    body.push(avc_packet_type::NALU);
    body.push((composition_time_ms >> 16) as u8);
    body.push((composition_time_ms >> 8) as u8);
    body.push(composition_time_ms as u8);
    body.extend_from_slice(data);
    body
}

/// Build one AAC `AudioTagHeader`+`AACAUDIODATA` frame body (§E.4.2):
/// `AudioTagHeader` byte + `AACPacketType::RAW` + the raw AAC access unit.
fn audio_frame_body(sound_type: u8, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(2 + data.len());
    body.push(audio_tag_header_byte(sound_type));
    body.push(aac_packet_type::RAW);
    body.extend_from_slice(data);
    body
}

/// Rescale `ticks` (in `timescale`-ticks-per-second) to FLV's millisecond
/// clock ([`FLV_TIMESCALE`]), for the streaming payload builders below.
///
/// Unlike [`FlvMux::package`], whose per-tag `Timestamp` is a running sum of
/// `Sample::duration` starting at zero (correct for muxing one whole
/// already-ms-normalised file, matching [`FlvDemux`]'s output), the
/// streaming builders below feed a live push driver that calls them once per
/// drained batch, never once for the whole stream — "start of this batch" is
/// not "start of the stream". They use each [`Sample::dts`]/`pts` directly
/// (absolute, in the track's own [`crate::pipeline::TrackSpec::timescale`],
/// per the media-plane architecture) and rescale here instead.
fn ticks_to_ms(ticks: i64, timescale: u32) -> Result<i64> {
    if timescale == 0 {
        return Err(Error::InvalidInput(
            "FLV mux requires a non-zero track timescale",
        ));
    }
    Ok(rescale_i64(
        ticks,
        timescale,
        FLV_TIMESCALE,
        Rounding::TowardZero,
    ))
}

/// [`ticks_to_ms`] for a value that must land in an FLV `Timestamp` (the UI24
/// low field plus the UI8 extended high byte, i.e. a 32-bit millisecond clock,
/// §E.4.1): a value outside `u32` is an error, never a silent wrap, and a
/// `timescale` of `0` is an error too — [`ticks_to_ms`] returns `0` for it as a
/// placeholder, which here would silently stamp *every* tag at millisecond 0.
/// (A track really can arrive that way: the IR does not forbid it, and
/// `RtpDepacketiser` has produced exactly that — see audit r04-W30.)
fn ms_from_ticks(ticks: i64, timescale: u32) -> core::result::Result<u32, FlvError> {
    if timescale == 0 {
        return Err(FlvError::Codec(Error::InvalidInput(
            "FLV mux requires a non-zero track timescale",
        )));
    }
    let ms = ticks_to_ms(ticks, timescale)?;
    if ms < 0 || ms > u32::MAX as i64 {
        return Err(FlvError::Codec(Error::InvalidInput(
            "FLV tag timestamp does not fit the 32-bit millisecond clock",
        )));
    }
    Ok(ms as u32)
}

/// [`ticks_to_ms`] for the RTMP/FLV payload path: a negative value (a DTS
/// before the clock origin) clamps to 0 as before, but a value past the 32-bit
/// millisecond clock is an error rather than a silent `as u32` wrap (#1129).
fn clamped_ms(ticks: i64, timescale: u32) -> core::result::Result<u32, FlvError> {
    u32::try_from(ticks_to_ms(ticks, timescale)?.max(0)).map_err(|_| {
        FlvError::Codec(Error::InvalidInput(
            "FLV tag timestamp does not fit the 32-bit millisecond clock",
        ))
    })
}

/// The `CompositionTime` field of an `AVCVIDEOPACKET` (§E.4.3.2) is a signed
/// 24-bit value in FLV's millisecond clock. `offset` is in `timescale` ticks,
/// so it is rescaled first; a value that does not fit SI24 is an error rather
/// than the silent truncation the raw write used to perform.
fn composition_time_ms(offset: i64, timescale: u32) -> core::result::Result<i32, FlvError> {
    let ms = ticks_to_ms(offset, timescale)?;
    if ms < SI24_MIN as i64 || ms > SI24_MAX as i64 {
        return Err(FlvError::Codec(Error::InvalidInput(
            "FLV CompositionTime does not fit the signed 24-bit field",
        )));
    }
    Ok(ms as i32)
}

/// One track's FLV tag *body* — the bytes an RTMP `send_video`/`send_audio`
/// message must carry (Adobe RTMP 1.0 §7.1.4/§7.1.5) — **without** FLV tag or
/// file framing (issue #934): no `TagType`/`DataSize`/`Timestamp` tag header,
/// no trailing `PreviousTagSize`. An RTMP message already carries its own
/// type + timestamp framing, so only the tag *body* belongs on the wire.
///
/// Built by [`flv_sequence_header_payloads`] (the `avcC`/ASC bodies, sent
/// once) and [`flv_frame_payloads`] (the per-frame bodies, sent
/// continuously) — see either for the exact byte layout, which is identical
/// to what [`FlvMux::package`] writes into each of its FLV tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlvPayload {
    /// Video or audio.
    pub kind: FlvPayloadKind,
    /// This payload's FLV tag `Timestamp` equivalent, in milliseconds — the
    /// DTS an RTMP message carries in its message header.
    pub timestamp_ms: u32,
    /// The tag body: `VideoTagHeader`+`AVCVIDEOPACKET` (§E.4.3) or
    /// `AudioTagHeader`+`AACAUDIODATA` (§E.4.2).
    pub body: Vec<u8>,
}

/// [`FlvPayload`]'s track kind.
///
/// `#[non_exhaustive]`: FLV's mainstream today is AVC video + AAC audio only
/// (see the module doc); a future carried codec is additive, not a breaking
/// match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlvPayloadKind {
    /// A `VideoTagHeader`+`AVCVIDEOPACKET` body (§E.4.3).
    Video,
    /// An `AudioTagHeader`+`AACAUDIODATA` body (§E.4.2).
    Audio,
}

impl FlvPayloadKind {
    /// The FLV tag-type token this payload kind corresponds to (issue #204).
    pub fn name(&self) -> &'static str {
        match self {
            FlvPayloadKind::Video => "video",
            FlvPayloadKind::Audio => "audio",
        }
    }
}

broadcast_common::impl_spec_display!(FlvPayloadKind);

/// Build the AVC/AAC sequence-header payloads for `media`'s (at most one)
/// AVC video track and (at most one) AAC audio track (issue #934) — the
/// `avcC` and `AudioSpecificConfig` bodies a downstream RTMP consumer needs
/// **once**, before any frame payload, to initialise its decoder. `media`'s
/// tracks need no samples for this — only [`TrackSpec::config`](crate::pipeline::TrackSpec::config)
/// is read. Timestamp is always 0, matching the sequence-header tags
/// [`FlvMux::package`] emits before its interleaved frame tags.
pub fn flv_sequence_header_payloads(
    media: &Media,
) -> core::result::Result<Vec<FlvPayload>, FlvError> {
    let (video, audio) = locate_av_tracks(media)?;
    let mut out = Vec::new();
    if let Some(vt) = video
        && let Some(body) = video_sequence_header_body(vt)?
    {
        out.push(FlvPayload {
            kind: FlvPayloadKind::Video,
            timestamp_ms: 0,
            body,
        });
    }
    if let Some(at) = audio
        && let Some(body) = audio_sequence_header_body(at)?
    {
        out.push(FlvPayload {
            kind: FlvPayloadKind::Audio,
            timestamp_ms: 0,
            body,
        });
    }
    Ok(out)
}

/// Build the per-frame `VIDEODATA`/`AUDIODATA` payload bodies for `media`'s
/// samples (issue #934), interleaved by absolute timestamp across tracks —
/// same tag-body layout [`FlvMux::package`] writes into each `OutTag`, without
/// file/tag framing. Each sample's [`Sample::dts`](crate::pipeline::Sample::dts)
/// (absolute, in its track's own timescale) is rescaled to FLV's millisecond
/// clock (this crate's internal `ticks_to_ms` helper) — safe to call once per
/// drained batch from a live push driver, unlike `package`'s zero-based
/// running sum (see that helper's doc for why). A sample with `dts: None` is
/// skipped — this crate never fabricates a timestamp (matches
/// [`Sample::composition_offset`]'s "no timestamp" convention).
pub fn flv_frame_payloads(media: &Media) -> core::result::Result<Vec<FlvPayload>, FlvError> {
    let (video, audio) = locate_av_tracks(media)?;
    // (timestamp_ms, seq_tiebreak, payload): seq preserves each track's own
    // emission order for same-millisecond ties, mirroring `package`.
    let mut items: Vec<(u32, u32, FlvPayload)> = Vec::new();
    let mut seq = 0u32;
    if let Some(vt) = video {
        let timescale = vt.spec.timescale;
        for s in &vt.samples {
            let Some(dts) = s.dts else { continue };
            let ts_ms = clamped_ms(dts, timescale)?;
            let comp_ms = composition_time_ms(s.composition_offset() as i64, timescale)?;
            let body = video_frame_body(s.flags.is_sync, comp_ms, &s.data);
            items.push((
                ts_ms,
                seq,
                FlvPayload {
                    kind: FlvPayloadKind::Video,
                    timestamp_ms: ts_ms,
                    body,
                },
            ));
            seq += 1;
        }
    }
    if let Some(at) = audio {
        let timescale = at.spec.timescale;
        let sound_type = audio_sound_type(at);
        for s in &at.samples {
            let Some(dts) = s.dts else { continue };
            let ts_ms = clamped_ms(dts, timescale)?;
            let body = audio_frame_body(sound_type, &s.data);
            items.push((
                ts_ms,
                seq,
                FlvPayload {
                    kind: FlvPayloadKind::Audio,
                    timestamp_ms: ts_ms,
                    body,
                },
            ));
            seq += 1;
        }
    }
    items.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    Ok(items.into_iter().map(|(_, _, p)| p).collect())
}

impl Package for FlvMux {
    type Media = Media;
    type Output = Vec<u8>;
    type Error = FlvError;

    fn package(&mut self, media: &Media) -> core::result::Result<Vec<u8>, FlvError> {
        let (video, audio) = locate_av_tracks(media)?;

        let mut out = Vec::new();

        // --- Header (§E.2) ---
        let header = FlvHeader::new(audio.is_some(), video.is_some());
        out.extend_from_slice(
            &header
                .try_to_bytes()
                .map_err(|e| Error::FieldOverflow(field_overflow(e)))?,
        );

        // --- onMetaData script tag (§E.4.1) ---
        let meta = on_metadata_body(media, Some(media_duration_seconds(media)))?;
        OutTag {
            tag_type: TagType::Script,
            timestamp: 0,
            body: meta,
        }
        .write_into(&mut out)?;

        // --- Sequence-header tags ---
        if let Some(vt) = video
            && let Some(body) = video_sequence_header_body(vt)?
        {
            OutTag {
                tag_type: TagType::Video,
                timestamp: 0,
                body,
            }
            .write_into(&mut out)?;
        }
        let sound_type = audio.map(audio_sound_type).unwrap_or(SOUND_TYPE_STEREO);
        if let Some(at) = audio
            && let Some(body) = audio_sequence_header_body(at)?
        {
            OutTag {
                tag_type: TagType::Audio,
                timestamp: 0,
                body,
            }
            .write_into(&mut out)?;
        }

        // --- Interleaved A/V type-1 tags, ordered by DTS ---
        // Precompute (dts, tag) for each track, then merge-sort by dts. Note:
        // this `dts` is a running sum of `Sample::duration` from zero (this
        // whole-file muxer's own model — see `ticks_to_ms`'s doc for how the
        // streaming builders below differ), in the *track's own* timescale,
        // rescaled to FLV's millisecond clock on the way into the tag (see
        // `ticks_to_ms`): FLV's `Timestamp`/`CompositionTime` are milliseconds
        // (§E.4.1/§E.4.3.2), and `Sample::duration` is in
        // [`TrackSpec::timescale`](crate::ir::TrackSpec::timescale). Those
        // coincide only when the input came from [`FlvDemux`] (timescale
        // 1000); a 90 kHz `TsDemux` input used to be written 90× too large,
        // silently truncating any composition offset past the SI24 range.
        let mut items: Vec<(u32, u32, OutTag)> = Vec::new(); // (dts, seq_tiebreak, tag)
        let mut seq = 0u32;
        if let Some(vt) = video {
            let timescale = vt.spec.timescale;
            let mut ticks = 0i64;
            for s in &vt.samples {
                // Rounding each absolute position (rather than accumulating
                // rounded deltas) keeps the tag clock non-decreasing: the
                // running tick sum is monotonic by construction, so truncating
                // division cannot step it backwards.
                let dts = ms_from_ticks(ticks, timescale)?;
                let comp = composition_time_ms(s.composition_offset() as i64, timescale)?;
                let body = video_frame_body(s.flags.is_sync, comp, &s.data);
                items.push((
                    dts,
                    seq,
                    OutTag {
                        tag_type: TagType::Video,
                        timestamp: dts,
                        body,
                    },
                ));
                seq += 1;
                ticks = ticks.saturating_add(s.duration.unwrap_or(0) as i64);
            }
        }
        if let Some(at) = audio {
            let timescale = at.spec.timescale;
            let mut ticks = 0i64;
            for s in &at.samples {
                let dts = ms_from_ticks(ticks, timescale)?;
                let body = audio_frame_body(sound_type, &s.data);
                items.push((
                    dts,
                    seq,
                    OutTag {
                        tag_type: TagType::Audio,
                        timestamp: dts,
                        body,
                    },
                ));
                seq += 1;
                ticks = ticks.saturating_add(s.duration.unwrap_or(0) as i64);
            }
        }
        // Stable sort by DTS, tie-broken by original emission order.
        items.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, _, tag) in &items {
            tag.write_into(&mut out)?;
        }

        Ok(out)
    }
}

/// The `AudioTagHeader` first byte for AAC (§E.4.2): SoundFormat(4)=10,
/// SoundRate(2)=3, SoundSize(1)=1, SoundType(1).
fn audio_tag_header_byte(sound_type: u8) -> u8 {
    (SOUND_FORMAT_AAC << 4) | (SOUND_RATE_44K << 2) | (SOUND_SIZE_16BIT << 1) | (sound_type & 1)
}

/// Extract the raw `AudioSpecificConfig` bytes from an `esds`'s DecoderSpecificInfo.
fn esds_asc_bytes(esds: &EsdsBox) -> core::result::Result<Vec<u8>, FlvError> {
    esds.es_descriptor
        .decoder_config
        .as_ref()
        .and_then(|dc| dc.decoder_specific_info.as_ref())
        .map(|dsi| dsi.data.clone())
        .ok_or(FlvError::Codec(Error::InvalidInput(
            "AAC esds has no AudioSpecificConfig (DecoderSpecificInfo)",
        )))
}

/// Longest track duration in whole seconds (integer, for the `onMetaData` field).
fn media_duration_seconds(media: &Media) -> f64 {
    let mut max = 0.0f64;
    for t in &media.tracks {
        let ticks: u64 = t
            .samples
            .iter()
            .map(|s| s.duration.unwrap_or(0) as u64)
            .sum();
        let ts = if t.spec.timescale == 0 {
            FLV_TIMESCALE
        } else {
            t.spec.timescale
        } as f64;
        let secs = ticks as f64 / ts;
        if secs > max {
            max = secs;
        }
    }
    max
}

// --- onMetaData (Adobe FLV v10.1 §E.4.1, AMF0 §2), encoded by rtmp-runtime -----

/// FLV `videocodecid` for AVC (= `CodecID` 7).
const META_VIDEOCODECID_AVC: f64 = 7.0;
/// FLV `audiocodecid` for AAC (= `SoundFormat` 10).
const META_AUDIOCODECID_AAC: f64 = 10.0;
/// The handler name an `onMetaData` script body starts with (§E.4.1).
const META_HANDLER_NAME: &str = "onMetaData";
/// The AAC channel count `onMetaData`'s `stereo` flag stands for (§E.5: the
/// same two-channel answer the FLV `SoundType` bit gives).
const AAC_CHANNELS_STEREO: u32 = 2;

/// The `onMetaData` body — the AMF0 `onMetaData` name followed by one ECMA
/// array of the informational properties FLV players read, in the order they
/// are written (Adobe FLV v10.1 §E.4.1, AMF0 §2) — that every FLV/RTMP writer
/// in the workspace emits, so a file muxed by [`FlvMux`] and a stream
/// published over RTMP describe the same tracks with the same bytes.
///
/// `duration` is the only property a caller decides: a file knows its own
/// length ([`FlvMux`] passes the media's), a live publisher does not and
/// passes `None`, which omits the key rather than claiming a length. A caller
/// that must carry the file's tag bytes must also read the properties back:
/// one written as an AMF0 Object re-reads as one, so re-encoding its value
/// writes an extra end-of-object marker the file never had.
///
/// Tracks FLV cannot carry ([`flv_track_kind`]) contribute nothing, but the
/// media must still hold at least one AVC video or AAC audio track.
///
/// # Errors
///
/// [`FlvError::NoSupportedTrack`] when no track is carryable, and
/// [`FlvError::Codec`] wrapping the AMF0 field overflow when a key exceeds
/// its length prefix.
pub fn on_metadata_body(
    media: &Media,
    duration: Option<f64>,
) -> core::result::Result<Vec<u8>, FlvError> {
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }));
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }));
    if video.is_none() && audio.is_none() {
        return Err(FlvError::NoSupportedTrack);
    }
    let (width, height) = match video.map(|t| &t.spec.config) {
        Some(CodecConfig::Avc { width, height, .. }) => (*width, *height),
        _ => (0, 0),
    };
    let audio = match audio.map(|t| &t.spec.config) {
        Some(CodecConfig::Aac {
            sample_rate,
            channel_count,
            ..
        }) => Some((*sample_rate, *channel_count)),
        _ => None,
    };
    let props = onmetadata_properties(duration, width, height, video.is_some(), audio);
    amf0_values(&[
        Amf0Value::String(META_HANDLER_NAME.to_string()),
        Amf0Value::EcmaArray(props),
    ])
    .map_err(|e| FlvError::Codec(Error::FieldOverflow(field_overflow(e))))
}

/// Whether FLV can carry `config`: AVC video or AAC audio, the two codecs of
/// the FLV mainstream (Adobe FLV v10.1 §E.4.3.2 / §E.4.2.2). Every other
/// codec — including the opaque [`CodecConfig::Data`] track a TS demux keeps
/// — has no FLV tag, so a FLV or RTMP writer excludes it.
#[must_use]
pub fn flv_track_kind(config: &CodecConfig) -> Option<FlvPayloadKind> {
    match config {
        CodecConfig::Avc { .. } => Some(FlvPayloadKind::Video),
        CodecConfig::Aac { .. } => Some(FlvPayloadKind::Audio),
        _ => None,
    }
}

/// The onMetaData property list of one media (AMF0 §2.10).
fn onmetadata_properties(
    duration: Option<f64>,
    width: u16,
    height: u16,
    has_video: bool,
    audio: Option<(u32, u16)>,
) -> Vec<(String, Amf0Value)> {
    let mut props: Vec<(String, Amf0Value)> = Vec::new();
    if let Some(duration) = duration {
        props.push(("duration".to_string(), Amf0Value::Number(duration)));
    }
    if has_video {
        props.push(("width".to_string(), Amf0Value::Number(f64::from(width))));
        props.push(("height".to_string(), Amf0Value::Number(f64::from(height))));
        props.push((
            "videocodecid".to_string(),
            Amf0Value::Number(META_VIDEOCODECID_AVC),
        ));
    }
    if let Some((sample_rate, channel_count)) = audio {
        props.push((
            "audiocodecid".to_string(),
            Amf0Value::Number(META_AUDIOCODECID_AAC),
        ));
        props.push((
            "audiosamplerate".to_string(),
            Amf0Value::Number(f64::from(sample_rate)),
        ));
        props.push((
            "stereo".to_string(),
            Amf0Value::Boolean(u32::from(channel_count) == AAC_CHANNELS_STEREO),
        ));
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn si24_sign_extends() {
        assert_eq!(read_si24(&[0x00, 0x00, 0x50]), 80);
        assert_eq!(read_si24(&[0x00, 0x00, 0x00]), 0);
        // 0xFFFFFF = -1
        assert_eq!(read_si24(&[0xFF, 0xFF, 0xFF]), -1);
    }

    #[test]
    fn audio_header_byte_layout() {
        // SoundFormat 10, rate 3, size 1, type stereo(1): 1010_11_1_1 = 0xAF.
        assert_eq!(audio_tag_header_byte(SOUND_TYPE_STEREO), 0xAF);
        // mono: 1010_11_1_0 = 0xAE.
        assert_eq!(audio_tag_header_byte(SOUND_TYPE_MONO), 0xAE);
    }

    /// A 16 MiB (2^24) tag body cannot fit the 24-bit `DataSize` field
    /// (#1129): unfixed, `(data_size as u32).to_be_bytes()[1..]` silently
    /// keeps only the low 24 bits.
    #[test]
    fn oversized_tag_body_errors() {
        let tag = OutTag {
            tag_type: TagType::Video,
            timestamp: 0,
            body: alloc::vec![0u8; 1 << 24],
        };
        let mut out = Vec::new();
        let err = tag.write_into(&mut out).unwrap_err();
        assert!(
            matches!(
                err,
                FlvError::Codec(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "DataSize",
                    ..
                }))
            ),
            "expected FieldOverflow for DataSize, got {err:?}"
        );
    }

    /// The boundary: exactly (2^24 - 1) bytes still writes and round-trips
    /// the DataSize field.
    #[test]
    fn max_tag_body_round_trips() {
        let tag = OutTag {
            tag_type: TagType::Video,
            timestamp: 0,
            body: alloc::vec![0xAAu8; (1 << 24) - 1],
        };
        let mut out = Vec::new();
        tag.write_into(&mut out).unwrap();
        let data_size = ((out[1] as u32) << 16) | ((out[2] as u32) << 8) | out[3] as u32;
        assert_eq!(data_size, (1 << 24) - 1);
    }

    /// Write one FLV tag (header + body + `PreviousTagSize`) into `out`.
    fn write_flv_tag(out: &mut Vec<u8>, tag_type: u8, body: &[u8]) {
        let start = out.len();
        out.push(tag_type);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(&[0, 0, 0, 0]); // timestamp + extended
        out.extend_from_slice(&[0, 0, 0]); // StreamID
        out.extend_from_slice(body);
        let size = (out.len() - start) as u32;
        out.extend_from_slice(&size.to_be_bytes());
    }

    /// An `avcC` declaring a 2-byte NAL length prefix (`lengthSizeMinusOne = 1`).
    fn avcc_length_size_2() -> Vec<u8> {
        let sps: [u8; 6] = [0x67, 0x42, 0x00, 0x1F, 0x00, 0x00];
        let mut out = alloc::vec![
            0x01, 0x42, 0x00, 0x1F, // version, profile, compat, level
            0xFD, // reserved(6) + lengthSizeMinusOne(2) = 1
            0xE1, // reserved(3) + numSPS(5) = 1
        ];
        out.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        out.extend_from_slice(&sps);
        out.push(0x00); // numPPS = 0
        out
    }

    /// r04-W43 (`FlvDemux`): a NALU tag whose sample uses 2-byte NAL prefixes is
    /// normalised to 4-byte, **and** the emitted `avcC` declares 4-byte lengths
    /// so a fMP4/CMAF/TS mux of the IR describes the samples correctly.
    #[test]
    fn flv_demux_two_byte_nal_lengths_normalised_to_four() {
        let mut flv = alloc::vec![b'F', b'L', b'V', 0x01, 0x01, 0x00, 0x00, 0x00, 0x09];
        flv.extend_from_slice(&[0, 0, 0, 0]); // PreviousTagSize0
        let mut seq = alloc::vec![0x17, 0x00, 0x00, 0x00, 0x00];
        seq.extend_from_slice(&avcc_length_size_2());
        write_flv_tag(&mut flv, tag_type::VIDEO, &seq);
        let mut nalu = alloc::vec![0x17, 0x01, 0x00, 0x00, 0x00];
        nalu.extend_from_slice(&[0x00, 0x05, 0x65, 0x88, 0x84, 0x00, 0x21]);
        nalu.extend_from_slice(&[0x00, 0x01, 0x41]);
        write_flv_tag(&mut flv, tag_type::VIDEO, &nalu);

        let media = FlvDemux::new().unpackage(&flv).expect("FLV -> IR");
        let track = &media.tracks[0];
        assert_eq!(
            track.samples[0].data.as_ref(),
            [
                0x00, 0x00, 0x00, 0x05, 0x65, 0x88, 0x84, 0x00, 0x21, 0x00, 0x00, 0x00, 0x01, 0x41,
            ],
            "2-byte NAL lengths must be rewritten to 4-byte"
        );
        let CodecConfig::Avc { config, .. } = &track.spec.config else {
            panic!("expected CodecConfig::Avc");
        };
        assert_eq!(
            config.config.length_size_minus_one, NAL_LENGTH_SIZE_MINUS_ONE,
            "the emitted avcC must declare the 4-byte length size"
        );
    }
}
