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
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType as EsdsStreamType,
};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};

// ---------------------------------------------------------------------------
// Spec constants (Adobe FLV v10.1 Annex E) — no magic numbers outside tests.
// ---------------------------------------------------------------------------

/// FLV signature `"FLV"` (§E.2).
const FLV_SIGNATURE: [u8; 3] = *b"FLV";
/// FLV file-format version this crate emits (§E.2).
const FLV_VERSION: u8 = 1;
/// FLV header length in bytes (§E.2): signature(3) + version(1) + flags(1) + data_offset(4).
const FLV_HEADER_LEN: usize = 9;
/// `TypeFlags` bit 0 — audio tags present (§E.2).
const TYPE_FLAG_AUDIO: u8 = 0x04;
/// `TypeFlags` bit 2 — video tags present (§E.2).
const TYPE_FLAG_VIDEO: u8 = 0x01;
/// Size of a tag header before its body (§E.4.1): type(1)+size(3)+ts(3)+tsext(1)+stream(3).
const TAG_HEADER_LEN: usize = 11;
/// Size of the `PreviousTagSize` trailer after every tag (§E.4.1).
const PREV_TAG_SIZE_LEN: usize = 4;

/// `TagType` values (§E.4.1).
mod tag_type {
    /// Audio tag (`AudioTagHeader` + AACAUDIODATA).
    pub const AUDIO: u8 = 8;
    /// Video tag (`VideoTagHeader` + AVCVIDEOPACKET).
    pub const VIDEO: u8 = 9;
    /// Script-data tag (`onMetaData`); informational.
    pub const SCRIPT: u8 = 18;
}

/// `CodecID` for AVC/H.264 in a `VideoTagHeader` (§E.4.3).
const CODEC_ID_AVC: u8 = 7;
/// `FrameType` for a keyframe / seekable frame (§E.4.3).
const FRAME_TYPE_KEYFRAME: u8 = 1;
/// `FrameType` for an inter frame (non-seekable) (§E.4.3).
const FRAME_TYPE_INTER: u8 = 2;

/// `AVCPacketType` values (§E.4.3.2).
mod avc_packet_type {
    /// AVC sequence header — the `AVCDecoderConfigurationRecord` (`avcC`).
    pub const SEQUENCE_HEADER: u8 = 0;
    /// One or more length-prefixed NAL units.
    pub const NALU: u8 = 1;
    /// End of sequence (empty body).
    pub const END_OF_SEQUENCE: u8 = 2;
}

/// `SoundFormat` for AAC in an `AudioTagHeader` (§E.4.2).
const SOUND_FORMAT_AAC: u8 = 10;
/// `SoundRate` code 3 = 44 kHz — always used for AAC (real rate is in the ASC) (§E.4.2).
const SOUND_RATE_44K: u8 = 3;
/// `SoundSize` code 1 = 16-bit samples (§E.4.2).
const SOUND_SIZE_16BIT: u8 = 1;
/// `SoundType` code 1 = stereo (§E.4.2).
const SOUND_TYPE_STEREO: u8 = 1;
/// `SoundType` code 0 = mono (§E.4.2).
const SOUND_TYPE_MONO: u8 = 0;

/// `AACPacketType` values (§E.4.2.2).
mod aac_packet_type {
    /// AAC sequence header — the `AudioSpecificConfig`.
    pub const SEQUENCE_HEADER: u8 = 0;
    /// One raw AAC access unit.
    pub const RAW: u8 = 1;
}

/// The IR timescale FLV uses: milliseconds (FLV tag timestamps are in ms, §E.4.1),
/// so sample durations / composition offsets round-trip losslessly.
const FLV_TIMESCALE: u32 = 1000;

// `esds` construction constants (mirroring `ts_demux`).
/// MPEG-4 Audio object type indication for AAC (ISO/IEC 14496-1 §7.2.6.6 Table 5).
const OTI_MPEG4_AUDIO: u8 = 0x40;
/// `streamType` = AudioStream (ISO/IEC 14496-1 §7.2.6.6.2 Table 6).
const STREAM_TYPE_AUDIO: u8 = 0x05;
/// `ES_ID` assigned to the single audio elementary stream.
const ESDS_AUDIO_ES_ID: u16 = 1;
/// `SLConfigDescriptor.predefined = 2` (MP4 default; ISO/IEC 14496-1 §7.3.2.3).
const SL_CONFIG_PREDEFINED_MP4: u8 = 0x02;
/// Audio sample size in bits carried in the sample entry (typically 16).
const AUDIO_SAMPLE_SIZE_BITS: u16 = 16;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors specific to FLV framing (Adobe FLV v10.1 Annex E).
#[derive(Debug, PartialEq, Eq)]
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
}

impl FlvDemux<'_> {
    /// Create a new demuxer.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// One parsed FLV tag (header fields + body slice).
struct FlvTag<'a> {
    tag_type: u8,
    timestamp: u32,
    body: &'a [u8],
}

/// Iterate the tags of an FLV stream after its 9-byte header, validating each
/// tag's `DataSize` against the buffer (§E.4.1).
fn iter_tags(input: &[u8]) -> Result<Vec<FlvTag<'_>>> {
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
        let body_end = body_start + data_size;
        if body_end + PREV_TAG_SIZE_LEN > input.len() {
            return Err(Error::from(FlvErrorAsError(FlvError::TagOverrun {
                offset: off,
                need: body_end + PREV_TAG_SIZE_LEN,
                have: input.len(),
            })));
        }
        tags.push(FlvTag {
            tag_type,
            timestamp,
            body: &input[body_start..body_end],
        });
        // Advance past the body and its trailing PreviousTagSize.
        off = body_end + PREV_TAG_SIZE_LEN;
    }
    Ok(tags)
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

        let tags = iter_tags(input).map_err(FlvError::Codec)?;

        // Video track state.
        let mut avc_config: Option<AVCConfigurationBox> = None;
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
                        avc_packet_type::SEQUENCE_HEADER => {
                            if avc_config.is_none() && !data.is_empty() {
                                let record = AVCDecoderConfigurationRecord::parse(data)
                                    .map_err(FlvError::Codec)?;
                                avc_config = Some(AVCConfigurationBox::new(record));
                            }
                        }
                        avc_packet_type::NALU => {
                            let dts = tag.timestamp;
                            let duration = delta_duration(&mut last_video_dts, dts);
                            video_samples.push(Sample {
                                // FLV NALU data is already 4-byte length-prefixed,
                                // matching the IR (crate::annexb) — pass through.
                                data: data.to_vec(),
                                duration,
                                is_sync: frame_type == FRAME_TYPE_KEYFRAME,
                                composition_offset: composition_time,
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
                        aac_packet_type::SEQUENCE_HEADER => {
                            // Some muxers emit a spurious empty AAC sequence
                            // header; keep looking until a non-empty ASC arrives.
                            if aac_esds.is_none() && !data.is_empty() {
                                let asc =
                                    AudioSpecificConfig::parse(data).map_err(FlvError::Codec)?;
                                aac_channels = asc.channel_configuration.raw() as u16;
                                aac_rate = asc_rate_hz(&asc);
                                aac_esds = Some(build_aac_esds(data.to_vec()));
                            }
                        }
                        aac_packet_type::RAW => {
                            let dts = tag.timestamp;
                            let duration = delta_duration(&mut last_audio_dts, dts);
                            audio_samples.push(Sample {
                                data: data.to_vec(),
                                duration,
                                is_sync: true,
                                composition_offset: 0,
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
        if let Some(config) = avc_config {
            if !video_samples.is_empty() {
                let (width, height) = crate::sps::decode_avc_sps(&config.config.sps[0].0)
                    .map(|i| (i.width as u16, i.height as u16))
                    .unwrap_or((0, 0));
                tracks.push(Track::new(
                    TrackSpec {
                        track_id,
                        timescale: FLV_TIMESCALE,
                        config: CodecConfig::Avc {
                            config,
                            width,
                            height,
                        },
                    },
                    video_samples,
                ));
                track_id += 1;
            }
        }
        if let Some(esds) = aac_esds {
            if !audio_samples.is_empty() {
                tracks.push(Track::new(
                    TrackSpec {
                        track_id,
                        timescale: FLV_TIMESCALE,
                        config: CodecConfig::Aac {
                            esds,
                            channel_count: aac_channels,
                            sample_rate: aac_rate,
                            sample_size: AUDIO_SAMPLE_SIZE_BITS,
                        },
                    },
                    audio_samples,
                ));
            }
        }

        if tracks.is_empty() {
            return Err(FlvError::NoSupportedTrack);
        }
        Ok(Media::new(tracks, FLV_TIMESCALE))
    }
}

/// Read a signed 24-bit big-endian integer (FLV `CompositionTime`, §E.4.3.2).
fn read_si24(b: &[u8]) -> i32 {
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

/// Build an AAC `esds` from a raw `AudioSpecificConfig` byte slice (mirrors the
/// `ts_demux` AAC path).
fn build_aac_esds(asc_bytes: Vec<u8>) -> EsdsBox {
    EsdsBox::new(ESDescriptor {
        es_id: ESDS_AUDIO_ES_ID,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(OTI_MPEG4_AUDIO),
            stream_type: EsdsStreamType(STREAM_TYPE_AUDIO),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: Some(DecoderSpecificInfo { data: asc_bytes }),
        }),
        sl_config: Some(SLConfigDescriptor {
            body: vec![SL_CONFIG_PREDEFINED_MP4],
        }),
    })
}

/// Sampling rate in Hz from a parsed ASC: the explicit escape rate if present,
/// else the `samplingFrequencyIndex` table (ISO/IEC 14496-3 §1.6.3.4 Table 1.10).
fn asc_rate_hz(asc: &AudioSpecificConfig) -> u32 {
    if let Some(f) = asc.sampling_frequency {
        return f;
    }
    match asc.sampling_frequency_index.raw() {
        0 => 96000,
        1 => 88200,
        2 => 64000,
        3 => 48000,
        4 => 44100,
        5 => 32000,
        6 => 24000,
        7 => 22050,
        8 => 16000,
        9 => 12000,
        10 => 11025,
        11 => 8000,
        12 => 7350,
        _ => 0,
    }
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
    fn write_into(&self, out: &mut Vec<u8>) {
        let data_size = self.body.len();
        let start = out.len();
        out.push(self.tag_type);
        out.extend_from_slice(&(data_size as u32).to_be_bytes()[1..]); // UI24
        let ts = self.timestamp;
        out.push((ts >> 16) as u8);
        out.push((ts >> 8) as u8);
        out.push(ts as u8);
        out.push((ts >> 24) as u8); // TimestampExtended
        out.extend_from_slice(&[0, 0, 0]); // StreamID = 0
        out.extend_from_slice(&self.body);
        let tag_size = (out.len() - start) as u32;
        out.extend_from_slice(&tag_size.to_be_bytes()); // PreviousTagSize
    }
}

impl Package for FlvMux {
    type Media = Media;
    type Output = Vec<u8>;
    type Error = FlvError;

    fn package(&mut self, media: &Media) -> core::result::Result<Vec<u8>, FlvError> {
        // Locate the (at most one) AVC video track and (at most one) AAC audio track.
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
        .write_into(&mut out);

        // --- Sequence-header tags ---
        if let Some(vt) = video {
            if let CodecConfig::Avc { config, .. } = &vt.spec.config {
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
                OutTag {
                    tag_type: tag_type::VIDEO,
                    timestamp: 0,
                    body,
                }
                .write_into(&mut out);
            }
        }
        let (sound_type, asc_bytes) = if let Some(at) = audio {
            if let CodecConfig::Aac {
                esds,
                channel_count,
                ..
            } = &at.spec.config
            {
                let asc = esds_asc_bytes(esds)?;
                let st = if *channel_count <= 1 {
                    SOUND_TYPE_MONO
                } else {
                    SOUND_TYPE_STEREO
                };
                let mut body = Vec::with_capacity(2 + asc.len());
                body.push(audio_tag_header_byte(st));
                body.push(aac_packet_type::SEQUENCE_HEADER);
                body.extend_from_slice(&asc);
                OutTag {
                    tag_type: tag_type::AUDIO,
                    timestamp: 0,
                    body,
                }
                .write_into(&mut out);
                (st, asc)
            } else {
                (SOUND_TYPE_STEREO, Vec::new())
            }
        } else {
            (SOUND_TYPE_STEREO, Vec::new())
        };
        let _ = asc_bytes;

        // --- Interleaved A/V type-1 tags, ordered by DTS ---
        // Precompute (dts, tag) for each track, then merge-sort by dts.
        let mut items: Vec<(u32, u32, OutTag)> = Vec::new(); // (dts, seq_tiebreak, tag)
        let mut seq = 0u32;
        if let Some(vt) = video {
            let mut dts = 0u32;
            for s in &vt.samples {
                let comp = s.composition_offset;
                let mut body = Vec::with_capacity(5 + s.data.len());
                let ft = if s.is_sync {
                    FRAME_TYPE_KEYFRAME
                } else {
                    FRAME_TYPE_INTER
                };
                body.push((ft << 4) | CODEC_ID_AVC);
                body.push(avc_packet_type::NALU);
                body.push((comp >> 16) as u8);
                body.push((comp >> 8) as u8);
                body.push(comp as u8);
                body.extend_from_slice(&s.data);
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
                dts = dts.saturating_add(s.duration);
            }
        }
        if let Some(at) = audio {
            let mut dts = 0u32;
            for s in &at.samples {
                let mut body = Vec::with_capacity(2 + s.data.len());
                body.push(audio_tag_header_byte(sound_type));
                body.push(aac_packet_type::RAW);
                body.extend_from_slice(&s.data);
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
                dts = dts.saturating_add(s.duration);
            }
        }
        // Stable sort by DTS, tie-broken by original emission order.
        items.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, _, tag) in &items {
            tag.write_into(&mut out);
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
    }
}

/// Longest track duration in whole seconds (integer, for the `onMetaData` field).
fn media_duration_seconds(media: &Media) -> f64 {
    let mut max = 0.0f64;
    for t in &media.tracks {
        let ticks: u64 = t.samples.iter().map(|s| s.duration as u64).sum();
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
}
