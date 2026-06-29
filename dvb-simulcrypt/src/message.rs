//! The generic SimulCrypt message: a 5-byte header + a TLV parameter list.
//!
//! ETSI TS 103 197 V1.5.1 §4.4.1, Table 1b (PDF p. 26). All multi-byte fields
//! are big-endian (Table 1b NOTE 1).
//!
//! ```text
//! generic_message {
//!   protocol_version    1 byte
//!   message_type        2 bytes
//!   message_length      2 bytes   # bytes following this field
//!   for (i=0; i<n; i++) {         # parameters, any order (NOTE 2)
//!     parameter_type    2 bytes
//!     parameter_length  2 bytes   # bytes of parameter_value
//!     parameter_value   <parameter_length> bytes
//!   }
//! }
//! ```
//!
//! `message_length` and each `parameter_length` are **recomputed on serialize**
//! from the typed fields — there is no raw passthrough. Unknown
//! `message_type`/`parameter_type` values decode to the `Reserved(_)`
//! catch-alls (TS 103 197: unknown types "shall be ignored", but this codec
//! preserves them losslessly rather than discarding).

use alloc::vec::Vec;

use broadcast_common::traits::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::registry::{Interface, MessageType, ParameterType};

/// Bytes of the fixed `generic_message` header: protocol_version (1) +
/// message_type (2) + message_length (2).
pub const HEADER_LEN: usize = 5;

/// Bytes of a parameter TLV's fixed prefix: parameter_type (2) +
/// parameter_length (2).
pub const PARAMETER_HEADER_LEN: usize = 4;

/// One TLV parameter of a [`SimulcryptMessage`] (Table 1b): a typed
/// `parameter_type` plus a borrowed, **opaque** `parameter_value`.
///
/// The value is never interpreted — CWs (`CP_CW_combination`/`CW_encryption`),
/// ECMs (`ECM_datagram`) and EMM/private data (`datagram`) are carried as raw
/// bytes. `parameter_length` is recomputed from `value.len()` on serialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Parameter<'a> {
    /// The interface-tagged `parameter_type`.
    pub ptype: ParameterType,
    /// The opaque `parameter_value` bytes (length = `parameter_length`).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub value: &'a [u8],
}

impl<'a> Parameter<'a> {
    /// Construct a parameter from a typed `parameter_type` and a value slice.
    #[must_use]
    pub const fn new(ptype: ParameterType, value: &'a [u8]) -> Self {
        Self { ptype, value }
    }

    /// The total wire length of this TLV (4-byte header + value).
    #[must_use]
    pub const fn wire_len(&self) -> usize {
        PARAMETER_HEADER_LEN + self.value.len()
    }
}

/// A generic SimulCrypt message (TS 103 197 Table 1b): the 5-byte header plus
/// an ordered list of TLV [`Parameter`]s.
///
/// The [`Interface`] is not on the wire — it is fixed by the TCP connection the
/// message arrived on, and is supplied to [`SimulcryptMessage::parse_on`] so the
/// raw `message_type`/`parameter_type` values can be decoded into the right
/// interface-scoped registry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SimulcryptMessage<'a> {
    /// `protocol_version` (Table 2; `0x03` for both implemented interfaces).
    pub protocol_version: u8,
    /// The interface-tagged `message_type`.
    pub message_type: MessageType,
    /// The TLV parameter list, in wire order (order is not significant per
    /// NOTE 2, but is preserved for byte-exact round-tripping).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub parameters: Vec<Parameter<'a>>,
}

impl<'a> SimulcryptMessage<'a> {
    /// Construct a message from typed fields.
    #[must_use]
    pub fn new(
        protocol_version: u8,
        message_type: MessageType,
        parameters: Vec<Parameter<'a>>,
    ) -> Self {
        Self {
            protocol_version,
            message_type,
            parameters,
        }
    }

    /// The interface this message was decoded against (from its `message_type`).
    #[must_use]
    pub const fn interface(&self) -> Interface {
        self.message_type.interface()
    }

    /// Sum of the wire lengths of every parameter TLV — i.e. the value the
    /// `message_length` field will carry on serialize.
    #[must_use]
    pub fn body_len(&self) -> usize {
        self.parameters.iter().map(Parameter::wire_len).sum()
    }

    /// Find the first parameter whose `parameter_type` matches `ptype`.
    #[must_use]
    pub fn find(&self, ptype: ParameterType) -> Option<&Parameter<'a>> {
        self.parameters.iter().find(|p| p.ptype == ptype)
    }

    /// Parse a message off the wire, decoding `message_type`/`parameter_type`
    /// against the given [`Interface`] (the connection's interface).
    ///
    /// # Errors
    /// Returns [`Error::BufferTooShort`] if the header or a TLV header is
    /// truncated, [`Error::InvalidMessageLength`] if `message_length` overruns
    /// the buffer, and [`Error::TruncatedParameter`] if a `parameter_length`
    /// runs past the message body.
    pub fn parse_on(iface: Interface, bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "generic_message header",
            });
        }
        let protocol_version = bytes[0];
        let raw_message_type = u16::from_be_bytes([bytes[1], bytes[2]]);
        let message_length = u16::from_be_bytes([bytes[3], bytes[4]]) as usize;

        let body = &bytes[HEADER_LEN..];
        if body.len() < message_length {
            return Err(Error::InvalidMessageLength {
                length: message_length as u16,
                reason: "message_length exceeds available bytes",
            });
        }
        let body = &body[..message_length];

        let message_type = MessageType::from_u16(iface, raw_message_type);

        let mut parameters = Vec::new();
        let mut off = 0usize;
        while off < body.len() {
            if body.len() - off < PARAMETER_HEADER_LEN {
                return Err(Error::BufferTooShort {
                    need: PARAMETER_HEADER_LEN,
                    have: body.len() - off,
                    what: "parameter TLV header",
                });
            }
            let raw_ptype = u16::from_be_bytes([body[off], body[off + 1]]);
            let plen = u16::from_be_bytes([body[off + 2], body[off + 3]]) as usize;
            let vstart = off + PARAMETER_HEADER_LEN;
            let remaining = body.len() - vstart;
            if remaining < plen {
                return Err(Error::TruncatedParameter {
                    ptype: raw_ptype,
                    need: plen,
                    have: remaining,
                });
            }
            let value = &body[vstart..vstart + plen];
            parameters.push(Parameter::new(
                ParameterType::from_u16(iface, raw_ptype),
                value,
            ));
            off = vstart + plen;
        }

        Ok(Self {
            protocol_version,
            message_type,
            parameters,
        })
    }
}

impl<'a> Parse<'a> for SimulcryptMessage<'a> {
    type Error = Error;

    /// Parse against the ECMG⇔SCS interface by default. Most callers should
    /// use [`SimulcryptMessage::parse_on`] with the connection's interface;
    /// this `Parse` impl exists to satisfy the workspace-wide trait contract.
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_on(Interface::EcmgScs, bytes)
    }
}

impl<'a> Serialize for SimulcryptMessage<'a> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }

        let body_len = self.body_len();
        if body_len > u16::MAX as usize {
            return Err(Error::FieldTooWide {
                what: "message_length",
                value: body_len,
                bits: 16,
            });
        }

        buf[0] = self.protocol_version;
        buf[1..3].copy_from_slice(&self.message_type.to_u16().to_be_bytes());
        buf[3..5].copy_from_slice(&(body_len as u16).to_be_bytes());

        let mut off = HEADER_LEN;
        for p in &self.parameters {
            let plen = p.value.len();
            if plen > u16::MAX as usize {
                return Err(Error::FieldTooWide {
                    what: "parameter_length",
                    value: plen,
                    bits: 16,
                });
            }
            buf[off..off + 2].copy_from_slice(&p.ptype.to_u16().to_be_bytes());
            buf[off + 2..off + 4].copy_from_slice(&(plen as u16).to_be_bytes());
            off += PARAMETER_HEADER_LEN;
            buf[off..off + plen].copy_from_slice(p.value);
            off += plen;
        }

        debug_assert_eq!(off, total);
        Ok(total)
    }
}
