//! AV3A-AATF RTP payload — T/AI 109.7-2024 clause 10.1 (AVS3 audio / Audio
//! Vivid over RTP) and the AATF frame header it carries (T/AI 109.3-2023 Annex A
//! Tables A.4-A.7).
//!
//! The RTP payload of AV3A-AATF has **no payload header of its own**: the payload
//! is an `aatf_frame()` (10.1.1.2 a), or, when a frame exceeds the MTU, a
//! consecutive fragment of one (b). The RTP `M` bit marks the packet holding a
//! complete `aatf_frame()` or the last fragment (10.1.2.2); `PT` is dynamic
//! (96..=127) and the timestamp clock defaults to 90 kHz. What this module types:
//!
//! * [`AatfFrame`] — the `aatf_frame()` start: `syncword`(12) `audio_codec_id`(4)
//!   `anc_data_index`(1) + the codec-specific `aatf_frame_header()`
//!   ([`AatfFrameHeader`]) for `audio_codec_id` 0 (general high-rate), 1
//!   (lossless) and 2 (general full-rate);
//! * [`packetize`] / [`Av3aDepacketizer`] — MTU fragmentation and `M`-bit
//!   reassembly;
//! * [`Av3aFmtp`] / [`CodecNnId`] — the SDP `a=fmtp` / MIME parameters
//!   (`rate`, `codec-nn-id`, `config`, `bitrate`) of encoding name `AV3A-AATF`.
//!
//! Source gaps (no value is guessed): the lossless (`audio_codec_id` 1) header
//! carries `channel_number` as `{4;8}` bits; that field is not yet typed (it is
//! defined by T/AI 109.3 Annex A, which is vendored), so the lossless header is
//! typed up to `aatf_error_check()` and the byte-aligned remainder
//! (`channel_number`, `resolution`, alignment, raw data) stays in
//! [`AatfFrame::body`]. T/AI 109.3 Table A.5 names `crc_check` as `rpchof` but
//! states no polynomial or initial value: CRC bytes are carried, not verified.
//! `audio_codec_id` 3..=15 have no defined header
//! ([`Error::InvalidValue`] on parse).

use crate::error::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;
use broadcast_common::bits::{BitReader, BitWriter};

/// SDP `a=rtpmap` / MIME subtype of the payload (T/AI 109.7 10.1.3.2, 10.1.4.2).
pub const ENCODING_NAME: &str = "AV3A-AATF";
/// Default RTP timestamp clock rate (10.1.2.2; 10.1.4.2 `rate` default 90000).
pub const DEFAULT_CLOCK_RATE: u32 = 90_000;
/// `syncword` of an `aatf_frame()` (Table A.4): `'1111 1111 1111'`.
pub const AATF_SYNCWORD: u16 = 0xFFF;
/// First dynamic RTP payload type (10.1.2.2: PT in `[96, 127]`).
pub const DYNAMIC_PT_MIN: u8 = 96;
/// Last dynamic RTP payload type.
pub const DYNAMIC_PT_MAX: u8 = 127;
/// `sampling_frequency_index` value that adds an explicit 24-bit `sampling_frequency`.
pub const SAMPLING_FREQUENCY_INDEX_EXPLICIT: u8 = 0xF;

/// Whether `pt` is in the dynamic payload-type range AV3A-AATF uses.
#[must_use]
pub const fn is_dynamic_payload_type(pt: u8) -> bool {
    pt >= DYNAMIC_PT_MIN && pt <= DYNAMIC_PT_MAX
}

fn short(what: &'static str) -> Error {
    Error::BufferTooShort {
        need: 0,
        have: 0,
        what,
    }
}

fn bad(field: &'static str, value: u64, reason: &'static str) -> Error {
    Error::InvalidValue {
        field,
        value,
        reason,
    }
}

/// `audio_codec_id == 0` header (general high-rate): Table A.6 fields in wire order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GhHeader {
    /// `coding_profile` (3 bits).
    pub coding_profile: u8,
    /// `sampling_frequency_index` (4 bits).
    pub sampling_frequency_index: u8,
    /// `raw_frame_length` (16 bits): total length of the current frame.
    pub raw_frame_length: u16,
    /// `aatf_error_check()` `crc_check` (8 bits).
    pub aatf_crc: u8,
    /// `channel_number_index` (7 bits).
    pub channel_number_index: u8,
    /// `resolution` (2 bits).
    pub resolution: u8,
    /// `bitrate_index` (4 bits).
    pub bitrate_index: u8,
    /// `bitstream_type` (1 bit).
    pub bitstream_type: bool,
}

/// `audio_codec_id == 1` header (lossless), typed up to `aatf_error_check()`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LlHeader {
    /// `coding_profile` (3 bits).
    pub coding_profile: u8,
    /// `sampling_frequency_index` (4 bits).
    pub sampling_frequency_index: u8,
    /// `sampling_frequency` (24 bits), present iff the index is 0xF.
    pub sampling_frequency: Option<u32>,
    /// `raw_frame_length` (16 bits).
    pub raw_frame_length: u16,
    /// `aatf_error_check()` `crc_check` (8 bits).
    pub aatf_crc: u8,
}

/// The `coding_profile`-selected fields of a general full-rate header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum GaContent {
    /// `coding_profile` 0: channel signal.
    Channel {
        /// `channel_number_index` (7 bits).
        channel_number_index: u8,
    },
    /// `coding_profile` 1, `soundBedType` 0: objects only.
    ObjectsOnly {
        /// `object_channel_number` (7 bits).
        object_channel_number: u8,
        /// `bitrate_index_per_channel` (4 bits).
        bitrate_index_per_channel: u8,
    },
    /// `coding_profile` 1, `soundBedType` 1: sound bed plus objects.
    ChannelAndObjects {
        /// `channel_number_index` (7 bits).
        channel_number_index: u8,
        /// `bitrate_index` (4 bits).
        bitrate_index: u8,
        /// `object_channel_number` (7 bits).
        object_channel_number: u8,
        /// `bitrate_index_per_channel` (4 bits).
        bitrate_index_per_channel: u8,
    },
    /// `coding_profile` 1 with a reserved `soundBedType` (2 or 3): no fields follow.
    ReservedSoundBed(u8),
    /// `coding_profile` 2: HOA signal.
    Hoa {
        /// `order` (4 bits): the HOA order is `order + 1`.
        order: u8,
    },
    /// `coding_profile` 3..=7 (reserved): no fields follow; carries the profile.
    ReservedProfile(u8),
}

impl GaContent {
    /// Spec label of the content form.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Channel { .. } => "channel signal",
            Self::ObjectsOnly { .. } => "objects only",
            Self::ChannelAndObjects { .. } => "channel signal and objects",
            Self::ReservedSoundBed(_) => "reserved soundBedType",
            Self::Hoa { .. } => "HOA signal",
            Self::ReservedProfile(_) => "reserved coding_profile",
        }
    }
}
broadcast_common::impl_spec_display!(GaContent);

/// `audio_codec_id == 2` header (general full-rate), Table A.6 + the trailing
/// `frame_error_check()` of Table A.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GaHeader {
    /// `nn_type` (3 bits).
    pub nn_type: u8,
    /// `coding_profile` (3 bits).
    pub coding_profile: u8,
    /// `sampling_frequency_index` (4 bits).
    pub sampling_frequency_index: u8,
    /// `aatf_error_check()` `crc_check` (8 bits).
    pub aatf_crc: u8,
    /// Fields selected by `coding_profile` (and `soundBedType`).
    pub content: GaContent,
    /// `resolution` (2 bits).
    pub resolution: u8,
    /// `bitrate_index` (4 bits), present iff `coding_profile != 1`.
    pub bitrate_index: Option<u8>,
    /// `frame_error_check()` `crc_check` (8 bits) after the header.
    pub frame_crc: u8,
}

/// The codec-specific `aatf_frame_header()`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AatfFrameHeader {
    /// `audio_codec_id` 0.
    GeneralHigh(GhHeader),
    /// `audio_codec_id` 1.
    Lossless(LlHeader),
    /// `audio_codec_id` 2.
    GeneralFull(GaHeader),
}

impl AatfFrameHeader {
    /// The 4-bit `audio_codec_id`.
    #[must_use]
    pub const fn audio_codec_id(&self) -> u8 {
        match self {
            Self::GeneralHigh(_) => 0,
            Self::Lossless(_) => 1,
            Self::GeneralFull(_) => 2,
        }
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::GeneralHigh(_) => "general high-rate audio coding",
            Self::Lossless(_) => "lossless audio coding",
            Self::GeneralFull(_) => "general full-rate audio coding",
        }
    }
}
broadcast_common::impl_spec_display!(AatfFrameHeader);

/// An `aatf_frame()` start: syncword, `audio_codec_id`, `anc_data_index`, the
/// codec header, and the bytes after the byte-aligned header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AatfFrame<'a> {
    /// `anc_data_index` (1 bit).
    pub anc_data_index: bool,
    /// The codec-specific header.
    pub header: AatfFrameHeader,
    /// `byte_alignment()` padding bits (value as read; 0 when hand-built). Its
    /// width is whatever reaches the next byte boundary after the header.
    pub alignment: u8,
    /// Everything after the byte-aligned header: the raw data block(s) (and, for
    /// codec ids 0 and 1, the trailing `frame_error_check()`; for the lossless
    /// header also `channel_number`/`resolution`, see the module docs).
    pub body: &'a [u8],
}

fn rd(r: &mut BitReader<'_>, n: u32, what: &'static str) -> Result<u64> {
    r.read_bits(n).map_err(|_| short(what))
}

impl<'a> AatfFrame<'a> {
    /// Parse the start of an `aatf_frame()` (an RTP payload that is a complete
    /// frame, or the first fragment of one).
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] on truncation; [`Error::InvalidValue`] for a
    /// wrong `syncword` or an `audio_codec_id` without a defined header (3..=15).
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let w = "aatf_frame header";
        let mut r = BitReader::new(bytes);
        let sync = rd(&mut r, 12, w)?;
        if sync != u64::from(AATF_SYNCWORD) {
            return Err(bad("syncword", sync, "must be 0xFFF"));
        }
        let id = rd(&mut r, 4, w)? as u8;
        let anc_data_index = rd(&mut r, 1, w)? != 0;
        let header = match id {
            0 => {
                let coding_profile = rd(&mut r, 3, w)? as u8;
                let sampling_frequency_index = rd(&mut r, 4, w)? as u8;
                let raw_frame_length = rd(&mut r, 16, w)? as u16;
                let aatf_crc = rd(&mut r, 8, w)? as u8;
                let channel_number_index = rd(&mut r, 7, w)? as u8;
                let resolution = rd(&mut r, 2, w)? as u8;
                let bitrate_index = rd(&mut r, 4, w)? as u8;
                let bitstream_type = rd(&mut r, 1, w)? != 0;
                AatfFrameHeader::GeneralHigh(GhHeader {
                    coding_profile,
                    sampling_frequency_index,
                    raw_frame_length,
                    aatf_crc,
                    channel_number_index,
                    resolution,
                    bitrate_index,
                    bitstream_type,
                })
            }
            1 => {
                let coding_profile = rd(&mut r, 3, w)? as u8;
                let sampling_frequency_index = rd(&mut r, 4, w)? as u8;
                let sampling_frequency =
                    if sampling_frequency_index == SAMPLING_FREQUENCY_INDEX_EXPLICIT {
                        Some(rd(&mut r, 24, w)? as u32)
                    } else {
                        None
                    };
                let raw_frame_length = rd(&mut r, 16, w)? as u16;
                let aatf_crc = rd(&mut r, 8, w)? as u8;
                AatfFrameHeader::Lossless(LlHeader {
                    coding_profile,
                    sampling_frequency_index,
                    sampling_frequency,
                    raw_frame_length,
                    aatf_crc,
                })
            }
            2 => {
                let nn_type = rd(&mut r, 3, w)? as u8;
                let coding_profile = rd(&mut r, 3, w)? as u8;
                let sampling_frequency_index = rd(&mut r, 4, w)? as u8;
                let aatf_crc = rd(&mut r, 8, w)? as u8;
                let content = match coding_profile {
                    0 => GaContent::Channel {
                        channel_number_index: rd(&mut r, 7, w)? as u8,
                    },
                    1 => match rd(&mut r, 2, w)? as u8 {
                        0 => GaContent::ObjectsOnly {
                            object_channel_number: rd(&mut r, 7, w)? as u8,
                            bitrate_index_per_channel: rd(&mut r, 4, w)? as u8,
                        },
                        1 => GaContent::ChannelAndObjects {
                            channel_number_index: rd(&mut r, 7, w)? as u8,
                            bitrate_index: rd(&mut r, 4, w)? as u8,
                            object_channel_number: rd(&mut r, 7, w)? as u8,
                            bitrate_index_per_channel: rd(&mut r, 4, w)? as u8,
                        },
                        other => GaContent::ReservedSoundBed(other),
                    },
                    2 => GaContent::Hoa {
                        order: rd(&mut r, 4, w)? as u8,
                    },
                    other => GaContent::ReservedProfile(other),
                };
                let resolution = rd(&mut r, 2, w)? as u8;
                let bitrate_index = if coding_profile != 1 {
                    Some(rd(&mut r, 4, w)? as u8)
                } else {
                    None
                };
                let frame_crc = rd(&mut r, 8, w)? as u8;
                AatfFrameHeader::GeneralFull(GaHeader {
                    nn_type,
                    coding_profile,
                    sampling_frequency_index,
                    aatf_crc,
                    content,
                    resolution,
                    bitrate_index,
                    frame_crc,
                })
            }
            other => {
                return Err(bad(
                    "audio_codec_id",
                    u64::from(other),
                    "no aatf_frame_header() defined (only 0, 1, 2)",
                ));
            }
        };
        let pad_bits = (8 - r.bits_read() % 8) % 8;
        let alignment = if pad_bits == 0 {
            0
        } else {
            rd(&mut r, pad_bits as u32, w)? as u8
        };
        let consumed = r.bits_read() / 8;
        Ok(Self {
            anc_data_index,
            header,
            alignment,
            body: &bytes[consumed..],
        })
    }

    fn header_bits(&self) -> usize {
        17 + match &self.header {
            AatfFrameHeader::GeneralHigh(_) => 3 + 4 + 16 + 8 + 7 + 2 + 4 + 1,
            AatfFrameHeader::Lossless(l) => {
                3 + 4
                    + if l.sampling_frequency.is_some() {
                        24
                    } else {
                        0
                    }
                    + 16
                    + 8
            }
            AatfFrameHeader::GeneralFull(g) => {
                let content = match g.content {
                    GaContent::Channel { .. } => 7,
                    GaContent::ObjectsOnly { .. } => 2 + 7 + 4,
                    GaContent::ChannelAndObjects { .. } => 2 + 7 + 4 + 7 + 4,
                    GaContent::ReservedSoundBed(_) => 2,
                    GaContent::Hoa { .. } => 4,
                    GaContent::ReservedProfile(_) => 0,
                };
                3 + 3 + 4 + 8 + content + 2 + if g.bitrate_index.is_some() { 4 } else { 0 } + 8
            }
        }
    }

    /// Serialized length: the byte-aligned header plus [`AatfFrame::body`].
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        self.header_bits().div_ceil(8) + self.body.len()
    }

    /// Serialize the frame (header bits, alignment padding, body).
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] when `buf` is too small; [`Error::InvalidValue`]
    /// when a field exceeds its bit width, a presence rule is violated, or a
    /// content variant disagrees with `coding_profile`.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: buf.len(),
                what: "aatf_frame",
            });
        }
        let hdr_bytes = self.header_bits().div_ceil(8);
        buf[..hdr_bytes].fill(0);
        {
            let mut w = BitWriter::new(&mut buf[..hdr_bytes]);
            let mut put = |v: u64, n: u32, field: &'static str| -> Result<()> {
                w.write_bits(v, n)
                    .map_err(|_| bad(field, v, "exceeds its bit width"))
            };
            put(u64::from(AATF_SYNCWORD), 12, "syncword")?;
            put(u64::from(self.header.audio_codec_id()), 4, "audio_codec_id")?;
            put(u64::from(self.anc_data_index), 1, "anc_data_index")?;
            match &self.header {
                AatfFrameHeader::GeneralHigh(h) => {
                    put(u64::from(h.coding_profile), 3, "coding_profile")?;
                    put(
                        u64::from(h.sampling_frequency_index),
                        4,
                        "sampling_frequency_index",
                    )?;
                    put(u64::from(h.raw_frame_length), 16, "raw_frame_length")?;
                    put(u64::from(h.aatf_crc), 8, "crc_check")?;
                    put(u64::from(h.channel_number_index), 7, "channel_number_index")?;
                    put(u64::from(h.resolution), 2, "resolution")?;
                    put(u64::from(h.bitrate_index), 4, "bitrate_index")?;
                    put(u64::from(h.bitstream_type), 1, "bitstream_type")?;
                }
                AatfFrameHeader::Lossless(l) => {
                    put(u64::from(l.coding_profile), 3, "coding_profile")?;
                    put(
                        u64::from(l.sampling_frequency_index),
                        4,
                        "sampling_frequency_index",
                    )?;
                    let explicit = l.sampling_frequency_index == SAMPLING_FREQUENCY_INDEX_EXPLICIT;
                    match (explicit, l.sampling_frequency) {
                        (true, Some(f)) => put(u64::from(f), 24, "sampling_frequency")?,
                        (false, None) => {}
                        _ => {
                            return Err(bad(
                                "sampling_frequency",
                                u64::from(l.sampling_frequency_index),
                                "present iff sampling_frequency_index == 0xF",
                            ));
                        }
                    }
                    put(u64::from(l.raw_frame_length), 16, "raw_frame_length")?;
                    put(u64::from(l.aatf_crc), 8, "crc_check")?;
                }
                AatfFrameHeader::GeneralFull(g) => {
                    put(u64::from(g.nn_type), 3, "nn_type")?;
                    put(u64::from(g.coding_profile), 3, "coding_profile")?;
                    put(
                        u64::from(g.sampling_frequency_index),
                        4,
                        "sampling_frequency_index",
                    )?;
                    put(u64::from(g.aatf_crc), 8, "crc_check")?;
                    let profile_ok = match g.content {
                        GaContent::Channel { .. } => g.coding_profile == 0,
                        GaContent::ObjectsOnly { .. }
                        | GaContent::ChannelAndObjects { .. }
                        | GaContent::ReservedSoundBed(_) => g.coding_profile == 1,
                        GaContent::Hoa { .. } => g.coding_profile == 2,
                        GaContent::ReservedProfile(p) => {
                            (3..=7).contains(&p) && p == g.coding_profile
                        }
                    };
                    if !profile_ok {
                        return Err(bad(
                            "coding_profile",
                            u64::from(g.coding_profile),
                            "does not match the content variant",
                        ));
                    }
                    match g.content {
                        GaContent::Channel {
                            channel_number_index,
                        } => {
                            put(u64::from(channel_number_index), 7, "channel_number_index")?;
                        }
                        GaContent::ObjectsOnly {
                            object_channel_number,
                            bitrate_index_per_channel,
                        } => {
                            put(0, 2, "soundBedType")?;
                            put(u64::from(object_channel_number), 7, "object_channel_number")?;
                            put(
                                u64::from(bitrate_index_per_channel),
                                4,
                                "bitrate_index_per_channel",
                            )?;
                        }
                        GaContent::ChannelAndObjects {
                            channel_number_index,
                            bitrate_index,
                            object_channel_number,
                            bitrate_index_per_channel,
                        } => {
                            put(1, 2, "soundBedType")?;
                            put(u64::from(channel_number_index), 7, "channel_number_index")?;
                            put(u64::from(bitrate_index), 4, "bitrate_index")?;
                            put(u64::from(object_channel_number), 7, "object_channel_number")?;
                            put(
                                u64::from(bitrate_index_per_channel),
                                4,
                                "bitrate_index_per_channel",
                            )?;
                        }
                        GaContent::ReservedSoundBed(s) => {
                            if !(2..=3).contains(&s) {
                                return Err(bad(
                                    "soundBedType",
                                    u64::from(s),
                                    "reserved values are 2 and 3",
                                ));
                            }
                            put(u64::from(s), 2, "soundBedType")?;
                        }
                        GaContent::Hoa { order } => put(u64::from(order), 4, "order")?,
                        GaContent::ReservedProfile(_) => {}
                    }
                    put(u64::from(g.resolution), 2, "resolution")?;
                    match (g.coding_profile != 1, g.bitrate_index) {
                        (true, Some(b)) => put(u64::from(b), 4, "bitrate_index")?,
                        (false, None) => {}
                        _ => {
                            return Err(bad(
                                "bitrate_index",
                                u64::from(g.coding_profile),
                                "present iff coding_profile != 1",
                            ));
                        }
                    }
                    put(u64::from(g.frame_crc), 8, "frame_error_check crc_check")?;
                }
            }
            let pad = hdr_bytes * 8 - self.header_bits();
            if pad > 0 {
                put(u64::from(self.alignment), pad as u32, "byte_alignment")?;
            }
        }
        buf[hdr_bytes..need].copy_from_slice(self.body);
        Ok(need)
    }
}

/// Split one `aatf_frame()` into RTP payloads of at most `max_payload` bytes
/// (T/AI 109.7 10.1.1.2 b). Each item is `(payload, marker)`; `marker` is `true`
/// only for the last fragment (a frame that fits yields one item with
/// `marker == true`; an empty frame yields none). `max_payload` of 0 is treated
/// as 1.
pub fn packetize(frame: &[u8], max_payload: usize) -> impl Iterator<Item = (&[u8], bool)> {
    let chunk = max_payload.max(1);
    let last = frame.len().div_ceil(chunk).saturating_sub(1);
    frame
        .chunks(chunk)
        .enumerate()
        .map(move |(i, c)| (c, i == last))
}

/// Reassembles `aatf_frame()` fragments carried in consecutive RTP packets.
///
/// Feed each packet's payload with its `M` bit in sequence order; a complete
/// frame is returned when `M == 1`. Loss detection (RTP sequence numbers) is the
/// caller's job: call [`Av3aDepacketizer::reset`] on a gap.
///
/// The partial frame is bounded: a stream that never sets `M` (lost marker,
/// hostile sender) would otherwise buffer without limit. The cap defaults to
/// [`DEFAULT_MAX_FRAME_BYTES`] and is set with
/// [`Av3aDepacketizer::with_max_frame_bytes`].
#[derive(Debug, Clone)]
pub struct Av3aDepacketizer {
    buf: Vec<u8>,
    max_frame_bytes: usize,
}

impl Default for Av3aDepacketizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Default upper bound on one reassembled `aatf_frame()`.
///
/// The frame length field `raw_frame_length` is 16 bits (T/AI 109.3 Annex A,
/// Table A.4/A.6), so a conformant frame is at most 64 KiB plus its short
/// header; 1 MiB is a deliberately conservative cap that no conformant
/// stream approaches.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 1 << 20;

impl Av3aDepacketizer {
    /// An empty reassembler with the [`DEFAULT_MAX_FRAME_BYTES`] cap.
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_frame_bytes(DEFAULT_MAX_FRAME_BYTES)
    }

    /// An empty reassembler that rejects frames larger than `max_frame_bytes`.
    #[must_use]
    pub fn with_max_frame_bytes(max_frame_bytes: usize) -> Self {
        Self {
            buf: Vec::new(),
            max_frame_bytes,
        }
    }

    /// Append one packet's payload; returns the complete `aatf_frame()` bytes
    /// when `marker` is set.
    ///
    /// # Errors
    /// [`Error::InvalidValue`] when the frame would exceed the configured
    /// maximum; the partial frame is dropped so the caller can carry on with
    /// the next one.
    pub fn push(&mut self, marker: bool, payload: &[u8]) -> Result<Option<Vec<u8>>> {
        let total = self.buf.len().saturating_add(payload.len());
        if total > self.max_frame_bytes {
            self.buf.clear();
            return Err(bad(
                "aatf_frame size",
                total as u64,
                "exceeds the depacketizer's maximum frame size",
            ));
        }
        self.buf.extend_from_slice(payload);
        Ok(marker.then(|| core::mem::take(&mut self.buf)))
    }

    /// Drop any partially collected frame (after packet loss).
    pub fn reset(&mut self) {
        self.buf.clear();
    }

    /// Bytes currently buffered (a partial frame).
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }
}

/// `codec-nn-id` (T/AI 109.7 10.1.4.2): two bytes, the first the
/// `audio_codec_id`, the second the `nn_type` (0 when the codec has no neural
/// network).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CodecNnId {
    /// `audio_codec_id` (high byte).
    pub audio_codec_id: u8,
    /// `nn_type` (low byte).
    pub nn_type: u8,
}

impl CodecNnId {
    /// The 16-bit value (`audio_codec_id << 8 | nn_type`).
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        ((self.audio_codec_id as u16) << 8) | self.nn_type as u16
    }

    /// From the 16-bit value.
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        Self {
            audio_codec_id: (v >> 8) as u8,
            nn_type: (v & 0xFF) as u8,
        }
    }
}

/// The SDP `a=fmtp` / MIME parameters of `AV3A-AATF` (T/AI 109.7 10.1.3.2,
/// 10.1.4.2): `rate`, `codec-nn-id`, `config` (hex of the `dca3`
/// `CA3SpecificBox` body), `bitrate` (kbps, the `total_bitrate`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Av3aFmtp {
    /// `rate`: RTP timestamp clock (default 90000 when absent).
    pub rate: Option<u32>,
    /// `codec-nn-id`.
    pub codec_nn_id: Option<CodecNnId>,
    /// `config`: the `CA3SpecificBox` bytes (hex on the wire).
    pub config: Option<Vec<u8>>,
    /// `bitrate` in kbps.
    pub bitrate: Option<u32>,
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Decimal digits only: `str::parse` also accepts a leading `+`, which is not
/// part of the SDP/MIME integer grammar.
fn parse_unsigned(v: &str, field: &'static str) -> Result<u32> {
    if v.is_empty() || !v.bytes().all(|c| c.is_ascii_digit()) {
        return Err(bad(field, 0, "not an unsigned decimal integer"));
    }
    v.parse().map_err(|_| bad(field, 0, "not a 32-bit integer"))
}

impl Av3aFmtp {
    /// The clock rate to use: `rate`, or [`DEFAULT_CLOCK_RATE`].
    #[must_use]
    pub fn clock_rate(&self) -> u32 {
        self.rate.unwrap_or(DEFAULT_CLOCK_RATE)
    }

    /// Parse a `;`-separated `name=value` parameter list (whitespace around
    /// separators is ignored; unknown names are skipped; names are
    /// case-insensitive).
    ///
    /// # Errors
    /// [`Error::InvalidValue`] for a malformed value of a known parameter
    /// (a leading `+`, `rate=0`) or a known parameter given twice.
    pub fn parse(s: &str) -> Result<Self> {
        let mut out = Self::default();
        let mut seen = [false; 4];
        for part in s.split(';') {
            let part = part.trim();
            let Some((k, v)) = part.split_once('=') else {
                continue;
            };
            let v = v.trim();
            let key = k.trim().to_ascii_lowercase();
            let slot = match key.as_str() {
                "rate" => Some((0, "rate")),
                "bitrate" => Some((1, "bitrate")),
                "codec-nn-id" => Some((2, "codec-nn-id")),
                "config" => Some((3, "config")),
                _ => None,
            };
            if let Some((i, name)) = slot {
                if seen[i] {
                    return Err(bad(name, 0, "parameter given more than once"));
                }
                seen[i] = true;
            }
            match key.as_str() {
                "rate" => {
                    let r: u32 = parse_unsigned(v, "rate")?;
                    if r == 0 {
                        return Err(bad("rate", 0, "RTP clock rate must be non-zero"));
                    }
                    out.rate = Some(r);
                }
                "bitrate" => {
                    out.bitrate = Some(parse_unsigned(v, "bitrate")?);
                }
                "codec-nn-id" => {
                    let h = v
                        .strip_prefix("0x")
                        .or_else(|| v.strip_prefix("0X"))
                        .unwrap_or(v);
                    if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
                        return Err(bad("codec-nn-id", 0, "not a 16-bit hex value"));
                    }
                    let n = u16::from_str_radix(h, 16)
                        .map_err(|_| bad("codec-nn-id", 0, "not a 16-bit hex value"))?;
                    out.codec_nn_id = Some(CodecNnId::from_u16(n));
                }
                "config" => {
                    let b = v.as_bytes();
                    if b.len() % 2 != 0 {
                        return Err(bad("config", b.len() as u64, "odd number of hex digits"));
                    }
                    let mut bytes = Vec::with_capacity(b.len() / 2);
                    for p in b.chunks_exact(2) {
                        let (Some(hi), Some(lo)) = (hex_val(p[0]), hex_val(p[1])) else {
                            return Err(bad("config", 0, "not a hex string"));
                        };
                        bytes.push((hi << 4) | lo);
                    }
                    out.config = Some(bytes);
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Render as a `;`-separated parameter list (`codec-nn-id` as `0xNNNN`,
    /// `config` as lowercase hex), in the order rate, codec-nn-id, config, bitrate.
    #[must_use]
    pub fn to_param_string(&self) -> String {
        use core::fmt::Write;
        let mut s = String::new();
        let sep = |s: &mut String| {
            if !s.is_empty() {
                s.push_str("; ");
            }
        };
        if let Some(r) = self.rate {
            sep(&mut s);
            let _ = write!(s, "rate={r}");
        }
        if let Some(c) = self.codec_nn_id {
            sep(&mut s);
            let _ = write!(s, "codec-nn-id=0x{:04X}", c.to_u16());
        }
        if let Some(c) = &self.config {
            sep(&mut s);
            s.push_str("config=");
            for b in c {
                let _ = write!(s, "{b:02x}");
            }
        }
        if let Some(b) = self.bitrate {
            sep(&mut s);
            let _ = write!(s, "bitrate={b}");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn round_trip(bytes: &[u8]) -> AatfFrame<'_> {
        let f = AatfFrame::parse(bytes).unwrap();
        let mut out = vec![0u8; f.serialized_len()];
        let n = f.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        f
    }

    /// Tables A.4/A.6, `audio_codec_id` 0: syncword 0xFFF, id 0, anc 0, then
    /// coding_profile 0, sfi 2, raw_frame_length 0x0100, aatf crc 0xAB,
    /// channel_number_index 2, resolution 2, bitrate_index 3, bitstream_type 1,
    /// two alignment bits, then a 3-byte body (62 header bits -> 8 bytes).
    #[test]
    fn gh_header_vector() {
        let bytes = [
            0xFF, 0xF0, 0x02, 0x01, 0x00, 0xAB, 0x05, 0x1C, 0x01, 0x02, 0x03,
        ];
        let f = round_trip(&bytes);
        assert!(!f.anc_data_index);
        let AatfFrameHeader::GeneralHigh(h) = f.header else {
            panic!()
        };
        assert_eq!((h.coding_profile, h.sampling_frequency_index), (0, 2));
        assert_eq!((h.raw_frame_length, h.aatf_crc), (0x0100, 0xAB));
        assert_eq!(
            (h.channel_number_index, h.resolution, h.bitrate_index),
            (2, 2, 3)
        );
        assert!(h.bitstream_type);
        assert_eq!(f.body, [1, 2, 3]);
    }

    /// Lossless: explicit 24-bit sampling frequency (96000 = 0x017700); the
    /// byte-aligned remainder is returned untouched (72 header bits, no pad).
    #[test]
    fn ll_header_explicit_frequency() {
        let bytes = [
            0xFF, 0xF1, 0x8F, 0x01, 0x77, 0x00, 0x04, 0x00, 0x11, 0x02, 0x80, 0xAA,
        ];
        let f = round_trip(&bytes);
        assert!(f.anc_data_index);
        let AatfFrameHeader::Lossless(l) = &f.header else {
            panic!()
        };
        assert_eq!(l.sampling_frequency, Some(96_000));
        assert_eq!((l.raw_frame_length, l.aatf_crc), (0x0400, 0x11));
        assert_eq!(f.body, [0x02, 0x80, 0xAA]);
    }

    /// General full-rate, one frame per `coding_profile` shape.
    #[test]
    fn ga_header_shapes_round_trip() {
        let mk = |profile: u8, content: GaContent, bitrate_index: Option<u8>| AatfFrame {
            anc_data_index: false,
            header: AatfFrameHeader::GeneralFull(GaHeader {
                nn_type: 1,
                coding_profile: profile,
                sampling_frequency_index: 2,
                aatf_crc: 0x5A,
                content,
                resolution: 2,
                bitrate_index,
                frame_crc: 0xC3,
            }),
            alignment: 0,
            body: &[9, 9],
        };
        for f in [
            mk(
                0,
                GaContent::Channel {
                    channel_number_index: 2,
                },
                Some(1),
            ),
            mk(
                1,
                GaContent::ObjectsOnly {
                    object_channel_number: 7,
                    bitrate_index_per_channel: 5,
                },
                None,
            ),
            mk(
                1,
                GaContent::ChannelAndObjects {
                    channel_number_index: 3,
                    bitrate_index: 2,
                    object_channel_number: 4,
                    bitrate_index_per_channel: 6,
                },
                None,
            ),
            mk(1, GaContent::ReservedSoundBed(3), None),
            mk(2, GaContent::Hoa { order: 2 }, Some(4)),
            mk(5, GaContent::ReservedProfile(5), Some(0)),
        ] {
            let mut out = vec![0u8; f.serialized_len()];
            f.serialize_into(&mut out).unwrap();
            let back = round_trip(&out);
            assert_eq!(back, f);
            assert_eq!(back.header.audio_codec_id(), 2);
        }
    }

    /// Hand-assembled `audio_codec_id` 2 frame (Tables A.4/A.6): nn_type 1,
    /// coding_profile 0, sfi 2, aatf crc 0x5A, channel_number_index 2,
    /// resolution 2, bitrate_index 1, frame crc 0xC3 -- 17+3+3+4+8+7+2+4+8 = 56
    /// bits, no padding.
    #[test]
    fn ga_header_vector() {
        // 1111 1111 | 1111 0010 | 0 001 000 0 | 010 01011 | 010 00000 | 10 10 0001 | 1100 0011
        let bytes = [0xFF, 0xF2, 0x10, 0x4B, 0x40, 0xA1, 0xC3];
        let f = round_trip(&bytes);
        let AatfFrameHeader::GeneralFull(g) = f.header else {
            panic!()
        };
        assert_eq!(
            (g.nn_type, g.coding_profile, g.sampling_frequency_index),
            (1, 0, 2)
        );
        assert_eq!(g.aatf_crc, 0x5A);
        assert_eq!(
            g.content,
            GaContent::Channel {
                channel_number_index: 2
            }
        );
        assert_eq!(
            (g.resolution, g.bitrate_index, g.frame_crc),
            (2, Some(1), 0xC3)
        );
        assert!(f.body.is_empty());
    }

    #[test]
    fn rejects_bad_sync_codec_id_and_truncation() {
        assert!(
            AatfFrame::parse(&[0xFF, 0xE0, 0, 0]).is_err(),
            "wrong syncword"
        );
        // syncword ok, audio_codec_id 3: no header defined
        assert!(AatfFrame::parse(&[0xFF, 0xF3, 0, 0]).is_err());
        assert!(AatfFrame::parse(&[0xFF, 0xF0]).is_err());
    }

    #[test]
    fn serialize_rejects_inconsistent_headers() {
        let ga = |profile: u8, content: GaContent, bitrate_index: Option<u8>| AatfFrame {
            anc_data_index: false,
            header: AatfFrameHeader::GeneralFull(GaHeader {
                nn_type: 0,
                coding_profile: profile,
                sampling_frequency_index: 0,
                aatf_crc: 0,
                content,
                resolution: 0,
                bitrate_index,
                frame_crc: 0,
            }),
            alignment: 0,
            body: &[],
        };
        let mut buf = [0u8; 32];
        // profile/content mismatch
        assert!(
            ga(
                1,
                GaContent::Channel {
                    channel_number_index: 1
                },
                Some(0)
            )
            .serialize_into(&mut buf)
            .is_err()
        );
        // bitrate_index present for profile 1
        assert!(
            ga(1, GaContent::ReservedSoundBed(2), Some(0))
                .serialize_into(&mut buf)
                .is_err()
        );
        // over-wide field
        assert!(
            ga(
                0,
                GaContent::Channel {
                    channel_number_index: 200
                },
                Some(0)
            )
            .serialize_into(&mut buf)
            .is_err()
        );
        // lossless frequency/index disagreement
        let ll = AatfFrame {
            anc_data_index: false,
            header: AatfFrameHeader::Lossless(LlHeader {
                coding_profile: 0,
                sampling_frequency_index: 0xF,
                sampling_frequency: None,
                raw_frame_length: 0,
                aatf_crc: 0,
            }),
            alignment: 0,
            body: &[],
        };
        assert!(ll.serialize_into(&mut buf).is_err());
        // output too small
        assert!(
            ga(
                0,
                GaContent::Channel {
                    channel_number_index: 1
                },
                Some(0)
            )
            .serialize_into(&mut [0u8; 2])
            .is_err()
        );
    }

    /// 10.1.1.2 (b) / 10.1.2.2: an `aatf_frame()` larger than the MTU is split;
    /// only the last fragment has M = 1; reassembly returns the original frame.
    #[test]
    fn fragmentation_and_reassembly() {
        let frame: Vec<u8> = (0..=249u8).collect();
        let parts: Vec<(&[u8], bool)> = packetize(&frame, 100).collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts.iter().map(|(p, _)| p.len()).collect::<Vec<_>>(),
            [100, 100, 50]
        );
        assert_eq!(
            parts.iter().map(|(_, m)| *m).collect::<Vec<_>>(),
            [false, false, true]
        );
        let mut d = Av3aDepacketizer::new();
        assert!(d.push(false, parts[0].0).unwrap().is_none());
        assert_eq!(d.buffered(), 100);
        assert!(d.push(false, parts[1].0).unwrap().is_none());
        assert_eq!(d.push(true, parts[2].0).unwrap().unwrap(), frame);
        assert_eq!(d.buffered(), 0);
        // a frame that fits is a single M=1 packet; an empty frame yields none
        let single: Vec<_> = packetize(&frame[..10], 100).collect();
        assert_eq!(single.len(), 1);
        assert!(single[0].1);
        assert_eq!(packetize(&[], 100).count(), 0);
        d.push(false, &[1]).unwrap();
        d.reset();
        assert_eq!(d.buffered(), 0);
    }

    #[test]
    fn dynamic_payload_type_range() {
        assert!(!is_dynamic_payload_type(95));
        assert!(is_dynamic_payload_type(96));
        assert!(is_dynamic_payload_type(127));
        assert!(!is_dynamic_payload_type(128));
        assert_eq!(ENCODING_NAME, "AV3A-AATF");
        assert_eq!(Av3aFmtp::default().clock_rate(), 90_000);
    }

    /// T/AI 109.7 10.1.4.2 codec-nn-id examples: 0x0000 (id 0, no NN), 0x0100
    /// (lossless, no NN), 0x0201 (general full-rate, low-complexity NN).
    #[test]
    fn codec_nn_id_spec_examples() {
        for (v, id, nn) in [(0x0000u16, 0u8, 0u8), (0x0100, 1, 0), (0x0201, 2, 1)] {
            let c = CodecNnId::from_u16(v);
            assert_eq!((c.audio_codec_id, c.nn_type), (id, nn));
            assert_eq!(c.to_u16(), v);
        }
    }

    #[test]
    fn fmtp_parse_and_render() {
        assert!(
            Av3aFmtp::parse("codec-nn-id=0x0201; config=2220040140 80; bitrate=320").is_err(),
            "space inside hex is rejected"
        );
        let f = Av3aFmtp::parse("rate=48000; codec-nn-id=0x0201; config=222004014080; bitrate=320")
            .unwrap();
        assert_eq!(f.clock_rate(), 48_000);
        assert_eq!(
            f.codec_nn_id,
            Some(CodecNnId {
                audio_codec_id: 2,
                nn_type: 1
            })
        );
        assert_eq!(
            f.config.as_deref(),
            Some(&[0x22, 0x20, 0x04, 0x01, 0x40, 0x80][..])
        );
        assert_eq!(f.bitrate, Some(320));
        assert_eq!(
            f.to_param_string(),
            "rate=48000; codec-nn-id=0x0201; config=222004014080; bitrate=320"
        );
        assert_eq!(Av3aFmtp::parse(&f.to_param_string()).unwrap(), f);
        assert!(Av3aFmtp::parse("config=abc").is_err());
        assert!(Av3aFmtp::parse("codec-nn-id=zz").is_err());
        assert!(Av3aFmtp::parse("rate=x").is_err());
        // no leading '+', no zero clock rate, no duplicated parameter
        assert!(Av3aFmtp::parse("codec-nn-id=+201").is_err());
        assert!(Av3aFmtp::parse("codec-nn-id=0x+201").is_err());
        assert!(Av3aFmtp::parse("rate=+48000").is_err());
        assert!(Av3aFmtp::parse("bitrate=+320").is_err());
        assert!(Av3aFmtp::parse("rate=0").is_err());
        assert!(Av3aFmtp::parse("rate=48000; rate=44100").is_err());
        assert!(Av3aFmtp::parse("RATE=48000; rate=48000").is_err());
        assert!(Av3aFmtp::parse("codec-nn-id=0x0201; codec-nn-id=0x0201").is_err());
        assert!(Av3aFmtp::parse("config=00; config=00").is_err());
        assert!(Av3aFmtp::parse("bitrate=1; bitrate=1").is_err());
        assert_eq!(Av3aFmtp::parse("rate=48000").unwrap().rate, Some(48000));
    }

    /// An endless run of M=0 packets must not buffer without bound.
    #[test]
    fn depacketizer_caps_buffered_frame() {
        let mut d = Av3aDepacketizer::with_max_frame_bytes(10);
        assert!(d.push(false, &[0; 6]).unwrap().is_none());
        assert!(d.push(false, &[0; 4]).unwrap().is_none());
        assert!(d.push(false, &[0; 1]).is_err(), "11 bytes > cap of 10");
        assert_eq!(d.buffered(), 0, "partial frame dropped on overflow");
        // a frame exactly at the cap still completes, and the next one works
        assert_eq!(d.push(true, &[7; 10]).unwrap().unwrap().len(), 10);
        assert_eq!(d.push(true, &[1]).unwrap().unwrap(), [1]);
        // the default cap applies to `new()`
        let mut d = Av3aDepacketizer::new();
        assert!(d.push(false, &vec![0u8; DEFAULT_MAX_FRAME_BYTES]).is_ok());
        assert!(d.push(false, &[0]).is_err());
        assert_eq!(Av3aFmtp::parse("").unwrap(), Av3aFmtp::default());
    }
}
