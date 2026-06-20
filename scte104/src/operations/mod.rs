//! SCTE 104 operations (request/response data structures).
//!
//! Every operation from Tables 8-3 and 8-4 of ANSI/SCTE 104 2023 is
//! implemented. Operations are organized by usage category:
//!
//! - **Single-operation (basic):** `general_response`, `init_request`,
//!   `init_response`, `alive_request`, `alive_response`, `inject_response`,
//!   `inject_complete_response`.
//! - **Multi-operation (Normal):** `splice_request`, `splice_null_request`,
//!   `start_schedule_download`, `time_signal_request`, `transmit_schedule`,
//!   `proprietary_command`, `inject_section_data`.
//! - **Multi-operation (Supplemental):** `component_mode_DPI`,
//!   `encrypted_DPI`, `insert_descriptor`, `insert_DTMF_descriptor`,
//!   `insert_avail_descriptor`, `insert_segmentation_descriptor`,
//!   `schedule_component_mode`, `schedule_definition`, `insert_tier`,
//!   `insert_time_descriptor`, `insert_audio_descriptor`,
//!   `insert_alternate_break_duration`.
//! - **Multi-operation (Control):** `insert_audio_provisioning`,
//!   `delete_ControlWord`, `update_ControlWord`.

pub mod any;
pub mod component_mode_dpi;
pub mod control_word;
pub mod encrypted_dpi;
pub mod inject_section_data;
pub mod insert_alternate_break_duration;
pub mod insert_audio_descriptor;
pub mod insert_audio_provisioning;
pub mod insert_avail_descriptor;
pub mod insert_descriptor;
pub mod insert_dtmf_descriptor;
pub mod insert_segmentation_descriptor;
pub mod insert_tier;
pub mod insert_time_descriptor;
pub mod proprietary_command;
pub mod schedule_component_mode;
pub mod schedule_definition;
pub mod splice_null_request;
pub mod splice_request;
pub mod start_schedule_download;
pub mod time_signal_request;
pub mod transmit_schedule;

pub use any::AnyOperation;

/// A single operation entry within a `multiple_operation_message` data loop,
/// pairing the wire `opID` with the parsed structure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Operation<'a> {
    /// The wire `opID` (Table 8-4).
    pub op_id: u16,
    /// The parsed operation body.
    pub data: AnyOperation<'a>,
}

impl<'a> Operation<'a> {
    /// Length of the operation body in bytes (does NOT include the 4-byte
    /// opID+data_length prefix).
    #[must_use]
    pub fn body_len(&self) -> usize {
        self.data.body_len()
    }
}

// Single-operation (basic) types — simple unit structs where the body is empty.
// Defined inline since they're just markers.

/// `general_response_data()` — §9.6.1, Table 9-12 (opID 0x0000).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GeneralResponse;

/// `init_request_data()` — §9.1.1, Table 9-1 (opID 0x0001). Empty body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InitRequest;

/// `init_response_data()` — §9.1.2, Table 9-2 (opID 0x0002). Empty body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InitResponse;

/// `alive_request_data()` — §9.2.1, Table 9-3 (opID 0x0003).
/// Carries a `time()` structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AliveRequest {
    /// `time()` (§12.4) — can be zero if time sync is not active.
    pub time: crate::time::Time,
}

impl Default for AliveRequest {
    fn default() -> Self {
        Self {
            time: crate::time::Time::zero(),
        }
    }
}

/// `alive_response_data()` — §9.2.2, Table 9-4 (opID 0x0004).
/// Carries a `time()` structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AliveResponse {
    /// `time()` (§12.4) — can be zero if time sync is not active.
    pub time: crate::time::Time,
}

impl Default for AliveResponse {
    fn default() -> Self {
        Self {
            time: crate::time::Time::zero(),
        }
    }
}

/// `inject_response_data()` — §9.6.2, Table 9-14 (opID 0x0007).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InjectResponse {
    /// `message_number` of the multiple_operation_message being acknowledged.
    pub message_number: u8,
}

/// `inject_complete_response_data()` — §9.6.3, Table 9-16 (opID 0x0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InjectCompleteResponse {
    /// `message_number` of the multiple_operation_message that completed.
    pub message_number: u8,
    /// `cue_message_count` — count of SCTE 35 splice_info_sections sent.
    pub cue_message_count: u8,
}

// ── Parse / Serialize impls for the simple single-operation types ──

use crate::error::{Error, Result};
use crate::time::TIME_LEN;
use dvb_common::{Parse, Serialize};

macro_rules! impl_empty_body {
    ($ty:ident, $what:literal, $oid:literal) => {
        impl<'a> Parse<'a> for $ty {
            type Error = Error;
            fn parse(_bytes: &'a [u8]) -> Result<Self> {
                Ok(Self)
            }
        }
        impl Serialize for $ty {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                0
            }
            fn serialize_into(&self, _buf: &mut [u8]) -> Result<usize> {
                Ok(0)
            }
        }
        impl<'a> crate::traits::OperationDef<'a> for $ty {
            const OP_ID: u16 = $oid;
            const NAME: &'static str = $what;
        }
    };
}

impl_empty_body!(GeneralResponse, "GENERAL_RESPONSE", 0x0000);
impl_empty_body!(InitRequest, "INIT_REQUEST", 0x0001);
impl_empty_body!(InitResponse, "INIT_RESPONSE", 0x0002);

impl<'a> Parse<'a> for AliveRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            time: crate::time::Time::parse(bytes)?,
        })
    }
}

impl Serialize for AliveRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        TIME_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.time.serialize_into(buf)
    }
}

impl<'a> crate::traits::OperationDef<'a> for AliveRequest {
    const OP_ID: u16 = 0x0003;
    const NAME: &'static str = "ALIVE_REQUEST";
}

impl<'a> Parse<'a> for AliveResponse {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            time: crate::time::Time::parse(bytes)?,
        })
    }
}

impl Serialize for AliveResponse {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        TIME_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.time.serialize_into(buf)
    }
}

impl<'a> crate::traits::OperationDef<'a> for AliveResponse {
    const OP_ID: u16 = 0x0004;
    const NAME: &'static str = "ALIVE_RESPONSE";
}

impl<'a> Parse<'a> for InjectResponse {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "inject_response message_number",
            });
        }
        Ok(Self {
            message_number: bytes[0],
        })
    }
}

impl Serialize for InjectResponse {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        1
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Err(Error::OutputBufferTooSmall { need: 1, have: 0 });
        }
        buf[0] = self.message_number;
        Ok(1)
    }
}

impl<'a> crate::traits::OperationDef<'a> for InjectResponse {
    const OP_ID: u16 = 0x0007;
    const NAME: &'static str = "INJECT_RESPONSE";
}

impl<'a> Parse<'a> for InjectCompleteResponse {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: bytes.len(),
                what: "inject_complete_response",
            });
        }
        Ok(Self {
            message_number: bytes[0],
            cue_message_count: bytes[1],
        })
    }
}

impl Serialize for InjectCompleteResponse {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        2
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 2 {
            return Err(Error::OutputBufferTooSmall {
                need: 2,
                have: buf.len(),
            });
        }
        buf[0] = self.message_number;
        buf[1] = self.cue_message_count;
        Ok(2)
    }
}

impl<'a> crate::traits::OperationDef<'a> for InjectCompleteResponse {
    const OP_ID: u16 = 0x0008;
    const NAME: &'static str = "INJECT_COMPLETE_RESPONSE";
}

// ── AnySingleOperation dispatch for single_operation_message ──

/// Unified dispatch for single-operation (basic) types.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AnySingleOperation<'a> {
    /// opID 0x0000 — no data body.
    GeneralResponse(GeneralResponse),
    /// opID 0x0001 — empty init request.
    InitRequest(InitRequest),
    /// opID 0x0002 — empty init response.
    InitResponse(InitResponse),
    /// opID 0x0003 — carries time().
    AliveRequest(AliveRequest),
    /// opID 0x0004 — carries time().
    AliveResponse(AliveResponse),
    /// opID 0x0007 — carries message_number.
    InjectResponse(InjectResponse),
    /// opID 0x0008 — carries message_number + cue_message_count.
    InjectCompleteResponse(InjectCompleteResponse),
    /// Unknown / unimplemented opID — raw body preserved for round-trip.
    Unknown {
        /// The raw `opID`.
        op_id: u16,
        /// The raw operation body bytes.
        body: &'a [u8],
    },
}

impl<'a> AnySingleOperation<'a> {
    /// Parse an operation body by its `op_id`.
    pub fn dispatch(op_id: u16, body: &'a [u8]) -> Result<Self> {
        match op_id {
            0x0000 => Ok(Self::GeneralResponse(GeneralResponse::parse(body)?)),
            0x0001 => Ok(Self::InitRequest(InitRequest::parse(body)?)),
            0x0002 => Ok(Self::InitResponse(InitResponse::parse(body)?)),
            0x0003 => Ok(Self::AliveRequest(AliveRequest::parse(body)?)),
            0x0004 => Ok(Self::AliveResponse(AliveResponse::parse(body)?)),
            0x0007 => Ok(Self::InjectResponse(InjectResponse::parse(body)?)),
            0x0008 => Ok(Self::InjectCompleteResponse(InjectCompleteResponse::parse(
                body,
            )?)),
            _ => Ok(Self::Unknown { op_id, body }),
        }
    }

    /// Length of the operation body in bytes.
    #[must_use]
    pub fn body_len(&self) -> usize {
        match self {
            Self::GeneralResponse(_) => 0,
            Self::InitRequest(_) => 0,
            Self::InitResponse(_) => 0,
            Self::AliveRequest(a) => a.serialized_len(),
            Self::AliveResponse(a) => a.serialized_len(),
            Self::InjectResponse(r) => r.serialized_len(),
            Self::InjectCompleteResponse(r) => r.serialized_len(),
            Self::Unknown { body, .. } => body.len(),
        }
    }

    /// Serialize just the operation body (no opID) into `buf`.
    pub fn serialize_body_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::GeneralResponse(g) => g.serialize_into(buf),
            Self::InitRequest(i) => i.serialize_into(buf),
            Self::InitResponse(i) => i.serialize_into(buf),
            Self::AliveRequest(a) => a.serialize_into(buf),
            Self::AliveResponse(a) => a.serialize_into(buf),
            Self::InjectResponse(r) => r.serialize_into(buf),
            Self::InjectCompleteResponse(r) => r.serialize_into(buf),
            Self::Unknown { body, .. } => {
                if buf.len() < body.len() {
                    return Err(Error::OutputBufferTooSmall {
                        need: body.len(),
                        have: buf.len(),
                    });
                }
                buf[..body.len()].copy_from_slice(body);
                Ok(body.len())
            }
        }
    }
}
