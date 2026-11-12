//! FLV (Flash Video) tag framing — Adobe Flash Video File Format Specification
//! **v10.1, Annex E**: the file header of §E.2 and the tag of §E.4.1.
//!
//! This is the one implementation of the FLV tag layer in the workspace (#2342): the
//! `rtmp-runtime` session engine hands its callers FLV bytes (RTMP Audio(8)/Video(9)/
//! Data-AMF0(18) message bodies are FLV tag bodies, Adobe RTMP 1.0 §7.1.4, §7.1.5,
//! §7.1.2), and `transmux` builds its FLV streams from the same values instead of a
//! second encoder. What a tag *contains* (codec headers, sequence headers, composition
//! time) is the caller's business; this module only frames it.
//!
//! # Layout (Annex E)
//!
//! - **Header** (§E.2, 9 bytes) + `PreviousTagSize0` (4 bytes, = 0): `"FLV"`
//!   signature, `Version`, `TypeFlags` (bit 0 audio, bit 2 video), and the
//!   `DataOffset`.
//! - **Tag** (§E.4.1): an 11-byte header (`TagType` UI8, `DataSize` UI24,
//!   `Timestamp` UI24 + `TimestampExtended` UI8, `StreamID` UI24 = 0), the body,
//!   and the UI32 `PreviousTagSize` (11 + `DataSize`).
//!
//! Spec: `private/specs-verified/rtmp/rtmp-runtime/rtmp.md` sections 6.6 and 9.

use broadcast_common::error::OutputBufferTooSmall;
use broadcast_common::len::{fit_u24, fit_u32, need_in, need_out};
use broadcast_common::{Parse, Serialize};

use crate::RtmpError;
use crate::chunk::Message;
use crate::message::msg_type;
use alloc::string::ToString;
use alloc::vec::Vec;

type FlvResult<T> = core::result::Result<T, RtmpError>;

/// The `OutputBufferTooSmall` a serializer returns when a buffer that passed
/// its pre-check still cannot be sliced to the needed length.
fn make_output_too_small(need: usize, have: usize) -> OutputBufferTooSmall {
    OutputBufferTooSmall { need, have }
}

// ── Aggregate message consts (§7.1.6 / Adobe FLV v10.1 Annex E.5) ───────

/// Byte width of an Aggregate (22) sub-message header (§7.1.6, FLV
/// Annex E.5): `Type`(1) + `DataSize`(3) + `Timestamp`(3) +
/// `TimestampExtended`(1) + `StreamID`(3).
pub(crate) const AGG_SUB_MESSAGE_HEADER_LEN: usize = 11;
/// Byte width of the `Back Pointer` field following each Aggregate
/// sub-message (§7.1.6): "size of the previous message including its
/// header" — the same value the enclosing FLV `PreviousTagSize` convention
/// describes (FLV Annex E.4.1's tag-header field + 24-bit `DataSize`).
pub(crate) const AGG_BACK_POINTER_LEN: usize = 4;
/// `TimestampExtended`'s shift into the top byte of a 32-bit timestamp:
/// same layout the top-level `flv_tag` writes (FLV Annex E.4.1) — the
/// 24-bit `Timestamp` field holds the low 24 bits, `TimestampExtended` the
/// top 8.
pub(crate) const AGG_TIMESTAMP_EXTENDED_SHIFT: u32 = 24;

/// Unpack an Aggregate Message (22, §7.1.6), calling `deliver(tag_type, timestamp, payload)` for
/// each contained sub-message as an ordinary Audio/Video/Data-AMF0 message would be delivered.
///
/// Sub-message layout (§7.1.6, identical to the FLV tag layout of Adobe FLV v10.1 Annex
/// E.4.1/E.5): `Type`(1) + `DataSize`(3) + `Timestamp`(3) + `TimestampExtended`(1) + `StreamID`(3,
/// always 0) + `Data` + `Back Pointer`(4). Per §7.1.6 the aggregate's own message stream id
/// overrides every sub-message's, and sub-message timestamps are renormalized by `offset =
/// aggregate ts - first sub-message ts` added to each sub-message timestamp.
///
/// A sub-message header or `DataSize`-claimed data run reaching past the end of the aggregate
/// body, or a `Back Pointer` that disagrees with the sub-message's actual size, is rejected with
/// `Malformed`; sub-messages before the malformed one were already delivered.
///
/// # Errors
/// [`RtmpError::Malformed`] as above, or whatever `deliver` returns.
pub(crate) fn unpack_aggregate(
    msg: &Message,
    mut deliver: impl FnMut(u8, u32, &[u8]) -> FlvResult<()>,
) -> FlvResult<()> {
    let body = &msg.payload;
    let mut first_ts: Option<u32> = None;
    let mut offset = 0usize;
    while offset < body.len() {
        if body.len() - offset < AGG_SUB_MESSAGE_HEADER_LEN {
            return Err(RtmpError::Malformed {
                what: "aggregate sub-message header truncated",
            });
        }
        let header = &body[offset..offset + AGG_SUB_MESSAGE_HEADER_LEN];
        let tag_type = header[0];
        let data_size = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
        let ts_low24 = u32::from_be_bytes([0, header[4], header[5], header[6]]);
        // Same layout `flv_tag` writes (Annex E.4.1): the 24-bit
        // `Timestamp` field holds bits 0..=23, `TimestampExtended`
        // holds the top 8.
        let raw_ts = ts_low24 | (u32::from(header[7]) << AGG_TIMESTAMP_EXTENDED_SHIFT);
        // header[8..11] is the sub-message `StreamID` (always 0):
        // §7.1.6 says the aggregate's own message stream id overrides
        // every sub-message's stream id anyway, so it is ignored.
        let data_start = offset + AGG_SUB_MESSAGE_HEADER_LEN;
        let Some(data_end) = data_size.checked_add(data_start) else {
            return Err(RtmpError::Malformed {
                what: "aggregate sub-message DataSize overflows",
            });
        };
        let back_end = match data_end.checked_add(AGG_BACK_POINTER_LEN) {
            Some(end) if end <= body.len() => end,
            _ => {
                return Err(RtmpError::Malformed {
                    what: "aggregate sub-message data or Back Pointer runs past the end",
                });
            }
        };
        let back_pointer = u32::from_be_bytes([
            body[back_end - AGG_BACK_POINTER_LEN],
            body[back_end - AGG_BACK_POINTER_LEN + 1],
            body[back_end - AGG_BACK_POINTER_LEN + 2],
            body[back_end - AGG_BACK_POINTER_LEN + 3],
        ]);
        let expected_back_pointer = match broadcast_common::len::fit_u32(
            AGG_SUB_MESSAGE_HEADER_LEN + data_size,
            "Back Pointer",
        ) {
            Ok(v) => v,
            Err(_) => {
                return Err(RtmpError::Malformed {
                    what: "aggregate sub-message exceeds the 32-bit Back Pointer field",
                });
            }
        };
        if back_pointer != expected_back_pointer {
            return Err(RtmpError::Malformed {
                what: "aggregate sub-message Back Pointer disagrees with its actual size",
            });
        }

        let ts_offset = *first_ts.get_or_insert(raw_ts);
        // §7.1.6: offset = (aggregate ts − first sub-message ts), added
        // to each sub-message ts.
        let adjusted = raw_ts.wrapping_add(msg.timestamp.wrapping_sub(ts_offset));
        deliver(tag_type, adjusted, &body[data_start..data_end])?;
        offset = back_end;
    }
    Ok(())
}

// ── File header (§E.2) ──────────────────────────────────────────────────

/// FLV file header `Signature` field (§E.2): `"FLV"`.
pub const FLV_SIGNATURE: [u8; 3] = *b"FLV";
/// FLV file header `Version` field this crate writes (§E.2).
pub const FLV_VERSION: u8 = 1;
/// `TypeFlags` bit 0 — audio tags present (§E.2).
pub const FLV_FLAG_AUDIO: u8 = 0x04;
/// `TypeFlags` bit 2 — video tags present (§E.2).
pub const FLV_FLAG_VIDEO: u8 = 0x01;
/// Byte width of the file header's fixed part (§E.2): `Signature`(3) +
/// `Version`(1) + `TypeFlags`(1) + `DataOffset`(4).
pub const FLV_HEADER_LEN: usize = 9;
/// Byte width of the `PreviousTagSize0` field that follows the file header
/// (§E.2) and of the `PreviousTagSize` field after every tag (§E.4.1).
pub const FLV_PREV_TAG_SIZE_LEN: usize = 4;
/// Byte width of one tag's fixed header (§E.4.1): `TagType`(1) +
/// `DataSize`(3) + `Timestamp`(3) + `TimestampExtended`(1) + `StreamID`(3).
pub const FLV_TAG_HEADER_LEN: usize = 11;
/// The `DataOffset` this crate writes: the header is exactly
/// [`FLV_HEADER_LEN`] bytes (§E.2).
pub const FLV_DATA_OFFSET: u32 = 9;
/// Bit position of `TimestampExtended`, which carries the top 8 bits of the
/// tag's 32-bit millisecond `Timestamp` (§E.4.1).
pub const FLV_TIMESTAMP_EXTENDED_SHIFT: u32 = 24;
/// `StreamID` value of every tag this crate writes (§E.4.1).
pub const FLV_STREAM_ID: u32 = 0;

/// The FLV file header (§E.2), including the `PreviousTagSize0` that always
/// follows it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FlvHeader<'a> {
    /// `TypeFlags` bit 0 (§E.2): the file carries audio tags.
    pub has_audio: bool,
    /// `TypeFlags` bit 2 (§E.2): the file carries video tags.
    pub has_video: bool,
    /// `DataOffset` as it was on the wire (§E.2): the first byte after the
    /// header. [`FLV_DATA_OFFSET`] on every header this crate writes; an
    /// input may declare a larger one (extra header bytes the caller skips),
    /// and keeping it makes a parse/serialize round trip byte-exact.
    pub data_offset: u32,
    /// The `DataOffset - 9` extra header bytes between the fixed 9-byte
    /// header and the `PreviousTagSize0` that the parser skipped (§E.2).
    /// Empty on every header this crate writes.
    pub extra: &'a [u8],
}

impl<'a> FlvHeader<'a> {
    /// A header declaring the given track kinds, with the standard
    /// `DataOffset`.
    pub fn new(has_audio: bool, has_video: bool) -> Self {
        Self {
            has_audio,
            has_video,
            data_offset: FLV_DATA_OFFSET,
            extra: &[],
        }
    }

    /// The `TypeFlags` byte (§E.2) this header puts on the wire.
    pub fn type_flags(&self) -> u8 {
        let mut flags = 0u8;
        if self.has_audio {
            flags |= FLV_FLAG_AUDIO;
        }
        if self.has_video {
            flags |= FLV_FLAG_VIDEO;
        }
        flags
    }

    /// Offset of the first tag's header: past the `DataOffset` the header
    /// declares and its `PreviousTagSize0` (§E.2). A `DataOffset` shorter
    /// than the header itself is clamped to [`FLV_HEADER_LEN`], since the
    /// first tag cannot start inside the header.
    pub fn first_tag_offset(&self) -> usize {
        usize::try_from(self.data_offset)
            .unwrap_or(usize::MAX)
            .max(FLV_HEADER_LEN)
            .saturating_add(FLV_PREV_TAG_SIZE_LEN)
    }
}

impl<'a> Parse<'a> for FlvHeader<'a> {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> FlvResult<Self> {
        need_in(bytes, FLV_HEADER_LEN, "FLV header")?;
        if bytes.get(..FLV_SIGNATURE.len()) != Some(FLV_SIGNATURE.as_slice()) {
            return Err(RtmpError::Malformed {
                what: "FLV header signature",
            });
        }
        let flags = bytes[4];
        let data_offset = u32::from_be_bytes(bytes[5..FLV_HEADER_LEN].try_into().unwrap());
        let data_offset = usize::try_from(data_offset).map_err(|_| RtmpError::Malformed {
            what: "FLV header DataOffset",
        })?;
        if data_offset < FLV_HEADER_LEN {
            return Err(RtmpError::Malformed {
                what: "FLV header DataOffset",
            });
        }
        need_in(bytes, data_offset, "FLV header")?;
        let extra = &bytes[FLV_HEADER_LEN..data_offset];
        Ok(Self {
            has_audio: flags & FLV_FLAG_AUDIO != 0,
            has_video: flags & FLV_FLAG_VIDEO != 0,
            data_offset: u32::try_from(data_offset).unwrap_or(u32::MAX),
            extra,
        })
    }
}

impl<'a> Serialize for FlvHeader<'a> {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        FLV_HEADER_LEN + self.extra.len() + FLV_PREV_TAG_SIZE_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> FlvResult<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let Some(out) = buf.get_mut(..len) else {
            return Err(RtmpError::from(make_output_too_small(len, buf.len())));
        };
        out.get_mut(..FLV_SIGNATURE.len())
            .unwrap_or_default()
            .copy_from_slice(&FLV_SIGNATURE);
        if let Some(b) = out.get_mut(FLV_SIGNATURE.len()) {
            *b = FLV_VERSION;
        }
        if let Some(b) = out.get_mut(FLV_SIGNATURE.len() + 1) {
            *b = self.type_flags();
        }
        let offset = usize::try_from(self.data_offset).unwrap_or(usize::MAX);
        let offset = fit_u32(offset, "DataOffset")?;
        out.get_mut(5..FLV_HEADER_LEN)
            .unwrap_or_default()
            .copy_from_slice(&offset.to_be_bytes());
        let extra_start = FLV_HEADER_LEN;
        let extra_end = extra_start + self.extra.len();
        if let Some(dst) = out.get_mut(extra_start..extra_end) {
            dst.copy_from_slice(self.extra);
        }
        out.get_mut(extra_end..len)
            .unwrap_or_default()
            .copy_from_slice(&0u32.to_be_bytes());
        Ok(len)
    }
}

// ── Tag (§E.4.1) ────────────────────────────────────────────────────────

/// FLV tag `TagType` (§E.4.1), carrying the RTMP message type ids of a media
/// tag: Audio (8), Video (9) and Data-AMF0 (18) are the same three values in
/// both formats (RTMP §7.1.4, §7.1.5, §7.1.2).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TagType {
    /// Audio tag (8): `AudioTagHeader` + `AACAUDIODATA` (§E.4.2).
    Audio,
    /// Video tag (9): `VideoTagHeader` + `AVCVIDEOPACKET` (§E.4.3).
    Video,
    /// Script-data tag (18): AMF0 `onMetaData` and friends (§E.4.1).
    Script,
    /// Any other tag type byte: legal on the wire, opaque to this codec.
    Reserved(u8),
}

impl TagType {
    /// The wire byte this tag type carries (§E.4.1).
    pub fn to_byte(self) -> u8 {
        match self {
            Self::Audio => msg_type::AUDIO,
            Self::Video => msg_type::VIDEO,
            Self::Script => msg_type::DATA_AMF0,
            Self::Reserved(v) => v,
        }
    }

    /// The tag type for a wire byte: the three known types, anything else
    /// [`TagType::Reserved`].
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            msg_type::AUDIO => Self::Audio,
            msg_type::VIDEO => Self::Video,
            msg_type::DATA_AMF0 => Self::Script,
            other => Self::Reserved(other),
        }
    }

    /// The spec token for the known tag types, `"reserved"` otherwise.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Script => "script",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(TagType, Reserved);

/// One FLV tag (§E.4.1): its header fields, body, and the `PreviousTagSize`
/// that follows it, so a single value is a self-contained slice of an FLV
/// stream.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FlvTag<'a> {
    /// `TagType` byte (§E.4.1).
    pub tag_type: TagType,
    /// Millisecond timestamp: the UI24 `Timestamp` field plus the UI8
    /// `TimestampExtended` as its top byte (§E.4.1).
    pub timestamp: u32,
    /// `StreamID` (§E.4.1), zero for every tag this crate writes.
    pub stream_id: u32,
    /// The tag's `Data` bytes (`DataSize` long).
    pub body: &'a [u8],
}

impl<'a> FlvTag<'a> {
    /// A tag on the stream every FLV stream uses (`StreamID` 0).
    pub fn new(tag_type: TagType, timestamp: u32, body: &'a [u8]) -> Self {
        Self {
            tag_type,
            timestamp,
            stream_id: FLV_STREAM_ID,
            body,
        }
    }

    /// The value of the `PreviousTagSize` field following this tag (§E.4.1):
    /// the tag's header and body, excluding that field itself.
    pub fn previous_tag_size(&self) -> FlvResult<u32> {
        fit_u32(
            FLV_TAG_HEADER_LEN.saturating_add(self.body.len()),
            "PreviousTagSize",
        )
        .map_err(Into::into)
    }

    /// The tag's declared `DataSize` (§E.4.1), read from `bytes`' first
    /// [`FLV_TAG_HEADER_LEN`] bytes — how many body bytes the tag claims,
    /// whether or not they have all arrived yet.
    ///
    /// # Errors
    /// [`RtmpError::BufferTooShort`] if `bytes` does not hold a full header.
    pub fn data_size(bytes: &'a [u8]) -> FlvResult<usize> {
        need_in(bytes, FLV_TAG_HEADER_LEN, "FLV tag header")?;
        let field = |at: usize| bytes.get(at).copied().unwrap_or_default();
        let data_size = u32::from_be_bytes([0, field(1), field(2), field(3)]);
        usize::try_from(data_size).map_err(|_| RtmpError::Malformed {
            what: "FLV tag DataSize",
        })
    }
}

impl<'a> Parse<'a> for FlvTag<'a> {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> FlvResult<Self> {
        let FlvTagHeader {
            tag_type,
            data_size,
            timestamp,
            stream_id,
        } = FlvTagHeader::parse(bytes)?;
        let body_end = FLV_TAG_HEADER_LEN
            .checked_add(
                usize::try_from(data_size).map_err(|_| RtmpError::Malformed {
                    what: "FLV tag DataSize",
                })?,
            )
            .ok_or(RtmpError::Malformed {
                what: "FLV tag DataSize",
            })?;
        // `PreviousTagSize` may be cut off a final tag: §E.4.1 makes it
        // redundant (it repeats this tag's size), so the body is what this
        // decode needs and the trailer is not checked.
        let body = bytes.get(FLV_TAG_HEADER_LEN..body_end).ok_or_else(|| {
            broadcast_common::error::BufferTooShort {
                need: body_end.saturating_add(FLV_PREV_TAG_SIZE_LEN),
                have: bytes.len(),
                what: "FLV tag",
            }
        })?;
        if bytes.len() >= body_end.saturating_add(FLV_PREV_TAG_SIZE_LEN) {
            let trailing_bytes = bytes
                .get(body_end..body_end + FLV_PREV_TAG_SIZE_LEN)
                .ok_or(RtmpError::Malformed {
                    what: "FLV tag PreviousTagSize",
                })?;
            let trailing = u32::from_be_bytes(trailing_bytes.try_into().map_err(|_| {
                RtmpError::Malformed {
                    what: "FLV tag PreviousTagSize",
                }
            })?);
            let expected = u32::try_from(FLV_TAG_HEADER_LEN.saturating_add(
                usize::try_from(data_size).map_err(|_| RtmpError::Malformed {
                    what: "FLV tag DataSize",
                })?,
            ))
            .map_err(|_| RtmpError::Malformed {
                what: "FLV tag PreviousTagSize",
            })?;
            if trailing != expected {
                return Err(RtmpError::Malformed {
                    what: "FLV tag PreviousTagSize",
                });
            }
        }
        Ok(Self {
            tag_type,
            timestamp,
            stream_id,
            body,
        })
    }
}

/// The 11-byte tag header of a tag whose body may not have arrived yet
/// (§E.4.1): `TagType`, `DataSize`, `Timestamp` + `TimestampExtended` and
/// `StreamID`, without the body — what an incremental reader needs to know how
/// many bytes the whole tag takes before it can decode one.
///
/// `FlvTag::parse` and the tolerant walk both read the header through here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlvTagHeader {
    /// `TagType` byte (§E.4.1).
    pub tag_type: TagType,
    /// `DataSize` (§E.4.1): the body length that follows the header.
    pub data_size: u32,
    /// Millisecond timestamp: the UI24 `Timestamp` plus the UI8
    /// `TimestampExtended` as its top byte (§E.4.1).
    pub timestamp: u32,
    /// `StreamID` (§E.4.1).
    pub stream_id: u32,
}

impl FlvTagHeader {
    /// The header of the tag starting at the front of `bytes`.
    ///
    /// # Errors
    /// [`RtmpError::BufferTooShort`] when fewer than [`FLV_TAG_HEADER_LEN`]
    /// bytes are present.
    pub fn parse(bytes: &[u8]) -> FlvResult<Self> {
        need_in(bytes, FLV_TAG_HEADER_LEN, "FLV tag header")?;
        let field = |at: usize| bytes.get(at).copied().unwrap_or_default();
        let data_size = u32::from_be_bytes([0, field(1), field(2), field(3)]);
        let ts_low24 = u32::from_be_bytes([0, field(4), field(5), field(6)]);
        let ts_extended = u32::from(field(7));
        let stream_id = u32::from_be_bytes([0, field(8), field(9), field(10)]);
        Ok(Self {
            tag_type: TagType::from_byte(field(0)),
            data_size,
            timestamp: ts_low24 | (ts_extended << FLV_TIMESTAMP_EXTENDED_SHIFT),
            stream_id,
        })
    }

    /// Bytes of the whole tag this header announces: header, body and the
    /// trailing `PreviousTagSize` (§E.4.1).
    #[must_use]
    pub fn total_len(&self) -> usize {
        let body = usize::try_from(self.data_size).unwrap_or(usize::MAX);
        FLV_TAG_HEADER_LEN
            .saturating_add(body)
            .saturating_add(FLV_PREV_TAG_SIZE_LEN)
    }
}

impl Serialize for FlvTagHeader {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        FLV_TAG_HEADER_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> FlvResult<usize> {
        need_out(buf, FLV_TAG_HEADER_LEN)?;
        let size = ui24(self.data_size, "FLV tag DataSize")?;
        let low = ui24(self.timestamp & TIMESTAMP_LOW24_MASK, "FLV tag Timestamp")?;
        let extended = self.timestamp >> FLV_TIMESTAMP_EXTENDED_SHIFT;
        let Some(extended) = u8::try_from(extended).ok() else {
            return Err(RtmpError::Malformed {
                what: "FLV tag TimestampExtended",
            });
        };
        let stream_id = ui24(self.stream_id, "FLV tag StreamID")?;
        let Some(header) = buf.get_mut(..FLV_TAG_HEADER_LEN) else {
            return Err(RtmpError::Malformed {
                what: "FLV tag header",
            });
        };
        header[0] = self.tag_type.to_byte();
        header[TAG_DATA_SIZE_AT..TAG_TIMESTAMP_AT].copy_from_slice(&size);
        header[TAG_TIMESTAMP_AT..TAG_TIMESTAMP_EXTENDED_AT].copy_from_slice(&low);
        header[TAG_TIMESTAMP_EXTENDED_AT] = extended;
        header[TAG_STREAM_ID_AT..FLV_TAG_HEADER_LEN].copy_from_slice(&stream_id);
        Ok(FLV_TAG_HEADER_LEN)
    }
}

impl Serialize for FlvTag<'_> {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        FLV_TAG_HEADER_LEN
            .saturating_add(self.body.len())
            .saturating_add(FLV_PREV_TAG_SIZE_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> FlvResult<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let data_size = ui24(fit_u24(self.body.len(), "DataSize")?, "FLV tag DataSize")?;
        let timestamp_low = ui24(self.timestamp & TIMESTAMP_LOW24_MASK, "FLV tag Timestamp")?;
        let timestamp_extended = self.timestamp >> FLV_TIMESTAMP_EXTENDED_SHIFT;
        let Some(extended) = u8::try_from(timestamp_extended).ok() else {
            return Err(RtmpError::Malformed {
                what: "FLV tag TimestampExtended",
            });
        };
        let stream_id = ui24(self.stream_id, "FLV tag StreamID")?;
        let previous_tag_size = self.previous_tag_size()?.to_be_bytes();
        let Some(out) = buf.get_mut(..len) else {
            return Err(RtmpError::Malformed {
                what: "FLV tag output buffer",
            });
        };
        let (header, rest) = out.split_at_mut(FLV_TAG_HEADER_LEN);
        let Some(tag_type) = header.first_mut() else {
            return Err(RtmpError::Malformed {
                what: "FLV tag header",
            });
        };
        *tag_type = self.tag_type.to_byte();
        let mut write = |at: usize, bytes: &[u8]| {
            if let Some(slot) = header.get_mut(at..at.saturating_add(bytes.len())) {
                slot.copy_from_slice(bytes);
            }
        };
        write(TAG_DATA_SIZE_AT, &data_size);
        write(TAG_TIMESTAMP_AT, &timestamp_low);
        write(TAG_TIMESTAMP_EXTENDED_AT, &[extended]);
        write(TAG_STREAM_ID_AT, &stream_id);
        let (body, trailer) = rest.split_at_mut(self.body.len());
        body.copy_from_slice(self.body);
        if let Some(slot) = trailer.get_mut(..FLV_PREV_TAG_SIZE_LEN) {
            slot.copy_from_slice(&previous_tag_size);
        }
        Ok(len)
    }
}

/// Offset of `DataSize` inside a tag header (§E.4.1).
const TAG_DATA_SIZE_AT: usize = 1;
/// Offset of the UI24 low `Timestamp` inside a tag header (§E.4.1).
const TAG_TIMESTAMP_AT: usize = 4;
/// Offset of `TimestampExtended` inside a tag header (§E.4.1).
const TAG_TIMESTAMP_EXTENDED_AT: usize = 7;
/// Offset of `StreamID` inside a tag header (§E.4.1).
const TAG_STREAM_ID_AT: usize = 8;
/// Byte width of a UI24 field.
const U24_LEN: usize = 3;
/// The low 24 bits of a tag's `Timestamp` (§E.4.1), the part the UI24 field
/// carries.
const TIMESTAMP_LOW24_MASK: u32 = 0x00FF_FFFF;

/// The three low big-endian bytes of a value that must fit a UI24 field.
fn ui24(value: u32, field: &'static str) -> FlvResult<[u8; U24_LEN]> {
    let widened = fit_u24(usize::try_from(value).unwrap_or(usize::MAX), field)?;
    let bytes = widened.to_be_bytes();
    let Some(low) = bytes.get(1..) else {
        return Err(RtmpError::Malformed { what: field });
    };
    <[u8; U24_LEN]>::try_from(low).map_err(|_| RtmpError::Malformed { what: field })
}

// ── Tag walk ────────────────────────────────────────────────────────────

/// The tags of an FLV stream, in order, starting at `header`'s first tag.
///
/// Strict: no resynchronisation. Every item is exactly one tag (header, body
/// and `PreviousTagSize`); a trailing run too short to be a complete tag is
/// yielded as an error item and ends the walk.
pub fn tags<'a>(input: &'a [u8], header: &FlvHeader<'a>) -> Tags<'a> {
    Tags {
        input,
        offset: header.first_tag_offset(),
    }
}

/// Iterator over an FLV stream's tags — see [`tags`].
#[derive(Debug, Clone)]
pub struct Tags<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for Tags<'a> {
    type Item = FlvResult<FlvTag<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let rest = self.input.get(self.offset..)?;
        if rest.is_empty() {
            return None;
        }
        let tag = match FlvTag::parse(rest) {
            Ok(tag) => tag,
            Err(err) => {
                self.offset = self.input.len();
                return Some(Err(err));
            }
        };
        self.offset = self.offset.saturating_add(tag.serialized_len());
        Some(Ok(tag))
    }
}

// ── Tolerant walk (resynchronising) ─────────────────────────────────────

/// A tag header at `off` that is structurally plausible: a readable header,
/// the always-zero 24-bit `StreamID` (§E.4.1) and, when `known_type`, one of
/// the tag types a media demuxer reads. Unknown types are legal (a walker
/// skips them leniently), so the chain check along the stream accepts them;
/// only a *resync candidate*, picked out of arbitrary bytes, is held to a
/// known type.
fn plausible_tag_header(input: &[u8], off: usize, known_type: bool) -> bool {
    off.saturating_add(FLV_TAG_HEADER_LEN) <= input.len()
        && (!known_type
            || matches!(
                input[off],
                msg_type::AUDIO | msg_type::VIDEO | msg_type::DATA_AMF0
            ))
        && input[off + TAG_STREAM_ID_AT..off + FLV_TAG_HEADER_LEN] == [0, 0, 0]
}

/// The `DataSize` of the tag whose header starts at `off` (the caller has
/// checked that the header is readable).
fn tag_data_size(input: &[u8], off: usize) -> usize {
    FlvTagHeader::parse(&input[off..])
        .ok()
        .and_then(|header| usize::try_from(header.data_size).ok())
        .unwrap_or_default()
}

/// Whether the tag starting at `off` is followed by a consistent chain: its
/// body fits, and what follows it is the end of the buffer or another
/// plausible tag header.
fn tag_chain_holds(input: &[u8], off: usize, known_type: bool) -> bool {
    if !plausible_tag_header(input, off, known_type) {
        return false;
    }
    let Some(body_end) = off
        .saturating_add(FLV_TAG_HEADER_LEN)
        .checked_add(tag_data_size(input, off))
    else {
        return false;
    };
    if body_end > input.len() {
        return false;
    }
    let next = body_end.saturating_add(FLV_PREV_TAG_SIZE_LEN);
    next.saturating_add(FLV_TAG_HEADER_LEN) > input.len()
        || plausible_tag_header(input, next, false)
}

/// First offset at or after `from` where a consistent chain of known-type tags
/// starts.
fn resync_tag(input: &[u8], from: usize) -> Option<usize> {
    (from..input.len()).find(|&at| tag_chain_holds(input, at, true))
}

/// An FLV stream walked tolerantly: every tag, the damaged spans skipped, and
/// whether the input ended on a short tail — see [`tags_resync`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResyncWalk<'a> {
    /// The complete tags, in order, with their offsets.
    pub tags: Vec<(usize, FlvTag<'a>)>,
    /// Offsets of the spans whose tag framing was damaged and skipped.
    pub damaged: Vec<usize>,
    /// `Some` when the input ends mid-tag: the tag it stopped at, and the
    /// bytes that tag would need against the bytes actually present.
    pub truncated_tail: Option<(usize, usize, usize)>,
}

/// Walk an FLV stream's tags tolerantly, from `header`'s first tag.
///
/// Unlike the strict [`tags`], a stream whose framing is damaged part-way
/// still yields the tags after the damage: a span whose tag chain does not hold
/// is skipped to the next consistent chain of known-type tags and reported in
/// [`ResyncWalk::damaged`], and a final tag whose body overruns the input stops
/// the walk (reported in [`ResyncWalk::truncated_tail`]) rather than losing the
/// good prefix. Two shapes are not truncation: a final tag whose body is
/// complete but whose 4-byte `PreviousTagSize` is missing (§E.4.1 makes that
/// field redundant — it repeats the tag's own size) is kept and the walk ends
/// cleanly, and a header too short to read at the very end of the input is
/// simply the end of the walk.
///
/// # Errors
/// [`RtmpError::BufferTooShort`] when `input` is shorter than the header and
/// its `PreviousTagSize0`, and when the *first* tag is already incomplete (that
/// is not a recoverable tail: there is nothing complete to return).
pub fn tags_resync<'a>(input: &'a [u8], header: &FlvHeader<'a>) -> FlvResult<ResyncWalk<'a>> {
    need_in(
        input,
        FLV_HEADER_LEN.saturating_add(FLV_PREV_TAG_SIZE_LEN),
        "FLV header",
    )?;
    let mut off = header.first_tag_offset();
    let mut walk = ResyncWalk {
        tags: Vec::new(),
        damaged: Vec::new(),
        truncated_tail: None,
    };
    while off.saturating_add(FLV_TAG_HEADER_LEN) <= input.len() {
        if !tag_chain_holds(input, off, false)
            && let Some(next) = resync_tag(input, off + 1)
        {
            walk.damaged.push(off);
            off = next;
            continue;
        }
        let Some(body_end) = off
            .saturating_add(FLV_TAG_HEADER_LEN)
            .checked_add(tag_data_size(input, off))
        else {
            return Err(RtmpError::Malformed {
                what: "FLV tag DataSize",
            });
        };
        if body_end > input.len() {
            let need = body_end.saturating_add(FLV_PREV_TAG_SIZE_LEN);
            if walk.tags.is_empty() && walk.damaged.is_empty() {
                return Err(RtmpError::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need,
                        have: input.len(),
                        what: "FLV tag body",
                    },
                ));
            }
            walk.truncated_tail = Some((off, need, input.len().saturating_sub(off)));
            return Ok(walk);
        }
        let codec_end = body_end
            .saturating_add(FLV_PREV_TAG_SIZE_LEN)
            .min(input.len());
        let tag = FlvTag::parse(input.get(off..codec_end).unwrap_or_default())?;
        walk.tags.push((off, tag));
        if body_end.saturating_add(FLV_PREV_TAG_SIZE_LEN) > input.len() {
            return Ok(walk);
        }
        off = body_end.saturating_add(FLV_PREV_TAG_SIZE_LEN);
    }
    Ok(walk)
}

// ── Session writer ──────────────────────────────────────────────────────

/// Builds the FLV byte stream of one RTMP session: the first tag is prefixed
/// with the file header ([`FlvHeader`] + `PreviousTagSize0`), every tag carries
/// its own header and `PreviousTagSize`, so concatenating every returned chunk
/// in order yields a valid FLV stream.
#[derive(Debug, Default)]
pub(crate) struct FlvTagWriter {
    header_sent: bool,
}

impl FlvTagWriter {
    /// One FLV tag for a media or script message body (file header first time).
    ///
    /// # Errors
    /// [`RtmpError::FieldOverflow`] if `payload` does not fit the tag's 24-bit
    /// `DataSize`, or the session's FLV bytes cannot be built.
    pub(crate) fn tag(
        &mut self,
        tag_type: u8,
        timestamp: u32,
        payload: &[u8],
    ) -> FlvResult<Vec<u8>> {
        let payload = if tag_type == msg_type::DATA_AMF0 {
            strip_set_data_frame(payload)
        } else {
            payload
        };
        let mut flv = if self.header_sent {
            Vec::new()
        } else {
            self.header_sent = true;
            FlvHeader::new(true, true).try_to_bytes()?
        };
        let tag = FlvTag::new(TagType::from_byte(tag_type), timestamp, payload);
        flv.extend_from_slice(&tag.try_to_bytes()?);
        Ok(flv)
    }
}

/// The AMF0 handler name a publisher puts in front of `onMetaData` (the FMLE
/// `@setDataFrame` convention); it addresses the server, not the FLV script tag.
const SET_DATA_FRAME: &str = "@setDataFrame";

/// `payload` without a leading AMF0 `@setDataFrame` string, so the FLV script tag
/// begins with `onMetaData` as Adobe FLV v10.1 Annex E.4.1 defines it.
fn strip_set_data_frame(payload: &[u8]) -> &[u8] {
    let Ok(name) = crate::amf0::Amf0Value::String(SET_DATA_FRAME.to_string()).try_to_bytes() else {
        return payload;
    };
    payload.strip_prefix(name.as_slice()).unwrap_or(payload)
}
