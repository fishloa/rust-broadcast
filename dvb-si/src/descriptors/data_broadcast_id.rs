//! Data Broadcast Id Descriptor — ETSI EN 300 468 §6.2.13 (tag 0x66).
//!
//! Table 32 (PDF p. 72). Identifies the data broadcast specification used by a
//! data component, plus an `id_selector_byte` tail whose interpretation depends
//! on the `data_broadcast_id` (see ETSI TS 101 162).
//!
//! The `id_selector` is decoded into [`IdSelector`]:
//!
//! - `data_broadcast_id = 0x000A` (SSU) → [`IdSelector::Ssu`] carrying a typed
//!   [`SsuIdSelector`] (TS 102 006 §7.1 Table 4).
//! - All other ids → [`IdSelector::Raw`] carrying the raw bytes, preserving
//!   unknown-id behaviour for all non-SSU users.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for data_broadcast_id_descriptor.
pub const TAG: u8 = 0x66;
const HEADER_LEN: usize = 2;
/// Fixed prefix length: the 16-bit data_broadcast_id (EN 300 468 Table 32).
const ID_LEN: usize = 2;

/// `data_broadcast_id` value for DVB System Software Update (TS 102 006).
pub const DATA_BROADCAST_ID_SSU: u16 = 0x000A;

// SSU id_selector constants (TS 102 006 Table 4).
/// OUI_data_length field: 1 byte.
const SSU_OUI_DATA_LENGTH_LEN: usize = 1;
/// Per-OUI fixed fields: OUI(3) + combined-byte-1(1) + combined-byte-2(1) +
/// selector_length(1) = 6 bytes before the selector bytes.
const SSU_OUI_FIXED_LEN: usize = 6;
const SSU_UPDATE_TYPE_MASK: u8 = 0x0F;
const SSU_UPDATE_VERSIONING_FLAG_MASK: u8 = 0x20;
const SSU_UPDATE_VERSION_MASK: u8 = 0x1F;

/// Returns the well-known name for a `data_broadcast_id`, or `None` if the
/// ID is not recognised.
///
/// Verified entries from the DVB Services registry.
#[must_use]
pub fn data_broadcast_id_name(id: u16) -> Option<&'static str> {
    match id {
        0x0005 => Some("Multiprotocol Encapsulation (MPE)"),
        0x0006 => Some("Data Carousel"),
        0x0007 => Some("Object Carousel"),
        DATA_BROADCAST_ID_SSU => Some("System Software Update (SSU)"),
        0x000B => Some("IP/MAC Notification (INT)"),
        0x00F0 => Some("MHP Object Carousel"),
        0x0123 => Some("HbbTV"),
        _ => None,
    }
}

/// One OUI entry in an SSU `id_selector` — TS 102 006 §7.1 Table 4.
///
/// Wire layout per entry (all in bits):
///
/// ```text
/// OUI                       24  bslbf
/// reserved                   4  bslbf
/// update_type                4  uimsbf  (Table 5)
/// reserved                   2  bslbf
/// update_versioning_flag     1  uimsbf
/// update_version             5  uimsbf
/// selector_length            8  uimsbf
/// selector_byte × N          8  uimsbf
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SsuOuiEntry<'a> {
    /// `OUI` — 24-bit IEEE Organizationally Unique Identifier.
    pub oui: [u8; 3],
    /// `update_type` `[3:0]` — TS 102 006 Table 5 coding.
    /// `0x1` = standard update carousel, `0x2` = carousel with UNT, etc.
    pub update_type: u8,
    /// `update_versioning_flag` — when `1`, `update_version` is valid.
    pub update_versioning_flag: bool,
    /// `update_version` `[4:0]` — version counter; only meaningful when
    /// `update_versioning_flag` is set.
    pub update_version: u8,
    /// `selector_byte` loop — additional targeting selector bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub selector: &'a [u8],
}

/// Typed `id_selector` for `data_broadcast_id = 0x000A` (SSU) —
/// TS 102 006 §7.1 Table 4 `system_software_update_info()`.
///
/// Wire layout:
///
/// ```text
/// system_software_update_info() {
///   OUI_data_length  8  uimsbf   (byte count of the OUI loop)
///   for (i=0; i<N; i++) {
///     OUI                       24
///     reserved | update_type     8  (upper 4 = reserved, lower 4 = update_type)
///     reserved | uvf | uversion  8  (upper 2 = reserved, bit5 = uvf, lower 5 = uversion)
///     selector_length            8
///     selector_byte × M          8
///   }
///   private_data_byte × P  8   (remainder after OUI loop)
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SsuIdSelector<'a> {
    /// OUI entries (the `OUI_data_length`-bounded loop).
    pub oui_entries: Vec<SsuOuiEntry<'a>>,
    /// `private_data_byte` tail — bytes after the OUI loop.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

/// Typed or raw `id_selector` — dispatch on `data_broadcast_id`.
///
/// Unknown ids fall through to `Raw`, preserving byte-exact content for
/// non-SSU callers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum IdSelector<'a> {
    /// `data_broadcast_id = 0x000A` — TS 102 006 §7.1 Table 4 SSU selector.
    Ssu(SsuIdSelector<'a>),
    /// All other `data_broadcast_id` values — raw bytes, no interpretation.
    #[cfg_attr(feature = "serde", serde(borrow))]
    Raw(&'a [u8]),
}

impl<'a> IdSelector<'a> {
    /// Parse bytes as the appropriate selector type for `data_broadcast_id`.
    pub fn parse(data_broadcast_id: u16, bytes: &'a [u8]) -> Result<Self> {
        if data_broadcast_id == DATA_BROADCAST_ID_SSU {
            Ok(IdSelector::Ssu(SsuIdSelector::parse(bytes)?))
        } else {
            Ok(IdSelector::Raw(bytes))
        }
    }

    /// Serialized byte length of this selector (not including the surrounding
    /// descriptor tag/length or the 16-bit data_broadcast_id field).
    pub fn serialized_len(&self) -> usize {
        match self {
            IdSelector::Ssu(s) => s.serialized_len(),
            IdSelector::Raw(b) => b.len(),
        }
    }

    /// Write the wire bytes into `buf` starting at offset `pos`.
    pub fn serialize_into_at(&self, buf: &mut [u8], pos: usize) -> Result<usize> {
        match self {
            IdSelector::Ssu(s) => s.serialize_into(&mut buf[pos..]),
            IdSelector::Raw(b) => {
                buf[pos..pos + b.len()].copy_from_slice(b);
                Ok(b.len())
            }
        }
    }
}

impl<'a> Parse<'a> for SsuIdSelector<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < SSU_OUI_DATA_LENGTH_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "SSU id_selector too short for OUI_data_length",
            });
        }
        let oui_data_length = bytes[0] as usize;
        let oui_loop_end = SSU_OUI_DATA_LENGTH_LEN + oui_data_length;
        if oui_loop_end > bytes.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "SSU id_selector OUI_data_length exceeds available bytes",
            });
        }
        let mut pos = SSU_OUI_DATA_LENGTH_LEN;
        let mut oui_entries = Vec::new();
        while pos < oui_loop_end {
            if pos + SSU_OUI_FIXED_LEN > oui_loop_end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "SSU id_selector OUI entry truncated",
                });
            }
            let oui = [bytes[pos], bytes[pos + 1], bytes[pos + 2]];
            let byte3 = bytes[pos + 3]; // reserved(4)|update_type(4)
            let byte4 = bytes[pos + 4]; // reserved(2)|uvf(1)|uversion(5)
            let selector_length = bytes[pos + 5] as usize;
            pos += SSU_OUI_FIXED_LEN;
            if pos + selector_length > oui_loop_end {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "SSU id_selector OUI selector_length exceeds OUI loop",
                });
            }
            let selector = &bytes[pos..pos + selector_length];
            pos += selector_length;
            oui_entries.push(SsuOuiEntry {
                oui,
                update_type: byte3 & SSU_UPDATE_TYPE_MASK,
                update_versioning_flag: (byte4 & SSU_UPDATE_VERSIONING_FLAG_MASK) != 0,
                update_version: byte4 & SSU_UPDATE_VERSION_MASK,
                selector,
            });
        }
        let private_data = &bytes[oui_loop_end..];
        Ok(SsuIdSelector {
            oui_entries,
            private_data,
        })
    }
}

impl Serialize for SsuIdSelector<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let oui_body: usize = self
            .oui_entries
            .iter()
            .map(|e| SSU_OUI_FIXED_LEN + e.selector.len())
            .sum();
        SSU_OUI_DATA_LENGTH_LEN + oui_body + self.private_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let oui_body: usize = self
            .oui_entries
            .iter()
            .map(|e| SSU_OUI_FIXED_LEN + e.selector.len())
            .sum();
        if oui_body > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "SSU OUI loop exceeds 255 bytes (OUI_data_length field overflow)",
            });
        }
        buf[0] = oui_body as u8;
        let mut pos = SSU_OUI_DATA_LENGTH_LEN;
        for e in &self.oui_entries {
            if e.selector.len() > u8::MAX as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "SSU OUI entry selector exceeds 255 bytes",
                });
            }
            buf[pos..pos + 3].copy_from_slice(&e.oui);
            buf[pos + 3] = e.update_type & SSU_UPDATE_TYPE_MASK; // reserved=0 | update_type
            let uvf_bit: u8 = if e.update_versioning_flag {
                SSU_UPDATE_VERSIONING_FLAG_MASK
            } else {
                0
            };
            buf[pos + 4] = uvf_bit | (e.update_version & SSU_UPDATE_VERSION_MASK);
            buf[pos + 5] = e.selector.len() as u8;
            pos += SSU_OUI_FIXED_LEN;
            buf[pos..pos + e.selector.len()].copy_from_slice(e.selector);
            pos += e.selector.len();
        }
        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(len)
    }
}

/// Data Broadcast Id Descriptor (tag 0x66).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DataBroadcastIdDescriptor<'a> {
    /// 16-bit data_broadcast_id (ETSI TS 101 162 registration).
    pub data_broadcast_id: u16,
    /// Raw `id_selector_byte` tail (wire bytes, unchanged on round-trip).
    /// Decode it on demand with [`Self::id_selector_decoded`] — typed for
    /// `data_broadcast_id = 0x000A` (SSU), raw for all other ids.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub id_selector: &'a [u8],
}

impl<'a> DataBroadcastIdDescriptor<'a> {
    /// Decode the `id_selector` tail according to `data_broadcast_id`:
    /// [`IdSelector::Ssu`] for `0x000A` (TS 102 006 §7.1 Table 4), else
    /// [`IdSelector::Raw`]. Decode-on-demand; the raw bytes remain in
    /// [`Self::id_selector`] and round-trip verbatim.
    pub fn id_selector_decoded(&self) -> Result<IdSelector<'a>> {
        IdSelector::parse(self.data_broadcast_id, self.id_selector)
    }
}

impl<'a> Parse<'a> for DataBroadcastIdDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DataBroadcastIdDescriptor",
            "unexpected tag for data_broadcast_id_descriptor",
        )?;
        if body.len() < ID_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "data_broadcast_id_descriptor body shorter than 2 bytes",
            });
        }
        let data_broadcast_id = u16::from_be_bytes([body[0], body[1]]);
        let id_selector = &body[ID_LEN..];
        Ok(Self {
            data_broadcast_id,
            id_selector,
        })
    }
}

impl Serialize for DataBroadcastIdDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ID_LEN + self.id_selector.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let body = ID_LEN + self.id_selector.len();
        if body > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "data_broadcast_id_descriptor body exceeds 255 bytes",
            });
        }
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body as u8;
        buf[HEADER_LEN..HEADER_LEN + ID_LEN].copy_from_slice(&self.data_broadcast_id.to_be_bytes());
        buf[HEADER_LEN + ID_LEN..len].copy_from_slice(self.id_selector);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for DataBroadcastIdDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DATA_BROADCAST_ID";
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── DataBroadcastIdDescriptor (non-SSU raw path) ──────────────────────────

    #[test]
    fn parse_extracts_id_and_raw_selector() {
        let bytes = [TAG, 0x05, 0x00, 0x0B, 0xAA, 0xBB, 0xCC];
        let d = DataBroadcastIdDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.data_broadcast_id, 0x000B);
        assert_eq!(d.id_selector, &[0xAA, 0xBB, 0xCC][..]);
        assert_eq!(
            d.id_selector_decoded().unwrap(),
            IdSelector::Raw(&[0xAA, 0xBB, 0xCC])
        );
    }

    #[test]
    fn parse_accepts_empty_raw_selector() {
        // Non-SSU id with no selector bytes.
        let bytes = [TAG, 0x02, 0x00, 0x0B];
        let d = DataBroadcastIdDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.data_broadcast_id, 0x000B);
        assert!(d.id_selector.is_empty());
        assert_eq!(d.id_selector_decoded().unwrap(), IdSelector::Raw(&[]));
    }

    #[test]
    fn data_broadcast_id_name_verified() {
        assert_eq!(
            data_broadcast_id_name(0x0005),
            Some("Multiprotocol Encapsulation (MPE)")
        );
        assert_eq!(data_broadcast_id_name(0x0006), Some("Data Carousel"));
        assert_eq!(data_broadcast_id_name(0x0007), Some("Object Carousel"));
        assert_eq!(
            data_broadcast_id_name(DATA_BROADCAST_ID_SSU),
            Some("System Software Update (SSU)")
        );
        assert_eq!(
            data_broadcast_id_name(0x000B),
            Some("IP/MAC Notification (INT)")
        );
        assert_eq!(data_broadcast_id_name(0x00F0), Some("MHP Object Carousel"));
        assert_eq!(data_broadcast_id_name(0x0123), Some("HbbTV"));
    }

    #[test]
    fn data_broadcast_id_name_removed_entries_return_none() {
        assert_eq!(data_broadcast_id_name(0x00F1), None);
        assert_eq!(data_broadcast_id_name(0x00F2), None);
        assert_eq!(data_broadcast_id_name(0x00F3), None);
        assert_eq!(data_broadcast_id_name(0x00F4), None);
    }

    #[test]
    fn data_broadcast_id_name_unknown() {
        assert_eq!(data_broadcast_id_name(0x0000), None);
        assert_eq!(data_broadcast_id_name(0xFFFF), None);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = DataBroadcastIdDescriptor::parse(&[0x65, 0x02, 0x00, 0x0A]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x65, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = DataBroadcastIdDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_body_too_short() {
        // length=1: not even the 16-bit id fits.
        let err = DataBroadcastIdDescriptor::parse(&[TAG, 0x01, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_length_overrun() {
        // length=5 but only 3 payload bytes available.
        let err = DataBroadcastIdDescriptor::parse(&[TAG, 0x05, 0x00, 0x0B, 0xAA]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn raw_round_trip() {
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: 0x0123,
            id_selector: &[0xDE, 0xAD, 0xBE, 0xEF],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = DataBroadcastIdDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: 0x0001,
            id_selector: &[0x01],
        };
        let mut tiny = [0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        // 254 selector bytes + 2 id bytes = 256 > 255.
        let sel = vec![0u8; 254];
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: 0x0001,
            id_selector: &sel,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_is_stable() {
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: 0x000B,
            id_selector: &[0x01, 0x02],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"data_broadcast_id\""));
        assert!(json.contains("\"id_selector\""));
        assert!(json.contains("11"));
    }

    // ── SsuIdSelector (data_broadcast_id = 0x000A) ───────────────────────────

    /// Hand-built wire bytes for an SSU id_selector with one OUI entry and
    /// no private data.
    ///
    /// Wire layout (inside the descriptor body, after the 2-byte
    /// data_broadcast_id field):
    ///
    /// ```text
    /// [0]    OUI_data_length = 0x09  (9 bytes = 1 entry × 6 fixed + 3 selector)
    /// [1..4] OUI = 00:15:0A
    /// [4]    0x02 = reserved(4)|update_type(4=0x2, UNT carousel)
    /// [5]    0x61 = reserved(2)|uvf(1=1)|uversion(5=0x01)
    ///              i.e. 0b00_1_00001 = 0x21 ... wait:
    ///              uvf bit5 = 0x20, uversion = 0x01 → 0x21
    /// [6]    selector_length = 0x03
    /// [7..10] selector bytes = AA BB CC
    /// [10]   (OUI loop ends; private_data empty)
    /// Total = 10 bytes
    /// ```
    fn sample_ssu_selector() -> SsuIdSelector<'static> {
        SsuIdSelector {
            oui_entries: vec![SsuOuiEntry {
                oui: [0x00, 0x15, 0x0A],
                update_type: 0x02, // UNT carousel
                update_versioning_flag: true,
                update_version: 0x01,
                selector: &[0xAA, 0xBB, 0xCC],
            }],
            private_data: &[],
        }
    }

    #[test]
    fn ssu_selector_hand_built_byte_anchor() {
        // OUI_data_length=9 (= 6 fixed + 3 selector), OUI=00:15:0A,
        // byte3=0x02 (update_type=2), byte4=0x21 (uvf=1,uversion=1),
        // selector_length=3, AA BB CC.
        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x09,                   // OUI_data_length = 9
            0x00, 0x15, 0x0A,       // OUI
            0x02,                   // reserved(0)|update_type(2)
            0x21,                   // reserved(0)|uvf(1)|uversion(1)
            0x03,                   // selector_length = 3
            0xAA, 0xBB, 0xCC,       // selector bytes
        ];
        let s = sample_ssu_selector();
        let mut buf = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), expected);
        let re = SsuIdSelector::parse(expected).unwrap();
        assert_eq!(re, s);
    }

    #[test]
    fn ssu_selector_round_trip() {
        let s = sample_ssu_selector();
        let mut buf = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut buf).unwrap();
        let re = SsuIdSelector::parse(&buf).unwrap();
        assert_eq!(re, s);
        let mut buf2 = vec![0u8; re.serialized_len()];
        re.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "SSU selector byte-exact re-serialize");
    }

    #[test]
    fn ssu_selector_empty_oui_loop() {
        let s = SsuIdSelector {
            oui_entries: vec![],
            private_data: &[0xDE, 0xAD],
        };
        let mut buf = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut buf).unwrap();
        // OUI_data_length=0, then 2 private bytes.
        assert_eq!(buf[0], 0x00);
        assert_eq!(&buf[1..], &[0xDE, 0xAD]);
        let re = SsuIdSelector::parse(&buf).unwrap();
        assert_eq!(re, s);
    }

    #[test]
    fn ssu_selector_no_versioning_flag() {
        let s = SsuIdSelector {
            oui_entries: vec![SsuOuiEntry {
                oui: [0x00, 0x00, 0x00],
                update_type: 0x01,
                update_versioning_flag: false,
                update_version: 0x00,
                selector: &[],
            }],
            private_data: &[],
        };
        let mut buf = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut buf).unwrap();
        // Wire layout: [0]=OUI_data_length, [1..4]=OUI, [4]=byte3(update_type),
        // [5]=byte4(uvf|uversion), [6]=selector_length.
        assert_eq!(buf[4], 0x01); // byte3: update_type = 1
        assert_eq!(buf[5], 0x00); // byte4: uvf=0, uversion=0
        let re = SsuIdSelector::parse(&buf).unwrap();
        assert_eq!(re, s);
    }

    /// End-to-end: DataBroadcastIdDescriptor with data_broadcast_id=0x000A
    /// dispatches to IdSelector::Ssu.
    ///
    /// Wire layout (complete descriptor):
    ///
    /// ```text
    /// [0]    tag = 0x66
    /// [1]    length = 2 + 10 = 12
    ///           (2 for data_broadcast_id + 10 for SSU selector)
    /// [2..4] data_broadcast_id = 0x00 0x0A
    /// [4..14] SSU id_selector (OUI_data_length=9, …)
    /// ```
    #[test]
    fn descriptor_ssu_round_trip() {
        let sel = sample_ssu_selector();
        let mut sel_bytes = vec![0u8; sel.serialized_len()];
        sel.serialize_into(&mut sel_bytes).unwrap();
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: DATA_BROADCAST_ID_SSU,
            id_selector: &sel_bytes,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // Sanity-check descriptor framing.
        assert_eq!(buf[0], TAG);
        assert_eq!(buf[1] as usize, ID_LEN + sel.serialized_len());
        assert_eq!(&buf[2..4], &[0x00, 0x0A]); // data_broadcast_id
        let re = DataBroadcastIdDescriptor::parse(&buf).unwrap();
        assert_eq!(re, d);
        // The raw id_selector decodes to the typed SSU selector.
        assert_eq!(
            re.id_selector_decoded().unwrap(),
            IdSelector::Ssu(sample_ssu_selector())
        );
        // Byte-identical re-serialize.
        let mut buf2 = vec![0u8; re.serialized_len()];
        re.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "SSU descriptor byte-exact re-serialize");
    }

    #[test]
    fn ssu_selector_parse_rejects_truncated_oui_loop() {
        // OUI_data_length=6 but only 2 bytes follow.
        let bytes = &[0x06, 0x00, 0x15];
        assert!(matches!(
            SsuIdSelector::parse(bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn ssu_selector_parse_rejects_selector_overflows_oui_loop() {
        // OUI_data_length=6 (exactly 1 entry with selector_length=0),
        // but we set selector_length=5 which would overflow the oui loop.
        #[rustfmt::skip]
        let bytes = &[
            0x06,                   // OUI_data_length = 6
            0x00, 0x15, 0x0A,       // OUI
            0x02,                   // byte3
            0x21,                   // byte4
            0x05,                   // selector_length=5 — overflows OUI loop
        ];
        assert!(matches!(
            SsuIdSelector::parse(bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn ssu_descriptor_serde_json() {
        let sel = sample_ssu_selector();
        let mut sel_bytes = vec![0u8; sel.serialized_len()];
        sel.serialize_into(&mut sel_bytes).unwrap();
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: DATA_BROADCAST_ID_SSU,
            id_selector: &sel_bytes,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"data_broadcast_id\":10"));
        // The typed SSU fields appear in the decoded selector's JSON.
        let decoded = d.id_selector_decoded().unwrap();
        let sj = serde_json::to_string(&decoded).unwrap();
        assert!(sj.contains("\"update_type\":2"));
        assert!(sj.contains("\"update_versioning_flag\":true"));
        assert!(sj.contains("\"update_version\":1"));
    }
}
