//! The 3GPP file-format code points — 3GPP TS 26.244 V19.01.00 §5–§8, §12–§13.
//!
//! Transcription: `private/specs-verified/3gpp/isobmff/ts-126-244-v19.md`.
//!
//! | Code | Type | Clause |
//! |------|------|--------|
//! | `samr`, `sawb` | [`ThreeGppAudioEntry`] | §6.5 |
//! | `s263` | [`H263SampleEntry`] | §6.6 |
//! | `damr` | [`AmrDecoderSpecificBox`] | §6.7 |
//! | `d263` | [`H263DecoderSpecificBox`] | §6.8 |
//! | `sawp` | [`ThreeGppAudioEntry`] | §6.9 |
//! | `dawp` | [`AmrwbPlusDecoderSpecificBox`] | §6.10 |
//! | `3gvo` | [`CvoSampleEntry`] | §6.11 |
//! | `3glo` | [`ThreeGppSimpleEntry`] | §6.12 |
//! | `3gor` | [`ThreeGppSimpleEntry`] | §6.13 |
//! | `sevs` | [`ThreeGppAudioEntry`] | §6.14 |
//! | `clsf` … `orie` | user-data boxes | §8.2 |
//! | `3gau` | [`ApplicationDataUnitInfoBox`] | §11.2 |
//! | `spdb` | [`ServiceProtectionDescriptionBox`] | §12.2.2 |
//! | `tfad`, `tfma` | fragment adjustment | §13.3 |
//!
//! Every fixed field is carried as read, including the reserved bytes and the
//! packed `Pad`/`Language` pair, so a 3GP file round-trips byte for byte.

use alloc::vec::Vec;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::{be_u16, be_u32, u16_at, u32_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, box_len, compact_form, parse_box,
    write_box_header, write_full_box_header,
};
use crate::error::{Error, Result};
use crate::init_segment::{
    AudioSampleEntryFields, ParsedAudioSampleEntry, audio_sample_entry_serialized_len,
    parse_audio_sample_entry, serialize_audio_sample_entry,
};
use crate::sample_entries::{SAMPLE_ENTRY_RESERVED_LEN, VisualSampleEntryFields};

/// `samr`: narrow-band AMR (§6.5).
pub const SAMR: [u8; BOX_TYPE_SIZE] = *b"samr";
/// `sawb`: wide-band AMR (§6.5).
pub const SAWB: [u8; BOX_TYPE_SIZE] = *b"sawb";
/// `sawp`: extended AMR-WB (§6.9).
pub const SAWP: [u8; BOX_TYPE_SIZE] = *b"sawp";
/// `sevs`: Enhanced Voice Services (§6.14).
pub const SEVS: [u8; BOX_TYPE_SIZE] = *b"sevs";
/// `s263`: H.263 video (§6.6).
pub const S263: [u8; BOX_TYPE_SIZE] = *b"s263";
/// `3glo`: location metadata (§6.12).
pub const LOCATION_ENTRY: [u8; BOX_TYPE_SIZE] = *b"3glo";
/// `3gor`: orientation metadata (§6.13).
pub const ORIENTATION_ENTRY: [u8; BOX_TYPE_SIZE] = *b"3gor";
/// `damr`: AMR decoder-specific box (§6.7).
pub const DAMR: [u8; BOX_TYPE_SIZE] = *b"damr";
/// `d263`: H.263 decoder-specific box (§6.8).
pub const D263: [u8; BOX_TYPE_SIZE] = *b"d263";
/// `dawp`: AMR-WB+ decoder-specific box (§6.10).
pub const DAWP: [u8; BOX_TYPE_SIZE] = *b"dawp";

/// `SampleEntry` fixed fields: `reserved[6]` + `data_reference_index`.
const BASE_ENTRY_FIXED: usize = SAMPLE_ENTRY_RESERVED_LEN + core::mem::size_of::<u16>();
/// Width of a `u16`/`u32`/`u64` field.
const U16_LEN: usize = core::mem::size_of::<u16>();
const U32_LEN: usize = core::mem::size_of::<u32>();
const U64_LEN: usize = core::mem::size_of::<u64>();
/// Bit width of a `u32`, and of its half.
const U32_BITS: usize = U32_LEN * 8;
const LOWER_U32_BITS: usize = U32_BITS >> 1;
/// Bits in one byte, and the three `Pad` bytes of `urat`.
const BYTE_BITS: usize = u8::BITS as usize;
const PAD_LEN: usize = 3;

/// Bytes of the `revision_level` + `vendor` block in the fixed audio part.
const AUDIO_VENDOR_BLOCK_LEN: usize = 6;
/// `samplesize` a 3GPP audio entry states (§6.5: 16 bits per sample).
const THREEGPP_SAMPLESIZE: u16 = 16;
/// The 16.16 `samplerate` field carries its integer hertz in the high half.
const SAMPLERATE_INTEGER_SHIFT: u32 = 16;
/// The 15-bit packed language sits after the `Pad` bit and the FullBox header.
const LANGUAGE_LEN: usize = 2;
/// Bits reserved for the language inside the packed pair.
const LANGUAGE_MASK: u16 = 0x7FFF;
/// Shift of the single `Pad` bit inside the packed pair.
const PAD_SHIFT: u32 = 15;

/// A FullBox header followed by the packed `Pad`(1) + `Language`(15) pair
/// (§8.2: every user-data box in this clause uses it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct LanguagePrefix {
    version: u8,
    flags: u32,
    pad: bool,
    language: u16,
}

impl LanguagePrefix {
    fn parse<'a>(body: &'a [u8], what: &'static str) -> Result<(Self, &'a [u8])> {
        need_in(body, FULLBOX_EXTRA_SIZE + LANGUAGE_LEN, what)?;
        let padded = u16_at(body, FULLBOX_EXTRA_SIZE);
        Ok((
            Self {
                version: body[0],
                flags: broadcast_core::wire::u24_at(body, 1),
                pad: padded >> PAD_SHIFT != 0,
                language: padded & LANGUAGE_MASK,
            },
            &body[FULLBOX_EXTRA_SIZE + LANGUAGE_LEN..],
        ))
    }

    fn serialized_len() -> usize {
        FULLBOX_EXTRA_SIZE + LANGUAGE_LEN
    }

    fn write(self, buf: &mut [u8], fourcc: &[u8; BOX_TYPE_SIZE], need: usize) -> Result<usize> {
        let mut c = write_full_box_header(buf, fourcc, need, self.version, self.flags, false)?;
        let padded = u16::from(self.pad) << PAD_SHIFT | self.language;
        buf[c..c + LANGUAGE_LEN].copy_from_slice(&padded.to_be_bytes());
        c += LANGUAGE_LEN;
        Ok(c)
    }
}

/// Read a null-terminated text field that runs to the end of the box.
fn text_to_end(rest: &[u8]) -> Vec<u8> {
    match rest.iter().position(|b| *b == 0) {
        Some(end) => rest[..end].to_vec(),
        None => rest.to_vec(),
    }
}

/// An AMR/AMR-WB/AMR-WB+/EVS audio sample entry (`samr`, `sawb`, `sawp`,
/// `sevs`) — §6.5, §6.9, §6.14.
///
/// All four use the `AudioSampleEntry` fixed layout and put the codec's
/// decoder-specific box among the children; `codec_type` records which one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ThreeGppAudioEntry {
    codec_type: [u8; BOX_TYPE_SIZE],
    entry: ParsedAudioSampleEntry,
}

impl ThreeGppAudioEntry {
    /// Build an entry for one of the four codes with a single mono channel and
    /// a 16-bit sample size, the layout §6.5 and §6.14 state.
    ///
    /// # Errors
    /// `codec_type` is not `samr`, `sawb`, `sawp` or `sevs`.
    pub fn new(codec_type: [u8; BOX_TYPE_SIZE], sample_rate_hz: u16) -> Result<Self> {
        if !matches!(&codec_type, b"samr" | b"sawb" | b"sawp" | b"sevs") {
            return Err(Error::UnexpectedBox {
                expected: "a 3GPP audio sample entry",
            });
        }
        Ok(Self {
            codec_type,
            entry: ParsedAudioSampleEntry {
                reserved: [0; SAMPLE_ENTRY_RESERVED_LEN],
                entry_version: 0,
                reserved_1: [0; AUDIO_VENDOR_BLOCK_LEN],
                data_reference_index: 1,
                channelcount: 1,
                samplesize: THREEGPP_SAMPLESIZE,
                compression_id_and_packet_size: [0; core::mem::size_of::<u32>()],
                samplerate: u32::from(sample_rate_hz) << SAMPLERATE_INTEGER_SHIFT,
                quicktime: None,
                config_boxes: Vec::new(),
                trailing: Vec::new(),
            },
        })
    }

    /// The entry's four-CC.
    #[must_use]
    pub const fn codec_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.codec_type
    }

    /// `channelcount`.
    #[must_use]
    pub const fn channelcount(&self) -> u16 {
        self.entry.channelcount
    }

    /// `samplesize`.
    #[must_use]
    pub const fn samplesize(&self) -> u16 {
        self.entry.samplesize
    }

    /// The 16.16 `samplerate` field as written.
    #[must_use]
    pub const fn samplerate(&self) -> u32 {
        self.entry.samplerate
    }

    /// Every child box after the fixed part, verbatim.
    #[must_use]
    pub fn config_boxes(&self) -> &[OpaqueBox] {
        &self.entry.config_boxes
    }

    /// The AMR decoder-specific box (`damr`), when present (§6.7).
    #[must_use]
    pub fn amr(&self) -> Option<AmrDecoderSpecificBox> {
        self.child(DAMR)
            .and_then(|data| AmrDecoderSpecificBox::parse_body(data).ok())
    }

    /// The AMR-WB+ decoder-specific box (`dawp`), when present (§6.10).
    #[must_use]
    pub fn amrwb_plus(&self) -> Option<AmrwbPlusDecoderSpecificBox> {
        self.child(DAWP)
            .and_then(|data| AmrwbPlusDecoderSpecificBox::parse_body(data).ok())
    }

    fn child(&self, fourcc: [u8; BOX_TYPE_SIZE]) -> Option<&[u8]> {
        self.entry
            .config_boxes
            .iter()
            .find(|b| b.box_type == fourcc)
            .map(|b| b.data.as_slice())
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let item = parse_box(bytes)?.0;
        let mut codec_type = [0u8; BOX_TYPE_SIZE];
        codec_type.copy_from_slice(&item.four_cc());
        if !matches!(&codec_type, b"samr" | b"sawb" | b"sawp" | b"sevs") {
            return Err(Error::UnexpectedBox {
                expected: "a 3GPP audio sample entry",
            });
        }
        Ok(Self {
            codec_type,
            entry: parse_audio_sample_entry(bytes, "3GPP audio sample entry")?,
        })
    }
}

impl Serialize for ThreeGppAudioEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(
            &self.entry.quicktime,
            &self.entry.config_boxes,
            &self.entry.trailing,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: &self.codec_type,
                reserved: self.entry.reserved,
                entry_version: self.entry.entry_version,
                reserved_1: self.entry.reserved_1,
                data_reference_index: self.entry.data_reference_index,
                channelcount: self.entry.channelcount,
                samplesize: self.entry.samplesize,
                compression_id_and_packet_size: self.entry.compression_id_and_packet_size,
                samplerate: self.entry.samplerate,
                quicktime: &self.entry.quicktime,
                config_boxes: &self.entry.config_boxes,
                trailing: &self.entry.trailing,
            },
        )
    }
}

/// The H.263 video sample entry (`s263`) — §6.6.
///
/// The standard 78-byte `VisualSampleEntry` fixed part; the H.263
/// decoder-specific box (`d263`) and an optional `bitr` follow as children.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct H263SampleEntry {
    visual: VisualSampleEntryFields,
    children: Vec<OpaqueBox>,
}

impl H263SampleEntry {
    /// The entry's four-CC.
    pub const FOURCC: [u8; BOX_TYPE_SIZE] = S263;

    /// Build an entry with the default fixed fields and no children.
    #[must_use]
    pub fn new() -> Self {
        Self {
            visual: VisualSampleEntryFields::default(),
            children: Vec::new(),
        }
    }

    /// Set the maximum coded dimensions.
    #[must_use]
    pub fn with_dimensions(mut self, width: u16, height: u16) -> Self {
        self.visual.width = width;
        self.visual.height = height;
        self
    }

    /// The fixed sample description fields.
    #[must_use]
    pub const fn visual(&self) -> &VisualSampleEntryFields {
        &self.visual
    }

    /// Every child box after the fixed part, verbatim.
    #[must_use]
    pub fn children(&self) -> &[OpaqueBox] {
        &self.children
    }

    /// The H.263 decoder-specific box (`d263`), when present (§6.8).
    #[must_use]
    pub fn decoder_specific(&self) -> Option<H263DecoderSpecificBox> {
        self.children
            .iter()
            .find(|b| b.box_type == D263)
            .and_then(|b| H263DecoderSpecificBox::parse_body(&b.data).ok())
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let item = parse_box(bytes)?.0;
        if item.four_cc() != S263 {
            return Err(Error::UnexpectedBox { expected: "s263" });
        }
        let mut visual = VisualSampleEntryFields::parse_body(bytes, "s263")?;
        let region = VisualSampleEntryFields::children_region(bytes, "s263")?;
        let (children, trailing) = crate::box_support::capture_children(region).into_parts();
        visual.trailing = trailing.into_boxed_slice();
        Ok(Self { visual, children })
    }
}

impl Default for H263SampleEntry {
    fn default() -> Self {
        Self::new()
    }
}

impl Serialize for H263SampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE
            + VisualSampleEntryFields::serialized_len()
            + self.visual.trailing.len();
        for child in &self.children {
            n += child.serialized_len();
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &S263, need)?;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        for child in &self.children {
            c += child.serialize_into(&mut buf[c..])?;
        }
        let end = c + self.visual.trailing.len();
        need_out(buf, end)?;
        buf[c..end].copy_from_slice(&self.visual.trailing);
        Ok(end)
    }
}

/// A 3GPP metadata sample entry that adds no field to the `SampleEntry` base
/// (`3glo`, `3gor`) — §6.12, §6.13.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ThreeGppSimpleEntry {
    codec_type: [u8; BOX_TYPE_SIZE],
    reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    data_reference_index: u16,
    children: Vec<OpaqueBox>,
    trailing: Vec<u8>,
}

impl ThreeGppSimpleEntry {
    /// Build an entry for `3glo` or `3gor`.
    ///
    /// # Errors
    /// `codec_type` is neither code.
    pub fn new(codec_type: [u8; BOX_TYPE_SIZE]) -> Result<Self> {
        if codec_type != LOCATION_ENTRY && codec_type != ORIENTATION_ENTRY {
            return Err(Error::UnexpectedBox {
                expected: "3glo or 3gor",
            });
        }
        Ok(Self {
            codec_type,
            reserved: [0; SAMPLE_ENTRY_RESERVED_LEN],
            data_reference_index: 1,
            children: Vec::new(),
            trailing: Vec::new(),
        })
    }

    /// The entry's four-CC.
    #[must_use]
    pub const fn codec_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.codec_type
    }

    /// The `data_reference_index`.
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// Every child box after the fixed part, verbatim.
    #[must_use]
    pub fn children(&self) -> &[OpaqueBox] {
        &self.children
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let item = parse_box(bytes)?.0;
        let mut codec_type = [0u8; BOX_TYPE_SIZE];
        codec_type.copy_from_slice(&item.four_cc());
        if codec_type != LOCATION_ENTRY && codec_type != ORIENTATION_ENTRY {
            return Err(Error::UnexpectedBox {
                expected: "3glo or 3gor",
            });
        }
        let body = crate::box_types::box_body(bytes, BASE_ENTRY_FIXED, "3GPP metadata entry")?;
        let mut reserved = [0u8; SAMPLE_ENTRY_RESERVED_LEN];
        reserved.copy_from_slice(&body[..SAMPLE_ENTRY_RESERVED_LEN]);
        let (children, trailing) =
            crate::box_support::capture_children(&body[BASE_ENTRY_FIXED..]).into_parts();
        Ok(Self {
            codec_type,
            reserved,
            data_reference_index: u16_at(body, SAMPLE_ENTRY_RESERVED_LEN),
            children,
            trailing,
        })
    }
}

impl Serialize for ThreeGppSimpleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + BASE_ENTRY_FIXED + self.trailing.len();
        for child in &self.children {
            n += child.serialized_len();
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &self.codec_type, need)?;
        buf[c..c + SAMPLE_ENTRY_RESERVED_LEN].copy_from_slice(&self.reserved);
        c += SAMPLE_ENTRY_RESERVED_LEN;
        buf[c..c + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.data_reference_index.to_be_bytes());
        c += core::mem::size_of::<u16>();
        for child in &self.children {
            c += child.serialize_into(&mut buf[c..])?;
        }
        buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(c + self.trailing.len())
    }
}

/// The CVO sample entry (`3gvo`) — §6.11.
///
/// Adds one byte, `granularity`, to the `SampleEntry` base.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CvoSampleEntry {
    reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    data_reference_index: u16,
    granularity: u8,
    children: Vec<OpaqueBox>,
    trailing: Vec<u8>,
}

/// Four-CC of the CVO sample entry (`3gvo`).
pub const CVO_ENTRY: [u8; BOX_TYPE_SIZE] = *b"3gvo";
/// Bytes of the CVO entry before its children: base + `granularity`.
const CVO_FIXED: usize = BASE_ENTRY_FIXED + 1;

impl CvoSampleEntry {
    /// Build an entry with `granularity` (2 for CVO, 6 for high granularity).
    #[must_use]
    pub fn new(granularity: u8) -> Self {
        Self {
            reserved: [0; SAMPLE_ENTRY_RESERVED_LEN],
            data_reference_index: 1,
            granularity,
            children: Vec::new(),
            trailing: Vec::new(),
        }
    }

    /// The `granularity` byte.
    #[must_use]
    pub const fn granularity(&self) -> u8 {
        self.granularity
    }

    /// The `data_reference_index`.
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let item = parse_box(bytes)?.0;
        if item.four_cc() != CVO_ENTRY {
            return Err(Error::UnexpectedBox { expected: "3gvo" });
        }
        let body = crate::box_types::box_body(bytes, CVO_FIXED, "3gvo")?;
        let mut reserved = [0u8; SAMPLE_ENTRY_RESERVED_LEN];
        reserved.copy_from_slice(&body[..SAMPLE_ENTRY_RESERVED_LEN]);
        let (children, trailing) =
            crate::box_support::capture_children(&body[CVO_FIXED..]).into_parts();
        Ok(Self {
            reserved,
            data_reference_index: u16_at(body, SAMPLE_ENTRY_RESERVED_LEN),
            granularity: body[CVO_FIXED - 1],
            children,
            trailing,
        })
    }
}

impl Serialize for CvoSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + CVO_FIXED + self.trailing.len();
        for child in &self.children {
            n += child.serialized_len();
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &CVO_ENTRY, need)?;
        buf[c..c + SAMPLE_ENTRY_RESERVED_LEN].copy_from_slice(&self.reserved);
        c += SAMPLE_ENTRY_RESERVED_LEN;
        buf[c..c + core::mem::size_of::<u16>()]
            .copy_from_slice(&self.data_reference_index.to_be_bytes());
        c += core::mem::size_of::<u16>();
        buf[c] = self.granularity;
        c += 1;
        for child in &self.children {
            c += child.serialize_into(&mut buf[c..])?;
        }
        buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(c + self.trailing.len())
    }
}

/// The AMR decoder-specific box (`damr`) — §6.7.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AmrDecoderSpecificBox {
    vendor: [u8; BOX_TYPE_SIZE],
    decoder_version: u8,
    mode_set: u16,
    mode_change_period: u8,
    frames_per_sample: u8,
    trailing: Vec<u8>,
}

/// Bytes of the AMR record: vendor + decoder_version + mode_set +
/// mode_change_period + frames_per_sample.
const AMR_RECORD_LEN: usize = BOX_TYPE_SIZE + 1 + 2 + 1 + 1;
const AMR_DECODER_VERSION_OFFSET: usize = BOX_TYPE_SIZE;
const AMR_MODE_SET_OFFSET: usize = AMR_DECODER_VERSION_OFFSET + 1;
const AMR_MODE_CHANGE_OFFSET: usize = AMR_MODE_SET_OFFSET + 2;
const AMR_FRAMES_PER_SAMPLE_OFFSET: usize = AMR_MODE_CHANGE_OFFSET + 1;

impl AmrDecoderSpecificBox {
    /// Build one from its fields.
    #[must_use]
    pub fn new(
        vendor: [u8; BOX_TYPE_SIZE],
        decoder_version: u8,
        mode_set: u16,
        mode_change_period: u8,
        frames_per_sample: u8,
    ) -> Self {
        Self {
            vendor,
            decoder_version,
            mode_set,
            mode_change_period,
            frames_per_sample,
            trailing: Vec::new(),
        }
    }

    /// The manufacturer four-CC.
    #[must_use]
    pub const fn vendor(&self) -> [u8; BOX_TYPE_SIZE] {
        self.vendor
    }

    /// The vendor's decoder version.
    #[must_use]
    pub const fn decoder_version(&self) -> u8 {
        self.decoder_version
    }

    /// The active AMR modes, one bit per frame type.
    #[must_use]
    pub const fn mode_set(&self) -> u16 {
        self.mode_set
    }

    /// The mode-change period (0: no restriction).
    #[must_use]
    pub const fn mode_change_period(&self) -> u8 {
        self.mode_change_period
    }

    /// Frames per sample.
    #[must_use]
    pub const fn frames_per_sample(&self) -> u8 {
        self.frames_per_sample
    }

    /// Bytes after the record, kept verbatim.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Parse the box payload (its header excluded).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, AMR_RECORD_LEN, "damr")?;
        let mut vendor = [0u8; BOX_TYPE_SIZE];
        vendor.copy_from_slice(&body[..BOX_TYPE_SIZE]);
        Ok(Self {
            vendor,
            decoder_version: body[AMR_DECODER_VERSION_OFFSET],
            mode_set: be_u16(body, AMR_MODE_SET_OFFSET, "damr mode_set")?,
            mode_change_period: body[AMR_MODE_CHANGE_OFFSET],
            frames_per_sample: body[AMR_FRAMES_PER_SAMPLE_OFFSET],
            trailing: body[AMR_RECORD_LEN..].to_vec(),
        })
    }

    /// Parse the whole box.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != DAMR {
            return Err(Error::UnexpectedBox { expected: "damr" });
        }
        Self::parse_body(item.body)
    }
}

impl<'a> Parse<'a> for AmrDecoderSpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for AmrDecoderSpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + AMR_RECORD_LEN + self.trailing.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &DAMR, need)?;
        buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.vendor);
        c += BOX_TYPE_SIZE;
        buf[c] = self.decoder_version;
        c += 1;
        buf[c..c + core::mem::size_of::<u16>()].copy_from_slice(&self.mode_set.to_be_bytes());
        c += core::mem::size_of::<u16>();
        buf[c] = self.mode_change_period;
        c += 1;
        buf[c] = self.frames_per_sample;
        c += 1;
        buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(c + self.trailing.len())
    }
}

/// The H.263 decoder-specific box (`d263`) — §6.8.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct H263DecoderSpecificBox {
    vendor: [u8; BOX_TYPE_SIZE],
    decoder_version: u8,
    h263_level: u8,
    h263_profile: u8,
    children: Vec<OpaqueBox>,
    trailing: Vec<u8>,
}

/// Bytes of the H.263 record: vendor + decoder_version + level + profile.
const H263_RECORD_LEN: usize = BOX_TYPE_SIZE + 3;
const H263_DECODER_VERSION_OFFSET: usize = BOX_TYPE_SIZE;
const H263_LEVEL_OFFSET: usize = H263_DECODER_VERSION_OFFSET + 1;
const H263_PROFILE_OFFSET: usize = H263_LEVEL_OFFSET + 1;

impl H263DecoderSpecificBox {
    /// Build one from its fields.
    #[must_use]
    pub fn new(
        vendor: [u8; BOX_TYPE_SIZE],
        decoder_version: u8,
        h263_level: u8,
        h263_profile: u8,
    ) -> Self {
        Self {
            vendor,
            decoder_version,
            h263_level,
            h263_profile,
            children: Vec::new(),
            trailing: Vec::new(),
        }
    }

    /// The manufacturer four-CC.
    #[must_use]
    pub const fn vendor(&self) -> [u8; BOX_TYPE_SIZE] {
        self.vendor
    }

    /// The vendor's decoder version.
    #[must_use]
    pub const fn decoder_version(&self) -> u8 {
        self.decoder_version
    }

    /// The H.263 level.
    #[must_use]
    pub const fn h263_level(&self) -> u8 {
        self.h263_level
    }

    /// The H.263 profile.
    #[must_use]
    pub const fn h263_profile(&self) -> u8 {
        self.h263_profile
    }

    /// The optional `bitr` box and any other child, verbatim.
    #[must_use]
    pub fn children(&self) -> &[OpaqueBox] {
        &self.children
    }

    /// Bytes after the last complete child.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Parse the box payload (its header excluded).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, H263_RECORD_LEN, "d263")?;
        let mut vendor = [0u8; BOX_TYPE_SIZE];
        vendor.copy_from_slice(&body[..BOX_TYPE_SIZE]);
        let (children, trailing) =
            crate::box_support::capture_children(&body[H263_RECORD_LEN..]).into_parts();
        Ok(Self {
            vendor,
            decoder_version: body[H263_DECODER_VERSION_OFFSET],
            h263_level: body[H263_LEVEL_OFFSET],
            h263_profile: body[H263_PROFILE_OFFSET],
            children,
            trailing,
        })
    }

    /// Parse the whole box.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != D263 {
            return Err(Error::UnexpectedBox { expected: "d263" });
        }
        Self::parse_body(item.body)
    }
}

impl<'a> Parse<'a> for H263DecoderSpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for H263DecoderSpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + H263_RECORD_LEN + self.trailing.len();
        for child in &self.children {
            n += child.serialized_len();
        }
        box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &D263, need)?;
        buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.vendor);
        c += BOX_TYPE_SIZE;
        buf[c] = self.decoder_version;
        c += 1;
        buf[c] = self.h263_level;
        c += 1;
        buf[c] = self.h263_profile;
        c += 1;
        for child in &self.children {
            c += child.serialize_into(&mut buf[c..])?;
        }
        buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(c + self.trailing.len())
    }
}

/// The AMR-WB+ decoder-specific box (`dawp`) — §6.10.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AmrwbPlusDecoderSpecificBox {
    vendor: [u8; BOX_TYPE_SIZE],
    decoder_version: u8,
    trailing: Vec<u8>,
}

/// Bytes of the AMR-WB+ record: vendor + decoder_version.
const DAWP_RECORD_LEN: usize = BOX_TYPE_SIZE + 1;

impl AmrwbPlusDecoderSpecificBox {
    /// Build one from its fields.
    #[must_use]
    pub fn new(vendor: [u8; BOX_TYPE_SIZE], decoder_version: u8) -> Self {
        Self {
            vendor,
            decoder_version,
            trailing: Vec::new(),
        }
    }

    /// The manufacturer four-CC.
    #[must_use]
    pub const fn vendor(&self) -> [u8; BOX_TYPE_SIZE] {
        self.vendor
    }

    /// The vendor's decoder version.
    #[must_use]
    pub const fn decoder_version(&self) -> u8 {
        self.decoder_version
    }

    /// Bytes after the record, kept verbatim.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Parse the box payload (its header excluded).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, DAWP_RECORD_LEN, "dawp")?;
        let mut vendor = [0u8; BOX_TYPE_SIZE];
        vendor.copy_from_slice(&body[..BOX_TYPE_SIZE]);
        Ok(Self {
            vendor,
            decoder_version: body[BOX_TYPE_SIZE],
            trailing: body[DAWP_RECORD_LEN..].to_vec(),
        })
    }

    /// Parse the whole box.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != DAWP {
            return Err(Error::UnexpectedBox { expected: "dawp" });
        }
        Self::parse_body(item.body)
    }
}

impl<'a> Parse<'a> for AmrwbPlusDecoderSpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for AmrwbPlusDecoderSpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + DAWP_RECORD_LEN + self.trailing.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &DAWP, need)?;
        buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.vendor);
        c += BOX_TYPE_SIZE;
        buf[c] = self.decoder_version;
        c += 1;
        buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(c + self.trailing.len())
    }
}

/// Four-CC of the track fragment adjustment box (`tfad`, §13.3).
pub const TFAD: [u8; BOX_TYPE_SIZE] = *b"tfad";
/// Four-CC of the track fragment media adjustment box (`tfma`, §13.3).
pub const TFMA: [u8; BOX_TYPE_SIZE] = *b"tfma";

/// The track fragment adjustment box (`tfad`) — §13.3.
///
/// "aligned(8) class TrackFragmentAdjustmentBox extends Box('tfad') { }": it
/// is a container marker with no fields of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TrackFragmentAdjustmentBox;

impl TrackFragmentAdjustmentBox {
    /// Parse the whole box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != TFAD {
            return Err(Error::UnexpectedBox { expected: "tfad" });
        }
        Ok(Self)
    }

    /// Parse the payload after the header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, 0, "tfad")?;
        Ok(Self)
    }
}

impl<'a> Parse<'a> for TrackFragmentAdjustmentBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for TrackFragmentAdjustmentBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        write_box_header(buf, &TFAD, need)
    }
}

/// One entry of a [`TrackFragmentMediaAdjustmentBox`] — §13.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TrackFragmentMediaAdjustment {
    segment_duration: u64,
    media_time: i64,
    media_rate_integer: i16,
    media_rate_fraction: i16,
}

impl TrackFragmentMediaAdjustment {
    /// Build one entry.
    #[must_use]
    pub const fn new(
        segment_duration: u64,
        media_time: i64,
        media_rate_integer: i16,
        media_rate_fraction: i16,
    ) -> Self {
        Self {
            segment_duration,
            media_time,
            media_rate_integer,
            media_rate_fraction,
        }
    }

    /// `segment_duration`, in the movie timescale.
    #[must_use]
    pub const fn segment_duration(self) -> u64 {
        self.segment_duration
    }

    /// `media_time` in the media timescale; `-1` is an empty edit.
    #[must_use]
    pub const fn media_time(self) -> i64 {
        self.media_time
    }

    /// `media_rate_integer` (0 is a dwell, otherwise 1).
    #[must_use]
    pub const fn media_rate_integer(self) -> i16 {
        self.media_rate_integer
    }

    /// `media_rate_fraction`, which the syntax fixes at 0.
    #[must_use]
    pub const fn media_rate_fraction(self) -> i16 {
        self.media_rate_fraction
    }
}

/// The track fragment media adjustment box (`tfma`) — §13.3.
///
/// Version 1 widens the duration and media time to 64 bits.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TrackFragmentMediaAdjustmentBox {
    version: u8,
    flags: u32,
    entries: Vec<TrackFragmentMediaAdjustment>,
}

/// Version-0 entry: `u32` duration + `i32` media time + two `i16` rates.
const TFMA_V0_ENTRY_LEN: usize = 12;
/// Version-1 entry: `u64` duration + `i64` media time + two `i16` rates.
const TFMA_V1_ENTRY_LEN: usize = 20;

impl TrackFragmentMediaAdjustmentBox {
    /// Build one entry list at the given version.
    #[must_use]
    pub fn new(version: u8, entries: Vec<TrackFragmentMediaAdjustment>) -> Self {
        Self {
            version,
            flags: 0,
            entries,
        }
    }

    /// The FullBox version (0 or 1).
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The adjustment entries.
    #[must_use]
    pub fn entries(&self) -> &[TrackFragmentMediaAdjustment] {
        &self.entries
    }

    /// Parse the whole box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != TFMA {
            return Err(Error::UnexpectedBox { expected: "tfma" });
        }
        Self::parse_body(item.body)
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "tfma",
        )?;
        let version = body[0];
        let flags = broadcast_core::wire::u24_at(body, 1);
        let count = u32_at(body, FULLBOX_EXTRA_SIZE) as usize;
        let entry_len = if version == 1 {
            TFMA_V1_ENTRY_LEN
        } else {
            TFMA_V0_ENTRY_LEN
        };
        let mut cursor = FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        let remaining = body.len().saturating_sub(cursor);
        crate::box_support::check_entry_count(remaining, entry_len, count, "tfma.entry_count")?;
        let mut entries = Vec::with_capacity(crate::box_support::bounded_entry_count(
            remaining, entry_len, count,
        ));
        for _ in 0..count {
            let (segment_duration, media_time) = if version == 1 {
                let duration = u64::from_be_bytes(
                    body.get(cursor..cursor + U64_LEN)
                        .and_then(|b| b.try_into().ok())
                        .ok_or(Error::BufferTooShort(
                            broadcast_core::error::BufferTooShort {
                                need: cursor + U64_LEN,
                                have: body.len(),
                                what: "tfma segment_duration",
                            },
                        ))?,
                );
                cursor += U64_LEN;
                let time = i64::from_be_bytes(
                    body.get(cursor..cursor + U64_LEN)
                        .and_then(|b| b.try_into().ok())
                        .ok_or(Error::BufferTooShort(
                            broadcast_core::error::BufferTooShort {
                                need: cursor + U64_LEN,
                                have: body.len(),
                                what: "tfma media_time",
                            },
                        ))?,
                );
                cursor += U64_LEN;
                (duration, time)
            } else {
                let duration = u64::from(be_u32(body, cursor, "tfma segment_duration")?);
                cursor += U32_LEN;
                let time = i64::from(be_u32(body, cursor, "tfma media_time")? as i32);
                cursor += U32_LEN;
                (duration, time)
            };
            let rate = be_u16(body, cursor, "tfma media_rate_integer")? as i16;
            cursor += U16_LEN;
            let fraction = be_u16(body, cursor, "tfma media_rate_fraction")? as i16;
            cursor += U16_LEN;
            entries.push(TrackFragmentMediaAdjustment {
                segment_duration,
                media_time,
                media_rate_integer: rate,
                media_rate_fraction: fraction,
            });
        }
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl Serialize for TrackFragmentMediaAdjustmentBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let entry_len = if self.version == 1 {
            TFMA_V1_ENTRY_LEN
        } else {
            TFMA_V0_ENTRY_LEN
        };
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + core::mem::size_of::<u32>()
                + entry_len * self.entries.len(),
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &TFMA, need, self.version, self.flags, false)?;
        let count = broadcast_core::len::fit_u32(self.entries.len(), "tfma entry_count")?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&count.to_be_bytes());
        c += core::mem::size_of::<u32>();
        for entry in &self.entries {
            if self.version == 1 {
                buf[c..c + U64_LEN].copy_from_slice(&entry.segment_duration.to_be_bytes());
                c += U64_LEN;
                buf[c..c + U64_LEN].copy_from_slice(&entry.media_time.to_be_bytes());
                c += U64_LEN;
            } else {
                let duration = u32::try_from(entry.segment_duration).map_err(|_| {
                    Error::InvalidInput("tfma version 0 segment_duration does not fit in 32 bits")
                })?;
                buf[c..c + U32_LEN].copy_from_slice(&duration.to_be_bytes());
                c += U32_LEN;
                let time = (entry.media_time as i32).to_be_bytes();
                buf[c..c + U32_LEN].copy_from_slice(&time);
                c += U32_LEN;
            }
            buf[c..c + U16_LEN].copy_from_slice(&entry.media_rate_integer.to_be_bytes());
            c += U16_LEN;
            buf[c..c + U16_LEN].copy_from_slice(&entry.media_rate_fraction.to_be_bytes());
            c += U16_LEN;
        }
        Ok(c)
    }
}

/// Four-CC of the Service Protection Description box (`spdb`, §12.2.2).
pub const SPDB: [u8; BOX_TYPE_SIZE] = *b"spdb";

/// The Service Protection Description box (`spdb`) — §12.2.2.
///
/// A version-0 FullBox whose payload is the XML-encoded Service Protection
/// Description metadata fragment, carried opaque.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ServiceProtectionDescriptionBox {
    version: u8,
    flags: u32,
    security_description: Vec<u8>,
}

impl ServiceProtectionDescriptionBox {
    /// Build one from the XML fragment's bytes.
    #[must_use]
    pub fn new(security_description: Vec<u8>) -> Self {
        Self {
            version: 0,
            flags: 0,
            security_description,
        }
    }

    /// The FullBox version, which the clause fixes at 0.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The XML Service Protection Description fragment, verbatim.
    #[must_use]
    pub fn security_description(&self) -> &[u8] {
        &self.security_description
    }

    /// Parse the whole box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != SPDB {
            return Err(Error::UnexpectedBox { expected: "spdb" });
        }
        Self::parse_body(item.body)
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "spdb")?;
        Ok(Self {
            version: body[0],
            flags: broadcast_core::wire::u24_at(body, 1),
            security_description: body[FULLBOX_EXTRA_SIZE..].to_vec(),
        })
    }
}

impl<'a> Parse<'a> for ServiceProtectionDescriptionBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for ServiceProtectionDescriptionBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.security_description.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let c = write_full_box_header(buf, &SPDB, need, self.version, self.flags, false)?;
        buf[c..c + self.security_description.len()].copy_from_slice(&self.security_description);
        Ok(c + self.security_description.len())
    }
}

/// Four-CC of the application data unit info box (`3gau`, §11.2).
pub const APPLICATION_DATA_UNIT_ENTRY: [u8; BOX_TYPE_SIZE] = *b"3gau";
/// One `3gau` entry: `numbytes`(32) + `decoder`(64) + `timestampoffset`(32).
const GAU_ENTRY_LEN: usize = 4 + 8 + 4;

/// One entry of an [`ApplicationDataUnitInfoBox`] — §11.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ApplicationDataUnitInfo {
    num_bytes: u32,
    decoder: u64,
    timestamp_offset: u32,
}

impl ApplicationDataUnitInfo {
    /// Build one entry.
    #[must_use]
    pub const fn new(num_bytes: u32, decoder: u64, timestamp_offset: u32) -> Self {
        Self {
            num_bytes,
            decoder,
            timestamp_offset,
        }
    }

    /// `numbytes`.
    #[must_use]
    pub const fn num_bytes(self) -> u32 {
        self.num_bytes
    }

    /// `decoder`.
    #[must_use]
    pub const fn decoder(self) -> u64 {
        self.decoder
    }

    /// `timestampoffset`.
    #[must_use]
    pub const fn timestamp_offset(self) -> u32 {
        self.timestamp_offset
    }
}

/// The 3GPP application data unit info box (`3gau`) — §11.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ApplicationDataUnitInfoBox {
    entries: Vec<ApplicationDataUnitInfo>,
}

impl ApplicationDataUnitInfoBox {
    /// Build one from its entries.
    #[must_use]
    pub fn new(entries: Vec<ApplicationDataUnitInfo>) -> Self {
        Self { entries }
    }

    /// The entries.
    #[must_use]
    pub fn entries(&self) -> &[ApplicationDataUnitInfo] {
        &self.entries
    }

    /// Parse the whole box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != APPLICATION_DATA_UNIT_ENTRY {
            return Err(Error::UnexpectedBox { expected: "3gau" });
        }
        Self::parse_body(item.body)
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, core::mem::size_of::<u16>(), "3gau")?;
        let count = usize::from(u16_at(body, 0));
        let remaining = body.len().saturating_sub(core::mem::size_of::<u16>());
        crate::box_support::check_entry_count(remaining, GAU_ENTRY_LEN, count, "3gau.entrycount")?;
        let mut entries = Vec::with_capacity(crate::box_support::bounded_entry_count(
            remaining,
            GAU_ENTRY_LEN,
            count,
        ));
        let mut cursor = core::mem::size_of::<u16>();
        for _ in 0..count {
            let num_bytes = be_u32(body, cursor, "3gau numbytes")?;
            cursor += U32_LEN;
            let decoder = u64::from_be_bytes(
                body.get(cursor..cursor + U64_LEN)
                    .and_then(|b| b.try_into().ok())
                    .ok_or(Error::BufferTooShort(
                        broadcast_core::error::BufferTooShort {
                            need: cursor + U64_LEN,
                            have: body.len(),
                            what: "3gau decoder",
                        },
                    ))?,
            );
            cursor += U64_LEN;
            let timestamp_offset = be_u32(body, cursor, "3gau timestampoffset")?;
            cursor += U32_LEN;
            entries.push(ApplicationDataUnitInfo {
                num_bytes,
                decoder,
                timestamp_offset,
            });
        }
        Ok(Self { entries })
    }
}

impl<'a> Parse<'a> for ApplicationDataUnitInfoBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for ApplicationDataUnitInfoBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE + core::mem::size_of::<u16>() + GAU_ENTRY_LEN * self.entries.len(),
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &APPLICATION_DATA_UNIT_ENTRY, need)?;
        let count = u16::try_from(self.entries.len())
            .map_err(|_| Error::InvalidInput("3gau entry count does not fit in 16 bits"))?;
        buf[c..c + core::mem::size_of::<u16>()].copy_from_slice(&count.to_be_bytes());
        c += core::mem::size_of::<u16>();
        for entry in &self.entries {
            buf[c..c + U32_LEN].copy_from_slice(&entry.num_bytes.to_be_bytes());
            c += U32_LEN;
            buf[c..c + U64_LEN].copy_from_slice(&entry.decoder.to_be_bytes());
            c += U64_LEN;
            buf[c..c + U32_LEN].copy_from_slice(&entry.timestamp_offset.to_be_bytes());
            c += U32_LEN;
        }
        Ok(c)
    }
}

/// Four-CC of the media classification box (`clsf`, §8.2 table 8.8).
pub const CLSF: [u8; BOX_TYPE_SIZE] = *b"clsf";
/// Four-CC of the collection name box (`coll`, §8.2 table 8.12a).
pub const COLL: [u8; BOX_TYPE_SIZE] = *b"coll";
/// Four-CC of the media keywords box (`kywd`, §8.2 table 8.9).
pub const KYWD: [u8; BOX_TYPE_SIZE] = *b"kywd";
/// Four-CC of the media rating box (`rtng`, §8.2 table 8.7).
pub const RTNG: [u8; BOX_TYPE_SIZE] = *b"rtng";
/// Four-CC of the user-rating box (`urat`, §8.2 table 8.12b).
pub const URAT: [u8; BOX_TYPE_SIZE] = *b"urat";
/// Four-CC of the thumbnail box (`thmb`, §8.2 table 8.12c).
pub const THMB: [u8; BOX_TYPE_SIZE] = *b"thmb";
/// Four-CC of the orientation information box (`orie`, §8.2 table 8.12d).
pub const ORIE: [u8; BOX_TYPE_SIZE] = *b"orie";

/// The media classification box (`clsf`) — §8.2 table 8.8.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MediaClassificationBox {
    prefix: LanguagePrefix,
    classification_entity: [u8; BOX_TYPE_SIZE],
    classification_table: u16,
    classification_info: Vec<u8>,
}

impl MediaClassificationBox {
    /// Build one.
    #[must_use]
    pub fn new(
        classification_entity: [u8; BOX_TYPE_SIZE],
        classification_table: u16,
        classification_info: Vec<u8>,
    ) -> Self {
        Self {
            prefix: LanguagePrefix {
                version: 0,
                flags: 0,
                pad: false,
                language: 0,
            },
            classification_entity,
            classification_table,
            classification_info,
        }
    }

    /// The classification entity four-CC.
    #[must_use]
    pub const fn classification_entity(&self) -> [u8; BOX_TYPE_SIZE] {
        self.classification_entity
    }

    /// The classification table index.
    #[must_use]
    pub const fn classification_table(&self) -> u16 {
        self.classification_table
    }

    /// The null-terminated classification text, terminator excluded.
    #[must_use]
    pub fn classification_info(&self) -> &[u8] {
        &self.classification_info
    }

    /// The packed language, as the 15-bit value.
    #[must_use]
    pub const fn language(&self) -> u16 {
        self.prefix.language
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (prefix, rest) = LanguagePrefix::parse(body, "clsf")?;
        need_in(rest, BOX_TYPE_SIZE + U16_LEN, "clsf")?;
        let mut entity = [0u8; BOX_TYPE_SIZE];
        entity.copy_from_slice(&rest[..BOX_TYPE_SIZE]);
        Ok(Self {
            prefix,
            classification_entity: entity,
            classification_table: be_u16(rest, BOX_TYPE_SIZE, "clsf table")?,
            classification_info: text_to_end(&rest[BOX_TYPE_SIZE + 2..]),
        })
    }
}

impl Serialize for MediaClassificationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + LanguagePrefix::serialized_len()
                + BOX_TYPE_SIZE
                + U16_LEN
                + self.classification_info.len()
                + 1,
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = self.prefix.write(buf, &CLSF, need)?;
        buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.classification_entity);
        c += BOX_TYPE_SIZE;
        buf[c..c + U16_LEN].copy_from_slice(&self.classification_table.to_be_bytes());
        c += U16_LEN;
        buf[c..c + self.classification_info.len()].copy_from_slice(&self.classification_info);
        c += self.classification_info.len();
        buf[c] = 0;
        Ok(c + 1)
    }
}

/// The collection name box (`coll`) — §8.2 table 8.12a.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CollectionNameBox {
    prefix: LanguagePrefix,
    name: Vec<u8>,
}

impl CollectionNameBox {
    /// Build one.
    #[must_use]
    pub fn new(name: Vec<u8>) -> Self {
        Self {
            prefix: LanguagePrefix {
                version: 0,
                flags: 0,
                pad: false,
                language: 0,
            },
            name,
        }
    }

    /// The null-terminated collection name, terminator excluded.
    #[must_use]
    pub fn collection_name(&self) -> &[u8] {
        &self.name
    }

    /// The packed language, as the 15-bit value.
    #[must_use]
    pub const fn language(&self) -> u16 {
        self.prefix.language
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (prefix, rest) = LanguagePrefix::parse(body, "coll")?;
        Ok(Self {
            prefix,
            name: text_to_end(rest),
        })
    }
}

impl Serialize for CollectionNameBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + LanguagePrefix::serialized_len() + self.name.len() + 1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = self.prefix.write(buf, &COLL, need)?;
        buf[c..c + self.name.len()].copy_from_slice(&self.name);
        c += self.name.len();
        buf[c] = 0;
        Ok(c + 1)
    }
}

/// The media keywords box (`kywd`) — §8.2 table 8.9.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct KeywordsBox {
    prefix: LanguagePrefix,
    keywords: Vec<Vec<u8>>,
}

impl KeywordsBox {
    /// Build one from its keyword texts.
    #[must_use]
    pub fn new(keywords: Vec<Vec<u8>>) -> Self {
        Self {
            prefix: LanguagePrefix {
                version: 0,
                flags: 0,
                pad: false,
                language: 0,
            },
            keywords,
        }
    }

    /// The keyword texts, terminators excluded.
    #[must_use]
    pub fn keywords(&self) -> &[Vec<u8>] {
        &self.keywords
    }

    /// The packed language, as the 15-bit value.
    #[must_use]
    pub const fn language(&self) -> u16 {
        self.prefix.language
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (prefix, rest) = LanguagePrefix::parse(body, "kywd")?;
        need_in(rest, 1, "kywd")?;
        let count = usize::from(rest[0]);
        let mut cursor = 1usize;
        let mut keywords = Vec::with_capacity(crate::box_support::bounded_entry_count(
            rest.len().saturating_sub(1),
            1,
            count,
        ));
        for _ in 0..count {
            let Some(&size) = rest.get(cursor) else {
                return Err(Error::BufferTooShort(
                    broadcast_core::error::BufferTooShort {
                        need: cursor + 1,
                        have: rest.len(),
                        what: "kywd keyword size",
                    },
                ));
            };
            cursor += 1;
            let end = cursor + usize::from(size);
            let Some(slice) = rest.get(cursor..end) else {
                return Err(Error::BufferTooShort(
                    broadcast_core::error::BufferTooShort {
                        need: end,
                        have: rest.len(),
                        what: "kywd keyword",
                    },
                ));
            };
            keywords.push(text_to_end(slice));
            cursor = end;
        }
        Ok(Self { prefix, keywords })
    }
}

impl Serialize for KeywordsBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let entries: usize = self.keywords.iter().map(|k| 1 + k.len() + 1).sum();
        box_len(BOX_HEADER_MIN_SIZE + LanguagePrefix::serialized_len() + 1 + entries)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = self.prefix.write(buf, &KYWD, need)?;
        let count = u8::try_from(self.keywords.len())
            .map_err(|_| Error::InvalidInput("kywd keyword count does not fit in 8 bits"))?;
        buf[c] = count;
        c += 1;
        for keyword in &self.keywords {
            let size = u8::try_from(keyword.len() + 1)
                .map_err(|_| Error::InvalidInput("kywd keyword does not fit in 8 bits"))?;
            buf[c] = size;
            c += 1;
            buf[c..c + keyword.len()].copy_from_slice(keyword);
            c += keyword.len();
            buf[c] = 0;
            c += 1;
        }
        Ok(c)
    }
}

/// The media rating box (`rtng`) — §8.2 table 8.7.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MediaRatingBox {
    prefix: LanguagePrefix,
    rating_entity: [u8; BOX_TYPE_SIZE],
    rating_criteria: [u8; BOX_TYPE_SIZE],
    rating_info: Vec<u8>,
}

impl MediaRatingBox {
    /// Build one.
    #[must_use]
    pub fn new(
        rating_entity: [u8; BOX_TYPE_SIZE],
        rating_criteria: [u8; BOX_TYPE_SIZE],
        rating_info: Vec<u8>,
    ) -> Self {
        Self {
            prefix: LanguagePrefix {
                version: 0,
                flags: 0,
                pad: false,
                language: 0,
            },
            rating_entity,
            rating_criteria,
            rating_info,
        }
    }

    /// The rating entity four-CC.
    #[must_use]
    pub const fn rating_entity(&self) -> [u8; BOX_TYPE_SIZE] {
        self.rating_entity
    }

    /// The rating criteria four-CC.
    #[must_use]
    pub const fn rating_criteria(&self) -> [u8; BOX_TYPE_SIZE] {
        self.rating_criteria
    }

    /// The null-terminated rating text, terminator excluded.
    #[must_use]
    pub fn rating_info(&self) -> &[u8] {
        &self.rating_info
    }

    /// The packed language, as the 15-bit value.
    #[must_use]
    pub const fn language(&self) -> u16 {
        self.prefix.language
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (prefix, rest) = LanguagePrefix::parse(body, "rtng")?;
        need_in(rest, U64_LEN, "rtng")?;
        let mut entity = [0u8; BOX_TYPE_SIZE];
        entity.copy_from_slice(&rest[..BOX_TYPE_SIZE]);
        let mut criteria = [0u8; BOX_TYPE_SIZE];
        criteria.copy_from_slice(&rest[BOX_TYPE_SIZE..U64_LEN]);
        Ok(Self {
            prefix,
            rating_entity: entity,
            rating_criteria: criteria,
            rating_info: text_to_end(&rest[U64_LEN..]),
        })
    }
}

impl Serialize for MediaRatingBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + LanguagePrefix::serialized_len()
                + U64_LEN
                + self.rating_info.len()
                + 1,
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = self.prefix.write(buf, &RTNG, need)?;
        buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.rating_entity);
        c += BOX_TYPE_SIZE;
        buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.rating_criteria);
        c += BOX_TYPE_SIZE;
        buf[c..c + self.rating_info.len()].copy_from_slice(&self.rating_info);
        c += self.rating_info.len();
        buf[c] = 0;
        Ok(c + 1)
    }
}

/// The user-rating box (`urat`) — §8.2 table 8.12b.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UserRatingBox {
    version: u8,
    flags: u32,
    padding: [u8; PAD_LEN],
    star_rating: u8,
}

impl UserRatingBox {
    /// Build one from a star rating (0, or 10..=50 for 1..=5 stars).
    #[must_use]
    pub fn new(star_rating: u8) -> Self {
        Self {
            version: 0,
            flags: 0,
            padding: [0; PAD_LEN],
            star_rating,
        }
    }

    /// The user's star rating byte.
    #[must_use]
    pub const fn star_rating(self) -> u8 {
        self.star_rating
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + 4, "urat")?;
        let mut padding = [0u8; PAD_LEN];
        padding.copy_from_slice(&body[FULLBOX_EXTRA_SIZE..FULLBOX_EXTRA_SIZE + PAD_LEN]);
        Ok(Self {
            version: body[0],
            flags: broadcast_core::wire::u24_at(body, 1),
            padding,
            star_rating: body[FULLBOX_EXTRA_SIZE + PAD_LEN],
        })
    }
}

impl<'a> Parse<'a> for UserRatingBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != URAT {
            return Err(Error::UnexpectedBox { expected: "urat" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for UserRatingBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + PAD_LEN + 1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &URAT, need, self.version, self.flags, false)?;
        buf[c..c + PAD_LEN].copy_from_slice(&self.padding);
        c += PAD_LEN;
        buf[c] = self.star_rating;
        Ok(c + 1)
    }
}

/// The thumbnail box (`thmb`) — §8.2 table 8.12c.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ThumbnailBox {
    version: u8,
    flags: u32,
    format: [u8; BOX_TYPE_SIZE],
    data: Vec<u8>,
}

impl ThumbnailBox {
    /// Build one from a coding-format four-CC and the image bytes.
    #[must_use]
    pub fn new(format: [u8; BOX_TYPE_SIZE], data: Vec<u8>) -> Self {
        Self {
            version: 0,
            flags: 0,
            format,
            data,
        }
    }

    /// The coding-format four-CC (`jpeg`).
    #[must_use]
    pub const fn format(&self) -> [u8; BOX_TYPE_SIZE] {
        self.format
    }

    /// The image data, verbatim.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + BOX_TYPE_SIZE, "thmb")?;
        let mut format = [0u8; BOX_TYPE_SIZE];
        format.copy_from_slice(&body[FULLBOX_EXTRA_SIZE..FULLBOX_EXTRA_SIZE + BOX_TYPE_SIZE]);
        Ok(Self {
            version: body[0],
            flags: broadcast_core::wire::u24_at(body, 1),
            format,
            data: body[FULLBOX_EXTRA_SIZE + BOX_TYPE_SIZE..].to_vec(),
        })
    }
}

impl<'a> Parse<'a> for ThumbnailBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != THMB {
            return Err(Error::UnexpectedBox { expected: "thmb" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for ThumbnailBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + BOX_TYPE_SIZE + self.data.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &THMB, need, self.version, self.flags, false)?;
        buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.format);
        c += BOX_TYPE_SIZE;
        buf[c..c + self.data.len()].copy_from_slice(&self.data);
        Ok(c + self.data.len())
    }
}

/// The orientation information box (`orie`) — §8.2 table 8.12d.
///
/// The table's `Pan_indication` is one bit, so `Pan`, `Rotation` and `Tilt`
/// start mid-byte; the body is 17 bytes and the final seven bits are padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct OrientationInformationBox {
    version: u8,
    flags: u32,
    digital_zoom: u16,
    optical_zoom: u16,
    pan_indication: bool,
    pan: u32,
    rotation: u32,
    tilt: u32,
    padding: u8,
}

/// Bits before the packed fields begin: `digital_zoom` + `optical_zoom`.
const ORIE_PREFIX_BITS: usize = 32;
/// `Pan_indication`(1) + `Pan`(32) + `Rotation`(32) + `Tilt`(32) = 97 bits.
const ORIE_PACKED_BITS: usize = 1 + 32 + 32 + 32;
const ORIE_PAN_BIT: usize = ORIE_PREFIX_BITS + 1;
const ORIE_ROTATION_BIT: usize = ORIE_PAN_BIT + 32;
const ORIE_TILT_BIT: usize = ORIE_ROTATION_BIT + 32;
const ORIE_PADDING_BITS: usize = 7;
/// Whole body length in bytes after the box header.
const ORIE_BODY_LEN: usize =
    ORIE_PREFIX_BITS / BYTE_BITS + (ORIE_PACKED_BITS + ORIE_PADDING_BITS).div_ceil(BYTE_BITS);

fn read_bits(data: &[u8], bit_offset: usize, width: usize) -> u64 {
    let mut value = 0u64;
    for index in 0..width {
        let at = bit_offset + index;
        let bit = data.get(at / BYTE_BITS).map_or(0, |byte| {
            (byte >> (u8::BITS as usize - 1 - (at % BYTE_BITS))) & 1
        });
        value = value << 1 | u64::from(bit);
    }
    value
}

fn write_bits(data: &mut [u8], bit_offset: usize, width: usize, value: u64) {
    for index in 0..width {
        let at = bit_offset + index;
        let bit = u8::try_from((value >> (width - 1 - index)) & 1).unwrap_or(0);
        if let Some(byte) = data.get_mut(at / BYTE_BITS) {
            let mask = 1 << (u8::BITS as usize - 1 - (at % BYTE_BITS));
            *byte = (*byte & !mask) | (bit << (u8::BITS as usize - 1 - (at % BYTE_BITS)));
        }
    }
}

impl OrientationInformationBox {
    /// Build one from the fields; the 7 padding bits are written as zero.
    #[must_use]
    pub fn new(
        digital_zoom: u16,
        optical_zoom: u16,
        pan_indication: bool,
        pan: u32,
        rotation: u32,
        tilt: u32,
    ) -> Self {
        Self {
            version: 0,
            flags: 0,
            digital_zoom,
            optical_zoom,
            pan_indication,
            pan,
            rotation,
            tilt,
            padding: 0,
        }
    }

    /// `Digital_zoom`, fixed-point 8.8.
    #[must_use]
    pub const fn digital_zoom(self) -> u16 {
        self.digital_zoom
    }

    /// `Optical_zoom`, fixed-point 8.8.
    #[must_use]
    pub const fn optical_zoom(self) -> u16 {
        self.optical_zoom
    }

    /// `Pan_indication`: true means "true north", false "magnetic north".
    #[must_use]
    pub const fn pan_indication(self) -> bool {
        self.pan_indication
    }

    /// `Pan`, fixed-point 16.15.
    #[must_use]
    pub const fn pan(self) -> u32 {
        self.pan
    }

    /// `Rotation`, fixed-point 16.16 degrees.
    #[must_use]
    pub const fn rotation(self) -> u32 {
        self.rotation
    }

    /// `Tilt`, fixed-point 16.16 degrees.
    #[must_use]
    pub const fn tilt(self) -> u32 {
        self.tilt
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + ORIE_BODY_LEN, "orie")?;
        let packed = &body[FULLBOX_EXTRA_SIZE..];
        Ok(Self {
            version: body[0],
            flags: broadcast_core::wire::u24_at(body, 1),
            digital_zoom: u16::try_from(read_bits(packed, 0, LOWER_U32_BITS)).unwrap_or(0),
            optical_zoom: u16::try_from(read_bits(packed, LOWER_U32_BITS, LOWER_U32_BITS))
                .unwrap_or(0),
            pan_indication: read_bits(packed, ORIE_PREFIX_BITS, 1) != 0,
            pan: u32::try_from(read_bits(packed, ORIE_PAN_BIT, U32_BITS)).unwrap_or(0),
            rotation: u32::try_from(read_bits(packed, ORIE_ROTATION_BIT, U32_BITS)).unwrap_or(0),
            tilt: u32::try_from(read_bits(packed, ORIE_TILT_BIT, U32_BITS)).unwrap_or(0),
            padding: u8::try_from(read_bits(
                packed,
                ORIE_TILT_BIT + U32_BITS,
                ORIE_PADDING_BITS,
            ))
            .unwrap_or(0),
        })
    }
}

impl<'a> Parse<'a> for OrientationInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != ORIE {
            return Err(Error::UnexpectedBox { expected: "orie" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for OrientationInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + ORIE_BODY_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &ORIE, need, self.version, self.flags, false)?;
        let packed = &mut buf[c..c + ORIE_BODY_LEN];
        write_bits(packed, 0, LOWER_U32_BITS, u64::from(self.digital_zoom));
        write_bits(
            packed,
            LOWER_U32_BITS,
            LOWER_U32_BITS,
            u64::from(self.optical_zoom),
        );
        write_bits(packed, ORIE_PREFIX_BITS, 1, u64::from(self.pan_indication));
        write_bits(packed, ORIE_PAN_BIT, U32_BITS, u64::from(self.pan));
        write_bits(
            packed,
            ORIE_ROTATION_BIT,
            U32_BITS,
            u64::from(self.rotation),
        );
        write_bits(packed, ORIE_TILT_BIT, U32_BITS, u64::from(self.tilt));
        write_bits(
            packed,
            ORIE_TILT_BIT + U32_BITS,
            ORIE_PADDING_BITS,
            u64::from(self.padding),
        );
        c += ORIE_BODY_LEN;
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn bytes<T: Serialize<Error = Error>>(value: &T) -> Vec<u8> {
        let mut out = vec![0u8; value.serialized_len()];
        value.serialize_into(&mut out).expect("serializes");
        out
    }

    #[test]
    fn amr_entry_round_trips_with_its_decoder_box() {
        let mut entry = ThreeGppAudioEntry::new(SAMR, 8000).expect("builds");
        let damr = AmrDecoderSpecificBox::new(*b"VXYZ", 1, 0x81FF, 0, 1);
        entry.entry.config_boxes.push(OpaqueBox {
            box_type: DAMR,
            data: bytes(&damr)[BOX_HEADER_MIN_SIZE..].to_vec(),
            to_end: false,
            largesize: false,
        });
        let out = bytes(&entry);
        let back = ThreeGppAudioEntry::parse_box(&out).expect("parses");
        assert_eq!(back.codec_type(), SAMR);
        assert_eq!(back.channelcount(), 1);
        assert_eq!(back.samplerate(), 8000 << 16);
        assert_eq!(back.amr().expect("damr").mode_set(), 0x81FF);
        assert_eq!(bytes(&back), out);
    }

    #[test]
    fn amrwb_plus_entry_round_trips() {
        let mut entry = ThreeGppAudioEntry::new(SAWP, 48000).expect("builds");
        let dawp = AmrwbPlusDecoderSpecificBox::new(*b"VXYZ", 2);
        entry.entry.config_boxes.push(OpaqueBox {
            box_type: DAWP,
            data: bytes(&dawp)[BOX_HEADER_MIN_SIZE..].to_vec(),
            to_end: false,
            largesize: false,
        });
        let back = ThreeGppAudioEntry::parse_box(&bytes(&entry)).expect("parses");
        assert_eq!(back.amrwb_plus().expect("dawp").decoder_version(), 2);
    }

    #[test]
    fn evs_entry_is_the_same_audio_layout() {
        let entry = ThreeGppAudioEntry::new(SEVS, 48000).expect("builds");
        let back = ThreeGppAudioEntry::parse_box(&bytes(&entry)).expect("parses");
        assert_eq!(back, entry);
    }

    #[test]
    fn h263_entry_round_trips_with_its_decoder_box() {
        let mut entry = H263SampleEntry::new().with_dimensions(176, 144);
        let d263 = H263DecoderSpecificBox::new(*b"VXYZ", 1, 10, 0);
        entry.children.push(OpaqueBox {
            box_type: D263,
            data: bytes(&d263)[BOX_HEADER_MIN_SIZE..].to_vec(),
            to_end: false,
            largesize: false,
        });
        let out = bytes(&entry);
        let back = H263SampleEntry::parse_box(&out).expect("parses");
        assert_eq!(back.visual().width, 176);
        assert_eq!(back.decoder_specific().expect("d263").h263_level(), 10);
        assert_eq!(bytes(&back), out);
    }

    #[test]
    fn cvo_entry_carries_its_granularity() {
        let entry = CvoSampleEntry::new(6);
        let back = CvoSampleEntry::parse_box(&bytes(&entry)).expect("parses");
        assert_eq!(back.granularity(), 6);
    }

    #[test]
    fn metadata_entries_round_trip() {
        for code in [LOCATION_ENTRY, ORIENTATION_ENTRY] {
            let entry = ThreeGppSimpleEntry::new(code).expect("builds");
            let back = ThreeGppSimpleEntry::parse_box(&bytes(&entry)).expect("parses");
            assert_eq!(back.codec_type(), code);
        }
    }

    #[test]
    fn fragment_adjustment_boxes_round_trip() {
        let tfad = TrackFragmentAdjustmentBox;
        assert_eq!(bytes(&tfad).len(), BOX_HEADER_MIN_SIZE);
        let tfma = TrackFragmentMediaAdjustmentBox::new(
            0,
            vec![
                TrackFragmentMediaAdjustment::new(1000, -1, 1, 0),
                TrackFragmentMediaAdjustment::new(2000, 50, 1, 0),
            ],
        );
        let back = TrackFragmentMediaAdjustmentBox::parse_box(&bytes(&tfma)).expect("parses");
        assert_eq!(back.entries()[1].media_time(), 50);
        let tfma = TrackFragmentMediaAdjustmentBox::new(
            1,
            vec![TrackFragmentMediaAdjustment::new(1 << 40, 1 << 40, 0, 0)],
        );
        let back = TrackFragmentMediaAdjustmentBox::parse_box(&bytes(&tfma)).expect("parses");
        assert_eq!(back.entries()[0].segment_duration(), 1 << 40);
    }

    #[test]
    fn protection_description_and_application_data_units_round_trip() {
        let spdb = ServiceProtectionDescriptionBox::new(b"<spd/>".to_vec());
        assert_eq!(
            ServiceProtectionDescriptionBox::parse_box(&bytes(&spdb))
                .expect("parses")
                .security_description(),
            b"<spd/>"
        );
        let gau = ApplicationDataUnitInfoBox::new(vec![ApplicationDataUnitInfo::new(42, 7, 9)]);
        let back = ApplicationDataUnitInfoBox::parse_box(&bytes(&gau)).expect("parses");
        assert_eq!(back.entries()[0].num_bytes(), 42);
    }

    #[test]
    fn user_data_boxes_round_trip() {
        let clsf = MediaClassificationBox::new(*b"BBFC", 1, b"PG".to_vec());
        assert_eq!(
            MediaClassificationBox::parse_body(&bytes(&clsf)[BOX_HEADER_MIN_SIZE..])
                .expect("parses"),
            clsf
        );
        let coll = CollectionNameBox::new(b"Set".to_vec());
        assert_eq!(
            CollectionNameBox::parse_body(&bytes(&coll)[BOX_HEADER_MIN_SIZE..]).expect("parses"),
            coll
        );
        let kywd = KeywordsBox::new(vec![b"one".to_vec(), b"two".to_vec()]);
        let back = KeywordsBox::parse_body(&bytes(&kywd)[BOX_HEADER_MIN_SIZE..]).expect("parses");
        assert_eq!(back.keywords().len(), 2);
        let rtng = MediaRatingBox::new(*b"BBFC", *b"PG13", b"12A".to_vec());
        assert_eq!(
            MediaRatingBox::parse_body(&bytes(&rtng)[BOX_HEADER_MIN_SIZE..]).expect("parses"),
            rtng
        );
        let urat = UserRatingBox::new(50);
        assert_eq!(
            UserRatingBox::parse(&bytes(&urat))
                .expect("parses")
                .star_rating(),
            50
        );
        let thmb = ThumbnailBox::new(*b"jpeg", vec![0xFF, 0xD8]);
        assert_eq!(
            ThumbnailBox::parse(&bytes(&thmb)).expect("parses").data(),
            &[0xFF, 0xD8]
        );
    }

    #[test]
    fn orientation_packs_the_unaligned_fields() {
        let orie =
            OrientationInformationBox::new(0x0180, 0x0100, true, 0x4000_0000, 90 << 16, 5 << 16);
        let body = bytes(&orie);
        assert_eq!(
            body.len(),
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + ORIE_BODY_LEN
        );
        let back = OrientationInformationBox::parse(&body).expect("parses");
        assert_eq!(back, orie);
        assert!(back.pan_indication());
        assert_eq!(back.pan(), 0x4000_0000);
    }
}
