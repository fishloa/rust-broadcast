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

/// A single ULE extension header in a chain (RFC 4326 §5).
///
/// Each variant carries the `H-Type`/`H-LEN` implicitly; the body bytes that
/// follow the introducing Type field are stored typed where the spec defines a
/// layout, else as opaque bytes for forward compatibility.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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

    /// Spec label for this header kind.
    pub fn name(&self) -> &'static str {
        match self {
            ExtensionHeader::Optional { h_type, .. } => match *h_type {
                H_TYPE_EXT_PADDING => "extension-padding",
                H_TYPE_TIMESTAMP => "timestamp",
                _ => "optional",
            },
            ExtensionHeader::Mandatory { h_type, .. } => match *h_type {
                H_TYPE_TEST_SNDU => "test-sndu",
                H_TYPE_BRIDGED_FRAME => "bridged-frame",
                H_TYPE_TS_CONCAT => "ts-concat",
                H_TYPE_PDU_CONCAT => "pdu-concat",
                _ => "mandatory",
            },
        }
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

dvb_common::impl_spec_display!(ExtensionHeader);

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
        let mut n = 0usize;
        for h in &self.headers {
            n += (h.wire_len() - 2) + 2;
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
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
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
            type_field: chain.base_type(),
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
            type_field: chain.base_type(),
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
            type_field: chain.base_type(),
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
}
