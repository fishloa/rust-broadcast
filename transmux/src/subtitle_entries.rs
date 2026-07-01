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
//! further boxes. Cue samples contain `vttc` boxes with `payl`/`sttg`/`iden` children,
//! and gap samples contain `vtte` boxes — these are sample-payload types, not boxes
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
        // Minimum: box header (8) + SampleEntry reserved+dri (8) + 3 NUL bytes = 19
        if bytes.len() < 19 {
            return Err(Error::BufferTooShort {
                need: 19,
                have: bytes.len(),
                what: "stpp",
            });
        }
        // body starts past the 8-byte box header
        let body = &bytes[8..];
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
            let end = (pos + sz).min(body.len());
            let bt = [body[pos + 4], body[pos + 5], body[pos + 6], body[pos + 7]];
            let data = body[pos + BOX_HDR..end].to_vec();
            extra_boxes.push(crate::init_segment::OpaqueBox::new(bt, data));
            pos += sz;
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
        n
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
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stpp");
        c += 4;
        // SampleEntry: reserved(6) zeros + data_reference_index(16)
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

/// WebVTT Configuration Box (`vttC`) — ISO/IEC 14496-30 §9.2.
///
/// Contains the WebVTT file header block (e.g. `"WEBVTT"`), stored as a raw
/// byte string (no NUL terminator on wire).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct WebVttConfigurationBox {
    /// The WebVTT header block string (e.g. `"WEBVTT"`).
    pub config: String,
}

impl WebVttConfigurationBox {
    /// Construct a new `vttC` box.
    pub fn new(config: impl Into<String>) -> Self {
        Self {
            config: config.into(),
        }
    }

    /// Parse from full box bytes (including 8-byte header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR,
                have: bytes.len(),
                what: "vttC",
            });
        }
        let body = &bytes[BOX_HDR..];
        let config = core::str::from_utf8(body).map_err(|_| Error::InvalidValue {
            field: "vttC config",
            value: 0,
            reason: "invalid UTF-8 in vttC config string",
        })?;
        Ok(Self {
            config: String::from(config),
        })
    }
}

impl<'a> Parse<'a> for WebVttConfigurationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::bare_parse(bytes)
    }
}

impl Serialize for WebVttConfigurationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HDR + self.config.len()
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
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"vttC");
        c += 4;
        buf[c..c + self.config.len()].copy_from_slice(self.config.as_bytes());
        c += self.config.len();
        Ok(c)
    }
}

/// WebVTT Cue Payload Box (`payl`) — ISO/IEC 14496-30 §9.4.
///
/// The cue text payload, stored without NUL terminator.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CuePayloadBox {
    /// The cue text.
    pub cue_text: String,
}

impl CuePayloadBox {
    /// Construct a new `payl` box.
    pub fn new(cue_text: impl Into<String>) -> Self {
        Self {
            cue_text: cue_text.into(),
        }
    }

    /// Parse from full box bytes (including 8-byte header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR,
                have: bytes.len(),
                what: "payl",
            });
        }
        let body = &bytes[BOX_HDR..];
        let s = core::str::from_utf8(body).map_err(|_| Error::InvalidValue {
            field: "payl cue_text",
            value: 0,
            reason: "invalid UTF-8 in payl cue_text",
        })?;
        Ok(Self {
            cue_text: String::from(s),
        })
    }
}

impl<'a> Parse<'a> for CuePayloadBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::bare_parse(bytes)
    }
}

impl Serialize for CuePayloadBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HDR + self.cue_text.len()
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
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"payl");
        c += 4;
        buf[c..c + self.cue_text.len()].copy_from_slice(self.cue_text.as_bytes());
        c += self.cue_text.len();
        Ok(c)
    }
}

/// WebVTT Cue Settings Box (`sttg`) — ISO/IEC 14496-30 §9.4.
///
/// WebVTT cue settings string (position, alignment, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CueSettingsBox {
    /// The cue settings string.
    pub settings: String,
}

impl CueSettingsBox {
    /// Construct a new `sttg` box.
    pub fn new(settings: impl Into<String>) -> Self {
        Self {
            settings: settings.into(),
        }
    }

    /// Parse from full box bytes (including 8-byte header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR,
                have: bytes.len(),
                what: "sttg",
            });
        }
        let body = &bytes[BOX_HDR..];
        let s = core::str::from_utf8(body).map_err(|_| Error::InvalidValue {
            field: "sttg settings",
            value: 0,
            reason: "invalid UTF-8 in sttg settings",
        })?;
        Ok(Self {
            settings: String::from(s),
        })
    }
}

impl<'a> Parse<'a> for CueSettingsBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::bare_parse(bytes)
    }
}

impl Serialize for CueSettingsBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HDR + self.settings.len()
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
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"sttg");
        c += 4;
        buf[c..c + self.settings.len()].copy_from_slice(self.settings.as_bytes());
        c += self.settings.len();
        Ok(c)
    }
}

/// WebVTT Cue ID Box (`iden`) — ISO/IEC 14496-30 §9.4.
///
/// Optional cue identifier string.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CueIdBox {
    /// The cue identifier.
    pub cue_id: String,
}

impl CueIdBox {
    /// Construct a new `iden` box.
    pub fn new(cue_id: impl Into<String>) -> Self {
        Self {
            cue_id: cue_id.into(),
        }
    }

    /// Parse from full box bytes (including 8-byte header).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR,
                have: bytes.len(),
                what: "iden",
            });
        }
        let body = &bytes[BOX_HDR..];
        let s = core::str::from_utf8(body).map_err(|_| Error::InvalidValue {
            field: "iden cue_id",
            value: 0,
            reason: "invalid UTF-8 in iden cue_id",
        })?;
        Ok(Self {
            cue_id: String::from(s),
        })
    }
}

impl<'a> Parse<'a> for CueIdBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::bare_parse(bytes)
    }
}

impl Serialize for CueIdBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HDR + self.cue_id.len()
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
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"iden");
        c += 4;
        buf[c..c + self.cue_id.len()].copy_from_slice(self.cue_id.as_bytes());
        c += self.cue_id.len();
        Ok(c)
    }
}

/// WebVTT Cue Box (`vttc`) — ISO/IEC 14496-30 §9.4.
///
/// A single presented cue. Contains a mandatory `payl` (payload) and
/// optional `sttg` (settings) and `iden` (identifier) children.
/// Used in media sample payloads, not in the sample entry itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VttCueBox {
    /// Required cue payload (`payl`).
    pub payload: CuePayloadBox,
    /// Optional cue settings string (`sttg`).
    pub settings: Option<CueSettingsBox>,
    /// Optional cue identifier (`iden`).
    pub cue_id: Option<CueIdBox>,
}

impl VttCueBox {
    /// Construct a new `vttc` box with a payload.
    pub fn new(payload: impl Into<String>) -> Self {
        Self {
            payload: CuePayloadBox::new(payload),
            settings: None,
            cue_id: None,
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
        let body = &bytes[BOX_HDR..];
        let mut payload: Option<CuePayloadBox> = None;
        let mut settings: Option<CueSettingsBox> = None;
        let mut cue_id: Option<CueIdBox> = None;
        let mut pos = 0usize;
        while pos + BOX_HDR <= body.len() {
            let sz = u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]])
                as usize;
            if sz < BOX_HDR {
                break;
            }
            let end = (pos + sz).min(body.len());
            let child = &body[pos..end];
            let fourcc = &child[4..8];
            match fourcc {
                b"payl" => payload = Some(CuePayloadBox::bare_parse(child)?),
                b"sttg" => settings = Some(CueSettingsBox::bare_parse(child)?),
                b"iden" => cue_id = Some(CueIdBox::bare_parse(child)?),
                _ => {} // unknown children skipped
            }
            pos += sz;
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
        })
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
        n
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
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"vttc");
        c += 4;
        c += self.payload.serialize_into(&mut buf[c..])?;
        if let Some(s) = &self.settings {
            c += s.serialize_into(&mut buf[c..])?;
        }
        if let Some(i) = &self.cue_id {
            c += i.serialize_into(&mut buf[c..])?;
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
        let body = &bytes[BOX_HDR..];
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
            let end = (pos + sz).min(body.len());
            let child = &body[pos..end];
            let fourcc = &child[4..8];
            match fourcc {
                b"vttC" => config = Some(WebVttConfigurationBox::bare_parse(child)?),
                _ => {
                    let mut bt = [0u8; 4];
                    bt.copy_from_slice(fourcc);
                    extra_boxes.push(crate::init_segment::OpaqueBox::new(
                        bt,
                        child[BOX_HDR..].to_vec(),
                    ));
                }
            }
            pos += sz;
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
        n
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
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"wvtt");
        c += 4;
        // SampleEntry: reserved(6) zeros + data_reference_index(16)
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
        let bytes = entry.to_bytes();
        assert_eq!(&bytes[4..8], b"stpp");
        let parsed = XmlSubtitleSampleEntry::bare_parse(&bytes).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn wvtt_sample_entry_round_trip() {
        let entry = WvttSampleEntry::new("WEBVTT");
        let bytes = entry.to_bytes();
        assert_eq!(&bytes[4..8], b"wvtt");
        let parsed = WvttSampleEntry::bare_parse(&bytes).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn vtt_cue_box_round_trip() {
        let cue = VttCueBox::new("Hello World");
        let bytes = cue.to_bytes();
        assert_eq!(&bytes[4..8], b"vttc");
        let parsed = VttCueBox::bare_parse(&bytes).unwrap();
        assert_eq!(parsed.payload.cue_text, "Hello World");
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn vtte_round_trip() {
        let vtte = VttEmptyCueBox;
        let bytes = vtte.to_bytes();
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[4..8], b"vtte");
        let parsed = VttEmptyCueBox::bare_parse(&bytes).unwrap();
        assert_eq!(parsed.to_bytes(), bytes);
    }
}
