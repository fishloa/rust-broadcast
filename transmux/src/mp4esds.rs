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

/// Number of bytes an `unused`-free varint needs for `value`: the minimal width
/// is what makes `serialized_len` exact, since the width is stored with each
/// descriptor.
fn varint_width(value: usize) -> usize {
    let mut w = 1;
    let mut v = value >> 7;
    while v != 0 {
        w += 1;
        v >>= 7;
    }
    w
}

/// Encode a varint in exactly `width` bytes, zero-extended with leading `0x80`
/// continuation bytes when needed — the fixed 4-byte expanded form ffmpeg and
/// other muxers emit, and the minimal 1-byte form GPAC/Apple/Bento4 emit.
///
/// The width is carried in [`DescriptorSize`] as parsed, so a minimal-width
/// `esds` re-serializes at the same width instead of always growing to 4
/// (r04-W24).
fn write_varint_width(
    buf: &mut [u8],
    cursor: &mut usize,
    value: usize,
    width: usize,
) -> Result<()> {
    if !(1..=MAX_VARINT_BYTES).contains(&width) {
        return Err(Error::InvalidValue {
            field: "descriptor size varint width",
            value: width as u64,
            reason: "varint width must be 1..=4 bytes",
        });
    }
    if value > MAX_DESCRIPTOR_SIZE {
        return Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "descriptor size",
            value: value as u64,
            max: MAX_DESCRIPTOR_SIZE as u64,
        }));
    }
    if varint_width(value) > width {
        return Err(Error::InvalidValue {
            field: "descriptor size varint width",
            value: width as u64,
            reason: "value does not fit the recorded varint width",
        });
    }
    if *cursor + width > buf.len() {
        return Err(Error::OutputBufferTooSmall {
            need: *cursor + width,
            have: buf.len(),
        });
    }
    for i in 0..width {
        let shift = 7 * (width - 1 - i);
        let mut b = ((value >> shift) & 0x7F) as u8;
        if i + 1 < width {
            b |= 0x80;
        }
        buf[*cursor + i] = b;
    }
    *cursor += width;
    Ok(())
}

/// A sub-descriptor the crate does not model (a `tag(8)`/`size`/body triple kept
/// verbatim), so an `esds` carrying one — e.g. the
/// `ProfileLevelIndicationIndexDescriptor` (0x14) or an
/// `IPI_DescrPointer`/language descriptor — is not dropped (r04-W24).
///
/// The descriptor's position relative to the modelled ones is preserved too:
/// [`UnknownDescriptor::position`] records how many modelled descriptors had
/// already been parsed, and serialization emits it after that many — so an
/// `esds` that interleaved an unmodelled descriptor keeps it in place.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UnknownDescriptor {
    /// The descriptor's `tag` byte.
    pub tag: u8,
    /// Width in bytes of the size varint as it appeared on the wire.
    pub size_width: usize,
    /// The descriptor body (after the size varint).
    pub data: Vec<u8>,
    /// How many modelled descriptors had already been seen when this one was
    /// parsed. Serialization emits this descriptor after that many of them, so
    /// an `esds` that interleaves unmodelled descriptors keeps them in place
    /// rather than collecting them at the end.
    pub position: usize,
}

impl UnknownDescriptor {
    /// Total encoded length: tag + size varint + body.
    pub fn serialized_len(&self) -> usize {
        1 + self.size_width + self.data.len()
    }

    /// Serialize tag + width-preserved size varint + body.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;
        buf[cursor] = self.tag;
        cursor += 1;
        write_varint_width(buf, &mut cursor, self.data.len(), self.size_width)?;
        buf[cursor..cursor + self.data.len()].copy_from_slice(&self.data);
        cursor += self.data.len();
        Ok(cursor)
    }
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
    /// Width in bytes of the size varint as it appeared on the wire (1..=4), so
    /// a minimal-width `esds` re-serializes at the same width (r04-W24).
    pub size_width: usize,
}

impl DecoderSpecificInfo {
    const TAG: u8 = TAG_DECODER_SPECIFIC_INFO;

    /// Wrap `data` as a `DecSpecificInfoTag` descriptor, using the fixed
    /// 4-byte varint width every real MP4 muxer emits.
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            size_width: VARINT_WIDTH_FIXED,
        }
    }
}

impl<'a> Parse<'a> for DecoderSpecificInfo {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        Ok(Self {
            data: body.to_vec(),
            size_width: VARINT_WIDTH_FIXED,
        })
    }
}

impl Serialize for DecoderSpecificInfo {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        1 + self.size_width + self.data.len()
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
        write_varint_width(buf, &mut cursor, self.data.len(), self.size_width)?;
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
    /// Width in bytes of the size varint as it appeared on the wire (1..=4).
    pub size_width: usize,
}

impl SLConfigDescriptor {
    const TAG: u8 = TAG_SL_CONFIG;

    /// Wrap `body` as an `SLConfigDescrTag` descriptor, using the fixed 4-byte
    /// varint width every real MP4 muxer emits.
    pub fn new(body: Vec<u8>) -> Self {
        Self {
            body,
            size_width: VARINT_WIDTH_FIXED,
        }
    }

    /// The MP4-storage `predefined = 2` form (ISO/IEC 14496-14 §3.1.2).
    pub fn predefined_two() -> Self {
        Self::new(alloc::vec![2])
    }
}

impl<'a> Parse<'a> for SLConfigDescriptor {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        Ok(Self {
            body: body.to_vec(),
            size_width: VARINT_WIDTH_FIXED,
        })
    }
}

impl Serialize for SLConfigDescriptor {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        1 + self.size_width + self.body.len()
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
        write_varint_width(buf, &mut cursor, self.body.len(), self.size_width)?;
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
    /// Sub-descriptors this crate does not model (e.g. the
    /// `ProfileLevelIndicationIndexDescriptor` 0x14), kept verbatim in wire
    /// order so they are not silently dropped on a round trip (r04-W24).
    pub unknown_descriptors: Vec<UnknownDescriptor>,
    /// Width in bytes of the size varint as it appeared on the wire (1..=4).
    pub size_width: usize,
}

impl DecoderConfigDescriptor {
    const TAG: u8 = TAG_DECODER_CONFIG;

    /// Build a decoder-config descriptor with no opaque sub-descriptors and the
    /// fixed 4-byte varint width.
    pub fn new(
        object_type_indication: u8,
        stream_type: u8,
        up_stream: bool,
        buffer_size_db: u32,
        max_bitrate: u32,
        avg_bitrate: u32,
        decoder_specific_info: Option<DecoderSpecificInfo>,
    ) -> Self {
        Self {
            object_type_indication: ObjectTypeIndication(object_type_indication),
            stream_type: StreamType(stream_type),
            up_stream,
            buffer_size_db,
            max_bitrate,
            avg_bitrate,
            decoder_specific_info,
            unknown_descriptors: Vec::new(),
            size_width: VARINT_WIDTH_FIXED,
        }
    }
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
        let mut unknown_descriptors = Vec::new();
        while cursor < body.len() {
            let sub_tag = body[cursor];
            cursor += 1;
            let (sub_size, sub_width) = parse_varint(body, &mut cursor)?;
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
                    let mut dsi = DecoderSpecificInfo::parse(sub_body)?;
                    dsi.size_width = sub_width;
                    decoder_specific_info = Some(dsi);
                }
                _ => {
                    // Not modelled by this crate: keep it verbatim, and record
                    // how many modelled descriptors preceded it so it can be
                    // written back in the same place (r04-W24).
                    unknown_descriptors.push(UnknownDescriptor {
                        tag: sub_tag,
                        size_width: sub_width,
                        data: sub_body.to_vec(),
                        position: usize::from(decoder_specific_info.is_some()),
                    });
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
            unknown_descriptors,
            size_width: VARINT_WIDTH_FIXED,
        })
    }
}

impl Serialize for DecoderConfigDescriptor {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        1 // tag
            + self.size_width // body size varint
            + DECODER_CONFIG_FIXED
            + self.decoder_specific_info.as_ref().map_or(0, |dsi| dsi.serialized_len())
            + self
                .unknown_descriptors
                .iter()
                .map(UnknownDescriptor::serialized_len)
                .sum::<usize>()
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
                .map_or(0, |dsi| dsi.serialized_len())
            + self
                .unknown_descriptors
                .iter()
                .map(UnknownDescriptor::serialized_len)
                .sum::<usize>();
        write_varint_width(buf, &mut cursor, body_size, self.size_width)?;
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
        for ud in &self.unknown_descriptors {
            if ud.position == 0 {
                cursor += ud.serialize_into(&mut buf[cursor..])?;
            }
        }
        if let Some(ref dsi) = self.decoder_specific_info {
            cursor += dsi.serialize_into(&mut buf[cursor..])?;
        }
        for ud in &self.unknown_descriptors {
            if ud.position > 0 {
                cursor += ud.serialize_into(&mut buf[cursor..])?;
            }
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
    /// `ES_Descriptor` sub-descriptors this crate does not model, kept verbatim
    /// in wire order (r04-W24).
    pub unknown_descriptors: Vec<UnknownDescriptor>,
    /// Width in bytes of the `ES_Descriptor` size varint as parsed (1..=4).
    pub size_width: usize,
}

impl ESDescriptor {
    const TAG: u8 = TAG_ES_DESCRIPTOR;

    /// Build an `ES_Descriptor` with no opaque sub-descriptors, no optional
    /// ES fields, and the fixed 4-byte varint width — the MP4-storage shape
    /// (ISO/IEC 14496-14 §3.1.2).
    pub fn new(
        es_id: u16,
        stream_priority: u8,
        decoder_config: Option<DecoderConfigDescriptor>,
        sl_config: Option<SLConfigDescriptor>,
    ) -> Self {
        Self {
            es_id,
            stream_dependence_flag: false,
            url_flag: false,
            ocr_stream_flag: false,
            stream_priority,
            depends_on_es_id: None,
            url: None,
            ocr_es_id: None,
            decoder_config,
            sl_config,
            unknown_descriptors: Vec::new(),
            size_width: VARINT_WIDTH_FIXED,
        }
    }
}

impl<'a> Parse<'a> for ESDescriptor {
    type Error = Error;

    /// Parse the bytes `Serialize` produces — the `ES_DescrTag` byte, then
    /// the expandable-size varint, then the body (ISO/IEC 14496-1 §7.2.6.5
    /// / §8.3.3) — so `parse(serialize(x)) == x` (#1148).
    ///
    /// Reading the *body* from offset 0 instead made the tag and size bytes
    /// be misread as `ES_ID`/flags/`URLlength`: `parse` then consumed a few
    /// bytes fewer than the tag+varint prefix it was fed, so its sub-descriptor
    /// walk ended short of where the descriptor actually ends — returning a
    /// silently truncated result, or, once a `URLstring` pushed the size
    /// varint wide, a spurious `BufferTooShort` even though every byte was
    /// present.
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut tag_cursor = 0usize;
        if tag_cursor >= bytes.len() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: bytes.len(),
                what: "ES_Descriptor tag",
            });
        }
        let tag = bytes[tag_cursor];
        tag_cursor += 1;
        if tag != Self::TAG {
            return Err(Error::InvalidValue {
                field: "descriptor_tag",
                value: tag as u64,
                reason: "expected ES_DescrTag (0x03)",
            });
        }
        let (size, size_width) = parse_varint(bytes, &mut tag_cursor)?;
        // The varint size is wire-controlled; bound it before slicing (r04-C4).
        let end = tag_cursor
            .checked_add(size)
            .filter(|&end| end <= bytes.len())
            .ok_or(Error::BufferTooShort {
                need: tag_cursor.saturating_add(size),
                have: bytes.len(),
                what: "ES_Descriptor body",
            })?;
        let body = &bytes[tag_cursor..end];

        let mut cursor = 0usize;
        let mut unknown_descriptors = Vec::new();

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
            let (sub_size, sub_width) = parse_varint(body, &mut cursor)?;
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
                    let mut dc = DecoderConfigDescriptor::parse(sub_body)?;
                    dc.size_width = sub_width;
                    decoder_config = Some(dc);
                }
                TAG_SL_CONFIG => {
                    let mut sl = SLConfigDescriptor::parse(sub_body)?;
                    sl.size_width = sub_width;
                    sl_config = Some(sl);
                }
                _ => {
                    // Not modelled by this crate (e.g. IPI_DescrPointer,
                    // language descriptor): keep it verbatim, with its position
                    // among the modelled descriptors (dc = 0, sl = 1) so the
                    // chain re-serializes in the same order (r04-W24).
                    let position =
                        usize::from(decoder_config.is_some()) + usize::from(sl_config.is_some());
                    unknown_descriptors.push(UnknownDescriptor {
                        tag: sub_tag,
                        size_width: sub_width,
                        data: sub_body.to_vec(),
                        position,
                    });
                }
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
            unknown_descriptors,
            size_width,
        })
    }
}

impl Serialize for ESDescriptor {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let body_size = 2 + 1 // ES_ID + flags
            + if self.depends_on_es_id.is_some() { 2 } else { 0 }
            + self.url.as_ref().map_or(0, |u| 1 + u.len())
            + if self.ocr_es_id.is_some() { 2 } else { 0 };

        1 // tag
            + self.size_width
            + body_size
            + self.decoder_config.as_ref().map_or(0, |dc| dc.serialized_len())
            + self.sl_config.as_ref().map_or(0, |sl| sl.serialized_len())
            + self
                .unknown_descriptors
                .iter()
                .map(UnknownDescriptor::serialized_len)
                .sum::<usize>()
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

        // Field *presence* is derived from the `Option`s, not from the
        // `*_flag` booleans: a flag set without its field (or a field set
        // without its flag) would make `body_size` count bytes that are never
        // written, misframing the descriptor while still returning `Ok`
        // (r04-W24).
        //
        // Deriving it the other way — writing from the flags — is not possible
        // without inventing a value, so a disagreement is an error.
        if self.stream_dependence_flag != self.depends_on_es_id.is_some() {
            return Err(Error::InvalidValue {
                field: "ES_Descriptor.streamDependenceFlag",
                value: self.stream_dependence_flag as u64,
                reason: "streamDependenceFlag disagrees with depends_on_es_id",
            });
        }
        if self.url_flag != self.url.is_some() {
            return Err(Error::InvalidValue {
                field: "ES_Descriptor.URL_Flag",
                value: self.url_flag as u64,
                reason: "URL_Flag disagrees with url",
            });
        }
        if self.ocr_stream_flag != self.ocr_es_id.is_some() {
            return Err(Error::InvalidValue {
                field: "ES_Descriptor.OCRstreamFlag",
                value: self.ocr_stream_flag as u64,
                reason: "OCRstreamFlag disagrees with ocr_es_id",
            });
        }

        // Compute body size (everything after tag+varint)
        let mut body_size = 2 + 1; // ES_ID + flags
        if self.depends_on_es_id.is_some() {
            body_size += 2;
        }
        if let Some(u) = &self.url {
            body_size += 1 + u.len();
        }
        if self.ocr_es_id.is_some() {
            body_size += 2;
        }
        body_size += self
            .decoder_config
            .as_ref()
            .map_or(0, |dc| dc.serialized_len())
            + self.sl_config.as_ref().map_or(0, |sl| sl.serialized_len())
            + self
                .unknown_descriptors
                .iter()
                .map(UnknownDescriptor::serialized_len)
                .sum::<usize>();

        write_varint_width(buf, &mut cursor, body_size, self.size_width)?;

        // ES_ID
        buf[cursor..cursor + 2].copy_from_slice(&self.es_id.to_be_bytes());
        cursor += 2;

        // flags
        let mut flags = self.stream_priority & 0x1F;
        if self.depends_on_es_id.is_some() {
            flags |= 0x80;
        }
        if self.url.is_some() {
            flags |= 0x40;
        }
        if self.ocr_es_id.is_some() {
            flags |= 0x20;
        }
        buf[cursor] = flags;
        cursor += 1;

        // dependsOn_ES_ID
        if let Some(dep) = self.depends_on_es_id {
            buf[cursor..cursor + 2].copy_from_slice(&dep.to_be_bytes());
            cursor += 2;
        }

        // URL
        if let Some(u) = &self.url {
            buf[cursor] = broadcast_common::len::fit_u8(u.len(), "URLstring length")?;
            cursor += 1;
            buf[cursor..cursor + u.len()].copy_from_slice(u.as_bytes());
            cursor += u.len();
        }

        // OCR_ES_Id
        if let Some(ocr) = self.ocr_es_id {
            buf[cursor..cursor + 2].copy_from_slice(&ocr.to_be_bytes());
            cursor += 2;
        }

        // Sub-descriptors: emit each unmodelled descriptor after the modelled
        // ones that preceded it, so an interleaved chain keeps its order
        // (r04-W24).
        for slot in 0..=1usize {
            for ud in &self.unknown_descriptors {
                if ud.position == slot {
                    cursor += ud.serialize_into(&mut buf[cursor..])?;
                }
            }
            match slot {
                0 => {
                    if let Some(ref d) = self.decoder_config {
                        cursor += d.serialize_into(&mut buf[cursor..])?;
                    }
                }
                _ => {
                    if let Some(ref d) = self.sl_config {
                        cursor += d.serialize_into(&mut buf[cursor..])?;
                    }
                }
            }
        }
        for ud in &self.unknown_descriptors {
            if ud.position > 1 {
                cursor += ud.serialize_into(&mut buf[cursor..])?;
            }
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
    /// The `DecoderSpecificInfo` payload (for AAC, the `AudioSpecificConfig`
    /// bytes), if the `DecoderConfigDescriptor` carries one.
    pub(crate) fn decoder_specific_info_data(&self) -> Option<&[u8]> {
        self.es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|dc| dc.decoder_specific_info.as_ref())
            .map(|dsi| dsi.data.as_slice())
    }

    /// The `DecoderSpecificInfo` payload, or the one "no DSI in `esds`" error
    /// every consumer reports (audit r05-O6 / #1141: `dash`, `ts_mux` and
    /// `smooth` each carried their own lookup + message).
    pub(crate) fn require_decoder_specific_info(&self) -> Result<&[u8]> {
        self.decoder_specific_info_data()
            .ok_or(Error::UnexpectedBox {
                expected: "DecoderSpecificInfo (AudioSpecificConfig) in esds",
            })
    }

    /// Parse the `AudioSpecificConfig` carried in the `DecoderSpecificInfo`.
    pub(crate) fn audio_specific_config(&self) -> Result<crate::aac_asc::AudioSpecificConfig> {
        crate::aac_asc::AudioSpecificConfig::parse(self.require_decoder_specific_info()?)
    }

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
        // ISO/IEC 14496-12 §4.2: size == 0 means the box extends to the end of
        // the enclosing data; a declared size past `data` is rejected (r04-C4).
        let end = match header.size {
            0 => data.len(),
            size => usize::try_from(size).unwrap_or(usize::MAX),
        };
        let body = data
            .get(header.header_size()..end)
            .ok_or(Error::BufferTooShort {
                need: end,
                have: data.len(),
                what: "esds box body",
            })?;
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

        // The payload is a single ES_Descriptor: tag + expandable size + body.
        // `ESDescriptor::parse` now reads that framing itself (the same bytes
        // `Serialize` writes — #1148), and records the size-varint width it
        // was authored with for re-serialization (r04-W24).
        let es_descriptor = ESDescriptor::parse(payload)?;

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
    fn test_write_varint_width_is_4_byte_expanded_and_round_trips() {
        // Width 4 always emits the 4-byte expanded form (matches ffmpeg).
        for &val in &[0usize, 1, 0x25, 0x17, 5, 0x4000, 0x0FFF_FFFF] {
            let mut buf = [0u8; 4];
            let mut c = 0usize;
            write_varint_width(&mut buf, &mut c, val, 4).unwrap();
            assert_eq!(c, 4, "width-4 varint is always 4 bytes for {val}");
            // high bit set on first three bytes, clear on last
            assert_eq!(buf[0] & 0x80, 0x80);
            assert_eq!(buf[3] & 0x80, 0x00);
            let mut rc = 0usize;
            let (decoded, _) = parse_varint(&buf, &mut rc).unwrap();
            assert_eq!(decoded, val, "round-trip {val}");
        }
    }

    /// r04-W24: the minimal 1-byte form GPAC/Apple/Bento4 emit must be
    /// reproduced at the same width, not grown to 4.
    #[test]
    fn test_write_varint_width_preserves_minimal_width() {
        for &(val, width) in &[
            (0usize, 1usize),
            (0x25, 1),
            (0x80, 2),
            (0x3FFF, 2),
            (0x4000, 3),
        ] {
            let mut buf = [0u8; 4];
            let mut c = 0usize;
            write_varint_width(&mut buf, &mut c, val, width).unwrap();
            assert_eq!(c, width, "value {val} written in {width} byte(s)");
            let mut rc = 0usize;
            let (decoded, used) = parse_varint(&buf[..c], &mut rc).unwrap();
            assert_eq!(decoded, val);
            assert_eq!(used, width, "parsed width matches written width");
        }
        // A value too large for the recorded width is an error, never a wrap.
        let mut buf = [0u8; 4];
        let mut c = 0usize;
        let err = write_varint_width(&mut buf, &mut c, 0x80, 1).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue {
                    field: "descriptor size varint width",
                    ..
                }
            ),
            "{err:?}"
        );
        let err = write_varint_width(&mut buf, &mut c, 0, 0).unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }), "{err:?}");
        let err = write_varint_width(&mut buf, &mut c, MAX_DESCRIPTOR_SIZE + 1, 4).unwrap_err();
        assert!(matches!(err, Error::FieldOverflow(_)), "{err:?}");
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

    /// r04-W24: an `esds` authored with *minimal* 1-byte descriptor sizes
    /// (GPAC/Apple/Bento4 output) must re-serialize at the same width. Unfixed,
    /// `write_varint_fixed` always emitted the 4-byte expanded form, so the box
    /// grew on every round trip.
    #[test]
    fn minimal_width_esds_round_trips_byte_exact() {
        // Same content as REAL_ESDS_BOX_AAC but with 1-byte size varints.
        // DecoderConfigDescr body = 13 fixed + (tag 0x05 + size + 2) = 17;
        // ES_Descr body = 3 + 19 + 3 = 25; box = 8 + 4 + 27 = 39.
        #[rustfmt::skip]
        let minimal: &[u8] = &[
            0x00, 0x00, 0x00, 0x27, 0x65, 0x73, 0x64, 0x73, // box size 39 + 'esds'
            0x00, 0x00, 0x00, 0x00,                         // FullBox version/flags
            0x03, 0x19,                                     // ES_DescrTag, size 25
            0x00, 0x01,                                     // ES_ID = 1
            0x00,                                           // flags = 0
            0x04, 0x11,                                     // DecoderConfigDescrTag, size 17
            0x40,                                           // objectTypeIndication = MPEG-4 Audio
            0x15,                                           // streamType(5=Audio)<<2 | upStream | rsvd
            0x00, 0x00, 0x00,                               // bufferSizeDB
            0x00, 0x01, 0x80, 0x7d,                         // maxBitrate
            0x00, 0x01, 0x77, 0x0d,                         // avgBitrate
            0x05, 0x02,                                     // DecSpecificInfoTag, size 2
            0x12, 0x08,                                     // AudioSpecificConfig
            0x06, 0x01,                                     // SLConfigDescrTag, size 1
            0x02,                                           // predefined = 2 (MP4)
        ];
        let esds = EsdsBox::parse_box(minimal).expect("parse minimal-width esds");
        assert_eq!(esds.serialized_len(), minimal.len());
        let mut out = vec![0u8; esds.serialized_len()];
        let n = esds.serialize_into(&mut out).expect("serialize");
        assert_eq!(n, minimal.len());
        assert_eq!(
            &out[..n],
            minimal,
            "minimal-width esds must round-trip exactly"
        );
        // The recorded widths are 1, not the default 4.
        let dc = esds.es_descriptor.decoder_config.as_ref().unwrap();
        assert_eq!(esds.es_descriptor.size_width, 1);
        assert_eq!(dc.size_width, 1);
        assert_eq!(dc.decoder_specific_info.as_ref().unwrap().size_width, 1);
        assert_eq!(esds.es_descriptor.sl_config.as_ref().unwrap().size_width, 1);
    }

    /// r04-W24: an `ES_Descriptor` sub-descriptor the crate does not model
    /// (here the `ProfileLevelIndicationIndexDescriptor` 0x14) must be kept
    /// verbatim. Unfixed, it was dropped and the descriptor chain shrank.
    #[test]
    fn unknown_sub_descriptors_are_preserved() {
        const TAG_PROFILE_LEVEL_INDEX: u8 = 0x14;
        // DecoderConfigDescr body = 13 + (5+2) + (2+2) = 24;
        // ES_Descr body = 3 + 28 + 7 = 38; box = 8 + 4 + 43 = 55.
        #[rustfmt::skip]
        let bytes: &[u8] = &[
            0x00, 0x00, 0x00, 0x37, 0x65, 0x73, 0x64, 0x73, // box size 55 + 'esds'
            0x00, 0x00, 0x00, 0x00,                         // FullBox version/flags
            0x03, 0x80, 0x80, 0x80, 0x26,                   // ES_DescrTag, size 38
            0x00, 0x01,                                     // ES_ID = 1
            0x00,                                           // flags = 0
            0x04, 0x80, 0x80, 0x80, 0x18,                   // DecoderConfigDescrTag, size 24
            0x40,                                           // objectTypeIndication
            0x15,                                           // streamType
            0x00, 0x00, 0x00,                               // bufferSizeDB
            0x00, 0x01, 0x80, 0x7d,                         // maxBitrate
            0x00, 0x01, 0x77, 0x0d,                         // avgBitrate
            0x05, 0x80, 0x80, 0x80, 0x02,                   // DecSpecificInfoTag, size 2
            0x12, 0x08,                                     // AudioSpecificConfig
            0x14, 0x02,                                     // ProfileLevelIndicationIndexDescriptor
            0x01, 0x02,                                     //   (unmodelled) body
            0x06, 0x80, 0x80, 0x80, 0x01,                   // SLConfigDescrTag, size 1
            0x02,                                           // predefined = 2
        ];
        let esds = EsdsBox::parse_box(bytes).expect("parse esds with unknown descriptor");
        let dc = esds.es_descriptor.decoder_config.as_ref().unwrap();
        assert_eq!(dc.unknown_descriptors.len(), 1);
        assert_eq!(dc.unknown_descriptors[0].tag, TAG_PROFILE_LEVEL_INDEX);
        assert_eq!(dc.unknown_descriptors[0].data, vec![0x01, 0x02]);
        assert_eq!(esds.serialized_len(), bytes.len());
        let mut out = vec![0u8; esds.serialized_len()];
        esds.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes, "unknown descriptor must survive the round trip");
    }

    /// r04-W24: an unmodelled descriptor *interleaved* between the modelled
    /// ones keeps its position, not just its bytes. Here an unknown descriptor
    /// sits between the decoder config and the SL config, so serialization must
    /// emit it there rather than collecting it after both.
    #[test]
    fn unknown_sub_descriptors_keep_their_position() {
        const TAG_LANGUAGE: u8 = 0x0E;
        // dc body = 13 + DSI(1 tag + 4 size + 2 data) + unknown(1 tag + 2 size
        // + 2 data) = 24; dc = 29. ES_Descr body = 3 + 29 + language(1 + 2 + 2 =
        // 4) + sl(6) = 42; box = 8 + 4 (FullBox) + 1 (tag) + 4 (size) + 42 = 59.
        #[rustfmt::skip]
        let bytes: &[u8] = &[
            0x00, 0x00, 0x00, 0x3B, 0x65, 0x73, 0x64, 0x73, // box size 59 + 'esds'
            0x00, 0x00, 0x00, 0x00,                         // FullBox version/flags
            0x03, 0x80, 0x80, 0x80, 0x2A,                   // ES_DescrTag, size 42
            0x00, 0x01,                                     // ES_ID = 1
            0x00,                                           // flags = 0
            0x04, 0x80, 0x80, 0x80, 0x18,                   // DecoderConfigDescrTag, size 24
            0x40,                                           // objectTypeIndication
            0x15,                                           // streamType
            0x00, 0x00, 0x00,                               // bufferSizeDB
            0x00, 0x01, 0x80, 0x7d,                         // maxBitrate
            0x00, 0x01, 0x77, 0x0d,                         // avgBitrate
            0x05, 0x80, 0x80, 0x80, 0x02,                   // DecSpecificInfoTag, size 2
            0x12, 0x08,                                     // AudioSpecificConfig
            0x14, 0x02,                                     // unmodelled, inside dc
            0x01, 0x02,
            0x0E, 0x02,                                     // unmodelled LanguageDescriptor
            0x65, 0x6E,                                     //   between dc and sl
            0x06, 0x80, 0x80, 0x80, 0x01,                   // SLConfigDescrTag, size 1
            0x02,                                           // predefined = 2
        ];
        let esds = EsdsBox::parse_box(bytes).expect("parse interleaved esds");
        let es = &esds.es_descriptor;
        assert_eq!(es.unknown_descriptors.len(), 1);
        assert_eq!(es.unknown_descriptors[0].tag, TAG_LANGUAGE);
        assert_eq!(es.unknown_descriptors[0].position, 1, "after dc, before sl");
        assert_eq!(
            es.decoder_config.as_ref().unwrap().unknown_descriptors[0].position,
            1
        );
        assert_eq!(esds.serialized_len(), bytes.len());
        let mut out = vec![0u8; esds.serialized_len()];
        esds.serialize_into(&mut out).unwrap();
        assert_eq!(
            out, bytes,
            "an interleaved unknown descriptor must stay between dc and sl"
        );
    }

    /// r04-W24: a flag that disagrees with its field makes the declared body
    /// size count bytes that are never written. Unfixed, the serializer emitted
    /// a misframed descriptor and returned `Ok`.
    #[test]
    fn flag_and_field_disagreement_errors() {
        let base = ESDescriptor::new(
            1,
            0,
            Some(DecoderConfigDescriptor::new(0x40, 5, false, 0, 0, 0, None)),
            Some(SLConfigDescriptor::predefined_two()),
        );

        // streamDependenceFlag set, no depends_on_es_id.
        let mut sdf = base.clone();
        sdf.stream_dependence_flag = true;
        let err = EsdsBox::new(sdf).try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue {
                    field: "ES_Descriptor.streamDependenceFlag",
                    ..
                }
            ),
            "got {err:?}"
        );

        // url_flag set, no url.
        let mut url = base.clone();
        url.url_flag = true;
        let err = EsdsBox::new(url).try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue {
                    field: "ES_Descriptor.URL_Flag",
                    ..
                }
            ),
            "got {err:?}"
        );

        // ocr_stream_flag set, no ocr_es_id.
        let mut ocr = base.clone();
        ocr.ocr_stream_flag = true;
        let err = EsdsBox::new(ocr).try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue {
                    field: "ES_Descriptor.OCRstreamFlag",
                    ..
                }
            ),
            "got {err:?}"
        );

        // The reverse (field present, flag clear) is rejected too.
        let mut reverse = base.clone();
        reverse.depends_on_es_id = Some(7);
        let err = EsdsBox::new(reverse).try_to_bytes().unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }), "got {err:?}");

        // With the flag and field in agreement, serialization succeeds and the
        // declared body size matches what is written.
        let mut ok = base.clone();
        ok.url_flag = true;
        ok.url = Some(alloc::string::String::from("x"));
        let bytes = EsdsBox::new(ok).try_to_bytes().unwrap();
        assert_eq!(
            EsdsBox::parse_box(&bytes)
                .unwrap()
                .es_descriptor
                .url
                .as_deref(),
            Some("x")
        );
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
        let es = EsdsBox::new(ESDescriptor::new(
            2,
            0,
            Some(DecoderConfigDescriptor::new(
                0x40,
                5,
                false,
                0,
                24576000,
                24576005,
                Some(DecoderSpecificInfo::new(vec![0x12, 0x08, 0x56, 0xe5, 0x00])),
            )),
            Some(SLConfigDescriptor::predefined_two()),
        ));

        let original = es.to_bytes();

        // Mutate objectTypeIndication
        let mut es2 = es.clone();
        let dc = es2.es_descriptor.decoder_config.as_mut().unwrap();
        dc.object_type_indication = ObjectTypeIndication(0x21); // AVC
        let mutated = es2.to_bytes();
        assert_ne!(mutated, original, "mutating OTI must change bytes");
    }

    fn es_descriptor_with_url(url: alloc::string::String) -> ESDescriptor {
        ESDescriptor {
            url_flag: true,
            url: Some(url),
            ..ESDescriptor::new(
                2,
                0,
                Some(DecoderConfigDescriptor::new(
                    0x40,
                    5,
                    false,
                    0,
                    24576000,
                    24576005,
                    Some(DecoderSpecificInfo::new(vec![0x12, 0x08, 0x56, 0xe5, 0x00])),
                )),
                Some(SLConfigDescriptor::predefined_two()),
            )
        }
    }

    /// A `URLstring` of 256 bytes cannot fit the 8-bit `URLlength` field
    /// (#1129): unfixed, `(u.len() as u8)` wraps 256 to 0 while the full
    /// URL bytes are still written, misframing the descriptor.
    #[test]
    fn oversized_url_length_errors() {
        let es = es_descriptor_with_url("x".repeat(256));
        let err = es.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "URLstring length",
                    ..
                })
            ),
            "expected FieldOverflow for URLstring length, got {err:?}"
        );
    }

    /// The boundary: exactly 255 bytes (u8::MAX) still round-trips.
    #[test]
    fn max_url_length_round_trips() {
        // Checks the written `URLlength` byte directly (the #1129 concern);
        // full parse/serialize symmetry for every length is covered by
        // `es_descriptor_url_lengths_round_trip` and
        // `es_descriptor_parse_is_symmetric_with_serialize` (#1148).
        let url = "x".repeat(255);
        let es = es_descriptor_with_url(url.clone());
        let bytes = es.try_to_bytes().unwrap();
        let url_bytes = url.as_bytes();
        let pos = bytes
            .windows(url_bytes.len())
            .position(|w| w == url_bytes)
            .expect("URL bytes present in output");
        assert_eq!(bytes[pos - 1], 255, "URLlength byte must be exactly 255");
    }

    /// Regression guard (NOT the #1148 bite): an `esds` carrying a `URLstring` at every legal `URLlength`
    /// (8 bits per ISO/IEC 14496-1 §7.2.6.5, so 0..=255) must survive
    /// parse -> serialize -> parse with equal values and byte-identical
    /// output — including across the 127/128 boundary where the descriptor's
    /// own expandable-size varint may widen. Goes through `EsdsBox`, whose
    /// body-parsing path was already correct before #1148; the test that
    /// fails on the old source is
    /// `es_descriptor_parse_is_symmetric_with_serialize` (public `Parse`).
    #[test]
    fn es_descriptor_url_lengths_round_trip() {
        for &len in &[0usize, 1, 100, 127, 128, 200, 255] {
            let url: alloc::string::String = "x".repeat(len);
            let es = es_descriptor_with_url(url.clone());
            let bytes = EsdsBox::new(es)
                .try_to_bytes()
                .unwrap_or_else(|e| panic!("serialize URL length {len} failed: {e:?}"));
            let parsed = EsdsBox::parse_box(&bytes)
                .unwrap_or_else(|e| panic!("parse URL length {len} failed: {e:?}"));
            assert_eq!(
                parsed.es_descriptor.url.as_deref(),
                Some(url.as_str()),
                "URL length {len} did not survive parse"
            );
            let mut out = vec![0u8; parsed.serialized_len()];
            let n = parsed.serialize_into(&mut out).unwrap();
            assert_eq!(&out[..n], &bytes[..], "URL length {len} not byte-identical");
            assert_eq!(parsed, EsdsBox::parse_box(&out[..n]).unwrap());
        }
    }

    /// #1148 THE BITE: `ESDescriptor::parse` must accept exactly the bytes
    /// `ESDescriptor::serialize_into` produces — the `ES_DescrTag` byte, the
    /// expandable-size varint, then the body — so `parse(serialize(x)) == x`
    /// (the crate's parse/serialize symmetry invariant, §7.2.6.5).
    ///
    /// This goes through the *public* `Parse` impl directly and fails on the
    /// pre-#1148 source, which misread the tag+varint prefix as ES_ID/flags/
    /// `URLlength`. The box-path tests are regression guards, not this.
    #[test]
    fn es_descriptor_parse_is_symmetric_with_serialize() {
        for &len in &[0usize, 1, 100, 101, 127, 128, 200, 255] {
            let url: alloc::string::String = "x".repeat(len);
            let es = es_descriptor_with_url(url);
            let bytes = es.try_to_bytes().unwrap();
            let parsed = ESDescriptor::parse(&bytes)
                .unwrap_or_else(|e| panic!("parse URL length {len} failed: {e:?}"));
            assert_eq!(parsed, es, "URL length {len} did not survive parse");
            let mut out = vec![0u8; parsed.serialized_len()];
            let n = parsed.serialize_into(&mut out).unwrap();
            assert_eq!(&out[..n], &bytes[..], "URL length {len} not byte-identical");
        }
    }

    /// The minimal expandable-size varint width that fits `size`
    /// (ISO/IEC 14496-1 §8.3.3: 1..=4 bytes, 7 bits each).
    fn minimal_varint_width(size: usize) -> usize {
        const W1_MAX: usize = 0x7F;
        const W2_MAX: usize = 0x3FFF;
        const W3_MAX: usize = 0x1F_FFFF;
        if size <= W1_MAX {
            1
        } else if size <= W2_MAX {
            2
        } else if size <= W3_MAX {
            3
        } else {
            4
        }
    }

    /// Build an `ES_Descriptor` for the flag matrix: all three optional ES
    /// fields as asked for, an unmodelled sub-descriptor chained directly
    /// after the `URLstring` (wire: URL, unknown tag, decoder config, SL
    /// config), and `pad` bytes of unmodelled payload to steer the total
    /// body size. Authored with `size_width` = `minimal`, so the body size
    /// itself decides the varint width.
    fn matrix_descriptor(
        stream_dependence_flag: bool,
        url_flag: bool,
        ocr_stream_flag: bool,
        url_len: usize,
        pad: usize,
    ) -> ESDescriptor {
        const TAG_PROFILE_LEVEL_INDEX: u8 = 0x14;
        let mut es = ESDescriptor::new(
            0x0102,
            7,
            Some(DecoderConfigDescriptor::new(
                0x40,
                5,
                false,
                0x0001_0000,
                0x0001_807d,
                0x0001_770d,
                Some(DecoderSpecificInfo::new(vec![0x12, 0x08])),
            )),
            Some(SLConfigDescriptor::predefined_two()),
        );
        es.stream_dependence_flag = stream_dependence_flag;
        es.depends_on_es_id = stream_dependence_flag.then_some(0x0AAA);
        es.url_flag = url_flag;
        es.url = url_flag.then(|| "u".repeat(url_len));
        es.ocr_stream_flag = ocr_stream_flag;
        es.ocr_es_id = ocr_stream_flag.then_some(0x0BBB);
        es.unknown_descriptors.push(UnknownDescriptor {
            tag: TAG_PROFILE_LEVEL_INDEX,
            size_width: minimal_varint_width(pad),
            data: vec![0x55; pad],
            position: 0,
        });
        // Pick the minimal width once the body size is known (serialize with
        // the default 4-byte width first, then trim).
        es.size_width = VARINT_WIDTH_FIXED;
        let wide_len = es.try_to_bytes().expect("matrix es serializes");
        let body_size = wide_len.len() - 1 - VARINT_WIDTH_FIXED;
        es.size_width = minimal_varint_width(body_size);
        es
    }

    /// #1148/B coverage: all 8 combinations of streamDependenceFlag x
    /// URL_Flag x OCRstreamFlag, each with a `URLstring` at
    /// {0, 1, 100, 200, 255} bytes (only when `URL_Flag`), chained directly
    /// by an unmodelled sub-descriptor *after* the URL. Each case:
    /// parse -> serialize -> byte-identical -> parse equal, and the body
    /// size forces the minimal 1/2-byte varint, plus two padded rows forcing
    /// 3- and 4-byte widths (all four widths observed).
    #[test]
    fn es_descriptor_flag_matrix_round_trips() {
        // Body padding large enough to cross the 2-byte (16384) and
        // 3-byte (2097152) expandable-size widths.
        const PAD_3BYTE: usize = 16_500;
        const PAD_4BYTE: usize = 2_100_000;

        let mut seen_width = [false; 5];
        let mut cases = Vec::new();
        for combo in 0..8u8 {
            let sdf = combo & 0b100 != 0;
            let url_flag = combo & 0b010 != 0;
            let ocr = combo & 0b001 != 0;
            let url_lens: &[usize] = if url_flag {
                &[0, 1, 100, 200, 255]
            } else {
                &[0]
            };
            for &url_len in url_lens {
                cases.push((sdf, url_flag, ocr, url_len, 3usize, "matrix"));
            }
        }
        cases.push((true, true, true, 100, PAD_3BYTE, "3-byte varint"));
        cases.push((false, true, false, 100, PAD_4BYTE, "4-byte varint"));

        for (sdf, url_flag, ocr, url_len, pad, what) in cases {
            let es = matrix_descriptor(sdf, url_flag, ocr, url_len, pad);
            let bytes = es
                .try_to_bytes()
                .unwrap_or_else(|e| panic!("serialize {what} failed: {e:?}"));
            let parsed = ESDescriptor::parse(&bytes)
                .unwrap_or_else(|e| panic!("parse {what} failed: {e:?}"));
            assert_eq!(
                parsed, es,
                "{what} (url {url_len}, pad {pad}) must round-trip"
            );
            assert_eq!(parsed.size_width, es.size_width);
            seen_width[es.size_width] = true;
            // The unmodelled sub-descriptor sits directly after the URL,
            // before the modelled ones.
            assert_eq!(parsed.unknown_descriptors.len(), 1, "{what}");
            assert_eq!(parsed.unknown_descriptors[0].position, 0, "{what}");
            // Actually exercise field values, not just struct equality.
            assert_eq!(parsed.stream_dependence_flag, sdf);
            assert_eq!(parsed.url_flag, url_flag);
            assert_eq!(
                parsed.url.as_deref(),
                url_flag.then(|| "u".repeat(url_len)).as_deref()
            );
            assert_eq!(parsed.ocr_stream_flag, ocr);
            // serialize -> byte-identical -> parse equal.
            let mut out = vec![0u8; parsed.serialized_len()];
            let n = parsed.serialize_into(&mut out).unwrap();
            assert_eq!(&out[..n], &bytes[..], "{what} not byte-identical");
            assert_eq!(ESDescriptor::parse(&out[..n]).unwrap(), parsed, "{what}");
        }
        for (width, produced) in seen_width.iter().enumerate().skip(1) {
            assert!(*produced, "matrix never produced a {width}-byte varint");
        }
    }

    /// #1148/B: over-range inputs are structured errors, never a wrap and
    /// never a panic — both on serialize (URL over `URLlength`'s 8 bits) and
    /// on parse (a hostile expandable-size varint larger than the buffer).
    #[test]
    fn es_descriptor_over_range_is_error_not_panic() {
        // Serialize: URL longer than 255 bytes overflows URLlength (checked
        // fit_u8, not `as u8` which would wrap 256 -> 0).
        for &len in &[256usize, 300, 1000] {
            let err = es_descriptor_with_url("x".repeat(len))
                .try_to_bytes()
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                        field: "URLstring length",
                        ..
                    })
                ),
                "len {len}: expected FieldOverflow, got {err:?}"
            );
        }

        // Parse: tag + a 4-byte expandable size of 0x0FFF_FFFF with only a
        // handful of body bytes — must be BufferTooShort (bounded before
        // slicing), not a slice-out-of-range panic.
        #[rustfmt::skip]
        let hostile: &[u8] = &[
            TAG_ES_DESCRIPTOR, 0xFF, 0xFF, 0xFF, 0x7F,
            0x00, 0x01, 0x00,
        ];
        let err =
            ESDescriptor::parse(hostile).expect_err("hostile size varint must fail, not panic");
        assert!(matches!(err, Error::BufferTooShort { .. }), "got {err:?}");
    }

    /// Regression guard (NOT the #1148 bite): a wire `esds` whose `ES_Descriptor` size varint is *minimal*
    /// width (2 bytes, as GPAC/Apple/Bento4 emit) and whose body exceeds the
    /// 127-byte single-byte varint range because of a >100-byte URL must
    /// parse and re-serialize byte-exactly. This exercises `EsdsBox` over
    /// hand-built wire bytes; the test that fails on the pre-#1148 public
    /// `Parse` is `es_descriptor_parse_is_symmetric_with_serialize`.
    #[test]
    fn minimal_width_esds_with_long_url_round_trips() {
        for &url_len in &[100usize, 127, 128, 200, 255] {
            let url = vec![b'x'; url_len];
            let dsi = [0x12, 0x08, 0x56, 0xe5, 0x00];
            let dc_body = 13 + 1 + 1 + dsi.len();
            let sl_body = 1usize;
            let es_body = 2 + 1 + 1 + url_len + (1 + 1 + dc_body) + (1 + 1 + sl_body);
            assert!(es_body <= 0x3FFF, "minimal ES size must fit 2 bytes");
            let mut wire = Vec::new();
            let es_total = 1 + 2 + es_body;
            let box_total = 8 + 4 + es_total;
            wire.extend_from_slice(&(box_total as u32).to_be_bytes());
            wire.extend_from_slice(b"esds");
            wire.extend_from_slice(&[0, 0, 0, 0]);
            wire.extend_from_slice(&[
                TAG_ES_DESCRIPTOR,
                0x80 | ((es_body >> 7) as u8),
                (es_body & 0x7F) as u8,
            ]);
            wire.extend_from_slice(&[0x00, 0x02]);
            wire.push(0x40);
            wire.push(url_len as u8);
            wire.extend_from_slice(&url);
            wire.extend_from_slice(&[TAG_DECODER_CONFIG, dc_body as u8]);
            wire.extend_from_slice(&[0x40, 0x15, 0, 0, 0, 0, 1, 0x77, 0, 0, 1, 0x77, 0]);
            wire.extend_from_slice(&[TAG_DECODER_SPECIFIC_INFO, dsi.len() as u8]);
            wire.extend_from_slice(&dsi);
            wire.extend_from_slice(&[TAG_SL_CONFIG, sl_body as u8, 0x02]);

            let esds = EsdsBox::parse_box(&wire).unwrap_or_else(|e| {
                panic!("parse minimal-width URL {url_len} failed: {e:?}");
            });
            assert_eq!(
                esds.es_descriptor.url.as_deref().map(str::len),
                Some(url_len)
            );
            assert_eq!(esds.serialized_len(), wire.len());
            let mut out = vec![0u8; esds.serialized_len()];
            let n = esds.serialize_into(&mut out).unwrap();
            assert_eq!(&out[..n], &wire[..], "URL {url_len} not byte-identical");
        }
    }

    // r04-C4: an ES_Descriptor whose varint size exceeds the remaining payload
    // used to slice past the end; must be Err, not panic.
    #[test]
    fn rejects_oversized_es_descriptor_size() {
        // FullBox version/flags + tag 0x03 + 4-byte expanded varint 0x0FFF_FFFF,
        // with no body bytes following.
        #[rustfmt::skip]
        let body: Vec<u8> = [
            0x00, 0x00, 0x00, 0x00, // FullBox version/flags
            TAG_ES_DESCRIPTOR, 0xFF, 0xFF, 0xFF, 0x7F,
        ]
        .to_vec();
        let err = EsdsBox::parse_body(&body).expect_err("oversized ES_Descriptor size must fail");
        assert!(
            matches!(err, Error::BufferTooShort { .. }),
            "expected BufferTooShort, got {err:?}"
        );
    }

    // r04-C4: ISO/IEC 14496-12 §4.2 — size == 0 means the box extends to the
    // end of the enclosing data; a well-formed size-0 esds must parse the same
    // as its explicit-size twin (used to slice [8..0] and panic).
    #[test]
    fn parses_size_zero_box_as_rest_of_buffer() {
        let explicit = EsdsBox::parse_box(REAL_ESDS_BOX_AAC).expect("explicit-size twin parses");
        let mut zeroed = REAL_ESDS_BOX_AAC.to_vec();
        zeroed[..4].copy_from_slice(&0u32.to_be_bytes());
        let to_eof = EsdsBox::parse_box(&zeroed).expect("size-0 esds parses as rest-of-buffer");
        assert_eq!(to_eof, explicit);
    }
}
