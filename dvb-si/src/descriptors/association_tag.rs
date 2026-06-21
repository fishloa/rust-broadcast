//! Association Tag Descriptor — ISO/IEC 13818-6 §11.4.1 / ETSI TR 101 202 §4.7.7.2 (tag 0x14).
//!
//! Table 4.18 (TR 101 202 v1.2.1 — the free reproduction of the ISO/IEC
//! 13818-6 DSM-CC descriptor; the ISO text itself cannot be vendored). Carried
//! in the PMT `ES_info` loop to label an elementary stream with a 16-bit
//! `association_tag`, associating all DSM-CC Taps carrying that tag with the
//! stream. The 16-bit cousin of EN 300 468's `stream_identifier_descriptor`
//! (8-bit `component_tag`); the wider tag identifies the PID carrying the
//! object carousel's ServiceGateway so receivers can bootstrap efficiently.
//!
//! Wire layout:
//!
//! ```text
//! association_tag_descriptor() {
//!   descriptor_tag      8   = 0x14
//!   descriptor_length   8
//!   association_tag    16   uimsbf
//!   use                16   uimsbf
//!   selector_length     8   uimsbf
//!   for (i=0; i<selector_length; i++) { selector_byte 8 }
//!   for (i=0; i<N; i++) { private_data_byte 8 }
//! }
//! ```
//!
//! `use` selects the selector semantics: `0x0000` (DSI with IOR of the Service
//! Gateway) → `selector_length` is `0x08` and the 8 selector bytes are
//! `transaction_id` (32) + `timeout` (32, microseconds); `0x0001` →
//! `selector_length` is `0x00`; `0x0100`–`0x1FFF` are DVB-reserved (default
//! `0x0100`); `0x2000`–`0xFFFF` are user-private. Structurally every branch is
//! `selector_length` selector bytes, so a single field-driven layout
//! round-trips all branches byte-exactly. [`transaction_id`] /
//! [`timeout`] expose the two big-endian u32s of the `use == 0x0000` selector.
//!
//! [`transaction_id`]: AssociationTagDescriptor::transaction_id
//! [`timeout`]: AssociationTagDescriptor::timeout

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for association_tag_descriptor.
pub const TAG: u8 = 0x14;

// ── field-width constants ─────────────────────────────────────────────────────
/// 2-byte descriptor outer header (tag + length).
const HEADER_LEN: usize = 2;
/// `association_tag` field width in bytes (16-bit uimsbf).
const ASSOCIATION_TAG_LEN: usize = 2;
/// `use` field width in bytes (16-bit uimsbf).
const USE_LEN: usize = 2;
/// `selector_length` field width in bytes (8-bit uimsbf).
const SELECTOR_LENGTH_LEN: usize = 1;
/// Fixed prefix of every descriptor body: association_tag + use + selector_length.
const BODY_PREFIX_LEN: usize = ASSOCIATION_TAG_LEN + USE_LEN + SELECTOR_LENGTH_LEN;

// ── `use` field values ────────────────────────────────────────────────────────
/// `use == 0x0000`: the PID carries the DownloadServerInitiate (DSI) message
/// with the IOR of the Service Gateway; the selector holds `transaction_id` +
/// `timeout`.
pub const USE_DSI_IOR: u16 = 0x0000;
/// `use == 0x0001`: `selector_length` is `0x00` (no selector bytes).
pub const USE_NO_SELECTOR: u16 = 0x0001;
/// Selector length (bytes) mandated when `use == 0x0000`: `transaction_id` (4)
/// + `timeout` (4).
const DSI_SELECTOR_LEN: usize = 8;

/// Association Tag Descriptor (tag 0x14) — ISO/IEC 13818-6 §11.4.1 /
/// ETSI TR 101 202 §4.7.7.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AssociationTagDescriptor<'a> {
    /// `association_tag` `[15:0]` — the 16-bit tag bound to this elementary
    /// stream's PID.
    pub association_tag: u16,
    /// `use` `[15:0]` — selects the selector semantics (spec field name `use`,
    /// a Rust keyword). See [`USE_DSI_IOR`] / [`USE_NO_SELECTOR`].
    pub usage: u16,
    /// `selector_byte` — the `selector_length` selector bytes. For
    /// `use == 0x0000` these are `transaction_id` (4) + `timeout` (4); use
    /// [`transaction_id`](Self::transaction_id) / [`timeout`](Self::timeout) to
    /// decode them.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub selector: &'a [u8],
    /// `private_data_byte` tail — zero or more bytes after the selector.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

impl<'a> AssociationTagDescriptor<'a> {
    /// The DSI `transaction_id` (`use == 0x0000`): the first 4 selector bytes,
    /// big-endian. `0xFFFFFFFF` means the DSI `transaction_id` is unknown but
    /// all DSI messages on the PID are valid. `None` when `use != 0x0000` or
    /// the selector is not the mandated 8 bytes.
    #[must_use]
    pub fn transaction_id(&self) -> Option<u32> {
        if self.usage == USE_DSI_IOR && self.selector.len() == DSI_SELECTOR_LEN {
            Some(u32::from_be_bytes([
                self.selector[0],
                self.selector[1],
                self.selector[2],
                self.selector[3],
            ]))
        } else {
            None
        }
    }

    /// The DSI acquisition `timeout` in **microseconds** (`use == 0x0000`): the
    /// last 4 selector bytes, big-endian. `0xFFFFFFFF` means no timeout value
    /// is known. `None` when `use != 0x0000` or the selector is not the
    /// mandated 8 bytes.
    #[must_use]
    pub fn timeout(&self) -> Option<u32> {
        if self.usage == USE_DSI_IOR && self.selector.len() == DSI_SELECTOR_LEN {
            Some(u32::from_be_bytes([
                self.selector[4],
                self.selector[5],
                self.selector[6],
                self.selector[7],
            ]))
        } else {
            None
        }
    }
}

impl<'a> Parse<'a> for AssociationTagDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AssociationTagDescriptor",
            "unexpected tag for association_tag_descriptor",
        )?;
        let (prefix, after_prefix) =
            body.split_first_chunk::<BODY_PREFIX_LEN>()
                .ok_or(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "association_tag_descriptor body shorter than 5 bytes",
                })?;
        let association_tag = u16::from_be_bytes([prefix[0], prefix[1]]);
        let usage = u16::from_be_bytes([prefix[2], prefix[3]]);
        let selector_length = prefix[BODY_PREFIX_LEN - SELECTOR_LENGTH_LEN] as usize;
        if selector_length > after_prefix.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "association_tag_descriptor selector_length exceeds descriptor body",
            });
        }
        let selector = &after_prefix[..selector_length];
        let private_data = &after_prefix[selector_length..];
        Ok(Self {
            association_tag,
            usage,
            selector,
            private_data,
        })
    }
}

impl Serialize for AssociationTagDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_PREFIX_LEN + self.selector.len() + self.private_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        let body_len = BODY_PREFIX_LEN + self.selector.len() + self.private_data.len();
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "association_tag_descriptor body exceeds 255 bytes",
            });
        }
        if self.selector.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "association_tag_descriptor selector exceeds 255 bytes",
            });
        }
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        let mut pos = HEADER_LEN;
        buf[pos..pos + ASSOCIATION_TAG_LEN].copy_from_slice(&self.association_tag.to_be_bytes());
        pos += ASSOCIATION_TAG_LEN;
        buf[pos..pos + USE_LEN].copy_from_slice(&self.usage.to_be_bytes());
        pos += USE_LEN;
        buf[pos] = self.selector.len() as u8;
        pos += SELECTOR_LENGTH_LEN;
        buf[pos..pos + self.selector.len()].copy_from_slice(self.selector);
        pos += self.selector.len();
        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(total)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for AssociationTagDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ASSOCIATION_TAG";
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── real fixture: ETSI Hot Bird MHP mux (.test-streams/hotbird-mhp.ts) ────
    //
    // Two association_tag_descriptor instances extracted from the live capture
    // (each appears 13× across the PMTs). Both use the `use == 0x0000` branch.
    //
    // Wire layout of the first instance:
    //   [0]  descriptor_tag      0x14
    //   [1]  descriptor_length   0x0D   = 13 bytes body
    //   [2]  association_tag[15:8] 0x00
    //   [3]  association_tag[7:0]  0x0A  association_tag = 0x000A
    //   [4]  use[15:8]            0x00
    //   [5]  use[7:0]             0x00   use = 0x0000 (DSI with IOR of SGW)
    //   [6]  selector_length      0x08
    //   [7]  transaction_id[31:24] 0x80
    //   [8]  transaction_id[23:16] 0x00
    //   [9]  transaction_id[15:8]  0x00
    //  [10]  transaction_id[7:0]   0x00  transaction_id = 0x80000000
    //  [11]  timeout[31:24]        0x00
    //  [12]  timeout[23:16]        0x18
    //  [13]  timeout[15:8]         0x70
    //  [14]  timeout[7:0]          0x40  timeout = 0x00187040 = 1_602_112 µs
    //
    // Total 15 bytes; body = 13 (0x0D); no private_data.
    #[rustfmt::skip]
    const HOTBIRD_A: &[u8] = &[
        0x14, 0x0D,
        0x00, 0x0A,                         // association_tag = 0x000A
        0x00, 0x00,                         // use = 0x0000
        0x08,                               // selector_length = 8
        0x80, 0x00, 0x00, 0x00,             // transaction_id = 0x80000000
        0x00, 0x18, 0x70, 0x40,             // timeout = 0x00187040
    ];
    // Second live instance: identical but association_tag = 0x000E.
    #[rustfmt::skip]
    const HOTBIRD_B: &[u8] = &[
        0x14, 0x0D,
        0x00, 0x0E,                         // association_tag = 0x000E
        0x00, 0x00,
        0x08,
        0x80, 0x00, 0x00, 0x00,
        0x00, 0x18, 0x70, 0x40,
    ];

    // ── real-fixture parse + typed accessors ──────────────────────────────────

    #[test]
    fn hotbird_parse_extracts_fields_and_typed_accessors() {
        let d = AssociationTagDescriptor::parse(HOTBIRD_A).unwrap();
        assert_eq!(d.association_tag, 0x000A);
        assert_eq!(d.usage, USE_DSI_IOR);
        assert_eq!(
            d.selector,
            &[0x80, 0x00, 0x00, 0x00, 0x00, 0x18, 0x70, 0x40]
        );
        assert!(d.private_data.is_empty());
        assert_eq!(d.transaction_id(), Some(0x8000_0000));
        assert_eq!(d.timeout(), Some(0x0018_7040));

        let d2 = AssociationTagDescriptor::parse(HOTBIRD_B).unwrap();
        assert_eq!(d2.association_tag, 0x000E);
        assert_eq!(d2.transaction_id(), Some(0x8000_0000));
        assert_eq!(d2.timeout(), Some(0x0018_7040));
    }

    #[test]
    fn hotbird_round_trip_byte_identical() {
        for fixture in [HOTBIRD_A, HOTBIRD_B] {
            let d = AssociationTagDescriptor::parse(fixture).unwrap();
            let mut buf = vec![0u8; d.serialized_len()];
            let n = d.serialize_into(&mut buf).unwrap();
            assert_eq!(n, fixture.len());
            assert_eq!(buf.as_slice(), fixture, "round-trip not byte-identical");
        }
    }

    // ── construct-from-fields → hand-computed wire bytes (anti-passthrough) ────

    #[test]
    fn construct_from_fields_matches_hotbird_bytes() {
        // Built purely from typed fields (no parse), must equal the real bytes.
        let d = AssociationTagDescriptor {
            association_tag: 0x000A,
            usage: USE_DSI_IOR,
            selector: &[0x80, 0x00, 0x00, 0x00, 0x00, 0x18, 0x70, 0x40],
            private_data: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), HOTBIRD_A);
    }

    // ── mutation bites: a field change must change the wire bytes ──────────────

    #[test]
    fn mutating_association_tag_changes_wire_bytes() {
        let mut d = AssociationTagDescriptor::parse(HOTBIRD_A).unwrap();
        let mut original = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut original).unwrap();
        d.association_tag = 0x1234;
        let mut mutated = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut mutated).unwrap();
        assert_ne!(original, mutated);
        // Specifically offsets [2..4] (association_tag) differ, nothing else.
        assert_eq!(&mutated[2..4], &[0x12, 0x34]);
        assert_eq!(&original[..2], &mutated[..2]); // tag/length unchanged
        assert_eq!(&original[4..], &mutated[4..]); // use/selector unchanged
    }

    #[test]
    fn mutating_transaction_id_changes_selector_bytes() {
        let mut d = AssociationTagDescriptor::parse(HOTBIRD_A).unwrap();
        // Patch the transaction_id to 0xFFFFFFFF (the "unknown" sentinel).
        let new_selector = [0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x18, 0x70, 0x40];
        d.selector = &new_selector;
        assert_eq!(d.transaction_id(), Some(0xFFFF_FFFF));
        assert_eq!(d.timeout(), Some(0x0018_7040));
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[7..11], &[0xFF, 0xFF, 0xFF, 0xFF]);
    }

    // ── the other two `use` branches ──────────────────────────────────────────

    #[test]
    fn use_no_selector_branch_round_trips() {
        // use == 0x0001 → selector_length 0x00, then private data.
        #[rustfmt::skip]
        let bytes = [
            TAG, 0x08,
            0x12, 0x34,             // association_tag
            0x00, 0x01,             // use = 0x0001
            0x00,                   // selector_length = 0
            0xAA, 0xBB, 0xCC,       // private_data
        ];
        let d = AssociationTagDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.association_tag, 0x1234);
        assert_eq!(d.usage, USE_NO_SELECTOR);
        assert!(d.selector.is_empty());
        assert_eq!(d.private_data, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(d.transaction_id(), None);
        assert_eq!(d.timeout(), None);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes);
    }

    #[test]
    fn user_private_use_with_explicit_selector_and_private_data() {
        // use == 0x2000 (user private) → explicit selector + private tail.
        #[rustfmt::skip]
        let bytes = [
            TAG, 0x0A,
            0x00, 0x05,             // association_tag = 5
            0x20, 0x00,             // use = 0x2000
            0x03,                   // selector_length = 3
            0xDE, 0xAD, 0xBE,       // selector
            0xEF, 0x42,             // private_data
        ];
        let d = AssociationTagDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.association_tag, 5);
        assert_eq!(d.usage, 0x2000);
        assert_eq!(d.selector, &[0xDE, 0xAD, 0xBE]);
        assert_eq!(d.private_data, &[0xEF, 0x42]);
        // Not a DSI selector → no typed transaction_id/timeout.
        assert_eq!(d.transaction_id(), None);
        assert_eq!(d.timeout(), None);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes);
    }

    #[test]
    fn use_zero_with_non_eight_selector_has_no_typed_accessors() {
        // use == 0x0000 but a non-conformant 4-byte selector: accessors None.
        #[rustfmt::skip]
        let bytes = [
            TAG, 0x09,
            0x00, 0x01,
            0x00, 0x00,             // use = 0x0000
            0x04,                   // selector_length = 4 (not the mandated 8)
            0x01, 0x02, 0x03, 0x04,
        ];
        let d = AssociationTagDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.transaction_id(), None);
        assert_eq!(d.timeout(), None);
    }

    // ── error cases ────────────────────────────────────────────────────────────

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = AssociationTagDescriptor::parse(&[0x13, 0x05, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x13, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = AssociationTagDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_body_too_short() {
        // length=4: association_tag + use but no selector_length byte.
        let err =
            AssociationTagDescriptor::parse(&[TAG, 0x04, 0x00, 0x01, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_selector_length_overrun() {
        // selector_length=0x08 but only 2 bytes follow the prefix.
        let err =
            AssociationTagDescriptor::parse(&[TAG, 0x07, 0x00, 0x01, 0x00, 0x00, 0x08, 0x01, 0x02])
                .unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_length_overrun() {
        // Declared length=10 but only 4 body bytes present.
        let err =
            AssociationTagDescriptor::parse(&[TAG, 0x0A, 0x00, 0x01, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = AssociationTagDescriptor::parse(HOTBIRD_A).unwrap();
        let mut tiny = [0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    // ── serde ──────────────────────────────────────────────────────────────────

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_fields_present() {
        let d = AssociationTagDescriptor::parse(HOTBIRD_A).unwrap();
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"association_tag\""));
        assert!(json.contains("\"usage\""));
        assert!(json.contains("\"selector\""));
        assert!(json.contains("\"private_data\""));
    }
}
