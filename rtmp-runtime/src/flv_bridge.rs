//! FLV bridge shared by the server (publish ingest) and the client (play): RTMP
//! Audio(8)/Video(9)/Data-AMF0(18) message bodies are FLV tag bodies (Adobe RTMP 1.0 §7.1.4,
//! §7.1.5, §7.1.2; Adobe FLV v10.1 Annex E.4), so both roles hand the demuxer an FLV byte stream
//! built here rather than a second representation. Aggregate messages (§7.1.6) are unpacked into
//! their sub-messages.
//!
//! Spec: `private/specs-verified/rtmp/rtmp-runtime/rtmp.md` sections 6.6 and 9.

use broadcast_common::Serialize;

use crate::RtmpError;
use crate::chunk::Message;
use alloc::string::ToString;
use alloc::vec::Vec;

type Result<T> = core::result::Result<T, RtmpError>;

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

// ── FLV mapping consts (private/specs-verified/mpeg/transmux/codec/flv.md, Annex E) ────────────

/// FLV file header `Signature` field (Annex E.2): `"FLV"`.
pub(crate) const FLV_SIGNATURE: [u8; 3] = *b"FLV";
/// FLV file header `Version` field (Annex E.2).
pub(crate) const FLV_VERSION: u8 = 1;
/// FLV file header `TypeFlags` field (Annex E.2): bit 0 (audio present) |
/// bit 2 (video present) — this ingest engine always advertises both, since
/// it does not know ahead of time which media types a publisher will send.
pub(crate) const FLV_TYPE_FLAGS_AUDIO_VIDEO: u8 = 0b0000_0101;
/// FLV file header `DataOffset` field (Annex E.2): header size in bytes.
pub(crate) const FLV_HEADER_SIZE: u32 = 9;
/// Byte width of one FLV tag's fixed header (Annex E.4.1): `TagType`(1) +
/// `DataSize`(3) + `Timestamp`(3) + `TimestampExtended`(1) + `StreamID`(3).
pub(crate) const FLV_TAG_HEADER_LEN: usize = 11;
/// Byte width of the `PreviousTagSize` field that follows every FLV tag
/// (Annex E.4.1), and the file header's `PreviousTagSize0` (Annex E.2).
pub(crate) const FLV_PREV_TAG_SIZE_LEN: usize = 4;
/// Largest value the FLV tag's 24-bit `DataSize` field can encode.
pub(crate) const FLV_MAX_DATA_SIZE: usize = 0x00FF_FFFF;

/// Builds the FLV byte stream of one RTMP session: the first tag is prefixed with the 13-byte
/// FLV file header, every tag carries its own header and `PreviousTagSize`, so concatenating
/// every returned chunk in order yields a valid FLV stream.
#[derive(Debug, Default)]
pub(crate) struct FlvTagWriter {
    header_sent: bool,
}

impl FlvTagWriter {
    /// One FLV tag for a media or script message body (file header first time).
    ///
    /// # Errors
    /// [`RtmpError::Unsupported`] if `payload` does not fit the tag's 24-bit `DataSize`.
    pub(crate) fn tag(&mut self, tag_type: u8, timestamp: u32, payload: &[u8]) -> Result<Vec<u8>> {
        let payload = if tag_type == crate::message::msg_type::DATA_AMF0 {
            strip_set_data_frame(payload)
        } else {
            payload
        };
        let mut flv = if self.header_sent {
            Vec::new()
        } else {
            self.header_sent = true;
            flv_file_header()
        };
        flv.extend(flv_tag(tag_type, timestamp, payload)?);
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
    mut deliver: impl FnMut(u8, u32, &[u8]) -> Result<()>,
) -> Result<()> {
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

/// Build the 13-byte FLV file header: 9-byte header (Annex E.2) + the first
/// `PreviousTagSize0` (always `0`).
pub(crate) fn flv_file_header() -> Vec<u8> {
    let mut v = Vec::with_capacity(FLV_HEADER_SIZE as usize + FLV_PREV_TAG_SIZE_LEN);
    v.extend_from_slice(&FLV_SIGNATURE);
    v.push(FLV_VERSION);
    v.push(FLV_TYPE_FLAGS_AUDIO_VIDEO);
    v.extend_from_slice(&FLV_HEADER_SIZE.to_be_bytes());
    v.extend_from_slice(&0u32.to_be_bytes());
    v
}

/// Build one FLV tag (Annex E.4.1): `TagType` + `DataSize` + `Timestamp` +
/// `TimestampExtended` + `StreamID`(always 0) + `Data` + `PreviousTagSize`.
///
/// # Errors
/// [`RtmpError::Unsupported`] if `payload` is too large for the tag's
/// 24-bit `DataSize` field.
pub(crate) fn flv_tag(tag_type: u8, timestamp: u32, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > FLV_MAX_DATA_SIZE {
        return Err(RtmpError::Unsupported {
            what: "flv tag payload exceeds the 24-bit DataSize field",
        });
    }
    let data_size = payload.len() as u32;

    let mut v = Vec::with_capacity(FLV_TAG_HEADER_LEN + payload.len() + FLV_PREV_TAG_SIZE_LEN);
    v.push(tag_type);
    v.push((data_size >> 16) as u8);
    v.push((data_size >> 8) as u8);
    v.push(data_size as u8);
    v.push((timestamp >> 16) as u8);
    v.push((timestamp >> 8) as u8);
    v.push(timestamp as u8);
    v.push((timestamp >> 24) as u8);
    v.extend_from_slice(&[0, 0, 0]); // StreamID, always 0.
    v.extend_from_slice(payload);
    let prev_tag_size = (FLV_TAG_HEADER_LEN + payload.len()) as u32;
    v.extend_from_slice(&prev_tag_size.to_be_bytes());
    Ok(v)
}
