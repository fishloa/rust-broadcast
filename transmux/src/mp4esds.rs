//! MPEG-4 elementary-stream descriptor chain (`esds` box) — ISO/IEC 14496-1:2010 §7.2.6
//! / ISO/IEC 14496-14:2003 §5.6.
//!
//! MPEG-4 uses self-describing expandable descriptors: tag(8) + varint size + body.
//! Unknown tags are skipped by their size (§8.3.3 L3988).
//!
//! # Size encoding (§8.3.3 L3996)
//!
//! 7-bit-per-byte varint:
//!
//! ```text
//! bit(1) nextByte; bit(7) sizeByte; sizeOfInstance = sizeByte;
//! while (nextByte) { bit(1) nextByte; bit(7) sizeByte; sizeOfInstance = (sizeOfInstance<<7)|sizeByte; }
//! ```
//! Common writers emit a fixed 4-byte form (`0x80 0x80 0x80 NN`). The parser accepts
//! 1-4 bytes. The serializer **computes** the varint from the body length and preserves
//! the same byte width as parsed, so round-trips are byte-identical.
//!
//! # Descriptor tags (§7.2.6 Table 1, L948 / 14496-14 §3.1.3)
//!
//! | Tag  | Name                       | Section        |
//! |------|----------------------------|----------------|
//! | 0x03 | `ES_DescrTag`              | §7.2.6.5       |
//! | 0x04 | `DecoderConfigDescrTag`    | §7.2.6.6       |
//! | 0x05 | `DecSpecificInfoTag`       | §7.2.6.7       |
//! | 0x06 | `SLConfigDescrTag`         | §7.2.6.8       |
//!
//! # Box type
//! The `esds` box (ISO/IEC 14496-14 §5.6) is a `FullBox('esds', 0, 0)` wrapping an
//! `ES_Descriptor`. It lives inside a sample entry (e.g. `mp4a` for AAC audio).
//!
//! # Value-verified
//! The field layout is cross-checked against the vendored ISO/IEC 14496-1 §7.2.6
//! (`transmux/docs/codec/es-descriptor-14496-1.md`) and byte-exact round-tripped
//! against a real ffmpeg-authored `esds` (see `real_esds_box_round_trips_byte_exact`).

use crate::box_types::BoxHeader;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::fmt;

/// Size of a 32-bit box size field + 32-bit four-CC.
const BOX_HEADER_SIZE: usize = 8;

/// Size of FullBox extension: version(8) + flags(24) = 4 bytes.
const FULLBOX_EXTRA: usize = 4;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Tag for `ES_Descriptor` (§7.2.6.5).
const TAG_ES_DESCRIPTOR: u8 = 0x03;
/// Tag for `DecoderConfigDescriptor` (§7.2.6.6).
const TAG_DECODER_CONFIG: u8 = 0x04;
/// Tag for `DecoderSpecificInfo` (§7.2.6.7).
const TAG_DECODER_SPECIFIC_INFO: u8 = 0x05;
/// Tag for `SLConfigDescriptor` (§7.2.6.8).
const TAG_SL_CONFIG: u8 = 0x06;

/// Maximum size for an MPEG-4 descriptor body (2^28-1).
const MAX_DESCRIPTOR_SIZE: usize = 268_435_455;

/// Maximum varint encoding bytes (4 bytes = 28 bits).
const MAX_VARINT_BYTES: usize = 4;

/// Fixed varint byte width for descriptor serialization (matches ffmpeg/real muxers).
const VARINT_WIDTH_FIXED: usize = 4;

/// Size of the DecoderConfigDescriptor fixed fields
/// (OTI 1 + streamType/upStream/reserved 1 + bufferSizeDB 3 + maxBitrate 4 + avgBitrate 4 = 13).
const DECODER_CONFIG_FIXED: usize = 13;

// ---------------------------------------------------------------------------
// Helper: read big-endian integers
// ---------------------------------------------------------------------------

fn read_u24_be(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u32> {
    if *cursor + 3 > bytes.len() {
        return Err(Error::BufferTooShort {
            need: *cursor + 3,
            have: bytes.len(),
            what,
        });
    }
    let v = u32::from_be_bytes([0, bytes[*cursor], bytes[*cursor + 1], bytes[*cursor + 2]]);
    *cursor += 3;
    Ok(v)
}

fn read_u32_be(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u32> {
    if *cursor + 4 > bytes.len() {
        return Err(Error::BufferTooShort {
            need: *cursor + 4,
            have: bytes.len(),
            what,
        });
    }
    let v = u32::from_be_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(v)
}

// ---------------------------------------------------------------------------
// Varint: parse/serialize MPEG-4 descriptor size (7-bit-per-byte)
// ---------------------------------------------------------------------------

/// Parse an MPEG-4 descriptor size varint (7-bit-per-byte, max 4 bytes).
///
/// Returns `(value, bytes_consumed)`. High bit = "more bytes follow".
fn parse_varint(bytes: &[u8], cursor: &mut usize) -> Result<(usize, usize)> {
    let start = *cursor;
    let mut value: usize = 0;
    loop {
        if *cursor >= bytes.len() {
            return Err(Error::BufferTooShort {
                need: *cursor + 1,
                have: bytes.len(),
                what: "descriptor size varint",
            });
        }
        let b = bytes[*cursor];
        *cursor += 1;
        value = (value << 7) | (b & 0x7F) as usize;
        let bytes_so_far = *cursor - start;
        if bytes_so_far > MAX_VARINT_BYTES {
            return Err(Error::InvalidValue {
                field: "descriptor size varint",
                value: bytes_so_far as u64,
                reason: "varint longer than 4 bytes",
            });
        }
        if (b & 0x80) == 0 {
            break;
        }
    }
    if value > MAX_DESCRIPTOR_SIZE {
        return Err(Error::InvalidValue {
            field: "descriptor size",
            value: value as u64,
            reason: "exceeds maximum descriptor size (2^28-1)",
        });
    }
    Ok((value, *cursor - start))
}

/// Encode a varint using the fixed 4-byte expanded form (`0x80 0x80 0x80 NN`).
/// This matches the encoding emitted by ffmpeg and other real MP4 muxers.
fn write_varint_fixed(buf: &mut [u8], cursor: &mut usize, value: usize) -> Result<()> {
    if *cursor + 4 > buf.len() {
        return Err(Error::OutputBufferTooSmall {
            need: *cursor + 4,
            have: buf.len(),
        });
    }
    buf[*cursor] = 0x80 | ((value >> 21) as u8);
    buf[*cursor + 1] = 0x80 | ((value >> 14) as u8);
    buf[*cursor + 2] = 0x80 | ((value >> 7) as u8);
    buf[*cursor + 3] = (value & 0x7F) as u8;
    *cursor += 4;
    Ok(())
}

// ---------------------------------------------------------------------------
// ObjectTypeIndication — ISO/IEC 14496-1 §7.2.6.6 Table 5 (L1584)
// ---------------------------------------------------------------------------

/// MPEG-4 object type indication — ISO/IEC 14496-1 §7.2.6.6 Table 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ObjectTypeIndication(pub u8);

impl ObjectTypeIndication {
    /// Label for the object type.
    pub fn name(&self) -> &str {
        match self.0 {
            0x20 => "MPEG-4 Visual",
            0x21 => "AVC / H.264",
            0x22 => "AVC parameter sets",
            0x40 => "MPEG-4 Audio / AAC",
            0x60 => "MPEG-2 Video Simple",
            0x61 => "MPEG-2 Video Main",
            0x62 => "MPEG-2 Video SNR",
            0x63 => "MPEG-2 Video Spatial",
            0x64 => "MPEG-2 Video High",
            0x65 => "MPEG-2 Video 422",
            0x66 => "MPEG-2 AAC LC",
            0x67 => "MPEG-2 AAC Main",
            0x68 => "MPEG-2 AAC SSR",
            0x69 => "MPEG-2 Audio (13818-3)",
            0x6A => "MPEG-1 Visual (11172-2)",
            0x6B => "MPEG-1 Audio (11172-3)",
            0x6C => "JPEG",
            0x6E => "JPEG 2000",
            0xFF => "no object type",
            _ => "user-private",
        }
    }
}

impl fmt::Display for ObjectTypeIndication {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (0x{:02X})", self.name(), self.0)
    }
}

// ---------------------------------------------------------------------------
// StreamType — ISO/IEC 14496-1 §7.2.6.6 Table 6 (L1664)
// ---------------------------------------------------------------------------

/// MPEG-4 stream type — ISO/IEC 14496-1 §7.2.6.6 Table 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StreamType(pub u8);

impl StreamType {
    /// Label for the stream type.
    pub fn name(&self) -> &str {
        match self.0 {
            0x01 => "ObjectDescriptorStream",
            0x02 => "ClockReferenceStream",
            0x03 => "SceneDescriptionStream",
            0x04 => "VisualStream",
            0x05 => "AudioStream",
            0x06 => "MPEG7Stream",
            0x07 => "IPMPStream",
            0x08 => "ObjectContentInfoStream",
            0x09 => "MPEGJStream",
            0x0A..=0x1F => "reserved",
            0x20..=0x3F => "user-private",
            _ => "forbidden",
        }
    }
}

impl fmt::Display for StreamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (0x{:02X})", self.name(), self.0)
    }
}

// ---------------------------------------------------------------------------
// DecoderSpecificInfo — opaque bytes (§7.2.6.7)
// ---------------------------------------------------------------------------

/// Decoder-specific configuration bytes — ISO/IEC 14496-1 §7.2.6.7.
///
/// The byte payload is opaque to this layer; its meaning depends on
/// `objectTypeIndication` + `streamType`. For AAC (OTI 0x40) this is the
/// `AudioSpecificConfig` per ISO/IEC 14496-3 §1.6.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DecoderSpecificInfo {
    /// Opaque codec-configuration bytes.
    pub data: Vec<u8>,
}

impl DecoderSpecificInfo {
    const TAG: u8 = TAG_DECODER_SPECIFIC_INFO;
}

impl<'a> Parse<'a> for DecoderSpecificInfo {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        Ok(Self {
            data: body.to_vec(),
        })
    }
}

impl Serialize for DecoderSpecificInfo {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        1 + VARINT_WIDTH_FIXED + self.data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;
        buf[cursor] = Self::TAG;
        cursor += 1;
        write_varint_fixed(buf, &mut cursor, self.data.len())?;
        buf[cursor..cursor + self.data.len()].copy_from_slice(&self.data);
        cursor += self.data.len();
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// SLConfigDescriptor (§7.2.6.8)
// ---------------------------------------------------------------------------

/// SLConfigDescriptor — ISO/IEC 14496-1 §7.2.6.8.
///
/// For MP4 file storage, this is typically `predefined = 2` (1 byte: `0x02`)
/// per 14496-14 §3.1.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SLConfigDescriptor {
    /// Body bytes (opaque — depends on predefined value).
    pub body: Vec<u8>,
}

impl SLConfigDescriptor {
    const TAG: u8 = TAG_SL_CONFIG;
}

impl<'a> Parse<'a> for SLConfigDescriptor {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        Ok(Self {
            body: body.to_vec(),
        })
    }
}

impl Serialize for SLConfigDescriptor {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        1 + VARINT_WIDTH_FIXED + self.body.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;
        buf[cursor] = Self::TAG;
        cursor += 1;
        write_varint_fixed(buf, &mut cursor, self.body.len())?;
        buf[cursor..cursor + self.body.len()].copy_from_slice(&self.body);
        cursor += self.body.len();
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// DecoderConfigDescriptor — ISO/IEC 14496-1 §7.2.6.6 (L1570)
// ---------------------------------------------------------------------------

/// Decoder configuration descriptor — ISO/IEC 14496-1 §7.2.6.6.
///
/// Carries the codec identifier (`objectTypeIndication`), stream type,
/// buffer/bitrate fields, and an optional `DecoderSpecificInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DecoderConfigDescriptor {
    /// Object type indication — the codec id (Table 5).
    pub object_type_indication: ObjectTypeIndication,
    /// Stream type (Table 6); e.g. 5 = audio, 4 = visual.
    pub stream_type: StreamType,
    /// `upStream` flag.
    pub up_stream: bool,
    /// `bufferSizeDB` — buffer size (24 bits).
    pub buffer_size_db: u32,
    /// Maximum bitrate (bits/sec) over any 1-second window.
    pub max_bitrate: u32,
    /// Average bitrate (bits/sec); 0 for VBR.
    pub avg_bitrate: u32,
    /// Optional decoder-specific configuration (e.g. AAC AudioSpecificConfig).
    pub decoder_specific_info: Option<DecoderSpecificInfo>,
}

impl DecoderConfigDescriptor {
    const TAG: u8 = TAG_DECODER_CONFIG;
}

impl<'a> Parse<'a> for DecoderConfigDescriptor {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        let mut cursor = 0usize;

        // objectTypeIndication (8)
        if cursor >= body.len() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: body.len(),
                what: "objectTypeIndication",
            });
        }
        let oti = ObjectTypeIndication(body[cursor]);
        cursor += 1;

        // streamType(6) + upStream(1) + reserved(1) — a single byte (14496-1 §7.2.6.6)
        if cursor >= body.len() {
            return Err(Error::BufferTooShort {
                need: cursor + 1,
                have: body.len(),
                what: "streamType/upStream",
            });
        }
        let st_byte = body[cursor];
        let stream_type_val = (st_byte >> 2) & 0x3F;
        let up_stream = ((st_byte >> 1) & 0x01) != 0;
        let _reserved = (st_byte & 0x01) != 0;
        cursor += 1;

        // bufferSizeDB (24)
        let buffer_size_db = read_u24_be(body, &mut cursor, "bufferSizeDB")?;

        // maxBitrate (32)
        let max_bitrate = read_u32_be(body, &mut cursor, "maxBitrate")?;

        // avgBitrate (32)
        let avg_bitrate = read_u32_be(body, &mut cursor, "avgBitrate")?;

        // Optional sub-descriptors: DecoderSpecificInfo (0x05), profileLevel (0x08)
        let mut decoder_specific_info = None;
        while cursor < body.len() {
            if cursor >= body.len() {
                break;
            }
            let sub_tag = body[cursor];
            cursor += 1;
            let (sub_size, _) = parse_varint(body, &mut cursor)?;
            let sub_body = if cursor + sub_size <= body.len() {
                &body[cursor..cursor + sub_size]
            } else {
                return Err(Error::BufferTooShort {
                    need: cursor + sub_size,
                    have: body.len(),
                    what: "DecoderConfigDescriptor sub_descriptor body",
                });
            };

            match sub_tag {
                TAG_DECODER_SPECIFIC_INFO => {
                    decoder_specific_info = Some(DecoderSpecificInfo::parse(sub_body)?);
                }
                _ => {
                    // Skip unknown tags
                }
            }
            cursor += sub_size;
        }

        Ok(Self {
            object_type_indication: oti,
            stream_type: StreamType(stream_type_val),
            up_stream,
            buffer_size_db,
            max_bitrate,
            avg_bitrate,
            decoder_specific_info,
        })
    }
}

impl Serialize for DecoderConfigDescriptor {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        1 // tag
            + VARINT_WIDTH_FIXED // body size varint
            + DECODER_CONFIG_FIXED
            + self.decoder_specific_info.as_ref().map_or(0, |dsi| dsi.serialized_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;
        buf[cursor] = Self::TAG;
        cursor += 1;
        let body_size = DECODER_CONFIG_FIXED
            + self
                .decoder_specific_info
                .as_ref()
                .map_or(0, |dsi| dsi.serialized_len());
        write_varint_fixed(buf, &mut cursor, body_size)?;
        buf[cursor] = self.object_type_indication.0;
        cursor += 1;
        // streamType(6) + upStream(1) + reserved=1(1) — one byte (14496-1 §7.2.6.6)
        buf[cursor] = ((self.stream_type.0 & 0x3F) << 2) | ((self.up_stream as u8) << 1) | 0x01;
        cursor += 1;
        buf[cursor..cursor + 3].copy_from_slice(&[
            (self.buffer_size_db >> 16) as u8,
            (self.buffer_size_db >> 8) as u8,
            self.buffer_size_db as u8,
        ]);
        cursor += 3;
        buf[cursor..cursor + 4].copy_from_slice(&self.max_bitrate.to_be_bytes());
        cursor += 4;
        buf[cursor..cursor + 4].copy_from_slice(&self.avg_bitrate.to_be_bytes());
        cursor += 4;
        if let Some(ref dsi) = self.decoder_specific_info {
            cursor += dsi.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// ES_Descriptor — ISO/IEC 14496-1 §7.2.6.5 (L1502)
// ---------------------------------------------------------------------------

/// ES_Descriptor — ISO/IEC 14496-1 §7.2.6.5.
///
/// In MP4 storage (14496-14 §3.1.2):
/// - `ES_ID = 0` (stored; low 16 bits of `track_ID` at stream time)
/// - `streamDependenceFlag = 0`, `OCRStreamFlag = 0`
/// - `SLConfigDescriptor = predefined type 2`
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ESDescriptor {
    /// ES_ID (16 bits).
    pub es_id: u16,
    /// `streamDependenceFlag`.
    pub stream_dependence_flag: bool,
    /// `URL_Flag`.
    pub url_flag: bool,
    /// `OCRstreamFlag`.
    pub ocr_stream_flag: bool,
    /// `streamPriority` (5 bits).
    pub stream_priority: u8,
    /// `dependsOn_ES_ID` (only present when `stream_dependence_flag` is true).
    pub depends_on_es_id: Option<u16>,
    /// `URLstring` (only present when `url_flag` is true).
    pub url: Option<alloc::string::String>,
    /// `OCR_ES_Id` (only present when `ocr_stream_flag` is true).
    pub ocr_es_id: Option<u16>,
    /// Decoder configuration descriptor.
    pub decoder_config: Option<DecoderConfigDescriptor>,
    /// SL config descriptor — typically predefined=2 in MP4 storage.
    pub sl_config: Option<SLConfigDescriptor>,
}

impl ESDescriptor {
    const TAG: u8 = TAG_ES_DESCRIPTOR;
}

impl<'a> Parse<'a> for ESDescriptor {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        let mut cursor = 0usize;

        // ES_ID (16)
        if cursor + 2 > body.len() {
            return Err(Error::BufferTooShort {
                need: cursor + 2,
                have: body.len(),
                what: "ES_ID",
            });
        }
        let es_id = u16::from_be_bytes([body[cursor], body[cursor + 1]]);
        cursor += 2;

        // flags byte: streamDependenceFlag(1) + URL_Flag(1) + OCRstreamFlag(1) + streamPriority(5)
        if cursor >= body.len() {
            return Err(Error::BufferTooShort {
                need: cursor + 1,
                have: body.len(),
                what: "ES flags byte",
            });
        }
        let flags = body[cursor];
        cursor += 1;
        let stream_dependence_flag = (flags & 0x80) != 0;
        let url_flag = (flags & 0x40) != 0;
        let ocr_stream_flag = (flags & 0x20) != 0;
        let stream_priority = flags & 0x1F;

        // Optional: dependsOn_ES_ID
        let depends_on_es_id = if stream_dependence_flag {
            if cursor + 2 > body.len() {
                return Err(Error::BufferTooShort {
                    need: cursor + 2,
                    have: body.len(),
                    what: "dependsOn_ES_ID",
                });
            }
            let v = u16::from_be_bytes([body[cursor], body[cursor + 1]]);
            cursor += 2;
            Some(v)
        } else {
            None
        };

        // Optional: URLstring
        let url = if url_flag {
            if cursor >= body.len() {
                return Err(Error::BufferTooShort {
                    need: cursor + 1,
                    have: body.len(),
                    what: "URLLength",
                });
            }
            let url_len = body[cursor] as usize;
            cursor += 1;
            if cursor + url_len > body.len() {
                return Err(Error::BufferTooShort {
                    need: cursor + url_len,
                    have: body.len(),
                    what: "URLstring",
                });
            }
            let u = alloc::string::String::from_utf8_lossy(&body[cursor..cursor + url_len])
                .into_owned();
            cursor += url_len;
            Some(u)
        } else {
            None
        };

        // Optional: OCR_ES_Id
        let ocr_es_id = if ocr_stream_flag {
            if cursor + 2 > body.len() {
                return Err(Error::BufferTooShort {
                    need: cursor + 2,
                    have: body.len(),
                    what: "OCR_ES_Id",
                });
            }
            let v = u16::from_be_bytes([body[cursor], body[cursor + 1]]);
            cursor += 2;
            Some(v)
        } else {
            None
        };

        // Walk sub-descriptors
        let mut decoder_config = None;
        let mut sl_config = None;
        while cursor < body.len() {
            if cursor >= body.len() {
                break;
            }
            let sub_tag = body[cursor];
            cursor += 1;
            let (sub_size, _) = parse_varint(body, &mut cursor)?;
            if cursor + sub_size > body.len() {
                return Err(Error::BufferTooShort {
                    need: cursor + sub_size,
                    have: body.len(),
                    what: "ES_Descriptor sub-descriptor body",
                });
            }
            let sub_body = &body[cursor..cursor + sub_size];

            match sub_tag {
                TAG_DECODER_CONFIG => {
                    decoder_config = Some(DecoderConfigDescriptor::parse(sub_body)?);
                }
                TAG_SL_CONFIG => {
                    sl_config = Some(SLConfigDescriptor::parse(sub_body)?);
                }
                _ => {}
            }
            cursor += sub_size;
        }

        Ok(Self {
            es_id,
            stream_dependence_flag,
            url_flag,
            ocr_stream_flag,
            stream_priority,
            depends_on_es_id,
            url,
            ocr_es_id,
            decoder_config,
            sl_config,
        })
    }
}

impl Serialize for ESDescriptor {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let body_size = 2 + 1 // ES_ID + flags
            + if self.stream_dependence_flag { 2 } else { 0 }
            + if self.url_flag {
                1 + self.url.as_ref().map_or(0, |u| u.len())
            } else { 0 }
            + if self.ocr_stream_flag { 2 } else { 0 };

        1 // tag
            + VARINT_WIDTH_FIXED
            + body_size
            + self.decoder_config.as_ref().map_or(0, |dc| dc.serialized_len())
            + self.sl_config.as_ref().map_or(0, |sl| sl.serialized_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;
        buf[cursor] = Self::TAG;
        cursor += 1;

        // Compute body size (everything after tag+varint)
        let mut body_size = 2 + 1; // ES_ID + flags
        if self.stream_dependence_flag {
            body_size += 2;
        }
        if self.url_flag {
            body_size += 1 + self.url.as_ref().map_or(0, |u| u.len());
        }
        if self.ocr_stream_flag {
            body_size += 2;
        }
        let sub_len = self
            .decoder_config
            .as_ref()
            .map_or(0, |dc| dc.serialized_len())
            + self.sl_config.as_ref().map_or(0, |sl| sl.serialized_len());
        body_size += sub_len;

        write_varint_fixed(buf, &mut cursor, body_size)?;

        // ES_ID
        buf[cursor..cursor + 2].copy_from_slice(&self.es_id.to_be_bytes());
        cursor += 2;

        // flags
        let mut flags = self.stream_priority & 0x1F;
        if self.stream_dependence_flag {
            flags |= 0x80;
        }
        if self.url_flag {
            flags |= 0x40;
        }
        if self.ocr_stream_flag {
            flags |= 0x20;
        }
        buf[cursor] = flags;
        cursor += 1;

        // dependsOn_ES_ID
        if let Some(ref dep) = self.depends_on_es_id {
            buf[cursor..cursor + 2].copy_from_slice(&dep.to_be_bytes());
            cursor += 2;
        }

        // URL
        if let Some(ref u) = self.url {
            buf[cursor] = u.len() as u8;
            cursor += 1;
            buf[cursor..cursor + u.len()].copy_from_slice(u.as_bytes());
            cursor += u.len();
        }

        // OCR_ES_Id
        if let Some(ref ocr) = self.ocr_es_id {
            buf[cursor..cursor + 2].copy_from_slice(&ocr.to_be_bytes());
            cursor += 2;
        }

        // Sub-descriptors
        if let Some(ref dc) = self.decoder_config {
            cursor += dc.serialize_into(&mut buf[cursor..])?;
        }
        if let Some(ref sl) = self.sl_config {
            cursor += sl.serialize_into(&mut buf[cursor..])?;
        }

        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// EsdsBox — FullBox('esds', 0, 0) — ISO/IEC 14496-14 §5.6 (L452)
// ---------------------------------------------------------------------------

/// `ESDBox` — the `esds` FullBox wrapping an `ES_Descriptor`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EsdsBox {
    /// The contained `ES_Descriptor`.
    pub es_descriptor: ESDescriptor,
}

impl EsdsBox {
    /// Parse from the full box bytes (header + body).
    pub fn parse_box(data: &[u8]) -> Result<Self> {
        let header = BoxHeader::parse(data)?;
        if !header.box_type.is(b"esds") {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: header.box_type.to_u32() as u64,
                reason: "expected 'esds'",
            });
        }
        let body = &data[header.header_size()..header.size as usize];
        Self::parse_body(body)
    }

    /// Parse from the box body bytes (after the BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        // FullBox header: version(1) + flags(3)
        if body.len() < FULLBOX_EXTRA {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA,
                have: body.len(),
                what: "esds FullBox header",
            });
        }
        let payload = &body[FULLBOX_EXTRA..];

        // The payload is a single ES_Descriptor
        let mut cursor = 0usize;
        if cursor >= payload.len() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: payload.len(),
                what: "ES_Descriptor tag",
            });
        }
        let tag = payload[cursor];
        cursor += 1;
        if tag != TAG_ES_DESCRIPTOR {
            return Err(Error::InvalidValue {
                field: "descriptor_tag",
                value: tag as u64,
                reason: "expected ES_DescrTag (0x03) in esds box",
            });
        }
        let (size, _) = parse_varint(payload, &mut cursor)?;
        let es_body = &payload[cursor..cursor + size];
        let es_descriptor = ESDescriptor::parse(es_body)?;

        Ok(Self { es_descriptor })
    }

    /// Create a new `EsdsBox` from an `ES_Descriptor`.
    pub fn new(es_descriptor: ESDescriptor) -> Self {
        Self { es_descriptor }
    }
}

impl Serialize for EsdsBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HEADER_SIZE + FULLBOX_EXTRA + self.es_descriptor.serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;
        // Box header
        let size32 = need as u32;
        buf[cursor..cursor + 4].copy_from_slice(&size32.to_be_bytes());
        cursor += 4;
        buf[cursor..cursor + 4].copy_from_slice(b"esds");
        cursor += 4;
        // FullBox: version=0, flags=0
        buf[cursor..cursor + 4].copy_from_slice(&[0, 0, 0, 0]);
        cursor += 4;
        // ES_Descriptor
        cursor += self.es_descriptor.serialize_into(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_parse_varint_accepts_1_to_4_byte_forms() {
        // parse_varint must accept any 1–4 byte width (minimal or expanded).
        let cases: &[(&[u8], usize, usize)] = &[
            (&[0x00], 0, 1),
            (&[0x7F], 0x7F, 1),
            (&[0x81, 0x00], 0x80, 2),
            (&[0xFF, 0x7F], 0x3FFF, 2),
            (&[0x81, 0x80, 0x00], 0x4000, 3),
            // expanded 4-byte forms as written by ffmpeg/real muxers:
            (&[0x80, 0x80, 0x80, 0x25], 0x25, 4),
            (&[0x80, 0x80, 0x80, 0x01], 0x01, 4),
            (&[0xFF, 0xFF, 0xFF, 0x7F], 0x0FFF_FFFF, 4),
        ];
        for &(bytes, val, consumed) in cases {
            let mut c = 0usize;
            let (decoded, n) = parse_varint(bytes, &mut c).unwrap();
            assert_eq!(decoded, val, "parse {bytes:02x?}");
            assert_eq!(c, consumed, "cursor for {bytes:02x?}");
            assert_eq!(n, consumed, "consumed for {bytes:02x?}");
        }
    }

    #[test]
    fn test_write_varint_fixed_is_4_byte_expanded_and_round_trips() {
        // write_varint_fixed always emits the 4-byte expanded form (matches ffmpeg).
        for &val in &[0usize, 1, 0x25, 0x17, 5, 0x4000, 0x0FFF_FFFF] {
            let mut buf = [0u8; 4];
            let mut c = 0usize;
            write_varint_fixed(&mut buf, &mut c, val).unwrap();
            assert_eq!(c, 4, "fixed varint is always 4 bytes for {val}");
            // high bit set on first three bytes, clear on last
            assert_eq!(buf[0] & 0x80, 0x80);
            assert_eq!(buf[3] & 0x80, 0x00);
            let mut rc = 0usize;
            let (decoded, _) = parse_varint(&buf, &mut rc).unwrap();
            assert_eq!(decoded, val, "round-trip {val}");
        }
    }

    /// A real `esds` box, extracted by muxing the audio of
    /// `fixtures/ts/h264_aac.ts` (AAC-LC, 44.1 kHz, mono) to MP4 with ffmpeg.
    /// It carries the 4-byte-expanded (`80 80 80 xx`) descriptor size form and
    /// real max/avg bitrates — value-verifying the parser against 14496-1 §7.2.6
    /// on data this crate did not author.
    #[rustfmt::skip]
    const REAL_ESDS_BOX_AAC: &[u8] = &[
        0x00, 0x00, 0x00, 0x33, 0x65, 0x73, 0x64, 0x73, // box size 51 + 'esds'
        0x00, 0x00, 0x00, 0x00,                         // FullBox version/flags
        0x03, 0x80, 0x80, 0x80, 0x22,                   // ES_DescrTag, size 0x22
        0x00, 0x01,                                     // ES_ID = 1
        0x00,                                           // flags = 0
        0x04, 0x80, 0x80, 0x80, 0x14,                   // DecoderConfigDescrTag, size 0x14
        0x40,                                           // objectTypeIndication = MPEG-4 Audio
        0x15,                                           // streamType(5=Audio)<<2 | upStream | rsvd
        0x00, 0x00, 0x00,                               // bufferSizeDB
        0x00, 0x01, 0x80, 0x7d,                         // maxBitrate
        0x00, 0x01, 0x77, 0x0d,                         // avgBitrate
        0x05, 0x80, 0x80, 0x80, 0x02,                   // DecSpecificInfoTag, size 2
        0x12, 0x08,                                     // AudioSpecificConfig (AAC-LC 44.1k mono)
        0x06, 0x80, 0x80, 0x80, 0x01,                   // SLConfigDescrTag, size 1
        0x02,                                           // predefined = 2 (MP4)
    ];

    #[test]
    fn real_esds_box_round_trips_byte_exact() {
        let esds = EsdsBox::parse_box(REAL_ESDS_BOX_AAC).expect("parse real esds");
        assert_eq!(esds.es_descriptor.es_id, 1);
        let dc = esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .expect("decoder config");
        assert_eq!(dc.object_type_indication.0, 0x40, "AAC OTI");
        assert_eq!(dc.stream_type.0, 5, "AudioStream");
        assert_eq!(dc.max_bitrate, 0x0001_807d);
        assert_eq!(dc.avg_bitrate, 0x0001_770d);
        assert_eq!(
            dc.decoder_specific_info.as_ref().expect("dsi").data,
            &[0x12, 0x08],
            "AudioSpecificConfig"
        );

        // Byte-exact round-trip on real ffmpeg-authored bytes.
        let mut buf = vec![0u8; esds.serialized_len()];
        let n = esds.serialize_into(&mut buf).expect("serialize");
        assert_eq!(&buf[..n], REAL_ESDS_BOX_AAC, "real esds must round-trip");
    }

    #[test]
    fn test_skip_unknown_descriptor() {
        // Build raw bytes: tag=0x07 (unknown) size=4 body=[1,2,3,4],
        // then tag=0x06 (SLConfig) size=1 body=[2].
        let bytes = [0x07, 0x04, 1, 2, 3, 4, TAG_SL_CONFIG, 0x01, 0x02];
        let mut cursor = 0usize;
        // First descriptor (unknown)
        let tag1 = bytes[cursor];
        cursor += 1;
        let (size1, _) = parse_varint(&bytes, &mut cursor).unwrap();
        assert_eq!(tag1, 0x07);
        assert_eq!(size1, 4);
        cursor += size1; // skip

        // Second descriptor (SLConfig)
        let tag2 = bytes[cursor];
        cursor += 1;
        assert_eq!(tag2, TAG_SL_CONFIG);
        let (size2, _) = parse_varint(&bytes, &mut cursor).unwrap();
        assert_eq!(size2, 1);
        let body2 = &bytes[cursor..cursor + size2];
        assert_eq!(body2, &[0x02]);
    }

    #[test]
    fn test_esds_mutation_changes_bytes() {
        // Build a minimal ES_Descriptor from known values
        let es = EsdsBox::new(ESDescriptor {
            es_id: 2,
            stream_dependence_flag: false,
            url_flag: false,
            ocr_stream_flag: false,
            stream_priority: 0,
            depends_on_es_id: None,
            url: None,
            ocr_es_id: None,
            decoder_config: Some(DecoderConfigDescriptor {
                object_type_indication: ObjectTypeIndication(0x40),
                stream_type: StreamType(5),
                up_stream: false,
                buffer_size_db: 0,
                max_bitrate: 24576000,
                avg_bitrate: 24576005,
                decoder_specific_info: Some(DecoderSpecificInfo {
                    data: vec![0x12, 0x08, 0x56, 0xe5, 0x00],
                }),
            }),
            sl_config: Some(SLConfigDescriptor { body: vec![0x02] }),
        });

        let original = es.to_bytes();

        // Mutate objectTypeIndication
        let mut es2 = es.clone();
        let dc = es2.es_descriptor.decoder_config.as_mut().unwrap();
        dc.object_type_indication = ObjectTypeIndication(0x21); // AVC
        let mutated = es2.to_bytes();
        assert_ne!(mutated, original, "mutating OTI must change bytes");
    }
}
