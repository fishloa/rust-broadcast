//! Audio element and rendering signalling — ISO/IEC 14496-12:2026 §12.2.8 to
//! §12.2.14.
//!
//! Transcription: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`.
//!
//! | Box | Type | Clause | Container |
//! |-----|------|--------|-----------|
//! | [`AudioRenderingIndicationBox`] | `ardi` | §12.2.8 | `prsl` |
//! | [`AudioElementBox`] | `aelm` | §12.2.9 | `aesb`, `prsl` |
//! | [`AudioElementDescriptionBox`] | `aedb` | §12.2.10 | `aelm` |
//! | [`AudioElementPosInteractivityPolarBox`] | `aepp` | §12.2.11 | `aedb`, `aesb` |
//! | [`AudioElementProminenceInteractivityBox`] | `aepr` | §12.2.12 | `aedb`, `aesb` |
//! | [`AudioElementSelectionBox`] | `aesb` | §12.2.13 | `prsl` |
//! | [`AudioElementSelectionDescriptionBox`] | `aesd` | §12.2.14 | `aesb` |
//!
//! The element containers keep their children in wire order, so a
//! `kind`/`labl`/`elng` inside an audio element round-trips in place.

use alloc::vec::Vec;

use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::{be_u32, i16_at, u24_at};
use broadcast_core::{Parse, Serialize};

use crate::audio_boxes::LabelBox;
use crate::box_support::{OpaqueBox, take_c_string, write_c_string};
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK, box_len, parse_box,
    write_box_header, write_full_box_header,
};
use crate::error::{Error, Result};
use crate::track_desc::{ExtendedLanguageBox, KindBox};

/// Four-CC of the Audio Rendering Indication Box (`ardi`, §12.2.8).
pub const ARDI: [u8; BOX_TYPE_SIZE] = *b"ardi";
/// Four-CC of the Audio Element Box (`aelm`, §12.2.9).
pub const AELM: [u8; BOX_TYPE_SIZE] = *b"aelm";
/// Four-CC of the Audio Element Description Box (`aedb`, §12.2.10).
pub const AEDB: [u8; BOX_TYPE_SIZE] = *b"aedb";
/// Four-CC of the Audio Element Positioning Interactivity Polar Box (`aepp`,
/// §12.2.11).
pub const AEPP: [u8; BOX_TYPE_SIZE] = *b"aepp";
/// Four-CC of the Audio Element Prominence Interactivity Box (`aepr`,
/// §12.2.12).
pub const AEPR: [u8; BOX_TYPE_SIZE] = *b"aepr";
/// Four-CC of the Audio Element Selection Box (`aesb`, §12.2.13).
pub const AESB: [u8; BOX_TYPE_SIZE] = *b"aesb";
/// Four-CC of the Audio Element Selection Description Box (`aesd`, §12.2.14).
pub const AESD: [u8; BOX_TYPE_SIZE] = *b"aesd";

/// Number of values one `aepp` coordinate triple carries (§12.2.11.2).
const AEPP_TRIPLE: usize = 3;
/// `aepp` flag: azimuth values are present.
const AEPP_AZIMUTH_PRESENT: u32 = 0x000001;
/// `aepp` flag: elevation values are present.
const AEPP_ELEVATION_PRESENT: u32 = 0x000002;
/// `aepp` flag: distance values are present.
const AEPP_DISTANCE_PRESENT: u32 = 0x000004;
/// Number of `aepr` prominence values (§12.2.12.2).
const AEPR_VALUES: usize = 3;
/// `aedb` `isToggleable` mask (the top bit of its flag byte).
const AEDB_TOGGLEABLE: u8 = 0x80;
/// `aedb` `isDefaultEnabled` mask (the bit below it).
const AEDB_DEFAULT_ENABLED: u8 = 0x40;
/// Mask of the six `aedb` reserved bits.
const AEDB_RESERVED_MASK: u8 = 0x3F;
/// Bit position of `isDefaultEnabled` in the `aedb` flag byte.
const AEDB_DEFAULT_ENABLED_SHIFT: u32 = 6;
/// Bit position of `isToggleable` in the `aedb` flag byte.
const AEDB_TOGGLEABLE_SHIFT: u32 = 7;

fn opaque_child(child: crate::box_types::BoxItem<'_>) -> OpaqueBox {
    OpaqueBox {
        box_type: child.four_cc(),
        data: child.payload().to_vec(),
        to_end: child.to_end(),
        largesize: child.has_largesize(),
    }
}

/// A preferred reproduction channel layout hint (§12.2.8.3, Table 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AudioRenderingIndication {
    /// 0: no preference for the reproduction channel layout.
    NoPreference,
    /// 1: stereo.
    Stereo,
    /// 2: two-dimensional (e.g. 5.1).
    TwoDimensional,
    /// 3: three-dimensional.
    ThreeDimensional,
    /// 4: pre-rendered for headphones.
    Headphones,
    /// Any unassigned code.
    Reserved(u8),
}

impl AudioRenderingIndication {
    /// The label for this indication (`"reserved"` for an unassigned code).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NoPreference => "no-preference",
            Self::Stereo => "stereo",
            Self::TwoDimensional => "two-dimensional",
            Self::ThreeDimensional => "three-dimensional",
            Self::Headphones => "headphones",
            Self::Reserved(_) => "reserved",
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::NoPreference => 0,
            Self::Stereo => 1,
            Self::TwoDimensional => 2,
            Self::ThreeDimensional => 3,
            Self::Headphones => 4,
            Self::Reserved(code) => *code,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            0 => Self::NoPreference,
            1 => Self::Stereo,
            2 => Self::TwoDimensional,
            3 => Self::ThreeDimensional,
            4 => Self::Headphones,
            other => Self::Reserved(other),
        }
    }
}

broadcast_core::impl_spec_display!(AudioRenderingIndication, Reserved);

/// Audio Rendering Indication Box (`ardi`) — ISO/IEC 14496-12:2026 §12.2.8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioRenderingIndicationBox {
    version: u8,
    flags: u32,
    indication: AudioRenderingIndication,
}

impl AudioRenderingIndicationBox {
    /// Build one from its indication.
    #[must_use]
    pub const fn new(indication: AudioRenderingIndication) -> Self {
        Self {
            version: 0,
            flags: 0,
            indication,
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The preferred reproduction channel layout hint.
    #[must_use]
    pub const fn indication(&self) -> AudioRenderingIndication {
        self.indication
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + 1, "ardi")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            indication: AudioRenderingIndication::from_code(body[FULLBOX_EXTRA_SIZE]),
        })
    }
}

impl<'a> Parse<'a> for AudioRenderingIndicationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != ARDI {
            return Err(Error::UnexpectedBox { expected: "ardi" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for AudioRenderingIndicationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + 1
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &ARDI, need, self.version, self.flags, false)?;
        buf[cursor] = self.indication.code();
        cursor += 1;
        Ok(cursor)
    }
}

/// One child of an audio element container — §12.2.9 to §12.2.14.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AudioElementChild {
    /// `aedb` — [`AudioElementDescriptionBox`] (§12.2.10).
    Description(AudioElementDescriptionBox),
    /// `aepp` — [`AudioElementPosInteractivityPolarBox`] (§12.2.11).
    Positioning(AudioElementPosInteractivityPolarBox),
    /// `aepr` — [`AudioElementProminenceInteractivityBox`] (§12.2.12).
    Prominence(AudioElementProminenceInteractivityBox),
    /// `aesd` — [`AudioElementSelectionDescriptionBox`] (§12.2.14).
    SelectionDescription(AudioElementSelectionDescriptionBox),
    /// `aelm` — [`AudioElementBox`] (§12.2.9).
    Element(AudioElementBox),
    /// `kind` — [`KindBox`] (§8.10.4).
    Kind(KindBox),
    /// `labl` — [`LabelBox`] (§8.10.5).
    Label(LabelBox),
    /// `elng` — [`ExtendedLanguageBox`] (§8.4.6).
    Language(ExtendedLanguageBox),
    /// Any other child, kept verbatim.
    Other(OpaqueBox),
}

impl Serialize for AudioElementChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Description(b) => b.serialized_len(),
            Self::Positioning(b) => b.serialized_len(),
            Self::Prominence(b) => b.serialized_len(),
            Self::SelectionDescription(b) => b.serialized_len(),
            Self::Element(b) => b.serialized_len(),
            Self::Kind(b) => b.serialized_len(),
            Self::Label(b) => b.serialized_len(),
            Self::Language(b) => b.serialized_len(),
            Self::Other(b) => b.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Description(b) => b.serialize_into(buf),
            Self::Positioning(b) => b.serialize_into(buf),
            Self::Prominence(b) => b.serialize_into(buf),
            Self::SelectionDescription(b) => b.serialize_into(buf),
            Self::Element(b) => b.serialize_into(buf),
            Self::Kind(b) => b.serialize_into(buf),
            Self::Label(b) => b.serialize_into(buf),
            Self::Language(b) => b.serialize_into(buf),
            Self::Other(b) => b.serialize_into(buf),
        }
    }
}

fn parse_children(body: &[u8]) -> Result<Vec<AudioElementChild>> {
    let mut children = Vec::new();
    for child in crate::box_types::BoxIter::new(body) {
        let child = child?;
        children.push(match &child.four_cc() {
            b"aedb" => {
                AudioElementChild::Description(AudioElementDescriptionBox::parse(child.bytes)?)
            }
            b"aepp" => AudioElementChild::Positioning(AudioElementPosInteractivityPolarBox::parse(
                child.bytes,
            )?),
            b"aepr" => AudioElementChild::Prominence(
                AudioElementProminenceInteractivityBox::parse(child.bytes)?,
            ),
            b"aesd" => AudioElementChild::SelectionDescription(
                AudioElementSelectionDescriptionBox::parse(child.bytes)?,
            ),
            b"aelm" => AudioElementChild::Element(AudioElementBox::parse(child.bytes)?),
            b"kind" => AudioElementChild::Kind(KindBox::parse(child.bytes)?),
            b"labl" => AudioElementChild::Label(LabelBox::parse(child.bytes)?),
            b"elng" => AudioElementChild::Language(ExtendedLanguageBox::parse(child.bytes)?),
            _ => AudioElementChild::Other(opaque_child(child)),
        });
    }
    Ok(children)
}

fn children_len(children: &[AudioElementChild]) -> usize {
    children
        .iter()
        .map(Serialize::serialized_len)
        .sum::<usize>()
}

fn serialize_children(
    children: &[AudioElementChild],
    buf: &mut [u8],
    start: usize,
) -> Result<usize> {
    let mut cursor = start;
    for child in children {
        cursor += child.serialize_into(&mut buf[cursor..])?;
    }
    Ok(cursor)
}

/// Audio Element Description Box (`aedb`) — ISO/IEC 14496-12:2026 §12.2.10.
///
/// The element's tag, whether it can be toggled and whether it starts
/// enabled, and its optional interactivity boxes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioElementDescriptionBox {
    version: u8,
    flags: u32,
    audio_element_tag: Vec<u8>,
    is_toggleable: bool,
    is_default_enabled: bool,
    reserved: u8,
    children: Vec<AudioElementChild>,
}

impl AudioElementDescriptionBox {
    /// Build one from the element's tag.
    #[must_use]
    pub fn new(audio_element_tag: &str) -> Self {
        Self {
            version: 0,
            flags: 0,
            audio_element_tag: audio_element_tag.as_bytes().to_vec(),
            is_toggleable: false,
            is_default_enabled: false,
            reserved: 0,
            children: Vec::new(),
        }
    }

    /// Set whether the element can be switched on and off independently.
    #[must_use]
    pub const fn with_toggleable(mut self, is_toggleable: bool) -> Self {
        self.is_toggleable = is_toggleable;
        self
    }

    /// Set whether the element is enabled by default.
    #[must_use]
    pub const fn with_default_enabled(mut self, is_default_enabled: bool) -> Self {
        self.is_default_enabled = is_default_enabled;
        self
    }

    /// Append one child box.
    #[must_use]
    pub fn with_child(mut self, child: AudioElementChild) -> Self {
        self.children.push(child);
        self
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The raw `audioElementTag` bytes.
    #[must_use]
    pub fn audio_element_tag(&self) -> &[u8] {
        &self.audio_element_tag
    }

    /// The element tag as UTF-8, when it is.
    #[must_use]
    pub fn audio_element_tag_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.audio_element_tag).ok()
    }

    /// Whether the element can be switched on and off independently.
    #[must_use]
    pub const fn is_toggleable(&self) -> bool {
        self.is_toggleable
    }

    /// Whether the element is enabled by default.
    #[must_use]
    pub const fn is_default_enabled(&self) -> bool {
        self.is_default_enabled
    }

    /// The six reserved bits, kept as read.
    #[must_use]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }

    /// The children, in wire order.
    #[must_use]
    pub fn children(&self) -> &[AudioElementChild] {
        &self.children
    }

    /// The positioning interactivity (`aepp`), when present.
    #[must_use]
    pub fn positioning(&self) -> Option<&AudioElementPosInteractivityPolarBox> {
        self.children.iter().find_map(|child| match child {
            AudioElementChild::Positioning(b) => Some(b),
            _ => None,
        })
    }

    /// The prominence interactivity (`aepr`), when present.
    #[must_use]
    pub fn prominence(&self) -> Option<&AudioElementProminenceInteractivityBox> {
        self.children.iter().find_map(|child| match child {
            AudioElementChild::Prominence(b) => Some(b),
            _ => None,
        })
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "aedb")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let audio_element_tag = take_c_string(body, &mut cursor, "aedb audioElementTag")?;
        need_in(body, cursor + 1, "aedb flags byte")?;
        let byte = body[cursor];
        cursor += 1;
        Ok(Self {
            version,
            flags,
            audio_element_tag,
            is_toggleable: (byte & AEDB_TOGGLEABLE) != 0,
            is_default_enabled: (byte & AEDB_DEFAULT_ENABLED) != 0,
            reserved: byte & AEDB_RESERVED_MASK,
            children: parse_children(&body[cursor..])?,
        })
    }
}

impl<'a> Parse<'a> for AudioElementDescriptionBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != AEDB {
            return Err(Error::UnexpectedBox { expected: "aedb" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for AudioElementDescriptionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + self.audio_element_tag.len()
                + 1
                + 1
                + children_len(&self.children),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &AEDB, need, self.version, self.flags, false)?;
        cursor += write_c_string(buf, cursor, &self.audio_element_tag)?;
        buf[cursor] = u8::from(self.is_default_enabled) << AEDB_DEFAULT_ENABLED_SHIFT
            | u8::from(self.is_toggleable) << AEDB_TOGGLEABLE_SHIFT
            | (self.reserved & AEDB_RESERVED_MASK);
        cursor += 1;
        serialize_children(&self.children, buf, cursor)
    }
}

/// Audio Element Positioning Interactivity Polar Box (`aepp`) —
/// ISO/IEC 14496-12:2026 §12.2.11.
///
/// Each flag bit makes one coordinate triple present: azimuth (degrees),
/// elevation (degrees in the advertised unit) and distance (millimetres).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioElementPosInteractivityPolarBox {
    version: u8,
    flags: u32,
    azimuth: Option<[i16; AEPP_TRIPLE]>,
    elevation: Option<[i8; AEPP_TRIPLE]>,
    distance: Option<[u32; AEPP_TRIPLE]>,
}

impl AudioElementPosInteractivityPolarBox {
    /// Build one with no coordinate triple present.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            version: 0,
            flags: 0,
            azimuth: None,
            elevation: None,
            distance: None,
        }
    }

    /// Set the minimum, maximum and default azimuth, and its presence flag.
    #[must_use]
    pub const fn with_azimuth(mut self, values: [i16; AEPP_TRIPLE]) -> Self {
        self.azimuth = Some(values);
        self.flags |= AEPP_AZIMUTH_PRESENT;
        self
    }

    /// Set the minimum, maximum and default elevation, and its presence flag.
    #[must_use]
    pub const fn with_elevation(mut self, values: [i8; AEPP_TRIPLE]) -> Self {
        self.elevation = Some(values);
        self.flags |= AEPP_ELEVATION_PRESENT;
        self
    }

    /// Set the minimum, maximum and default distance, and its presence flag.
    #[must_use]
    pub const fn with_distance(mut self, values: [u32; AEPP_TRIPLE]) -> Self {
        self.distance = Some(values);
        self.flags |= AEPP_DISTANCE_PRESENT;
        self
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `minAzimuth`, `maxAzimuth` and `defaultAzimuth`, when present.
    #[must_use]
    pub const fn azimuth(&self) -> Option<[i16; AEPP_TRIPLE]> {
        self.azimuth
    }

    /// `minElevation`, `maxElevation` and `defaultElevation`, when present.
    #[must_use]
    pub const fn elevation(&self) -> Option<[i8; AEPP_TRIPLE]> {
        self.elevation
    }

    /// `minDistance`, `maxDistance` and `defaultDistance`, when present.
    #[must_use]
    pub const fn distance(&self) -> Option<[u32; AEPP_TRIPLE]> {
        self.distance
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "aepp")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let mut azimuth = None;
        if (flags & AEPP_AZIMUTH_PRESENT) != 0 {
            let mut values = [0i16; AEPP_TRIPLE];
            for slot in &mut values {
                *slot = i16_at(body, cursor);
                cursor += core::mem::size_of::<i16>();
            }
            azimuth = Some(values);
        }
        let mut elevation = None;
        if (flags & AEPP_ELEVATION_PRESENT) != 0 {
            need_in(body, cursor + AEPP_TRIPLE, "aepp elevation")?;
            let mut values = [0i8; AEPP_TRIPLE];
            for slot in &mut values {
                *slot = body[cursor] as i8;
                cursor += 1;
            }
            elevation = Some(values);
        }
        let mut distance = None;
        if (flags & AEPP_DISTANCE_PRESENT) != 0 {
            let mut values = [0u32; AEPP_TRIPLE];
            for slot in &mut values {
                *slot = be_u32(body, cursor, "aepp distance")?;
                cursor += core::mem::size_of::<u32>();
            }
            distance = Some(values);
        }
        Ok(Self {
            version,
            flags,
            azimuth,
            elevation,
            distance,
        })
    }
}

impl Default for AudioElementPosInteractivityPolarBox {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Parse<'a> for AudioElementPosInteractivityPolarBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != AEPP {
            return Err(Error::UnexpectedBox { expected: "aepp" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for AudioElementPosInteractivityPolarBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + self
                    .azimuth
                    .map_or(0, |_| AEPP_TRIPLE * core::mem::size_of::<i16>())
                + self.elevation.map_or(0, |_| AEPP_TRIPLE)
                + self
                    .distance
                    .map_or(0, |_| AEPP_TRIPLE * core::mem::size_of::<u32>()),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &AEPP, need, self.version, self.flags, false)?;
        if let Some(values) = &self.azimuth {
            for value in values {
                buf[cursor..cursor + core::mem::size_of::<i16>()]
                    .copy_from_slice(&value.to_be_bytes());
                cursor += core::mem::size_of::<i16>();
            }
        }
        if let Some(values) = &self.elevation {
            for value in values {
                buf[cursor] = *value as u8;
                cursor += 1;
            }
        }
        if let Some(values) = &self.distance {
            for value in values {
                buf[cursor..cursor + core::mem::size_of::<u32>()]
                    .copy_from_slice(&value.to_be_bytes());
                cursor += core::mem::size_of::<u32>();
            }
        }
        Ok(cursor)
    }
}

/// Audio Element Prominence Interactivity Box (`aepr`) — ISO/IEC 14496-12:2026
/// §12.2.12.
///
/// Minimum, maximum and default prominence in dB. The clause declares each
/// field `double(32)`, so each is one 32-bit word; this type keeps the word
/// exactly as it reads and offers it as IEEE-754 single precision through
/// [`AudioElementProminenceInteractivityBox::min_prominence`] and its
/// siblings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioElementProminenceInteractivityBox {
    version: u8,
    flags: u32,
    values: [u32; AEPR_VALUES],
}

impl AudioElementProminenceInteractivityBox {
    /// Build one from the three 32-bit prominence words.
    #[must_use]
    pub const fn new(values: [u32; AEPR_VALUES]) -> Self {
        Self {
            version: 0,
            flags: 0,
            values,
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The three raw 32-bit words, in `min`/`max`/`default` order.
    #[must_use]
    pub const fn values(&self) -> [u32; AEPR_VALUES] {
        self.values
    }

    /// `minProminence` read as an IEEE-754 single-precision value.
    #[must_use]
    pub fn min_prominence(&self) -> f32 {
        f32::from_bits(self.values[0])
    }

    /// `maxProminence` read as an IEEE-754 single-precision value.
    #[must_use]
    pub fn max_prominence(&self) -> f32 {
        f32::from_bits(self.values[1])
    }

    /// `defaultProminence` read as an IEEE-754 single-precision value.
    #[must_use]
    pub fn default_prominence(&self) -> f32 {
        f32::from_bits(self.values[2])
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + AEPR_VALUES * core::mem::size_of::<u32>(),
            "aepr",
        )?;
        let mut values = [0u32; AEPR_VALUES];
        for (index, slot) in values.iter_mut().enumerate() {
            *slot = be_u32(
                body,
                FULLBOX_EXTRA_SIZE + index * core::mem::size_of::<u32>(),
                "aepr prominence",
            )?;
        }
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            values,
        })
    }
}

impl<'a> Parse<'a> for AudioElementProminenceInteractivityBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != AEPR {
            return Err(Error::UnexpectedBox { expected: "aepr" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for AudioElementProminenceInteractivityBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + AEPR_VALUES * core::mem::size_of::<u32>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &AEPR, need, self.version, self.flags, false)?;
        for value in &self.values {
            buf[cursor..cursor + core::mem::size_of::<u32>()].copy_from_slice(&value.to_be_bytes());
            cursor += core::mem::size_of::<u32>();
        }
        Ok(cursor)
    }
}

/// Audio Element Selection Description Box (`aesd`) — ISO/IEC 14496-12:2026
/// §12.2.14.
///
/// The selection's tag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioElementSelectionDescriptionBox {
    version: u8,
    flags: u32,
    selection_tag: Vec<u8>,
}

impl AudioElementSelectionDescriptionBox {
    /// Build one from the selection's tag.
    #[must_use]
    pub fn new(selection_tag: &str) -> Self {
        Self {
            version: 0,
            flags: 0,
            selection_tag: selection_tag.as_bytes().to_vec(),
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The raw `selectionTag` bytes.
    #[must_use]
    pub fn selection_tag(&self) -> &[u8] {
        &self.selection_tag
    }

    /// The tag as UTF-8, when it is.
    #[must_use]
    pub fn selection_tag_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.selection_tag).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "aesd")?;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            selection_tag: take_c_string(body, &mut cursor, "aesd selectionTag")?,
        })
    }
}

impl<'a> Parse<'a> for AudioElementSelectionDescriptionBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != AESD {
            return Err(Error::UnexpectedBox { expected: "aesd" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for AudioElementSelectionDescriptionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.selection_tag.len() + 1)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let cursor = write_full_box_header(buf, &AESD, need, self.version, self.flags, false)?;
        let written = write_c_string(buf, cursor, &self.selection_tag)?;
        Ok(cursor + written)
    }
}

/// Audio Element Box (`aelm`) — ISO/IEC 14496-12:2026 §12.2.9.
///
/// The empty-syntax container that aggregates one audio element's semantic
/// information: its description (`aedb`), interactivity, and any
/// `kind`/`labl`/`elng` labels the clause allows here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioElementBox {
    children: Vec<AudioElementChild>,
}

impl AudioElementBox {
    /// An element with no children.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one child box.
    #[must_use]
    pub fn with_child(mut self, child: AudioElementChild) -> Self {
        self.children.push(child);
        self
    }

    /// The children, in wire order.
    #[must_use]
    pub fn children(&self) -> &[AudioElementChild] {
        &self.children
    }

    /// The element's description (`aedb`), when present.
    #[must_use]
    pub fn description(&self) -> Option<&AudioElementDescriptionBox> {
        self.children.iter().find_map(|child| match child {
            AudioElementChild::Description(b) => Some(b),
            _ => None,
        })
    }

    /// The element's kind labels (`kind`), in wire order.
    #[must_use]
    pub fn kinds(&self) -> Vec<&KindBox> {
        self.children
            .iter()
            .filter_map(|child| match child {
                AudioElementChild::Kind(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    /// The element's labels (`labl`), in wire order.
    #[must_use]
    pub fn labels(&self) -> Vec<&LabelBox> {
        self.children
            .iter()
            .filter_map(|child| match child {
                AudioElementChild::Label(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        Ok(Self {
            children: parse_children(body)?,
        })
    }
}

impl<'a> Parse<'a> for AudioElementBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != AELM {
            return Err(Error::UnexpectedBox { expected: "aelm" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for AudioElementBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + children_len(&self.children))
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let cursor = write_box_header(buf, &AELM, need)?;
        serialize_children(&self.children, buf, cursor)
    }
}

/// Audio Element Selection Box (`aesb`) — ISO/IEC 14496-12:2026 §12.2.13.
///
/// A collection of audio elements of which only one can be switched on at a
/// time, with its selection description (`aesd`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioElementSelectionBox {
    children: Vec<AudioElementChild>,
}

impl AudioElementSelectionBox {
    /// A selection with no children.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one child box.
    #[must_use]
    pub fn with_child(mut self, child: AudioElementChild) -> Self {
        self.children.push(child);
        self
    }

    /// The children, in wire order.
    #[must_use]
    pub fn children(&self) -> &[AudioElementChild] {
        &self.children
    }

    /// The selection's description (`aesd`), when present.
    #[must_use]
    pub fn selection_description(&self) -> Option<&AudioElementSelectionDescriptionBox> {
        self.children.iter().find_map(|child| match child {
            AudioElementChild::SelectionDescription(b) => Some(b),
            _ => None,
        })
    }

    /// The audio elements (`aelm`), in wire order.
    #[must_use]
    pub fn elements(&self) -> Vec<&AudioElementBox> {
        self.children
            .iter()
            .filter_map(|child| match child {
                AudioElementChild::Element(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        Ok(Self {
            children: parse_children(body)?,
        })
    }
}

impl<'a> Parse<'a> for AudioElementSelectionBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != AESB {
            return Err(Error::UnexpectedBox { expected: "aesb" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for AudioElementSelectionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + children_len(&self.children))
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let cursor = write_box_header(buf, &AESB, need)?;
        serialize_children(&self.children, buf, cursor)
    }
}
