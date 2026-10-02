//! FLV (Flash Video) container spoke — Adobe Flash Video File Format
//! Specification **v10.1, Annex E**; see `transmux/docs/codec/flv.md`.
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

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;

use broadcast_common::{Package, Parse, Serialize, Unpackage};

use crate::aac_asc::AudioSpecificConfig;
use crate::annexb::{NAL_LENGTH_SIZE, NAL_LENGTH_SIZE_MINUS_ONE, normalise_nal_length_size};
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};

// ---------------------------------------------------------------------------
// Spec constants (Adobe FLV v10.1 Annex E) — no magic numbers outside tests.
// ---------------------------------------------------------------------------

/// FLV signature `"FLV"` (§E.2).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738) to
/// validate the header incrementally without duplicating this constant.
pub(crate) const FLV_SIGNATURE: [u8; 3] = *b"FLV";
/// FLV file-format version this crate emits (§E.2).
const FLV_VERSION: u8 = 1;
/// FLV header length in bytes (§E.2): signature(3) + version(1) + flags(1) + data_offset(4).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const FLV_HEADER_LEN: usize = 9;
/// `TypeFlags` bit 0 — audio tags present (§E.2).
const TYPE_FLAG_AUDIO: u8 = 0x04;
/// `TypeFlags` bit 2 — video tags present (§E.2).
const TYPE_FLAG_VIDEO: u8 = 0x01;
/// Size of a tag header before its body (§E.4.1): type(1)+size(3)+ts(3)+tsext(1)+stream(3).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const TAG_HEADER_LEN: usize = 11;
/// Size of the `PreviousTagSize` trailer after every tag (§E.4.1).
///
/// `pub(crate)`: reused by [`crate::flv_stream::StreamingFlvDemux`] (#738).
pub(crate) const PREV_TAG_SIZE_LEN: usize = 4;
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
#[derive(Debug, PartialEq)]
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
    tag_type: u8,
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
}

/// Iterate the tags of an FLV stream after its 9-byte header, validating each
/// tag's `DataSize` against the buffer (§E.4.1).
///
/// A truncated **final** tag is not an error: a recorded or captured live FLV
/// routinely ends mid-tag, and the tags before it are complete and usable. The
/// walk stops at the short tail, keeping what it has (`ac3`/`dts`/`mpeg_legacy`
/// syncframe splitters in this crate take the same "stop at the truncated tail,
/// never drop the good prefix" position) and reporting it through
/// [`TagWalk::truncated_tail`], so nothing is dropped silently.
///
/// Two shapes are *not* truncation, and both are accepted:
/// - a final tag whose **body is entirely present** but whose 4-byte
///   `PreviousTagSize` is missing (`body_end <= len < body_end + 4`): the body
///   is complete, so it is kept and the walk ends cleanly;
/// - a tag header at the very end of the buffer that cannot even be read yet
///   (`off + TAG_HEADER_LEN > len`) — there is nothing to interpret there.
///
/// A tag whose body overruns the buffer is only tolerated where it can be
/// *known* to be the tail, which is exactly what the loop guarantees: it is the
/// last tag the buffer can start. A stream whose very first tag is already
/// incomplete has nothing to return at all, so that alone is
/// [`FlvError::TagOverrun`] — the caller gets a diagnosis rather than a bare
/// `NoSupportedTrack`.
fn iter_tags(input: &[u8]) -> Result<TagWalk<'_>> {
    if input.len() < FLV_HEADER_LEN + PREV_TAG_SIZE_LEN {
        return Err(Error::BufferTooShort {
            need: FLV_HEADER_LEN + PREV_TAG_SIZE_LEN,
            have: input.len(),
            what: "FLV header",
        });
    }
    // DataOffset (bytes [5:9]) points past the header to the first PreviousTagSize0.
    let data_offset = u32::from_be_bytes([input[5], input[6], input[7], input[8]]) as usize;
    // Skip the header and the first PreviousTagSize0 (4 bytes).
    let mut off = data_offset.max(FLV_HEADER_LEN) + PREV_TAG_SIZE_LEN;

    let mut tags = Vec::new();
    while off + TAG_HEADER_LEN <= input.len() {
        let tag_type = input[off];
        let data_size =
            u32::from_be_bytes([0, input[off + 1], input[off + 2], input[off + 3]]) as usize;
        // Timestamp: UI24 low + UI8 extended high byte → 32-bit ms.
        let ts_lo = u32::from_be_bytes([0, input[off + 4], input[off + 5], input[off + 6]]);
        let ts_ext = input[off + 7] as u32;
        let timestamp = (ts_ext << 24) | ts_lo;
        // input[off+8..off+11] = StreamID (always 0), ignored.
        let body_start = off + TAG_HEADER_LEN;
        // Checked: a hostile `DataSize` (up to 2^24-1) added at a near-`usize::MAX`
        // offset would otherwise wrap.
        let Some(body_end) = body_start.checked_add(data_size) else {
            return Err(Error::from(FlvErrorAsError(FlvError::TagOverrun {
                offset: off,
                need: usize::MAX,
                have: input.len(),
            })));
        };
        if body_end > input.len() {
            // The tag's *body* is not all here, so this is the truncated tail —
            // the only tag that may be treated as one, since it is the last the
            // buffer can start. Exposed, never dropped silently.
            if tags.is_empty() {
                return Err(Error::from(FlvErrorAsError(FlvError::TagOverrun {
                    offset: off,
                    need: body_end + PREV_TAG_SIZE_LEN,
                    have: input.len(),
                })));
            }
            return Ok(TagWalk {
                tags,
                truncated_tail: true,
            });
        }
        tags.push(FlvTag {
            tag_type,
            timestamp,
            body: &input[body_start..body_end],
        });
        // Advance past the body and its trailing PreviousTagSize. A tag whose
        // body is complete but whose `PreviousTagSize` is cut off is still a
        // complete tag (§E.4.1 makes that field redundant — it repeats the
        // preceding tag's size), so it is kept and the walk simply ends.
        if body_end + PREV_TAG_SIZE_LEN > input.len() {
            return Ok(TagWalk {
                tags,
                truncated_tail: false,
            });
        }
        off = body_end + PREV_TAG_SIZE_LEN;
    }
    Ok(TagWalk {
        tags,
        truncated_tail: false,
    })
}

// A tiny bridge so `iter_tags` can surface FLV-specific overrun through the
// crate `Error` used by the reused config helpers. `TagOverrun` maps onto the
// structured `BufferTooShort` shape.
struct FlvErrorAsError(FlvError);
impl From<FlvErrorAsError> for Error {
    fn from(w: FlvErrorAsError) -> Self {
        match w.0 {
            FlvError::TagOverrun { need, have, .. } => Error::BufferTooShort {
                need,
                have,
                what: "FLV tag body",
            },
            other => Error::InvalidInput(match other {
                FlvError::BadSignature(_) => "FLV bad signature",
                FlvError::NoSupportedTrack => "FLV no supported track",
                FlvError::UnsupportedCodec { .. } => "FLV unsupported codec",
                FlvError::HeaderTooLarge { .. } => "FLV header DataOffset too large",
                _ => "FLV error",
            }),
        }
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
            return Err(FlvError::Codec(Error::BufferTooShort {
                need: FLV_HEADER_LEN,
                have: input.len(),
                what: "FLV header",
            }));
        }
        if input[0..3] != FLV_SIGNATURE {
            return Err(FlvError::BadSignature([input[0], input[1], input[2]]));
        }

        let walk = iter_tags(input).map_err(FlvError::Codec)?;
        self.truncated_tail = walk.truncated_tail;
        let tags = walk.tags;

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
                tag_type::VIDEO => {
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
                            let mut record = AVCDecoderConfigurationRecord::parse(data)
                                .map_err(FlvError::Codec)?;
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
                            let duration = delta_duration(&mut last_video_dts, dts);
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
                            // declared (§E.4.3.1 / ISO/IEC 14496-15 §5.3.3) —
                            // 1, 2 or 4 bytes. The rest of the pipeline is
                            // fixed at 4, so rewrite them (r04-W43).
                            let length_size = avc_source_length_size.unwrap_or(NAL_LENGTH_SIZE);
                            let data = normalise_nal_length_size(data, length_size)
                                .map_err(FlvError::Codec)?;
                            video_samples.push(Sample {
                                data: data.into(),
                                dts: Some(dts_abs),
                                pts: Some(pts_abs),
                                duration: Some(duration),
                                flags: crate::ir::SampleFlags::new(
                                    frame_type == FRAME_TYPE_KEYFRAME,
                                ),
                                provenance: None,
                            });
                        }
                        avc_packet_type::END_OF_SEQUENCE => {}
                        _ => {}
                    }
                }
                tag_type::AUDIO => {
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
                            let asc = AudioSpecificConfig::parse(data).map_err(FlvError::Codec)?;
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
                            });
                        }
                        _ => {}
                    }
                }
                tag_type::SCRIPT => { /* onMetaData — informational, skipped */ }
                _ => { /* unknown tag type — skipped leniently */ }
            }
        }

        // Backfill the final sample's duration from the previous delta (no next
        // tag to measure against): reuse the second-to-last delta.
        backfill_last_duration(&mut video_samples);
        backfill_last_duration(&mut audio_samples);

        let mut tracks: Vec<Track> = Vec::new();
        let mut track_id = 1u32;
        if let Some(config) = avc_config
            && !video_samples.is_empty()
        {
            // #738 T11a review (Critical): `AVCDecoderConfigurationRecord::parse`
            // now rejects 0 SPS, so this is unreachable via `parse` — but
            // `config.sps` is a public field and a directly-constructed
            // record could still be empty, so index defensively via
            // `.first()` rather than `[0]` (no panic either way).
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
            track_id += 1;
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
        Ok(Media::new(tracks, FLV_TIMESCALE))
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
struct OutTag {
    tag_type: u8,
    timestamp: u32,
    body: Vec<u8>,
}

impl OutTag {
    fn write_into(&self, out: &mut Vec<u8>) -> core::result::Result<(), FlvError> {
        let data_size = broadcast_common::len::fit_u24(self.body.len(), "DataSize")
            .map_err(Error::FieldOverflow)?;
        let start = out.len();
        out.push(self.tag_type);
        out.extend_from_slice(&data_size.to_be_bytes()[1..]); // UI24
        let ts = self.timestamp;
        out.push((ts >> 16) as u8);
        out.push((ts >> 8) as u8);
        out.push(ts as u8);
        out.push((ts >> 24) as u8); // TimestampExtended
        out.extend_from_slice(&[0, 0, 0]); // StreamID = 0
        out.extend_from_slice(&self.body);
        let tag_size = (out.len() - start) as u32;
        out.extend_from_slice(&tag_size.to_be_bytes()); // PreviousTagSize
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
                    codec: codec_name(other),
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
fn ticks_to_ms(ticks: i64, timescale: u32) -> i64 {
    if timescale == 0 {
        return 0;
    }
    (ticks as i128 * FLV_TIMESCALE as i128 / timescale as i128) as i64
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
    let ms = ticks_to_ms(ticks, timescale);
    if ms < 0 || ms > u32::MAX as i64 {
        return Err(FlvError::Codec(Error::InvalidInput(
            "FLV tag timestamp does not fit the 32-bit millisecond clock",
        )));
    }
    Ok(ms as u32)
}

/// The `CompositionTime` field of an `AVCVIDEOPACKET` (§E.4.3.2) is a signed
/// 24-bit value in FLV's millisecond clock. `offset` is in `timescale` ticks,
/// so it is rescaled first; a value that does not fit SI24 is an error rather
/// than the silent truncation the raw write used to perform.
fn composition_time_ms(offset: i64, timescale: u32) -> core::result::Result<i32, FlvError> {
    let ms = ticks_to_ms(offset, timescale);
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
            let ts_ms = ticks_to_ms(dts, timescale).max(0) as u32;
            let comp_ms = ticks_to_ms(s.composition_offset() as i64, timescale) as i32;
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
            let ts_ms = ticks_to_ms(dts, timescale).max(0) as u32;
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
        let mut type_flags = 0u8;
        if video.is_some() {
            type_flags |= TYPE_FLAG_VIDEO;
        }
        if audio.is_some() {
            type_flags |= TYPE_FLAG_AUDIO;
        }
        out.extend_from_slice(&FLV_SIGNATURE);
        out.push(FLV_VERSION);
        out.push(type_flags);
        out.extend_from_slice(&(FLV_HEADER_LEN as u32).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // PreviousTagSize0 = 0

        // --- onMetaData script tag (§E.4.1) ---
        let (width, height) = match video.map(|t| &t.spec.config) {
            Some(CodecConfig::Avc { width, height, .. }) => (*width, *height),
            _ => (0, 0),
        };
        let duration_s = media_duration_seconds(media);
        let meta = build_onmetadata(duration_s, width, height, video.is_some(), audio.is_some());
        OutTag {
            tag_type: tag_type::SCRIPT,
            timestamp: 0,
            body: meta,
        }
        .write_into(&mut out)?;

        // --- Sequence-header tags ---
        if let Some(vt) = video
            && let Some(body) = video_sequence_header_body(vt)?
        {
            OutTag {
                tag_type: tag_type::VIDEO,
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
                tag_type: tag_type::AUDIO,
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
                        tag_type: tag_type::VIDEO,
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
                        tag_type: tag_type::AUDIO,
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

/// Codec name for the [`FlvError::UnsupportedCodec`] message.
fn codec_name(c: &CodecConfig) -> &'static str {
    match c {
        CodecConfig::Avc { .. } => "AVC",
        CodecConfig::Hevc { .. } => "HEVC",
        CodecConfig::Vvc { .. } => "VVC",
        CodecConfig::Aac { .. } => "AAC",
        CodecConfig::Ac3 { .. } => "AC-3",
        CodecConfig::Eac3 { .. } => "E-AC-3",
        CodecConfig::Av1 { .. } => "AV1",
        CodecConfig::Vp9 { .. } => "VP9",
        CodecConfig::Opus { .. } => "Opus",
        CodecConfig::Flac { .. } => "FLAC",
        CodecConfig::Ac4 { .. } => "AC-4",
        CodecConfig::MpegH { .. } => "MPEG-H",
        CodecConfig::Mpeg2Video { .. } => "MPEG-2 video",
        CodecConfig::MpegAudio { .. } => "MPEG audio",
        CodecConfig::Dts { .. } => "DTS",
        CodecConfig::Vp8 { .. } => "VP8",
        CodecConfig::Vorbis { .. } => "Vorbis",
        CodecConfig::Data { .. } => "Data",
        CodecConfig::Subtitle { .. } => "Subtitle",
    }
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

// --- Minimal AMF0 onMetaData (Adobe FLV v10.1 §E.4.1, AMF0 §2) --------------

/// AMF0 type marker: number (double, §2.2).
const AMF0_NUMBER: u8 = 0x00;
/// AMF0 type marker: boolean (§2.3).
const AMF0_BOOLEAN: u8 = 0x01;
/// AMF0 type marker: string (§2.4).
const AMF0_STRING: u8 = 0x02;
/// AMF0 type marker: ECMA array (§2.10).
const AMF0_ECMA_ARRAY: u8 = 0x08;
/// AMF0 object-end marker (§2.11): a 0-length key followed by 0x09.
const AMF0_OBJECT_END: u8 = 0x09;
/// FLV `videocodecid` for AVC (= `CodecID` 7).
const META_VIDEOCODECID_AVC: f64 = 7.0;
/// FLV `audiocodecid` for AAC (= `SoundFormat` 10).
const META_AUDIOCODECID_AAC: f64 = 10.0;

fn amf0_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn amf0_named_number(out: &mut Vec<u8>, key: &str, v: f64) {
    amf0_string(out, key);
    out.push(AMF0_NUMBER);
    out.extend_from_slice(&v.to_be_bytes());
}

fn amf0_named_bool(out: &mut Vec<u8>, key: &str, v: bool) {
    amf0_string(out, key);
    out.push(AMF0_BOOLEAN);
    out.push(v as u8);
}

/// Build the `onMetaData` script-tag body: an AMF0 string `"onMetaData"`
/// followed by an ECMA array of the standard informational properties.
fn build_onmetadata(
    duration: f64,
    width: u16,
    height: u16,
    has_video: bool,
    has_audio: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(AMF0_STRING);
    amf0_string(&mut out, "onMetaData");

    out.push(AMF0_ECMA_ARRAY);
    // Count properties for the ECMA-array header (approximate is legal; players
    // read to the object-end marker regardless — §2.10).
    let mut props: Vec<(&str, Prop)> = Vec::new();
    props.push(("duration", Prop::Num(duration)));
    if has_video {
        props.push(("width", Prop::Num(width as f64)));
        props.push(("height", Prop::Num(height as f64)));
        props.push(("videocodecid", Prop::Num(META_VIDEOCODECID_AVC)));
    }
    if has_audio {
        props.push(("audiocodecid", Prop::Num(META_AUDIOCODECID_AAC)));
        props.push(("stereo", Prop::Bool(true)));
    }
    out.extend_from_slice(&(props.len() as u32).to_be_bytes());
    for (k, v) in &props {
        match v {
            Prop::Num(n) => amf0_named_number(&mut out, k, *n),
            Prop::Bool(b) => amf0_named_bool(&mut out, k, *b),
        }
    }
    // Object end: empty key (u16 len 0) + object-end marker.
    out.extend_from_slice(&0u16.to_be_bytes());
    out.push(AMF0_OBJECT_END);
    out
}

enum Prop {
    Num(f64),
    Bool(bool),
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
            tag_type: tag_type::VIDEO,
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
            tag_type: tag_type::VIDEO,
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
