//! ULE Extension Headers — RFC 4326 §5, RFC 5163 §3.
//!
//! Extension headers are chained: each is introduced by a 16-bit Type field
//! (the [`TypeField`] of the *preceding* header, or the SNDU base header's Type
//! for the first one). A Type field `< 0x0600` introduces a further extension
//! header; a Type field `>= 0x0600` is the EtherType of the PDU that follows.
//!
//! H-LEN semantics (RFC 4326 §5):
//!
//! - `H-LEN = 0` — Mandatory Extension Header: length is predefined per H-Type,
//!   not signalled in H-LEN. (Test SNDU 0x00, Bridged-Frame 0x01, TS-Concat
//!   0x02, PDU-Concat 0x03 — these consume the rest of the SNDU payload.)
//! - `H-LEN = 1..=5` — Optional Extension Header: total extension length is
//!   `2 * H-LEN` bytes **including** the 2-byte Type field, so the body is
//!   `2 * H-LEN - 2` bytes.
//! - `H-LEN >= 6` — not a Next-Header (the 16-bit field is itself an
//!   EtherType); handled by [`TypeField`], never reaches this module.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::type_field::TypeField;

/// H-Type of the Test-SNDU mandatory extension header (RFC 4326 §5.1).
pub const H_TYPE_TEST_SNDU: u8 = 0x00;
/// H-Type of the Bridged-Frame mandatory extension header (RFC 4326 §5.2).
pub const H_TYPE_BRIDGED_FRAME: u8 = 0x01;
/// H-Type of the MPEG-2 TS-Concat mandatory extension header (RFC 5163 §3.1).
pub const H_TYPE_TS_CONCAT: u8 = 0x02;
/// H-Type of the PDU-Concat mandatory extension header (RFC 5163 §3.2).
pub const H_TYPE_PDU_CONCAT: u8 = 0x03;
/// H-Type of the TimeStamp optional extension header (RFC 5163 §3.3),
/// decimal 257 → `H-Type` byte `0x01` with `H-LEN = 3`.
pub const H_TYPE_TIMESTAMP: u8 = 0x01;
/// H-Type of the Extension-Padding optional extension header (RFC 4326 §5.3),
/// IANA value `0x100` → `H-Type` byte `0x00`, `H-LEN` 1..=5.
pub const H_TYPE_EXT_PADDING: u8 = 0x00;

/// Minimum legal `H-LEN` for an Optional extension header (RFC 4326 §5).
const OPTIONAL_H_LEN_MIN: u8 = 1;
/// Maximum legal `H-LEN` for an Optional extension header (RFC 4326 §5).
const OPTIONAL_H_LEN_MAX: u8 = 5;

/// Typed H-Type for a Mandatory extension header (`H-LEN = 0`, RFC 4326 §5).
///
/// Mandatory H-Types and Optional H-Types are separate IANA registries; value
/// `0x00` means "Test SNDU" in the mandatory space and "Extension-Padding" in
/// the optional space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MandatoryHType {
    /// Test SNDU — H-Type `0x00` (RFC 4326 §5.1).
    TestSndu,
    /// Bridged Frame — H-Type `0x01` (RFC 4326 §5.2).
    BridgedFrame,
    /// MPEG-2 TS Concatenation — H-Type `0x02` (RFC 5163 §3.1).
    TsConcat,
    /// PDU Concatenation — H-Type `0x03` (RFC 5163 §3.2).
    PduConcat,
    /// An unrecognised mandatory H-Type.
    Other(u8),
}

impl MandatoryHType {
    /// Decode from the raw 8-bit H-Type byte.
    pub fn from_u8(raw: u8) -> Self {
        match raw {
            H_TYPE_TEST_SNDU => MandatoryHType::TestSndu,
            H_TYPE_BRIDGED_FRAME => MandatoryHType::BridgedFrame,
            H_TYPE_TS_CONCAT => MandatoryHType::TsConcat,
            H_TYPE_PDU_CONCAT => MandatoryHType::PduConcat,
            other => MandatoryHType::Other(other),
        }
    }

    /// Encode back to the raw 8-bit H-Type byte.
    pub fn to_u8(self) -> u8 {
        match self {
            MandatoryHType::TestSndu => H_TYPE_TEST_SNDU,
            MandatoryHType::BridgedFrame => H_TYPE_BRIDGED_FRAME,
            MandatoryHType::TsConcat => H_TYPE_TS_CONCAT,
            MandatoryHType::PduConcat => H_TYPE_PDU_CONCAT,
            MandatoryHType::Other(v) => v,
        }
    }

    /// Spec label for this mandatory H-Type.
    pub fn name(&self) -> &'static str {
        match self {
            MandatoryHType::TestSndu => "test-sndu",
            MandatoryHType::BridgedFrame => "bridged-frame",
            MandatoryHType::TsConcat => "ts-concat",
            MandatoryHType::PduConcat => "pdu-concat",
            MandatoryHType::Other(_) => "mandatory",
        }
    }
}

broadcast_common::impl_spec_display!(MandatoryHType, Other);

/// Typed H-Type for an Optional extension header (`H-LEN = 1..=5`, RFC 4326 §5).
///
/// Optional H-Types share the `H-Type` byte namespace with Mandatory H-Types
/// but are distinguished by a non-zero `H-LEN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum OptionalHType {
    /// Extension-Padding — H-Type `0x00`, `H-LEN` 1..=5 (RFC 4326 §5.3).
    ExtPadding,
    /// TimeStamp — H-Type `0x01`, `H-LEN = 3` (RFC 5163 §3.3).
    TimeStamp,
    /// An unrecognised optional H-Type.
    Other(u8),
}

impl OptionalHType {
    /// Decode from the raw 8-bit H-Type byte.
    pub fn from_u8(raw: u8) -> Self {
        match raw {
            H_TYPE_EXT_PADDING => OptionalHType::ExtPadding,
            H_TYPE_TIMESTAMP => OptionalHType::TimeStamp,
            other => OptionalHType::Other(other),
        }
    }

    /// Encode back to the raw 8-bit H-Type byte.
    pub fn to_u8(self) -> u8 {
        match self {
            OptionalHType::ExtPadding => H_TYPE_EXT_PADDING,
            OptionalHType::TimeStamp => H_TYPE_TIMESTAMP,
            OptionalHType::Other(v) => v,
        }
    }

    /// Spec label for this optional H-Type.
    pub fn name(&self) -> &'static str {
        match self {
            OptionalHType::ExtPadding => "extension-padding",
            OptionalHType::TimeStamp => "timestamp",
            OptionalHType::Other(_) => "optional",
        }
    }
}

broadcast_common::impl_spec_display!(OptionalHType, Other);

/// A single ULE extension header in a chain (RFC 4326 §5).
///
/// Each variant carries the `H-Type`/`H-LEN` implicitly; the body bytes that
/// follow the introducing Type field are stored typed where the spec defines a
/// layout, else as opaque bytes for forward compatibility.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ExtensionHeader {
    /// Optional header (`H-LEN = 1..=5`), opaque body of `2 * h_len - 2` bytes.
    ///
    /// Covers TimeStamp, Extension-Padding and any unrecognised optional
    /// header: the body is preserved verbatim so the chain round-trips.
    Optional {
        /// 3-bit length selector (`1..=5`).
        h_len: u8,
        /// 8-bit type code.
        h_type: u8,
        /// Body bytes (`2 * h_len - 2` of them).
        body: Vec<u8>,
    },
    /// Mandatory header (`H-LEN = 0`) whose body consumes the remainder of the
    /// SNDU payload up to (but excluding) the CRC.
    ///
    /// Test SNDU / Bridged-Frame / TS-Concat / PDU-Concat are all of this form;
    /// their inner structure is preserved as opaque bytes (the SNDU `Length`
    /// and CRC give the boundary).
    Mandatory {
        /// 8-bit type code (`0x00`..`0x03` for the RFC-registered set).
        h_type: u8,
        /// Body bytes — everything up to the CRC.
        body: Vec<u8>,
    },
}

impl ExtensionHeader {
    /// The `H-LEN` nibble this header serializes with.
    pub fn h_len(&self) -> u8 {
        match self {
            ExtensionHeader::Optional { h_len, .. } => *h_len,
            ExtensionHeader::Mandatory { .. } => 0,
        }
    }

    /// The `H-Type` byte this header serializes with.
    pub fn h_type(&self) -> u8 {
        match self {
            ExtensionHeader::Optional { h_type, .. } => *h_type,
            ExtensionHeader::Mandatory { h_type, .. } => *h_type,
        }
    }

    /// The introducing [`TypeField`] for this header.
    pub fn type_field(&self) -> TypeField {
        TypeField::NextHeader {
            h_len: self.h_len(),
            h_type: self.h_type(),
        }
    }

    /// `true` if this is a mandatory (`H-LEN = 0`) extension header.
    pub fn is_mandatory(&self) -> bool {
        matches!(self, ExtensionHeader::Mandatory { .. })
    }

    /// The typed [`MandatoryHType`] for a Mandatory header, or `None` if this
    /// is an Optional header.
    pub fn mandatory_h_type(&self) -> Option<MandatoryHType> {
        match self {
            ExtensionHeader::Mandatory { h_type, .. } => Some(MandatoryHType::from_u8(*h_type)),
            ExtensionHeader::Optional { .. } => None,
        }
    }

    /// The typed [`OptionalHType`] for an Optional header, or `None` if this
    /// is a Mandatory header.
    pub fn optional_h_type(&self) -> Option<OptionalHType> {
        match self {
            ExtensionHeader::Optional { h_type, .. } => Some(OptionalHType::from_u8(*h_type)),
            ExtensionHeader::Mandatory { .. } => None,
        }
    }

    /// Spec label for this header kind.
    pub fn name(&self) -> &'static str {
        match self {
            ExtensionHeader::Optional { h_type, .. } => OptionalHType::from_u8(*h_type).name(),
            ExtensionHeader::Mandatory { h_type, .. } => MandatoryHType::from_u8(*h_type).name(),
        }
    }

    /// Validate this header's invariants (ULE-W1, #1120):
    ///
    /// - Optional: `h_len` must be `1..=5` (`0` is Mandatory's own space;
    ///   `6`/`7` are not a legal Optional length — RFC 4326 §5 defines only
    ///   `1..=5`, and encoding one via [`TypeField::to_u16`]'s
    ///   `(h_len & 0x07) << 8` would additionally produce a raw value
    ///   `>= 0x0600`, which [`TypeField::from_u16`] decodes back as an
    ///   `EtherType`, not a Next-Header), and `body.len()` must equal exactly
    ///   `2 * h_len - 2` — the length `h_len` itself declares
    ///   (`Self::wire_len`/the serializer both trust `h_len`, so a
    ///   mismatching `body` misframes the chain, and `h_len == 0` underflows
    ///   computing `wire_len() - 2`).
    /// - Mandatory: always `Ok(())` (no such invariant applies).
    ///
    /// # Errors
    /// [`Error::InvalidExtensionHeader`] if an Optional header's `h_len`/
    /// `body` are inconsistent.
    pub fn validate(&self) -> Result<()> {
        if let ExtensionHeader::Optional { h_len, body, .. } = self {
            if !(OPTIONAL_H_LEN_MIN..=OPTIONAL_H_LEN_MAX).contains(h_len) {
                return Err(Error::InvalidExtensionHeader {
                    reason: "Optional header h_len must be 1..=5",
                });
            }
            let expected_body_len = 2 * usize::from(*h_len) - 2;
            if body.len() != expected_body_len {
                return Err(Error::InvalidExtensionHeader {
                    reason: "Optional header body.len() must equal 2*h_len-2",
                });
            }
        }
        Ok(())
    }

    /// Total wire length of this header *including* its 2-byte introducing Type
    /// field.
    pub fn wire_len(&self) -> usize {
        match self {
            ExtensionHeader::Optional { h_len, .. } => 2 * (*h_len as usize),
            ExtensionHeader::Mandatory { body, .. } => 2 + body.len(),
        }
    }
}

broadcast_common::impl_spec_display!(ExtensionHeader);

/// The decoded payload area of an SNDU (RFC 4326 §5): a chain of extension
/// headers terminated by a final [`TypeField`] (an EtherType, or the
/// introducing Type of a trailing Mandatory header) and the opaque PDU bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PayloadChain<'a> {
    /// Zero or more optional extension headers, in wire order.
    ///
    /// A Mandatory header is always last and is represented by `final_type`
    /// being a Next-Header plus the PDU being its body, so this list only ever
    /// holds Optional headers in the typed-chain model.
    pub headers: Vec<ExtensionHeader>,
    /// The Type field that terminates the optional-header chain: either an
    /// EtherType naming the PDU, or a Next-Header introducing a final Mandatory
    /// header (whose body is `pdu`).
    pub final_type: TypeField,
    /// The opaque PDU bytes (or the Mandatory header's body).
    pub pdu: &'a [u8],
}

impl<'a> PayloadChain<'a> {
    /// Parse a payload chain: walk the optional extension headers, then read
    /// the final Type field and treat everything after it as the PDU.
    ///
    /// `first_type` is the SNDU base header's Type field; `data` is the SNDU
    /// payload area between the base header (+NPA) and the CRC.
    pub fn parse(first_type: TypeField, data: &'a [u8]) -> Result<Self> {
        let mut headers = Vec::new();
        let mut cur = first_type;
        let mut off = 0usize;

        loop {
            match cur {
                TypeField::EtherType(_) => {
                    // Terminal: the rest is the PDU.
                    return Ok(PayloadChain {
                        headers,
                        final_type: cur,
                        pdu: &data[off..],
                    });
                }
                TypeField::NextHeader { h_len, h_type } => {
                    if h_len == 0 {
                        // Mandatory header — body runs to the CRC. Terminal.
                        return Ok(PayloadChain {
                            headers,
                            final_type: cur,
                            pdu: &data[off..],
                        });
                    }
                    // Optional header: total = 2*h_len bytes incl. the 2-byte
                    // Type field that introduced it (already consumed when we
                    // read `cur`, except for the very first which sits in the
                    // base header). The body is 2*h_len - 2 bytes, followed by
                    // the next Type field (2 bytes).
                    let body_len = 2 * (h_len as usize) - 2;
                    let next_type_at = off + body_len;
                    if next_type_at + 2 > data.len() {
                        return Err(Error::InvalidExtensionHeader {
                            reason: "optional extension header body/next-type exceeds payload",
                        });
                    }
                    let body = data[off..next_type_at].to_vec();
                    headers.push(ExtensionHeader::Optional {
                        h_len,
                        h_type,
                        body,
                    });
                    let next_raw = u16::from_be_bytes([data[next_type_at], data[next_type_at + 1]]);
                    cur = TypeField::from_u16(next_raw);
                    off = next_type_at + 2;
                }
            }
        }
    }

    /// Wire length of the chain *excluding* the SNDU base header's Type field
    /// (which the SNDU serializer writes), i.e. the bytes from the first
    /// optional-header body onward, including intervening Type fields, the
    /// final Type field, and the PDU.
    pub fn serialized_len(&self) -> usize {
        // The SNDU base header writes the *first* Type field (`base_type()`),
        // so the chain content here begins at the first header's body. The wire
        // is:  body₀, type₁, body₁, type₂, …, body_{n-1}, final_type, pdu
        // i.e. for N headers: Σ bodyᵢ + N·2 (each body is followed by a 2-byte
        // Type field, the last being `final_type`) + pdu. With zero headers the
        // chain content is just the PDU (`final_type` is the base Type).
        // `wire_len()` already includes the 2-byte Type field; summing it
        // directly (rather than `(wire_len() - 2) + 2`) avoids an underflow
        // panic for a malformed `Optional { h_len: 0, .. }` header, whose
        // `wire_len()` is `0` (ULE-W1, #1120) — `serialize_into` separately
        // rejects such a header via `ExtensionHeader::validate` before
        // trusting this length for anything.
        let mut n = 0usize;
        for h in &self.headers {
            n += h.wire_len();
        }
        n + self.pdu.len()
    }

    /// The Type field the SNDU base header must carry to introduce this chain:
    /// the first optional header's Type, or `final_type` when there are no
    /// optional headers.
    pub fn base_type(&self) -> TypeField {
        match self.headers.first() {
            Some(h) => h.type_field(),
            None => self.final_type,
        }
    }

    /// Serialize the chain into `out`, starting *after* the base header's Type
    /// field. Returns the number of bytes written.
    ///
    /// # Errors
    /// [`Error::InvalidExtensionHeader`] if any header fails
    /// [`ExtensionHeader::validate`] (checked before any bytes are written),
    /// or is a Mandatory header that is not the chain terminator.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        // Validate every header BEFORE writing any bytes (ULE-W1, #1120): an
        // inconsistent `h_len`/`body` would otherwise either misframe the
        // chain (wrong `body.len()` used for both sizing and writing) or, for
        // `h_len == 0`, have already underflowed inside `serialized_len()`.
        for h in &self.headers {
            h.validate()?;
        }
        let need = self.serialized_len();
        if out.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: out.len(),
            });
        }
        // Wire order after the base-header Type field is:
        //   body₀, type₁, body₁, type₂, …, type_final, pdu
        // where typeᵢ introduces headerᵢ and the first header is introduced by
        // the base-header Type field (written by the SNDU serializer).
        let mut off = 0usize;
        for (i, h) in self.headers.iter().enumerate() {
            let body = match h {
                ExtensionHeader::Optional { body, .. } => body,
                ExtensionHeader::Mandatory { .. } => {
                    return Err(Error::InvalidExtensionHeader {
                        reason: "mandatory header must be the chain terminator, not a link",
                    });
                }
            };
            out[off..off + body.len()].copy_from_slice(body);
            off += body.len();
            let following = if i + 1 < self.headers.len() {
                self.headers[i + 1].type_field()
            } else {
                self.final_type
            };
            out[off..off + 2].copy_from_slice(&following.to_u16().to_be_bytes());
            off += 2;
        }
        // When there are no optional headers, `final_type` IS the base Type
        // field (written by the SNDU serializer), so the chain content is just
        // the PDU — nothing extra to write here.
        out[off..off + self.pdu.len()].copy_from_slice(self.pdu);
        off += self.pdu.len();
        Ok(off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // An optional (TimeStamp-shaped, H-LEN=3) header followed by an EtherType
    // terminator round-trips through a full SNDU.
    #[test]
    fn optional_header_chain_round_trip() {
        use crate::sndu::Sndu;

        // TimeStamp: H-LEN=3 (6 bytes total = 2 Type + 4 body), H-Type=0x01.
        let ts = ExtensionHeader::Optional {
            h_len: 3,
            h_type: H_TYPE_TIMESTAMP,
            body: alloc::vec![0xAA, 0xBB, 0xCC, 0xDD],
        };
        assert_eq!(ts.wire_len(), 6);
        let pdu = [0x45u8, 0x00, 0x00, 0x10];
        let chain = PayloadChain {
            headers: alloc::vec![ts.clone()],
            final_type: TypeField::EtherType(0x0800),
            pdu: &pdu,
        };
        // base_type must be the TimeStamp Next-Header (H-LEN=3,H-Type=1)=0x0301.
        assert_eq!(chain.base_type().to_u16(), 0x0301);

        let sndu = Sndu {
            dest_address: None,
            payload: chain.clone(),
        };
        let mut buf = alloc::vec![0u8; sndu.serialized_len()];
        sndu.serialize_into(&mut buf).unwrap();
        let parsed = Sndu::parse(&buf).unwrap();
        assert_eq!(parsed.payload.headers.len(), 1);
        assert_eq!(parsed.payload.headers[0], ts);
        assert_eq!(parsed.payload.final_type, TypeField::EtherType(0x0800));
        assert_eq!(parsed.payload.pdu, &pdu);
        assert_eq!(parsed, sndu);
    }

    // A mandatory header (Test-SNDU, H-LEN=0) terminates the chain; its body is
    // the rest of the payload.
    #[test]
    fn mandatory_header_round_trip() {
        use crate::sndu::Sndu;

        let body = [0xDEu8, 0xAD, 0xBE, 0xEF, 0x00];
        // Base Type field = Mandatory Next-Header: H-LEN=0, H-Type=0x00 -> 0x0000.
        let chain = PayloadChain {
            headers: Vec::new(),
            final_type: TypeField::NextHeader {
                h_len: 0,
                h_type: H_TYPE_TEST_SNDU,
            },
            pdu: &body,
        };
        assert_eq!(chain.base_type().to_u16(), 0x0000);

        let sndu = Sndu {
            dest_address: Some([1, 2, 3, 4, 5, 6]),
            payload: chain,
        };
        let mut buf = alloc::vec![0u8; sndu.serialized_len()];
        sndu.serialize_into(&mut buf).unwrap();
        let parsed = Sndu::parse(&buf).unwrap();
        assert!(parsed.payload.headers.is_empty());
        assert_eq!(
            parsed.payload.final_type,
            TypeField::NextHeader {
                h_len: 0,
                h_type: 0
            }
        );
        assert_eq!(parsed.payload.pdu, &body);
        assert_eq!(parsed, sndu);
    }

    // Two chained optional headers (H-LEN=1 and H-LEN=2) before an EtherType.
    #[test]
    fn two_optional_headers_chain() {
        use crate::sndu::Sndu;

        let h1 = ExtensionHeader::Optional {
            h_len: 1,
            h_type: H_TYPE_EXT_PADDING,
            body: Vec::new(), // 2*1-2 = 0 body bytes
        };
        let h2 = ExtensionHeader::Optional {
            h_len: 2,
            h_type: 0x42,
            body: alloc::vec![0x11, 0x22], // 2*2-2 = 2 body bytes
        };
        let pdu = [0x99u8];
        let chain = PayloadChain {
            headers: alloc::vec![h1.clone(), h2.clone()],
            final_type: TypeField::EtherType(0x86DD),
            pdu: &pdu,
        };
        let sndu = Sndu {
            dest_address: None,
            payload: chain,
        };
        let mut buf = alloc::vec![0u8; sndu.serialized_len()];
        sndu.serialize_into(&mut buf).unwrap();
        let parsed = Sndu::parse(&buf).unwrap();
        assert_eq!(parsed.payload.headers, alloc::vec![h1, h2]);
        assert_eq!(parsed.payload.final_type, TypeField::EtherType(0x86DD));
        assert_eq!(parsed.payload.pdu, &pdu);
        assert_eq!(parsed, sndu);
    }

    // typed H-Type accessors return the expected variants.
    #[test]
    fn typed_h_type_accessors() {
        let ts = ExtensionHeader::Optional {
            h_len: 3,
            h_type: H_TYPE_TIMESTAMP,
            body: alloc::vec![0, 0, 0, 0],
        };
        assert_eq!(ts.optional_h_type(), Some(OptionalHType::TimeStamp));
        assert_eq!(ts.mandatory_h_type(), None);

        let mand = ExtensionHeader::Mandatory {
            h_type: H_TYPE_BRIDGED_FRAME,
            body: alloc::vec![],
        };
        assert_eq!(mand.mandatory_h_type(), Some(MandatoryHType::BridgedFrame));
        assert_eq!(mand.optional_h_type(), None);

        // Other arms
        let unk_m = ExtensionHeader::Mandatory {
            h_type: 0xF0,
            body: alloc::vec![],
        };
        assert_eq!(unk_m.mandatory_h_type(), Some(MandatoryHType::Other(0xF0)));

        let unk_o = ExtensionHeader::Optional {
            h_len: 2,
            h_type: 0xF0,
            body: alloc::vec![0, 0],
        };
        assert_eq!(unk_o.optional_h_type(), Some(OptionalHType::Other(0xF0)));
    }

    // Every H_TYPE_* constant must map to a non-default name() — so a new
    // registered H-Type without a label arm fails CI.
    #[test]
    fn all_h_type_constants_have_non_default_mandatory_label() {
        let mandatory_constants: &[(u8, &str)] = &[
            (H_TYPE_TEST_SNDU, "test-sndu"),
            (H_TYPE_BRIDGED_FRAME, "bridged-frame"),
            (H_TYPE_TS_CONCAT, "ts-concat"),
            (H_TYPE_PDU_CONCAT, "pdu-concat"),
        ];
        for &(raw, expected_label) in mandatory_constants {
            let t = MandatoryHType::from_u8(raw);
            assert_ne!(
                t.name(),
                "mandatory",
                "H_TYPE constant 0x{raw:02X} maps to the default fallback label — add a named arm"
            );
            assert_eq!(
                t.name(),
                expected_label,
                "H_TYPE constant 0x{raw:02X} label mismatch"
            );
        }
    }

    #[test]
    fn all_h_type_constants_have_non_default_optional_label() {
        let optional_constants: &[(u8, &str)] = &[
            (H_TYPE_EXT_PADDING, "extension-padding"),
            (H_TYPE_TIMESTAMP, "timestamp"),
        ];
        for &(raw, expected_label) in optional_constants {
            let t = OptionalHType::from_u8(raw);
            assert_ne!(
                t.name(),
                "optional",
                "optional H_TYPE constant 0x{raw:02X} maps to the default fallback label — add a named arm"
            );
            assert_eq!(
                t.name(),
                expected_label,
                "optional H_TYPE constant 0x{raw:02X} label mismatch"
            );
        }
    }

    /// ULE-W1 (audit issue #1120): `body.len()` longer than `h_len` declares
    /// (`2*h_len-2` bytes). Observed pre-fix: `chain.serialized_len()` sized
    /// the output buffer from `h_len` alone (trusting it over `body.len()`),
    /// so `serialize_into` panicked slicing `out[off..off+body.len()]` past
    /// the end of that too-small buffer.
    #[test]
    fn rejects_body_longer_than_h_len_declares() {
        let bad = ExtensionHeader::Optional {
            h_len: 2, // declares body.len() should be 2*2-2 = 2
            h_type: H_TYPE_EXT_PADDING,
            body: alloc::vec![0xAAu8; 10], // far more than 2
        };
        let chain = PayloadChain {
            headers: alloc::vec![bad],
            final_type: TypeField::EtherType(0x0800),
            pdu: &[],
        };
        let mut buf = alloc::vec![0u8; chain.serialized_len()];
        assert!(matches!(
            chain.serialize_into(&mut buf),
            Err(Error::InvalidExtensionHeader { .. })
        ));
    }

    /// ULE-W1 (audit issue #1120): `body.len()` shorter than `h_len` declares.
    /// Observed pre-fix: `serialize_into` returned `Ok`, but the bytes
    /// written did not match what `h_len` promises a receiver — the
    /// following Type field landed 2 bytes earlier than a receiver walking
    /// `2*h_len` bytes per header would expect, misframing the rest of the
    /// chain (a round-trip asymmetry, not a panic, for this sub-case).
    #[test]
    fn rejects_body_shorter_than_h_len_declares() {
        let bad = ExtensionHeader::Optional {
            h_len: 3, // declares body.len() should be 2*3-2 = 4
            h_type: H_TYPE_TIMESTAMP,
            body: alloc::vec![0xAAu8, 0xBB], // only 2
        };
        let chain = PayloadChain {
            headers: alloc::vec![bad],
            final_type: TypeField::EtherType(0x0800),
            pdu: &[0x01, 0x02],
        };
        let mut buf = alloc::vec![0u8; chain.serialized_len()];
        assert!(matches!(
            chain.serialize_into(&mut buf),
            Err(Error::InvalidExtensionHeader { .. })
        ));
    }

    /// ULE-W1 (audit issue #1120): `h_len == 0` for an `Optional` header (`0`
    /// is Mandatory's own space). Observed pre-fix: `chain.serialized_len()`
    /// itself panicked computing `wire_len() - 2` (`0 - 2`, a `usize`
    /// underflow) — before `serialize_into` was ever reached.
    #[test]
    fn rejects_optional_header_with_h_len_zero() {
        let bad = ExtensionHeader::Optional {
            h_len: 0,
            h_type: H_TYPE_EXT_PADDING,
            body: alloc::vec![],
        };
        let chain = PayloadChain {
            headers: alloc::vec![bad],
            final_type: TypeField::EtherType(0x0800),
            pdu: &[],
        };
        // `serialized_len()` itself must not panic post-fix.
        let mut buf = alloc::vec![0u8; chain.serialized_len() + 16];
        assert!(matches!(
            chain.serialize_into(&mut buf),
            Err(Error::InvalidExtensionHeader { .. })
        ));
    }

    /// ULE-W1 (audit issue #1120): `h_len` of `6`/`7` is not a legal Optional
    /// length — RFC 4326 §5 defines only `1..=5` — and would additionally
    /// encode (via `TypeField::to_u16`) to a raw value `>= 0x0600`, which
    /// decodes back as an `EtherType`, not a Next-Header at all.
    #[test]
    fn rejects_optional_header_with_h_len_six() {
        let bad = ExtensionHeader::Optional {
            h_len: 6,
            h_type: H_TYPE_EXT_PADDING,
            body: alloc::vec![0u8; 10], // 2*6-2 = 10, internally "consistent"
        };
        let chain = PayloadChain {
            headers: alloc::vec![bad],
            final_type: TypeField::EtherType(0x0800),
            pdu: &[],
        };
        let mut buf = alloc::vec![0u8; chain.serialized_len()];
        assert!(matches!(
            chain.serialize_into(&mut buf),
            Err(Error::InvalidExtensionHeader { .. })
        ));
    }
}
