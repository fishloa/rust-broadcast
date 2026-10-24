//! ISO/IEC 14496-30 subtitle sample entries — `stpp` (TTML/IMSC) and `wvtt` (WebVTT).
//!
//! # Sample entry types
//!
//! | Box type | Spec class                  | Handler | Doc              |
//! |----------|-----------------------------|---------|------------------|
//! | `stpp`   | XMLSubtitleSampleEntry      | `subt`  | ISO/IEC 14496-30 §7.2 |
//! | `wvtt`   | WVTTSampleEntry             | `text`  | ISO/IEC 14496-30 §9.2 |
//!
//! Both follow the `SampleEntry` base (6 reserved bytes + `data_reference_index` u16,
//! ISO/IEC 14496-12 §8.5.2) and carry no `VisualSampleEntry` or `AudioSampleEntry`
//! sub-base — they are plain subtitle sample entries.
//!
//! `stpp` appends three null-terminated strings directly after the SampleEntry header;
//! any remaining bytes are optional boxes preserved verbatim for round-trip.
//!
//! `wvtt` appends a mandatory `vttC` box (WebVTT header block string) and optional
//! further boxes. Cue samples contain `vttc` boxes with `payl`/`sttg`/`iden` children
//! (plus `vtta` boxes for `NOTE` text), and gap samples contain `vtte` boxes — these are sample-payload types, not boxes
//! inside the sample entry, but they are provided here for completeness so callers can
//! build and parse cue sample payloads.

use crate::error::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

const BOX_HDR: usize = 8;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a null-terminated C string from `buf` starting at `pos`.
/// Returns `(string, new_pos)` where `new_pos` points past the NUL byte.
fn parse_cstring(buf: &[u8], pos: usize) -> Result<(String, usize)> {
    let rest = &buf[pos..];
    let nul = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or(Error::BufferTooShort {
            need: pos + rest.len() + 1,
            have: buf.len(),
            what: "null-terminated string",
        })?;
    let s = core::str::from_utf8(&rest[..nul]).map_err(|_| Error::InvalidValue {
        field: "utf8 string",
        value: 0,
        reason: "invalid UTF-8 in null-terminated string",
    })?;
    Ok((String::from(s), pos + nul + 1))
}

/// Serialized length of a null-terminated string (payload + NUL byte).
fn cstring_len(s: &str) -> usize {
    s.len() + 1
}

// ---------------------------------------------------------------------------
// XmlSubtitleSampleEntry — 'stpp' (ISO/IEC 14496-30 §7.2)
// ---------------------------------------------------------------------------

/// XML subtitle sample entry (`stpp`) — ISO/IEC 14496-30 §7.2.
///
/// Carries TTML/IMSC subtitle tracks. Samples are whole XML documents.
/// The sample entry follows `SampleEntry` (reserved `[6]` + `data_reference_index`)
/// then three null-terminated strings identifying the XML namespace family,
/// followed by optional boxes (`btrt`, etc.) preserved verbatim.
///
/// Wire layout (ISO/IEC 14496-30 §7.2):
/// ```text
/// class XMLSubtitleSampleEntry extends SampleEntry('stpp') {
///     string namespace;             // null-terminated
///     string schema_location;       // null-terminated (may be empty)
///     string auxiliary_mime_types;  // null-terminated (may be empty)
///     // optional: BitRateBox, …
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct XmlSubtitleSampleEntry {
    /// Data reference index (SampleEntry field `[6:7]`).
    pub data_reference_index: u16,
    /// Space-separated XML namespace URIs (null-terminated on wire).
    pub namespace: String,
    /// Namespace–schema location pairs, null-terminated (may be empty).
    pub schema_location: String,
    /// Auxiliary MIME types, null-terminated (may be empty).
    pub auxiliary_mime_types: String,
    /// Optional trailing boxes (e.g. `btrt`) preserved verbatim for round-trip.
    pub extra_boxes: Vec<crate::init_segment::OpaqueBox>,
}

impl XmlSubtitleSampleEntry {
    /// Create a new `stpp` sample entry with the given namespace.
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            data_reference_index: 1,
            namespace: namespace.into(),
            schema_location: String::new(),
            auxiliary_mime_types: String::new(),
            extra_boxes: Vec::new(),
        }
    }

    /// Parse from full box bytes (including the 8-byte box header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        // Minimum body: SampleEntry reserved+dri (8) + 3 NUL bytes = 11
        let body = crate::box_types::box_body(bytes, 11, "stpp")?;
        // SampleEntry: reserved(6) + data_reference_index(2)
        let data_reference_index = u16::from_be_bytes([body[6], body[7]]);
        let pos = 8usize; // into body
        let (namespace, pos) = parse_cstring(body, pos)?;
        let (schema_location, pos) = parse_cstring(body, pos)?;
        let (auxiliary_mime_types, mut pos) = parse_cstring(body, pos)?;

        // Collect optional trailing boxes verbatim
        let mut extra_boxes = Vec::new();
        while pos + BOX_HDR <= body.len() {
            let sz = u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]])
                as usize;
            if sz < BOX_HDR {
                break;
            }
            let end = pos.saturating_add(sz).min(body.len());
            let bt = [body[pos + 4], body[pos + 5], body[pos + 6], body[pos + 7]];
            let data = body[pos + BOX_HDR..end].to_vec();
            extra_boxes.push(crate::init_segment::OpaqueBox::new(bt, data));
            pos = pos.saturating_add(sz);
        }

        Ok(Self {
            data_reference_index,
            namespace,
            schema_location,
            auxiliary_mime_types,
            extra_boxes,
        })
    }
}

impl<'a> Parse<'a> for XmlSubtitleSampleEntry {
    type Error = Error;
    /// Parse from full box bytes (including the 8-byte box header).
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::bare_parse(bytes)
    }
}

impl Serialize for XmlSubtitleSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        // box header (8) + reserved(6) + dri(2)
        let mut n = BOX_HDR + 8;
        // three null-terminated strings
        n += cstring_len(&self.namespace);
        n += cstring_len(&self.schema_location);
        n += cstring_len(&self.auxiliary_mime_types);
        // optional extra boxes
        for eb in &self.extra_boxes {
            n += eb.serialized_len();
        }
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        // box header: size(32) + fourcc(32)
        c += crate::box_types::write_box_header(buf, b"stpp", need)?;
        // SampleEntry: reserved(6) zeros + data_reference_index(16). The zeros
        // are written, not skipped: a reused/dirty caller buffer would
        // otherwise keep garbage there (r04-W45).
        buf[c..c + 6].fill(0);
        c += 6;
        buf[c..c + 2].copy_from_slice(&self.data_reference_index.to_be_bytes());
        c += 2;
        // namespace (null-terminated)
        buf[c..c + self.namespace.len()].copy_from_slice(self.namespace.as_bytes());
        c += self.namespace.len();
        buf[c] = 0;
        c += 1;
        // schema_location (null-terminated)
        buf[c..c + self.schema_location.len()].copy_from_slice(self.schema_location.as_bytes());
        c += self.schema_location.len();
        buf[c] = 0;
        c += 1;
        // auxiliary_mime_types (null-terminated)
        buf[c..c + self.auxiliary_mime_types.len()]
            .copy_from_slice(self.auxiliary_mime_types.as_bytes());
        c += self.auxiliary_mime_types.len();
        buf[c] = 0;
        c += 1;
        // extra boxes
        for eb in &self.extra_boxes {
            c += eb.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// WebVTT boxes — vttC, vttc, vtte, payl, sttg, iden
// ---------------------------------------------------------------------------

/// Define a WebVTT box whose body is one UTF-8 string with no terminator
/// (`vttC`, `payl`, `sttg`, `iden`): the struct, `new`, `bare_parse`, `Parse`
/// and the symmetric `Serialize`, identical bar the four-CC and field name.
macro_rules! utf8_text_box {
    (
        $(#[$meta:meta])*
        $name:ident, $fourcc:literal, $cc:literal, $field:ident, $field_doc:literal
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        pub struct $name {
            #[doc = $field_doc]
            pub $field: String,
        }

        impl $name {
            #[doc = concat!("Construct a new `", $fourcc, "` box.")]
            pub fn new($field: impl Into<String>) -> Self {
                Self {
                    $field: $field.into(),
                }
            }

            /// Parse from full box bytes (including 8-byte header).
            pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
                if bytes.len() < BOX_HDR {
                    return Err(Error::BufferTooShort {
                        need: BOX_HDR,
                        have: bytes.len(),
                        what: $fourcc,
                    });
                }
                let body = &bytes[crate::box_types::box_header_len(bytes)?..];
                let text = core::str::from_utf8(body).map_err(|_| Error::InvalidValue {
                    field: concat!($fourcc, " ", stringify!($field)),
                    value: 0,
                    reason: concat!("invalid UTF-8 in ", $fourcc, " ", stringify!($field)),
                })?;
                Ok(Self {
                    $field: String::from(text),
                })
            }
        }

        impl<'a> Parse<'a> for $name {
            type Error = Error;
            fn parse(bytes: &'a [u8]) -> Result<Self> {
                Self::bare_parse(bytes)
            }
        }

        impl Serialize for $name {
            type Error = Error;

            fn serialized_len(&self) -> usize {
                crate::box_types::box_len(BOX_HDR + self.$field.len())
            }

            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let need = self.serialized_len();
                if buf.len() < need {
                    return Err(Error::OutputBufferTooSmall {
                        need,
                        have: buf.len(),
                    });
                }
                let mut c = 0usize;
                c += crate::box_types::write_box_header(buf, $cc, need)?;
                buf[c..c + self.$field.len()].copy_from_slice(self.$field.as_bytes());
                c += self.$field.len();
                Ok(c)
            }
        }
    };
}

utf8_text_box! {
    /// WebVTT Configuration Box (`vttC`) — ISO/IEC 14496-30 §9.2.
    ///
    /// Contains the WebVTT file header block (e.g. `"WEBVTT"`), stored as a raw
    /// byte string (no NUL terminator on wire).
    WebVttConfigurationBox, "vttC", b"vttC", config, "The WebVTT header block string (e.g. `\"WEBVTT\"`)."
}

utf8_text_box! {
    /// WebVTT Cue Payload Box (`payl`) — ISO/IEC 14496-30 §9.4.
    ///
    /// The cue text payload, stored without NUL terminator.
    CuePayloadBox, "payl", b"payl", cue_text, "The cue text."
}

utf8_text_box! {
    /// WebVTT Cue Settings Box (`sttg`) — ISO/IEC 14496-30 §9.4.
    ///
    /// WebVTT cue settings string (position, alignment, etc.).
    CueSettingsBox, "sttg", b"sttg", settings, "The cue settings string."
}

utf8_text_box! {
    /// WebVTT Cue ID Box (`iden`) — ISO/IEC 14496-30 §9.4.
    ///
    /// Optional cue identifier string.
    CueIdBox, "iden", b"iden", cue_id, "The cue identifier."
}

utf8_text_box! {
    /// WebVTT Additional Text Box (`vtta`) — ISO/IEC 14496-30 §9.4.
    ///
    /// A sample-level box carrying WebVTT text that is not a cue, such as a
    /// `NOTE` comment block, as one UTF-8 string with no terminator. MP4Box and
    /// gpac both write it ahead of the cue that follows the comment (observed in
    /// `fixtures/subtitles/`).
    VttAdditionalTextBox, "vtta", b"vtta", text, "The additional text (e.g. a `NOTE` block, without the trailing blank line)."
}

/// Which child of a `vttc` sits at one position of its wire order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueChild {
    Iden,
    Sttg,
    Payl,
    /// Index into [`VttCueBox::extra_children`].
    Extra(usize),
}

/// WebVTT Cue Box (`vttc`) — ISO/IEC 14496-30 §9.4.
///
/// A single presented cue. Contains a mandatory `payl` (payload) and
/// optional `sttg` (settings) and `iden` (identifier) children, plus any child
/// this crate does not model (`vsid` cue source id, `ctim` cue time, ...) kept
/// verbatim in [`extra_children`](Self::extra_children).
///
/// The child order seen on the wire is remembered, so a parsed cue serializes
/// to the same bytes: MP4Box and gpac both write `iden`, `sttg`, `payl`, which
/// is also the order of the spec's class (the identifier and settings precede
/// the payload). A cue built with [`new`](Self::new) is written in that order
/// too, after any extra children.
/// Used in media sample payloads, not in the sample entry itself.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VttCueBox {
    /// Required cue payload (`payl`).
    pub payload: CuePayloadBox,
    /// Optional cue settings string (`sttg`).
    pub settings: Option<CueSettingsBox>,
    /// Optional cue identifier string (`iden`).
    pub cue_id: Option<CueIdBox>,
    /// Children this crate has no typed model for, verbatim (box header
    /// preserved through [`OpaqueBox`](crate::init_segment::OpaqueBox)).
    pub extra_children: Vec<crate::init_segment::OpaqueBox>,
    /// Wire order of the children of a parsed cue; empty for a built one.
    #[cfg_attr(feature = "serde", serde(skip))]
    child_order: Vec<CueChild>,
}

/// Two cues are equal when their content is: the remembered wire order of the
/// children does not take part (a cue read from the wire equals the same cue
/// built with [`VttCueBox::new`]).
impl PartialEq for VttCueBox {
    fn eq(&self, other: &Self) -> bool {
        self.payload == other.payload
            && self.settings == other.settings
            && self.cue_id == other.cue_id
            && self.extra_children == other.extra_children
    }
}

impl Eq for VttCueBox {}

impl VttCueBox {
    /// Construct a new `vttc` box with a payload.
    pub fn new(payload: impl Into<String>) -> Self {
        Self {
            payload: CuePayloadBox::new(payload),
            settings: None,
            cue_id: None,
            extra_children: Vec::new(),
            child_order: Vec::new(),
        }
    }

    /// Parse from full box bytes (including 8-byte header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR,
                have: bytes.len(),
                what: "vttc",
            });
        }
        let body = &bytes[crate::box_types::box_header_len(bytes)?..];
        let mut payload: Option<CuePayloadBox> = None;
        let mut settings: Option<CueSettingsBox> = None;
        let mut cue_id: Option<CueIdBox> = None;
        let mut extra_children: Vec<crate::init_segment::OpaqueBox> = Vec::new();
        let mut child_order: Vec<CueChild> = Vec::new();
        let mut pos = 0usize;
        while pos + BOX_HDR <= body.len() {
            let sz = u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]])
                as usize;
            if sz < BOX_HDR {
                break;
            }
            let end = pos.saturating_add(sz).min(body.len());
            let child = &body[pos..end];
            let fourcc = &child[4..8];
            // ISO/IEC 14496-30 `VTTCueBox` lists each of `iden`/`sttg`/`payl`
            // once (docs/codec/captions-14496-30.md): a repeat cannot be held
            // in the typed fields (the last would win while the order listed
            // both, so `serialized_len` and `serialize_into` disagreed), so
            // it is rejected.
            match fourcc {
                b"payl" => {
                    if payload.is_some() {
                        return Err(duplicate_cue_child("payl"));
                    }
                    payload = Some(CuePayloadBox::bare_parse(child)?);
                    child_order.push(CueChild::Payl);
                }
                b"sttg" => {
                    if settings.is_some() {
                        return Err(duplicate_cue_child("sttg"));
                    }
                    settings = Some(CueSettingsBox::bare_parse(child)?);
                    child_order.push(CueChild::Sttg);
                }
                b"iden" => {
                    if cue_id.is_some() {
                        return Err(duplicate_cue_child("iden"));
                    }
                    cue_id = Some(CueIdBox::bare_parse(child)?);
                    child_order.push(CueChild::Iden);
                }
                other => {
                    let mut bt = [0u8; 4];
                    bt.copy_from_slice(other);
                    child_order.push(CueChild::Extra(extra_children.len()));
                    extra_children.push(crate::init_segment::OpaqueBox::new(
                        bt,
                        child[BOX_HDR..].to_vec(),
                    ));
                }
            }
            pos = pos.saturating_add(sz);
        }
        let payload = payload.ok_or(Error::BufferTooShort {
            need: 0,
            have: 0,
            what: "vttc missing required payl child",
        })?;
        Ok(Self {
            payload,
            settings,
            cue_id,
            extra_children,
            child_order,
        })
    }

    /// The children in the order they are written: the remembered wire order
    /// for the ones that are still present, then whatever was added since
    /// (extras, then `iden`, `sttg`, `payl`).
    fn write_order(&self) -> Vec<CueChild> {
        let present = |c: &CueChild| match c {
            CueChild::Iden => self.cue_id.is_some(),
            CueChild::Sttg => self.settings.is_some(),
            CueChild::Payl => true,
            CueChild::Extra(i) => *i < self.extra_children.len(),
        };
        let mut order: Vec<CueChild> = self.child_order.iter().copied().filter(present).collect();
        let mut add = |c: CueChild| {
            if present(&c) && !order.contains(&c) {
                order.push(c);
            }
        };
        for i in 0..self.extra_children.len() {
            add(CueChild::Extra(i));
        }
        add(CueChild::Iden);
        add(CueChild::Sttg);
        add(CueChild::Payl);
        order
    }
}

/// A `vttc` child that the class admits at most once appeared a second time.
fn duplicate_cue_child(field: &'static str) -> Error {
    Error::InvalidValue {
        field,
        value: 2,
        reason: "vttc admits at most one such child (ISO/IEC 14496-30 VTTCueBox)",
    }
}

impl<'a> Parse<'a> for VttCueBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::bare_parse(bytes)
    }
}

impl Serialize for VttCueBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + self.payload.serialized_len();
        if let Some(s) = &self.settings {
            n += s.serialized_len();
        }
        if let Some(i) = &self.cue_id {
            n += i.serialized_len();
        }
        for e in &self.extra_children {
            n += BOX_HDR + e.data.len();
        }
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        c += crate::box_types::write_box_header(buf, b"vttc", need)?;
        for child in self.write_order() {
            match child {
                CueChild::Iden => {
                    if let Some(i) = &self.cue_id {
                        c += i.serialize_into(&mut buf[c..])?;
                    }
                }
                CueChild::Sttg => {
                    if let Some(s) = &self.settings {
                        c += s.serialize_into(&mut buf[c..])?;
                    }
                }
                CueChild::Payl => c += self.payload.serialize_into(&mut buf[c..])?,
                CueChild::Extra(i) => {
                    let e = &self.extra_children[i];
                    let len = BOX_HDR + e.data.len();
                    c += crate::box_types::write_box_header(&mut buf[c..], &e.box_type, len)?;
                    buf[c..c + e.data.len()].copy_from_slice(&e.data);
                    c += e.data.len();
                }
            }
        }
        Ok(c)
    }
}

/// WebVTT Empty Cue Box (`vtte`) — ISO/IEC 14496-30 §9.4.
///
/// A gap sample with no active cue. Wire: 8-byte header only (size=8, type=`vtte`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VttEmptyCueBox;

impl VttEmptyCueBox {
    /// Parse from full box bytes (including 8-byte header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR,
                have: bytes.len(),
                what: "vtte",
            });
        }
        Ok(Self)
    }
}

impl<'a> Parse<'a> for VttEmptyCueBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::bare_parse(bytes)
    }
}

impl Serialize for VttEmptyCueBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HDR
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = BOX_HDR;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[..4].copy_from_slice(&(BOX_HDR as u32).to_be_bytes());
        buf[4..8].copy_from_slice(b"vtte");
        Ok(BOX_HDR)
    }
}

// ---------------------------------------------------------------------------
// WebVTT sample payloads — a sequence of vttc / vtte / vtta boxes
// ---------------------------------------------------------------------------

/// One box of a `wvtt` media sample (ISO/IEC 14496-30 §9.4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum WvttSampleBox {
    /// `vttc`: a cue active during the sample.
    Cue(VttCueBox),
    /// `vtte`: no cue is active (a gap).
    Empty(VttEmptyCueBox),
    /// `vtta`: additional non-cue text such as a `NOTE` block.
    AdditionalText(VttAdditionalTextBox),
    /// Any other box, verbatim.
    Other(crate::init_segment::OpaqueBox),
}

/// Split a `wvtt` media sample into its boxes.
///
/// A sample is a plain sequence of boxes (no wrapper): one or more `vttc` (a
/// cue each; overlapping cues give several), `vtta` (a `NOTE` comment) or a
/// single `vtte` (a gap). Unknown box types are kept as
/// [`WvttSampleBox::Other`]. A box whose size field runs past the sample, or is
/// smaller than its header, is an error, as is a trailing partial header.
pub fn parse_wvtt_sample(sample: &[u8]) -> Result<Vec<WvttSampleBox>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < sample.len() {
        let rest = &sample[pos..];
        if rest.len() < BOX_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR,
                have: rest.len(),
                what: "wvtt sample box header",
            });
        }
        let size = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if size < BOX_HDR || size > rest.len() {
            return Err(Error::InvalidValue {
                field: "wvtt sample box size",
                value: size as u64,
                reason: "box size is below the 8-byte header or past the end of the sample",
            });
        }
        let boxed = &rest[..size];
        out.push(match &boxed[4..8] {
            b"vttc" => WvttSampleBox::Cue(VttCueBox::bare_parse(boxed)?),
            b"vtte" => WvttSampleBox::Empty(VttEmptyCueBox::bare_parse(boxed)?),
            b"vtta" => WvttSampleBox::AdditionalText(VttAdditionalTextBox::bare_parse(boxed)?),
            other => {
                let mut bt = [0u8; 4];
                bt.copy_from_slice(other);
                WvttSampleBox::Other(crate::init_segment::OpaqueBox::new(
                    bt,
                    boxed[BOX_HDR..].to_vec(),
                ))
            }
        });
        pos = pos.saturating_add(size);
    }
    Ok(out)
}

impl Serialize for WvttSampleBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            WvttSampleBox::Cue(b) => b.serialized_len(),
            WvttSampleBox::Empty(b) => b.serialized_len(),
            WvttSampleBox::AdditionalText(b) => b.serialized_len(),
            WvttSampleBox::Other(b) => BOX_HDR + b.data.len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            WvttSampleBox::Cue(b) => b.serialize_into(buf),
            WvttSampleBox::Empty(b) => b.serialize_into(buf),
            WvttSampleBox::AdditionalText(b) => b.serialize_into(buf),
            WvttSampleBox::Other(b) => {
                let need = BOX_HDR + b.data.len();
                if buf.len() < need {
                    return Err(Error::OutputBufferTooSmall {
                        need,
                        have: buf.len(),
                    });
                }
                let mut c = crate::box_types::write_box_header(buf, &b.box_type, need)?;
                buf[c..c + b.data.len()].copy_from_slice(&b.data);
                c += b.data.len();
                Ok(c)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// WVTTSampleEntry — 'wvtt' (ISO/IEC 14496-30 §9.2)
// ---------------------------------------------------------------------------

/// WebVTT sample entry (`wvtt`) — ISO/IEC 14496-30 §9.2.
///
/// Carries WebVTT subtitle tracks. Samples contain `vttc` or `vtte` boxes.
/// The sample entry follows `SampleEntry` (reserved `[6]` + `data_reference_index`)
/// then a mandatory `vttC` configuration box, followed by optional boxes preserved
/// verbatim for round-trip (e.g. `vlab`, `btrt`).
///
/// Wire layout (ISO/IEC 14496-30 §9.2):
/// ```text
/// class WVTTSampleEntry extends SampleEntry('wvtt') {
///     WebVTTConfigurationBox config;  // 'vttC'
///     // optional: WebVTTSourceLabelBox 'vlab', BitRateBox 'btrt', ...
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct WvttSampleEntry {
    /// Data reference index (SampleEntry field `[6:7]`).
    pub data_reference_index: u16,
    /// WebVTT configuration box (`vttC`), mandatory.
    pub config: WebVttConfigurationBox,
    /// Optional trailing boxes preserved verbatim for round-trip.
    pub extra_boxes: Vec<crate::init_segment::OpaqueBox>,
}

impl WvttSampleEntry {
    /// Construct a new `wvtt` sample entry with the given WebVTT header config.
    pub fn new(config: impl Into<String>) -> Self {
        Self {
            data_reference_index: 1,
            config: WebVttConfigurationBox::new(config),
            extra_boxes: Vec::new(),
        }
    }

    /// Parse from full box bytes (including 8-byte header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        // Minimum: box header (8) + SampleEntry reserved+dri (8) + vttC header (8) = 24
        if bytes.len() < 24 {
            return Err(Error::BufferTooShort {
                need: 24,
                have: bytes.len(),
                what: "wvtt",
            });
        }
        let body = &bytes[crate::box_types::box_header_len(bytes)?..];
        let data_reference_index = u16::from_be_bytes([body[6], body[7]]);

        // Walk child boxes in body[8..]
        let mut pos = 8usize;
        let mut config: Option<WebVttConfigurationBox> = None;
        let mut extra_boxes: Vec<crate::init_segment::OpaqueBox> = Vec::new();

        while pos + BOX_HDR <= body.len() {
            let sz = u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]])
                as usize;
            if sz < BOX_HDR {
                break;
            }
            let end = pos.saturating_add(sz).min(body.len());
            let child = &body[pos..end];
            let fourcc = &child[4..8];
            match fourcc {
                b"vttC" => config = Some(WebVttConfigurationBox::bare_parse(child)?),
                _ => {
                    let mut bt = [0u8; 4];
                    bt.copy_from_slice(fourcc);
                    extra_boxes.push(crate::init_segment::OpaqueBox::new(
                        bt,
                        child[crate::box_types::box_header_len(child)?..].to_vec(),
                    ));
                }
            }
            pos = pos.saturating_add(sz);
        }

        let config = config.ok_or(Error::BufferTooShort {
            need: 0,
            have: 0,
            what: "wvtt missing required vttC child",
        })?;

        Ok(Self {
            data_reference_index,
            config,
            extra_boxes,
        })
    }
}

impl<'a> Parse<'a> for WvttSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::bare_parse(bytes)
    }
}

impl Serialize for WvttSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        // box header (8) + SampleEntry reserved+dri (8) + vttC + extra boxes
        let mut n = BOX_HDR + 8 + self.config.serialized_len();
        for eb in &self.extra_boxes {
            n += eb.serialized_len();
        }
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        c += crate::box_types::write_box_header(buf, b"wvtt", need)?;
        // SampleEntry: reserved(6) zeros + data_reference_index(16). Written
        // explicitly so a reused/dirty caller buffer cannot leave garbage in the
        // reserved bytes (r04-W45).
        buf[c..c + 6].fill(0);
        c += 6;
        buf[c..c + 2].copy_from_slice(&self.data_reference_index.to_be_bytes());
        c += 2;
        // vttC configuration box
        c += self.config.serialize_into(&mut buf[c..])?;
        // optional extra boxes
        for eb in &self.extra_boxes {
            c += eb.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;

    #[test]
    fn xml_subtitle_sample_entry_round_trip() {
        let entry = XmlSubtitleSampleEntry::new("http://www.w3.org/ns/ttml");
        let bytes = entry.try_to_bytes().unwrap();
        assert_eq!(&bytes[4..8], b"stpp");
        let parsed = XmlSubtitleSampleEntry::bare_parse(&bytes).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn wvtt_sample_entry_round_trip() {
        let entry = WvttSampleEntry::new("WEBVTT");
        let bytes = entry.try_to_bytes().unwrap();
        assert_eq!(&bytes[4..8], b"wvtt");
        let parsed = WvttSampleEntry::bare_parse(&bytes).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn vtt_cue_box_round_trip() {
        let cue = VttCueBox::new("Hello World");
        let bytes = cue.try_to_bytes().unwrap();
        assert_eq!(&bytes[4..8], b"vttc");
        let parsed = VttCueBox::bare_parse(&bytes).unwrap();
        assert_eq!(parsed.payload.cue_text, "Hello World");
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    /// A `vttc` with children this crate does not model (`vsid`, `ctim`) and in
    /// an order other than iden/sttg/payl keeps every child and its position
    /// through parse -> serialize (spec-vector: no producer on hand writes
    /// `vsid`/`ctim`), and a cue built from scratch is written iden, sttg, payl.
    #[test]
    fn vttc_keeps_unmodelled_children_in_wire_order() {
        fn child(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut v = ((8 + body.len()) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(ty);
            v.extend_from_slice(body);
            v
        }
        let mut body = Vec::new();
        body.extend(child(b"vsid", &[0, 0, 0, 7]));
        body.extend(child(b"payl", b"text"));
        body.extend(child(b"ctim", b"00:00:01.000"));
        body.extend(child(b"iden", b"id1"));
        let wire = child(b"vttc", &body);
        let cue = VttCueBox::bare_parse(&wire).unwrap();
        assert_eq!(cue.payload.cue_text, "text");
        assert_eq!(cue.cue_id.as_ref().unwrap().cue_id, "id1");
        assert_eq!(cue.extra_children.len(), 2);
        assert_eq!(&cue.extra_children[0].box_type, b"vsid");
        assert_eq!(cue.try_to_bytes().unwrap(), wire, "wire order preserved");

        let mut built = VttCueBox::new("p");
        built.settings = Some(CueSettingsBox::new("align:end"));
        built.cue_id = Some(CueIdBox::new("i"));
        let bytes = built.try_to_bytes().unwrap();
        let pos = |needle: &[u8]| bytes.windows(4).position(|w| w == needle).unwrap();
        assert!(pos(b"iden") < pos(b"sttg") && pos(b"sttg") < pos(b"payl"));
        assert_eq!(VttCueBox::bare_parse(&bytes).unwrap(), built);
    }

    #[test]
    fn vtta_round_trips_a_note_block() {
        let b = VttAdditionalTextBox::new("NOTE hello");
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(&bytes[4..8], b"vtta");
        assert_eq!(VttAdditionalTextBox::bare_parse(&bytes).unwrap(), b);
        let sample = [bytes.clone(), VttEmptyCueBox.try_to_bytes().unwrap()].concat();
        let parsed = parse_wvtt_sample(&sample).unwrap();
        assert!(matches!(parsed[0], WvttSampleBox::AdditionalText(_)));
        assert!(matches!(parsed[1], WvttSampleBox::Empty(_)));
        // A box that runs past the sample is an error, not a silent truncation.
        assert!(parse_wvtt_sample(&bytes[..bytes.len() - 1]).is_err());
        assert!(parse_wvtt_sample(&[0, 0, 0, 4, b'v', b't', b't', b'e']).is_err());
    }

    #[test]
    fn vtte_round_trip() {
        let vtte = VttEmptyCueBox;
        let bytes = vtte.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[4..8], b"vtte");
        let parsed = VttEmptyCueBox::bare_parse(&bytes).unwrap();
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    /// r04-W45: the `stpp`/`wvtt` SampleEntry reserved 6 bytes must be written
    /// as zeros, not skipped — `Serialize` accepts any `&mut [u8]`, so a dirty
    /// caller buffer would otherwise leak garbage into a reserved field.
    /// Serializing into a 0xFF-filled buffer must equal `to_bytes()`.
    #[test]
    fn reserved_bytes_written_as_zeros_into_a_dirty_buffer() {
        fn check<T: Serialize>(value: &T, what: &str)
        where
            <T as Serialize>::Error: core::fmt::Debug,
        {
            let clean = value.try_to_bytes().unwrap();
            let mut dirty = alloc::vec![0xFFu8; clean.len()];
            value
                .serialize_into(&mut dirty)
                .expect("serialize into a correctly-sized buffer");
            assert_eq!(dirty, clean, "{what}: dirty-buffer bytes must match");
            // The six reserved bytes sit right after the 8-byte box header.
            assert_eq!(&clean[8..14], &[0u8; 6], "{what}: 6 reserved bytes");
        }
        check(
            &XmlSubtitleSampleEntry::new("http://www.w3.org/ns/ttml"),
            "stpp",
        );
        check(&WvttSampleEntry::new("WEBVTT"), "wvtt");
    }
}
