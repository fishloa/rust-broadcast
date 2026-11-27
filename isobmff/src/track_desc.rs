//! Track-description boxes — ISO/IEC 14496-12:2026 §8.4.6, §8.10.3, §8.10.4,
//! §8.3.4, §13.5.4.
//!
//! | Box | Type | Clause | Container |
//! |-----|------|--------|-----------|
//! | [`ExtendedLanguageBox`] | `elng` | §8.4.6 | `mdia` |
//! | [`KindBox`] | `kind` | §8.10.4 | track `udta` |
//! | [`TrackGroupBox`] | `trgr` | §8.3.4 | `trak` |
//! | [`TrackGroupTypeBox`] | `msrc`/`ster` | §8.3.4.4 | `trgr` |
//! | [`TrackSelectionBox`] | `tsel` | §8.10.3 | track `udta` |
//! | [`StereoVideoBox`] | `stvi` | §13.5.4.2 | `schi` |
//!
//! Every type is private-field with a builder and accessors, and round-trips
//! byte-identically.

use alloc::vec::Vec;

use broadcast_core::len::{fit_u32, need_in, need_out};
use broadcast_core::wire::{be_u32, i16_at, u24_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::{take_c_string, write_c_string};
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK,
    box_len, parse_box, write_box_header, write_full_box_header,
};
use crate::error::{Error, Result};

/// Four-CC of the Extended Language Box (`elng`, ISO/IEC 14496-12:2026 §8.4.6).
pub const ELNG: [u8; BOX_TYPE_SIZE] = *b"elng";
/// Four-CC of the Track Kind Box (`kind`, §8.10.4).
pub const KIND: [u8; BOX_TYPE_SIZE] = *b"kind";
/// Four-CC of the Track Group Box (`trgr`, §8.3.4).
pub const TRGR: [u8; BOX_TYPE_SIZE] = *b"trgr";
/// Four-CC of the Track Selection Box (`tsel`, §8.10.3).
pub const TSEL: [u8; BOX_TYPE_SIZE] = *b"tsel";
/// Four-CC of the Stereo Video Box (`stvi`, §13.5.4.2).
pub const STVI: [u8; BOX_TYPE_SIZE] = *b"stvi";
/// `track_group_type` of a multi-source presentation group (§8.3.4.4.1).
pub const TRACK_GROUP_MSRC: [u8; BOX_TYPE_SIZE] = *b"msrc";
/// `track_group_type` of a stereoscopic pair (§8.3.4.4.2).
pub const TRACK_GROUP_STER: [u8; BOX_TYPE_SIZE] = *b"ster";

/// Bits of `single_view_allowed`.
const STVI_SINGLE_VIEW_BITS: u32 = 2;
/// Mask of the two `single_view_allowed` bits.
const STVI_SINGLE_VIEW_MASK: u32 = (1 << STVI_SINGLE_VIEW_BITS) - 1;

// ---------------------------------------------------------------------------
// ExtendedLanguageBox — elng (§8.4.6)
// ---------------------------------------------------------------------------

/// Extended Language Box (`elng`) — ISO/IEC 14496-12:2026 §8.4.6.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ExtendedLanguageBox {
    version: u8,
    flags: u32,
    language: Vec<u8>,
}

impl ExtendedLanguageBox {
    /// Build one from an IETF BCP 47 tag.
    #[must_use]
    pub fn new(language: &str) -> Self {
        Self {
            version: 0,
            flags: 0,
            language: language.as_bytes().to_vec(),
        }
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The raw `extended_language` bytes.
    #[must_use]
    pub fn language(&self) -> &[u8] {
        &self.language
    }

    /// The extended language as UTF-8, when it is.
    #[must_use]
    pub fn language_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.language).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "elng body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let language = take_c_string(body, &mut cursor, "elng extended_language")?;
        Ok(Self {
            version,
            flags,
            language,
        })
    }
}

impl<'a> Parse<'a> for ExtendedLanguageBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "elng box")? != u32::from_be_bytes(ELNG) {
            return Err(Error::InvalidInput("expected elng"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "elng box",
        )?)
    }
}

impl Serialize for ExtendedLanguageBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.language.len() + 1)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &ELNG, need, self.version, self.flags, false)?;
        c += write_c_string(buf, c, &self.language)?;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// KindBox — kind (§8.10.4)
// ---------------------------------------------------------------------------

/// Track Kind Box (`kind`) — ISO/IEC 14496-12:2026 §8.10.4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct KindBox {
    version: u8,
    flags: u32,
    scheme_uri: Vec<u8>,
    value: Option<Vec<u8>>,
}

impl KindBox {
    /// Build one from a scheme URI with no value.
    #[must_use]
    pub fn new(scheme_uri: &str) -> Self {
        Self {
            version: 0,
            flags: 0,
            scheme_uri: scheme_uri.as_bytes().to_vec(),
            value: None,
        }
    }

    /// Append the scheme's value.
    #[must_use]
    pub fn with_value(mut self, value: &str) -> Self {
        self.value = Some(value.as_bytes().to_vec());
        self
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The raw `schemeURI` bytes.
    #[must_use]
    pub fn scheme_uri(&self) -> &[u8] {
        &self.scheme_uri
    }

    /// The raw `value` bytes, when present.
    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    /// The scheme URI as UTF-8, when it is.
    #[must_use]
    pub fn scheme_uri_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.scheme_uri).ok()
    }

    /// The value as UTF-8, when present and valid.
    #[must_use]
    pub fn value_str(&self) -> Option<&str> {
        core::str::from_utf8(self.value.as_deref()?).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "kind body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let scheme_uri = take_c_string(body, &mut cursor, "kind schemeURI")?;
        let value = if cursor < body.len() {
            Some(take_c_string(body, &mut cursor, "kind value")?)
        } else {
            None
        };
        Ok(Self {
            version,
            flags,
            scheme_uri,
            value,
        })
    }
}

impl<'a> Parse<'a> for KindBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "kind box")? != u32::from_be_bytes(KIND) {
            return Err(Error::InvalidInput("expected kind"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "kind box",
        )?)
    }
}

impl Serialize for KindBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.scheme_uri.len() + 1;
        if let Some(value) = &self.value {
            n += value.len() + 1;
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &KIND, need, self.version, self.flags, false)?;
        c += write_c_string(buf, c, &self.scheme_uri)?;
        if let Some(value) = &self.value {
            c += write_c_string(buf, c, value)?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// TrackGroupBox and TrackGroupTypeBox — trgr / msrc / ster (§8.3.4)
// ---------------------------------------------------------------------------

/// One `TrackGroupTypeBox` child of a [`TrackGroupBox`] (§8.3.4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TrackGroupTypeBox {
    group_type: [u8; BOX_TYPE_SIZE],
    version: u8,
    flags: u32,
    group_id: u32,
    left_view_flag: Option<bool>,
    reserved: u32,
    trailing: Vec<u8>,
}

impl TrackGroupTypeBox {
    /// Build a multi-source (`msrc`) group.
    #[must_use]
    pub fn multi_source(group_id: u32) -> Self {
        Self {
            group_type: TRACK_GROUP_MSRC,
            version: 0,
            flags: 0,
            group_id,
            left_view_flag: None,
            reserved: 0,
            trailing: Vec::new(),
        }
    }

    /// Build a stereoscopic-pair (`ster`) group.
    #[must_use]
    pub fn stereo(group_id: u32, left_view: bool) -> Self {
        Self {
            group_type: TRACK_GROUP_STER,
            version: 0,
            flags: 0,
            group_id,
            left_view_flag: Some(left_view),
            reserved: 0,
            trailing: Vec::new(),
        }
    }

    /// Set the group's `track_group_id`.
    #[must_use]
    pub fn with_group_id(mut self, group_id: u32) -> Self {
        self.group_id = group_id;
        self
    }

    /// Set the FullBox `flags` (bit 0 means the id is not a `track_ID`).
    #[must_use]
    pub fn with_flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }

    /// The `track_group_type` four-CC.
    #[must_use]
    pub const fn group_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.group_type
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

    /// The `track_group_id`.
    #[must_use]
    pub const fn group_id(&self) -> u32 {
        self.group_id
    }

    /// The `ster` `left_view_flag`, when this is a stereo group.
    #[must_use]
    pub const fn left_view_flag(&self) -> Option<bool> {
        self.left_view_flag
    }

    /// The 31 reserved bits of a `ster` body (preserved verbatim).
    #[must_use]
    pub const fn reserved(&self) -> u32 {
        self.reserved
    }

    /// The trailing bytes of a group type this crate does not model.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Parse one group box (header included).
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        let fourcc = item.four_cc();
        let body = crate::box_types::box_body(bytes, FULLBOX_EXTRA_SIZE, "track group box")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "track group body",
        )?;
        let group_id = be_u32(body, FULLBOX_EXTRA_SIZE, "track_group_id")?;
        let rest = &body[FULLBOX_EXTRA_SIZE + 4..];
        let mut out = Self {
            group_type: fourcc,
            version,
            flags,
            group_id,
            left_view_flag: None,
            reserved: 0,
            trailing: Vec::new(),
        };
        if fourcc == TRACK_GROUP_STER && rest.len() >= core::mem::size_of::<u32>() {
            let word = be_u32(rest, 0, "ster left_view_flag")?;
            out.left_view_flag = Some(word & 1 != 0);
            out.reserved = word >> 1;
        } else {
            out.trailing = rest.to_vec();
        }
        Ok(out)
    }

    fn serialized_len(&self) -> usize {
        let extra = if self.group_type == TRACK_GROUP_STER && self.left_view_flag.is_some() {
            core::mem::size_of::<u32>()
        } else {
            self.trailing.len()
        };
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() + extra)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c =
            write_full_box_header(buf, &self.group_type, need, self.version, self.flags, false)?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.group_id.to_be_bytes());
        c += core::mem::size_of::<u32>();
        if self.group_type == TRACK_GROUP_STER && self.left_view_flag.is_some() {
            let left = u32::from(self.left_view_flag.unwrap_or(false));
            let word = (self.reserved << 1) | left;
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&word.to_be_bytes());
            c += core::mem::size_of::<u32>();
        } else {
            buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
            c += self.trailing.len();
        }
        Ok(c)
    }
}

/// Track Group Box (`trgr`) — ISO/IEC 14496-12:2026 §8.3.4.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TrackGroupBox {
    groups: Vec<TrackGroupTypeBox>,
}

impl TrackGroupBox {
    /// Build an empty group box.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one group.
    #[must_use]
    pub fn with_group(mut self, group: TrackGroupTypeBox) -> Self {
        self.groups.push(group);
        self
    }

    /// The groups, in wire order.
    #[must_use]
    pub fn groups(&self) -> &[TrackGroupTypeBox] {
        &self.groups
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut groups = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            groups.push(TrackGroupTypeBox::parse(child.bytes)?);
        }
        Ok(Self { groups })
    }
}

impl<'a> Parse<'a> for TrackGroupBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "trgr box")? != u32::from_be_bytes(TRGR) {
            return Err(Error::InvalidInput("expected trgr"));
        }
        Self::parse_body(crate::box_types::box_body(bytes, 0, "trgr box")?)
    }
}

impl Serialize for TrackGroupBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for group in &self.groups {
            n += group.serialized_len();
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &TRGR, need)?;
        for group in &self.groups {
            c += group.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// TrackSelectionBox — tsel (§8.10.3)
// ---------------------------------------------------------------------------

/// Track Selection Box (`tsel`) — ISO/IEC 14496-12:2026 §8.10.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TrackSelectionBox {
    version: u8,
    flags: u32,
    switch_group: i32,
    attributes: Vec<[u8; BOX_TYPE_SIZE]>,
}

impl TrackSelectionBox {
    /// Build a box with `switch_group == 0` (no switching information).
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: 0,
            flags: 0,
            switch_group: 0,
            attributes: Vec::new(),
        }
    }

    /// Set the switch group.
    #[must_use]
    pub fn with_switch_group(mut self, switch_group: i32) -> Self {
        self.switch_group = switch_group;
        self
    }

    /// Append one attribute four-CC.
    #[must_use]
    pub fn with_attribute(mut self, attribute: [u8; BOX_TYPE_SIZE]) -> Self {
        self.attributes.push(attribute);
        self
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The switch group (`0` when switching is not signalled).
    #[must_use]
    pub const fn switch_group(&self) -> i32 {
        self.switch_group
    }

    /// The attribute list, in wire order.
    #[must_use]
    pub fn attributes(&self) -> &[[u8; BOX_TYPE_SIZE]] {
        &self.attributes
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "tsel body",
        )?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let switch_group = be_u32(body, FULLBOX_EXTRA_SIZE, "tsel switch_group")? as i32;
        let rest = &body[FULLBOX_EXTRA_SIZE + 4..];
        let count = rest.len() / BOX_TYPE_SIZE;
        let mut attributes = Vec::with_capacity(count);
        for chunk in rest.chunks_exact(BOX_TYPE_SIZE) {
            let mut attribute = [0u8; BOX_TYPE_SIZE];
            attribute.copy_from_slice(chunk);
            attributes.push(attribute);
        }
        Ok(Self {
            version,
            flags,
            switch_group,
            attributes,
        })
    }
}

impl Default for TrackSelectionBox {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Parse<'a> for TrackSelectionBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "tsel box")? != u32::from_be_bytes(TSEL) {
            return Err(Error::InvalidInput("expected tsel"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "tsel box",
        )?)
    }
}

impl Serialize for TrackSelectionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + core::mem::size_of::<u32>()
                + self.attributes.len() * BOX_TYPE_SIZE,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &TSEL, need, self.version, self.flags, false)?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.switch_group.to_be_bytes());
        c += core::mem::size_of::<u32>();
        for attribute in &self.attributes {
            buf[c..c + BOX_TYPE_SIZE].copy_from_slice(attribute);
            c += BOX_TYPE_SIZE;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// StereoVideoBox — stvi (§13.5.4.2)
// ---------------------------------------------------------------------------

/// Stereo Video Box (`stvi`) — ISO/IEC 14496-12:2026 §13.5.4.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct StereoVideoBox {
    version: u8,
    flags: u32,
    single_view_allowed: u8,
    stereo_scheme: u32,
    stereo_indication: Vec<u8>,
    extra: Vec<u8>,
}

impl StereoVideoBox {
    /// Build one with the given scheme and indication bytes.
    #[must_use]
    pub fn new(stereo_scheme: u32, stereo_indication: Vec<u8>) -> Self {
        Self {
            version: 0,
            flags: 0,
            single_view_allowed: 0,
            stereo_scheme,
            stereo_indication,
            extra: Vec::new(),
        }
    }

    /// Set `single_view_allowed`.
    #[must_use]
    pub fn with_single_view_allowed(mut self, allowed: u8) -> Self {
        self.single_view_allowed = allowed;
        self
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The `single_view_allowed` view mask.
    #[must_use]
    pub const fn single_view_allowed(&self) -> u8 {
        self.single_view_allowed
    }

    /// The `stereo_scheme`.
    #[must_use]
    pub const fn stereo_scheme(&self) -> u32 {
        self.stereo_scheme
    }

    /// The `stereo_indication_type` bytes.
    #[must_use]
    pub fn stereo_indication(&self) -> &[u8] {
        &self.stereo_indication
    }

    /// The bytes after the declared indication length (optional child boxes).
    #[must_use]
    pub fn extra(&self) -> &[u8] {
        &self.extra
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u64>(),
            "stvi body",
        )?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let first = be_u32(body, c, "stvi single_view_allowed")?;
        let single_view_allowed = (first & STVI_SINGLE_VIEW_MASK) as u8;
        c += core::mem::size_of::<u32>();
        let stereo_scheme = be_u32(body, c, "stvi stereo_scheme")?;
        c += core::mem::size_of::<u32>();
        let length = be_u32(body, c, "stvi length")? as usize;
        c += core::mem::size_of::<u32>();
        need_in(body, c + length, "stvi stereo_indication_type")?;
        let stereo_indication = body[c..c + length].to_vec();
        c += length;
        let extra = body[c..].to_vec();
        Ok(Self {
            version,
            flags,
            single_view_allowed,
            stereo_scheme,
            stereo_indication,
            extra,
        })
    }
}

impl<'a> Parse<'a> for StereoVideoBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "stvi box")? != u32::from_be_bytes(STVI) {
            return Err(Error::InvalidInput("expected stvi"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u64>(),
            "stvi box",
        )?)
    }
}

impl Serialize for StereoVideoBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + core::mem::size_of::<u64>()
                + core::mem::size_of::<u32>()
                + self.stereo_indication.len()
                + self.extra.len(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &STVI, need, self.version, self.flags, false)?;
        let first = u32::from(self.single_view_allowed);
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&first.to_be_bytes());
        c += core::mem::size_of::<u32>();
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.stereo_scheme.to_be_bytes());
        c += core::mem::size_of::<u32>();
        let length = fit_u32(self.stereo_indication.len(), "stvi length")?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&length.to_be_bytes());
        c += core::mem::size_of::<u32>();
        buf[c..c + self.stereo_indication.len()].copy_from_slice(&self.stereo_indication);
        c += self.stereo_indication.len();
        buf[c..c + self.extra.len()].copy_from_slice(&self.extra);
        c += self.extra.len();
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SubTrackBox — strk (§8.13.3)
// SubTrackInformationBox — stri (§8.13.4)
// SubTrackDefinitionBox — strd (§8.13.5)
// ---------------------------------------------------------------------------

/// Four-CC of the Sub Track Box (`strk`, §8.13.3).
pub const STRK: [u8; BOX_TYPE_SIZE] = *b"strk";
/// Four-CC of the Sub Track Information Box (`stri`, §8.13.4).
pub const STRI: [u8; BOX_TYPE_SIZE] = *b"stri";
/// Four-CC of the Sub Track Definition Box (`strd`, §8.13.5).
pub const STRD: [u8; BOX_TYPE_SIZE] = *b"strd";
/// `stri` fixed body after the FullBox header: three 16/32-bit fields (§8.13.4.2).
const STRI_FIXED_BODY_LEN: usize = core::mem::size_of::<u16>() * 2 + core::mem::size_of::<u32>();

/// Sub Track Information Box (`stri`) — ISO/IEC 14496-12:2026 §8.13.4.
///
/// The sub-track's switch and alternate groups, its local identifier and its
/// attribute four-CC list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SubTrackInformationBox {
    version: u8,
    flags: u32,
    switch_group: i16,
    alternate_group: i16,
    sub_track_id: u32,
    attributes: Vec<[u8; BOX_TYPE_SIZE]>,
}

impl SubTrackInformationBox {
    /// Build one with no groups and no attributes.
    #[must_use]
    pub fn new(sub_track_id: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            switch_group: 0,
            alternate_group: 0,
            sub_track_id,
            attributes: Vec::new(),
        }
    }

    /// Set the switch group.
    #[must_use]
    pub const fn with_switch_group(mut self, switch_group: i16) -> Self {
        self.switch_group = switch_group;
        self
    }

    /// Set the alternate group.
    #[must_use]
    pub const fn with_alternate_group(mut self, alternate_group: i16) -> Self {
        self.alternate_group = alternate_group;
        self
    }

    /// Append one attribute four-CC.
    #[must_use]
    pub fn with_attribute(mut self, attribute: [u8; BOX_TYPE_SIZE]) -> Self {
        self.attributes.push(attribute);
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

    /// The switch group (`0` when switching is not signalled).
    #[must_use]
    pub const fn switch_group(&self) -> i16 {
        self.switch_group
    }

    /// The alternate group (`0` when no relation is signalled).
    #[must_use]
    pub const fn alternate_group(&self) -> i16 {
        self.alternate_group
    }

    /// The sub-track's local identifier (`0` when unassigned).
    #[must_use]
    pub const fn sub_track_id(&self) -> u32 {
        self.sub_track_id
    }

    /// The attribute list, in wire order.
    #[must_use]
    pub fn attributes(&self) -> &[[u8; BOX_TYPE_SIZE]] {
        &self.attributes
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + STRI_FIXED_BODY_LEN, "stri body")?;
        let offset = FULLBOX_EXTRA_SIZE;
        let switch_group = i16_at(body, offset);
        let alternate_group = i16_at(body, offset + core::mem::size_of::<i16>());
        let sub_track_id = be_u32(
            body,
            offset + 2 * core::mem::size_of::<i16>(),
            "stri sub_track_ID",
        )?;
        let rest = &body[offset + STRI_FIXED_BODY_LEN..];
        let mut attributes = Vec::with_capacity(rest.len() / BOX_TYPE_SIZE);
        for chunk in rest.chunks_exact(BOX_TYPE_SIZE) {
            let mut attribute = [0u8; BOX_TYPE_SIZE];
            attribute.copy_from_slice(chunk);
            attributes.push(attribute);
        }
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            switch_group,
            alternate_group,
            sub_track_id,
            attributes,
        })
    }
}

impl<'a> Parse<'a> for SubTrackInformationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "stri box")? != u32::from_be_bytes(STRI) {
            return Err(Error::InvalidInput("expected stri"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + STRI_FIXED_BODY_LEN,
            "stri box",
        )?)
    }
}

impl Serialize for SubTrackInformationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + STRI_FIXED_BODY_LEN
                + self.attributes.len() * BOX_TYPE_SIZE,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &STRI, need, self.version, self.flags, false)?;
        let u16_len = core::mem::size_of::<u16>();
        buf[c..c + u16_len].copy_from_slice(&self.switch_group.to_be_bytes());
        c += u16_len;
        buf[c..c + u16_len].copy_from_slice(&self.alternate_group.to_be_bytes());
        c += u16_len;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.sub_track_id.to_be_bytes());
        c += core::mem::size_of::<u32>();
        for attribute in &self.attributes {
            buf[c..c + BOX_TYPE_SIZE].copy_from_slice(attribute);
            c += BOX_TYPE_SIZE;
        }
        Ok(c)
    }
}

/// Sub Track Definition Box (`strd`) — ISO/IEC 14496-12:2026 §8.13.5.
///
/// A container of the boxes that define one sub-track; the sample-group
/// definitions (`stsg`) it may carry are typed by
/// [`crate::sample_tables_extra::SubTrackSampleGroupBox`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SubTrackDefinitionBox {
    sample_groups: Vec<crate::sample_tables_extra::SubTrackSampleGroupBox>,
    other_boxes: Vec<crate::box_support::OpaqueBox>,
}

impl SubTrackDefinitionBox {
    /// An empty definition.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one `stsg`.
    #[must_use]
    pub fn with_sample_group(
        mut self,
        group: crate::sample_tables_extra::SubTrackSampleGroupBox,
    ) -> Self {
        self.sample_groups.push(group);
        self
    }

    /// The `stsg` boxes, in wire order.
    #[must_use]
    pub fn sample_groups(&self) -> &[crate::sample_tables_extra::SubTrackSampleGroupBox] {
        &self.sample_groups
    }

    /// Children the clause does not define, kept verbatim.
    #[must_use]
    pub fn other_boxes(&self) -> &[crate::box_support::OpaqueBox] {
        &self.other_boxes
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut sample_groups = Vec::new();
        let mut other_boxes = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            if child.four_cc() == crate::sample_tables_extra::STSG {
                sample_groups.push(crate::sample_tables_extra::SubTrackSampleGroupBox::parse(
                    child.bytes,
                )?);
            } else {
                other_boxes.push(crate::box_support::OpaqueBox {
                    box_type: child.four_cc(),
                    data: child.payload().to_vec(),
                    to_end: child.to_end(),
                    largesize: child.has_largesize(),
                });
            }
        }
        Ok(Self {
            sample_groups,
            other_boxes,
        })
    }
}

impl<'a> Parse<'a> for SubTrackDefinitionBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "strd box")? != u32::from_be_bytes(STRD) {
            return Err(Error::InvalidInput("expected strd"));
        }
        Self::parse_body(crate::box_types::box_body(bytes, 0, "strd box")?)
    }
}

impl Serialize for SubTrackDefinitionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + self
                    .sample_groups
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>()
                + self
                    .other_boxes
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &STRD, need)?;
        for group in &self.sample_groups {
            c += group.serialize_into(&mut buf[c..])?;
        }
        for other in &self.other_boxes {
            c += other.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

/// Sub Track Box (`strk`) — ISO/IEC 14496-12:2026 §8.13.3.
///
/// The `stri` describing a sub-track and the `strd` defining it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SubTrackBox {
    information: SubTrackInformationBox,
    definition: Option<SubTrackDefinitionBox>,
    other_boxes: Vec<crate::box_support::OpaqueBox>,
}

impl SubTrackBox {
    /// Build one from its mandatory `stri`.
    #[must_use]
    pub fn new(information: SubTrackInformationBox) -> Self {
        Self {
            information,
            definition: None,
            other_boxes: Vec::new(),
        }
    }

    /// Set the definition (`strd`).
    #[must_use]
    pub fn with_definition(mut self, definition: SubTrackDefinitionBox) -> Self {
        self.definition = Some(definition);
        self
    }

    /// The sub-track information (`stri`).
    #[must_use]
    pub const fn information(&self) -> &SubTrackInformationBox {
        &self.information
    }

    /// The sub-track definition (`strd`), when present.
    #[must_use]
    pub const fn definition(&self) -> Option<&SubTrackDefinitionBox> {
        self.definition.as_ref()
    }

    /// Children the clause does not define, kept verbatim.
    #[must_use]
    pub fn other_boxes(&self) -> &[crate::box_support::OpaqueBox] {
        &self.other_boxes
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut information = None;
        let mut definition = None;
        let mut other_boxes = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            match &child.four_cc() {
                b"stri" => information = Some(SubTrackInformationBox::parse(child.bytes)?),
                b"strd" => definition = Some(SubTrackDefinitionBox::parse(child.bytes)?),
                _ => other_boxes.push(crate::box_support::OpaqueBox {
                    box_type: child.four_cc(),
                    data: child.payload().to_vec(),
                    to_end: child.to_end(),
                    largesize: child.has_largesize(),
                }),
            }
        }
        let information = information.ok_or(Error::InvalidInput("strk without a stri"))?;
        Ok(Self {
            information,
            definition,
            other_boxes,
        })
    }
}

impl<'a> Parse<'a> for SubTrackBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "strk box")? != u32::from_be_bytes(STRK) {
            return Err(Error::InvalidInput("expected strk"));
        }
        Self::parse_body(crate::box_types::box_body(bytes, 0, "strk box")?)
    }
}

impl Serialize for SubTrackBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + self.information.serialized_len()
                + self.definition.as_ref().map_or(0, |b| b.serialized_len())
                + self
                    .other_boxes
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &STRK, need)?;
        c += self.information.serialize_into(&mut buf[c..])?;
        if let Some(definition) = &self.definition {
            c += definition.serialize_into(&mut buf[c..])?;
        }
        for other in &self.other_boxes {
            c += other.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}
