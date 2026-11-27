//! Restricted, incomplete and scrambled stream scheme boxes —
//! ISO/IEC 14496-12:2026 §13.4.8, §13.5.3, §13.5.5, §13.5.6.2, §13.6.3.
//!
//! Transcription: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`.
//!
//! | Box | Type | Clause | Container |
//! |-----|------|--------|-----------|
//! | [`RestrictedSchemeInfoBox`] | `rinf` | §13.5.3 | restricted sample entry, sample entry |
//! | [`CompatibleSchemeTypeBox`] | `csch` | §13.5.5 | `rinf` |
//! | [`SamplePackingInformationBox`] | `spki` | §13.5.6.2 | `schi` |
//! | [`ScrambleSchemeInfoBox`] | `scrb` | §13.4.8 | sample entry, `ipco` |
//! | [`CompleteTrackInfoBox`] | `cinf` | §13.6.3 | incomplete-track sample entry |
//!
//! Each container keeps the boxes it does not model verbatim, so an
//! unmodelled scheme payload round-trips byte-identically.

use alloc::vec::Vec;

use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::{be_u32, u24_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK, box_len, parse_box,
    write_box_header, write_full_box_header,
};
use crate::cenc::{
    OriginalFormatBox, SCHM_FLAG_HAS_SCHEME_URI, SchemeInformationBox, SchemeTypeBox,
};
use crate::error::{Error, Result};

/// Four-CC of the Restricted Scheme Information Box (`rinf`, §13.5.3).
pub const RINF: [u8; BOX_TYPE_SIZE] = *b"rinf";
/// Four-CC of the Compatible Scheme Type Box (`csch`, §13.5.5).
pub const CSCH: [u8; BOX_TYPE_SIZE] = *b"csch";
/// Four-CC of the Sample Packing Information Box (`spki`, §13.5.6.2).
pub const SPKI: [u8; BOX_TYPE_SIZE] = *b"spki";
/// Four-CC of the Scramble Scheme Information Box (`scrb`, §13.4.8).
pub const SCRB: [u8; BOX_TYPE_SIZE] = *b"scrb";
/// Four-CC of the Complete Track Information Box (`cinf`, §13.6.3).
pub const CINF: [u8; BOX_TYPE_SIZE] = *b"cinf";
/// `csch` fixed body after the FullBox header: `scheme_type` + `scheme_version`.
const CSCH_FIXED_BODY_LEN: usize = BOX_TYPE_SIZE + core::mem::size_of::<u32>();

fn opaque_child(child: crate::box_types::BoxItem<'_>) -> OpaqueBox {
    OpaqueBox {
        box_type: child.four_cc(),
        data: child.payload().to_vec(),
        to_end: child.to_end(),
        largesize: child.has_largesize(),
    }
}

/// Parse a `schm` box's body (its FullBox header included) into a
/// [`SchemeTypeBox`].
fn parse_scheme_type_box(body: &[u8]) -> Result<SchemeTypeBox> {
    need_in(body, FULLBOX_EXTRA_SIZE, "schm")?;
    SchemeTypeBox::parse_body(
        &body[FULLBOX_EXTRA_SIZE..],
        body[0],
        u24_at(body, 1) & FULLBOX_FLAGS_MASK,
    )
}

/// Compatible Scheme Type Box (`csch`) — ISO/IEC 14496-12:2026 §13.5.5.
///
/// One further scheme type the track conforms to, with `schm`'s own syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CompatibleSchemeTypeBox {
    version: u8,
    flags: u32,
    scheme_type: [u8; BOX_TYPE_SIZE],
    scheme_version: u32,
    scheme_uri: Option<Vec<u8>>,
}

impl CompatibleSchemeTypeBox {
    /// Build one from a scheme type and version, with no scheme URI.
    #[must_use]
    pub fn new(scheme_type: [u8; BOX_TYPE_SIZE], scheme_version: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            scheme_type,
            scheme_version,
            scheme_uri: None,
        }
    }

    /// Attach the scheme's browser URI and set the flag that announces it.
    #[must_use]
    pub fn with_scheme_uri(mut self, uri: &str) -> Self {
        self.scheme_uri = Some(uri.as_bytes().to_vec());
        self.flags |= SCHM_FLAG_HAS_SCHEME_URI;
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

    /// `scheme_type`, a four-CC identifying the scheme.
    #[must_use]
    pub const fn scheme_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.scheme_type
    }

    /// `scheme_version`.
    #[must_use]
    pub const fn scheme_version(&self) -> u32 {
        self.scheme_version
    }

    /// The raw `scheme_uri` bytes, when the flag announces one.
    #[must_use]
    pub fn scheme_uri(&self) -> Option<&[u8]> {
        self.scheme_uri.as_deref()
    }

    /// The scheme URI as UTF-8, when present and valid.
    #[must_use]
    pub fn scheme_uri_str(&self) -> Option<&str> {
        core::str::from_utf8(self.scheme_uri.as_deref()?).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + CSCH_FIXED_BODY_LEN, "csch")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let scheme_type = body[FULLBOX_EXTRA_SIZE..FULLBOX_EXTRA_SIZE + BOX_TYPE_SIZE]
            .try_into()
            .map_err(|_| Error::InvalidInput("csch scheme_type"))?;
        let scheme_version = be_u32(
            body,
            FULLBOX_EXTRA_SIZE + BOX_TYPE_SIZE,
            "csch scheme_version",
        )?;
        let scheme_uri = if (flags & SCHM_FLAG_HAS_SCHEME_URI) != 0 {
            Some(body[FULLBOX_EXTRA_SIZE + CSCH_FIXED_BODY_LEN..].to_vec())
        } else {
            None
        };
        Ok(Self {
            version,
            flags,
            scheme_type,
            scheme_version,
            scheme_uri,
        })
    }
}

impl<'a> Parse<'a> for CompatibleSchemeTypeBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != CSCH {
            return Err(Error::UnexpectedBox { expected: "csch" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for CompatibleSchemeTypeBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + CSCH_FIXED_BODY_LEN
                + self.scheme_uri.as_ref().map_or(0, |uri| uri.len()),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &CSCH, need, self.version, self.flags, false)?;
        buf[cursor..cursor + BOX_TYPE_SIZE].copy_from_slice(&self.scheme_type);
        cursor += BOX_TYPE_SIZE;
        buf[cursor..cursor + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.scheme_version.to_be_bytes());
        cursor += core::mem::size_of::<u32>();
        if let Some(uri) = &self.scheme_uri {
            buf[cursor..cursor + uri.len()].copy_from_slice(uri);
            cursor += uri.len();
        }
        Ok(cursor)
    }
}

/// Sample Packing Information Box (`spki`) — ISO/IEC 14496-12:2026 §13.5.6.2.
///
/// `mode` is 0 when every sample of the original streams is preserved, 1 when
/// only some are.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SamplePackingInformationBox {
    version: u8,
    flags: u32,
    mode: u8,
}

impl SamplePackingInformationBox {
    /// Build one from its `mode`.
    #[must_use]
    pub const fn new(mode: u8) -> Self {
        Self {
            version: 0,
            flags: 0,
            mode,
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

    /// `mode`: 0 all samples preserved, 1 only some.
    #[must_use]
    pub const fn mode(&self) -> u8 {
        self.mode
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + 1, "spki")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            mode: body[FULLBOX_EXTRA_SIZE],
        })
    }
}

impl<'a> Parse<'a> for SamplePackingInformationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != SPKI {
            return Err(Error::UnexpectedBox { expected: "spki" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for SamplePackingInformationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + 1
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &SPKI, need, self.version, self.flags, false)?;
        buf[cursor] = self.mode;
        cursor += 1;
        Ok(cursor)
    }
}

/// Restricted Scheme Information Box (`rinf`) — ISO/IEC 14496-12:2026
/// §13.5.3.
///
/// Documents the restriction scheme (`schm`), the original sample-entry type
/// in an `frma` when the entry was transformed, the scheme's own data in an
/// optional `schi`, and any `csch` types the track also conforms to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RestrictedSchemeInfoBox {
    original_format: Option<OriginalFormatBox>,
    scheme: SchemeTypeBox,
    scheme_info: Option<SchemeInformationBox>,
    compatible_schemes: Vec<CompatibleSchemeTypeBox>,
    other_boxes: Vec<OpaqueBox>,
}

impl RestrictedSchemeInfoBox {
    /// Build one from its mandatory scheme type box.
    #[must_use]
    pub fn new(scheme: SchemeTypeBox) -> Self {
        Self {
            original_format: None,
            scheme,
            scheme_info: None,
            compatible_schemes: Vec::new(),
            other_boxes: Vec::new(),
        }
    }

    /// Set the original sample-entry format (`frma`).
    #[must_use]
    pub fn with_original_format(mut self, original_format: OriginalFormatBox) -> Self {
        self.original_format = Some(original_format);
        self
    }

    /// Set the scheme's own data (`schi`).
    #[must_use]
    pub fn with_scheme_info(mut self, scheme_info: SchemeInformationBox) -> Self {
        self.scheme_info = Some(scheme_info);
        self
    }

    /// Append one compatible scheme type (`csch`).
    #[must_use]
    pub fn with_compatible_scheme(mut self, scheme: CompatibleSchemeTypeBox) -> Self {
        self.compatible_schemes.push(scheme);
        self
    }

    /// The original format (`frma`), when the entry was transformed.
    #[must_use]
    pub const fn original_format(&self) -> Option<&OriginalFormatBox> {
        self.original_format.as_ref()
    }

    /// The restriction scheme (`schm`).
    #[must_use]
    pub const fn scheme(&self) -> &SchemeTypeBox {
        &self.scheme
    }

    /// The scheme's own data (`schi`), when present.
    #[must_use]
    pub const fn scheme_info(&self) -> Option<&SchemeInformationBox> {
        self.scheme_info.as_ref()
    }

    /// The compatible scheme types (`csch`), in wire order.
    #[must_use]
    pub fn compatible_schemes(&self) -> &[CompatibleSchemeTypeBox] {
        &self.compatible_schemes
    }

    /// Children the clause does not define, kept verbatim.
    #[must_use]
    pub fn other_boxes(&self) -> &[OpaqueBox] {
        &self.other_boxes
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut original_format = None;
        let mut scheme = None;
        let mut scheme_info = None;
        let mut compatible_schemes = Vec::new();
        let mut other_boxes = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            match &child.four_cc() {
                b"frma" => original_format = Some(OriginalFormatBox::parse(child.bytes)?),
                b"schm" => scheme = Some(parse_scheme_type_box(child.body)?),
                b"schi" => scheme_info = Some(SchemeInformationBox::parse(child.bytes)?),
                b"csch" => compatible_schemes.push(CompatibleSchemeTypeBox::parse(child.bytes)?),
                _ => other_boxes.push(opaque_child(child)),
            }
        }
        let scheme = scheme.ok_or(Error::InvalidInput("rinf without a schm"))?;
        Ok(Self {
            original_format,
            scheme,
            scheme_info,
            compatible_schemes,
            other_boxes,
        })
    }
}

impl<'a> Parse<'a> for RestrictedSchemeInfoBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != RINF {
            return Err(Error::UnexpectedBox { expected: "rinf" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for RestrictedSchemeInfoBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + self
                    .original_format
                    .as_ref()
                    .map_or(0, |b| b.serialized_len())
                + self.scheme.serialized_len()
                + self.scheme_info.as_ref().map_or(0, |b| b.serialized_len())
                + self
                    .compatible_schemes
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
        let mut cursor = write_box_header(buf, &RINF, need)?;
        if let Some(original_format) = &self.original_format {
            cursor += original_format.serialize_into(&mut buf[cursor..])?;
        }
        cursor += self.scheme.serialize_into(&mut buf[cursor..])?;
        if let Some(scheme_info) = &self.scheme_info {
            cursor += scheme_info.serialize_into(&mut buf[cursor..])?;
        }
        for scheme in &self.compatible_schemes {
            cursor += scheme.serialize_into(&mut buf[cursor..])?;
        }
        for other in &self.other_boxes {
            cursor += other.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// Scramble Scheme Information Box (`scrb`) — ISO/IEC 14496-12:2026 §13.4.8.
///
/// A `ProtectionSchemeInfoBox` without the `frma`: the scrambling scheme and
/// its data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ScrambleSchemeInfoBox {
    scheme: SchemeTypeBox,
    scheme_info: Option<SchemeInformationBox>,
    other_boxes: Vec<OpaqueBox>,
}

impl ScrambleSchemeInfoBox {
    /// Build one from its scheme type box.
    #[must_use]
    pub fn new(scheme: SchemeTypeBox) -> Self {
        Self {
            scheme,
            scheme_info: None,
            other_boxes: Vec::new(),
        }
    }

    /// Set the scheme's own data (`schi`).
    #[must_use]
    pub fn with_scheme_info(mut self, scheme_info: SchemeInformationBox) -> Self {
        self.scheme_info = Some(scheme_info);
        self
    }

    /// The scrambling scheme (`schm`).
    #[must_use]
    pub const fn scheme(&self) -> &SchemeTypeBox {
        &self.scheme
    }

    /// The scheme's own data (`schi`), when present.
    #[must_use]
    pub const fn scheme_info(&self) -> Option<&SchemeInformationBox> {
        self.scheme_info.as_ref()
    }

    /// Children the clause does not define, kept verbatim.
    #[must_use]
    pub fn other_boxes(&self) -> &[OpaqueBox] {
        &self.other_boxes
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut scheme = None;
        let mut scheme_info = None;
        let mut other_boxes = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            match &child.four_cc() {
                b"schm" => scheme = Some(parse_scheme_type_box(child.body)?),
                b"schi" => scheme_info = Some(SchemeInformationBox::parse(child.bytes)?),
                _ => other_boxes.push(opaque_child(child)),
            }
        }
        let scheme = scheme.ok_or(Error::InvalidInput("scrb without a schm"))?;
        Ok(Self {
            scheme,
            scheme_info,
            other_boxes,
        })
    }
}

impl<'a> Parse<'a> for ScrambleSchemeInfoBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != SCRB {
            return Err(Error::UnexpectedBox { expected: "scrb" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for ScrambleSchemeInfoBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + self.scheme.serialized_len()
                + self.scheme_info.as_ref().map_or(0, |b| b.serialized_len())
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
        let mut cursor = write_box_header(buf, &SCRB, need)?;
        cursor += self.scheme.serialize_into(&mut buf[cursor..])?;
        if let Some(scheme_info) = &self.scheme_info {
            cursor += scheme_info.serialize_into(&mut buf[cursor..])?;
        }
        for other in &self.other_boxes {
            cursor += other.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// Complete Track Information Box (`cinf`) — ISO/IEC 14496-12:2026 §13.6.3.
///
/// Holds the original format of the complete track an incomplete track was
/// transformed from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CompleteTrackInfoBox {
    original_format: OriginalFormatBox,
    other_boxes: Vec<OpaqueBox>,
}

impl CompleteTrackInfoBox {
    /// Build one from the complete track's sample-entry format.
    #[must_use]
    pub fn new(original_format: OriginalFormatBox) -> Self {
        Self {
            original_format,
            other_boxes: Vec::new(),
        }
    }

    /// The complete track's original format (`frma`).
    #[must_use]
    pub const fn original_format(&self) -> &OriginalFormatBox {
        &self.original_format
    }

    /// Children the clause does not define, kept verbatim.
    #[must_use]
    pub fn other_boxes(&self) -> &[OpaqueBox] {
        &self.other_boxes
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut original_format = None;
        let mut other_boxes = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            if child.four_cc() == *b"frma" {
                original_format = Some(OriginalFormatBox::parse(child.bytes)?);
            } else {
                other_boxes.push(opaque_child(child));
            }
        }
        let original_format = original_format.ok_or(Error::InvalidInput("cinf without a frma"))?;
        Ok(Self {
            original_format,
            other_boxes,
        })
    }
}

impl<'a> Parse<'a> for CompleteTrackInfoBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != CINF {
            return Err(Error::UnexpectedBox { expected: "cinf" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for CompleteTrackInfoBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + self.original_format.serialized_len()
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
        let mut cursor = write_box_header(buf, &CINF, need)?;
        cursor += self.original_format.serialize_into(&mut buf[cursor..])?;
        for other in &self.other_boxes {
            cursor += other.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// Restricted sample entries — §13.5.2 (Table 14)
// ---------------------------------------------------------------------------

/// `revision_level` + `vendor` width in an audio sample entry's fixed part.
const AUDIO_VENDOR_BLOCK_LEN: usize = 6;
/// `samplesize` a default audio entry states.
const DEFAULT_SAMPLESIZE: u16 = 16;

/// Parse a `rinf` child out of a captured child list (§13.5.3).
fn parse_rinf(children: &[crate::box_support::OpaqueBox]) -> Option<RestrictedSchemeInfoBox> {
    let bytes = find_child(children, RINF)?;
    let mut whole = alloc::vec![0u8; BOX_HEADER_MIN_SIZE + bytes.len()];
    let size = broadcast_core::len::fit_u32(whole.len(), "rinf").ok()?;
    whole[..core::mem::size_of::<u32>()].copy_from_slice(&size.to_be_bytes());
    whole[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE].copy_from_slice(&RINF);
    whole[BOX_HEADER_MIN_SIZE..].copy_from_slice(bytes);
    RestrictedSchemeInfoBox::parse(&whole).ok()
}

/// Four-CC of the restricted video sample entry (`resv`, §13.5.2).
pub const RESV: [u8; BOX_TYPE_SIZE] = *b"resv";
/// Four-CC of the restricted audio sample entry (`resa`, §13.5.2).
pub const RESA: [u8; BOX_TYPE_SIZE] = *b"resa";

fn find_child(
    children: &[crate::box_support::OpaqueBox],
    fourcc: [u8; BOX_TYPE_SIZE],
) -> Option<&[u8]> {
    children
        .iter()
        .find(|child| child.box_type == fourcc)
        .map(|child| child.data.as_slice())
}

/// The restricted video sample entry (`resv`) — §13.5.2 Table 14.
///
/// A `VisualSampleEntry` whose four-CC is replaced and a `rinf` added; every
/// other box is left unmodified, so the children are kept verbatim in wire
/// order and `rinf` is parsed out of them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RestrictedVideoSampleEntry {
    visual: crate::sample_entries::VisualSampleEntryFields,
    children: Vec<crate::box_support::OpaqueBox>,
}

impl RestrictedVideoSampleEntry {
    /// The entry's four-CC.
    pub const FOURCC: [u8; BOX_TYPE_SIZE] = RESV;

    /// Build an empty entry with default fixed fields.
    #[must_use]
    pub fn new() -> Self {
        Self {
            visual: crate::sample_entries::VisualSampleEntryFields::default(),
            children: Vec::new(),
        }
    }

    /// The fixed sample description fields.
    #[must_use]
    pub const fn visual(&self) -> &crate::sample_entries::VisualSampleEntryFields {
        &self.visual
    }

    /// The children after the fixed part, `rinf` included, verbatim.
    #[must_use]
    pub fn children(&self) -> &[crate::box_support::OpaqueBox] {
        &self.children
    }

    /// The restriction scheme info (`rinf`, §13.5.3), when present.
    #[must_use]
    pub fn restricted_scheme(&self) -> Option<RestrictedSchemeInfoBox> {
        parse_rinf(&self.children)
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&RESV) {
            return Err(Error::UnexpectedBox { expected: "resv" });
        }
        let mut visual = crate::sample_entries::VisualSampleEntryFields::parse_body(bytes, "resv")?;
        let region =
            crate::sample_entries::VisualSampleEntryFields::children_region(bytes, "resv")?;
        let (children, trailing) = crate::box_support::capture_children(region).into_parts();
        visual.trailing = trailing.into_boxed_slice();
        Ok(Self { visual, children })
    }
}

impl Default for RestrictedVideoSampleEntry {
    fn default() -> Self {
        Self::new()
    }
}

impl Serialize for RestrictedVideoSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE
            + crate::sample_entries::VisualSampleEntryFields::serialized_len()
            + self.visual.trailing.len();
        for child in &self.children {
            n += child.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_box_header(buf, &RESV, need)?;
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

/// The restricted audio sample entry (`resa`) — §13.5.2 Table 14.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RestrictedAudioSampleEntry {
    entry: crate::init_segment::ParsedAudioSampleEntry,
}

impl RestrictedAudioSampleEntry {
    /// The entry's four-CC.
    pub const FOURCC: [u8; BOX_TYPE_SIZE] = RESA;

    /// Build an empty entry with default fixed fields.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entry: crate::init_segment::ParsedAudioSampleEntry {
                reserved: [0; crate::sample_entries::SAMPLE_ENTRY_RESERVED_LEN],
                entry_version: 0,
                reserved_1: [0; AUDIO_VENDOR_BLOCK_LEN],
                data_reference_index: 1,
                channelcount: 1,
                samplesize: DEFAULT_SAMPLESIZE,
                compression_id_and_packet_size: [0; core::mem::size_of::<u32>()],
                samplerate: 0,
                quicktime: None,
                config_boxes: Vec::new(),
                trailing: Vec::new(),
            },
        }
    }

    /// `channelcount`.
    #[must_use]
    pub const fn channelcount(&self) -> u16 {
        self.entry.channelcount
    }

    /// The children after the fixed part, `rinf` included, verbatim.
    #[must_use]
    pub fn children(&self) -> &[crate::box_support::OpaqueBox] {
        &self.entry.config_boxes
    }

    /// The restriction scheme info (`rinf`, §13.5.3), when present.
    #[must_use]
    pub fn restricted_scheme(&self) -> Option<RestrictedSchemeInfoBox> {
        parse_rinf(&self.entry.config_boxes)
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&RESA) {
            return Err(Error::UnexpectedBox { expected: "resa" });
        }
        Ok(Self {
            entry: crate::init_segment::parse_audio_sample_entry(bytes, "resa")?,
        })
    }
}

impl Default for RestrictedAudioSampleEntry {
    fn default() -> Self {
        Self::new()
    }
}

impl Serialize for RestrictedAudioSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::init_segment::audio_sample_entry_serialized_len(
            &self.entry.quicktime,
            &self.entry.config_boxes,
            &self.entry.trailing,
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        crate::init_segment::serialize_audio_sample_entry(
            buf,
            crate::init_segment::AudioSampleEntryFields {
                fourcc: &RESA,
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
