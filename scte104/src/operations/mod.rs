//! SCTE 104 operations (request/response data structures).
//!
//! Every operation from Tables 8-3 and 8-4 of ANSI/SCTE 104 2023 is
//! implemented. Operations are organized by usage category:
//!
//! - **Single-operation (basic):** `general_response`, `init_request`,
//!   `init_response`, `alive_request`, `alive_response`, `inject_response`,
//!   `inject_complete_response`, `config_request`, `config_response`,
//!   `provisioning_request`, `provisioning_response`, `fault_request`,
//!   `fault_response`, `as_alive_request`, `as_alive_response` (the last
//!   eight are PAMS⇔AS messages, §10, `docs/ansi_scte_104/pams_operations.md`).
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
pub mod provisioning_request;
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

impl Operation<'_> {
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

// ── PAMS ⇔ AS messages (§10, Tables 10-1/10-2/10-5, docs/ansi_scte_104/pams_operations.md) ──

/// `config_request_data()` — §10.4.1, Table 10-1 (opID 0x0009). AS ==> PAMS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConfigRequest {
    /// `AS_IP_address` — 4 bytes. Zero in a bi-directional serial architecture.
    pub as_ip_address: u32,
    /// `AS_socket_number` — 2 bytes. Zero in a bi-directional serial architecture.
    pub as_socket_number: u16,
    /// `activeflag` — 1 byte. Zero = backup AS, non-zero = primary AS.
    pub activeflag: u8,
    /// `protocol_version` — 1 byte, shall be `0x00`.
    pub protocol_version: u8,
    /// `last_AS_index` — 1 byte. `AS_index` from a previous initialization, or
    /// zero if unused / first initialization.
    pub last_as_index: u8,
    /// `last_injectorcount` — 2 bytes. `injectorcount` last provided by the
    /// PAMS; zero on first initialization.
    pub last_injectorcount: u16,
    /// `permanent_connection_requested` — 1 byte. Non-zero requests a
    /// permanent TCP/IP link.
    pub permanent_connection_requested: u8,
}

/// Fixed wire length of `config_request_data()`.
pub const CONFIG_REQUEST_LEN: usize = 12;

/// `config_response_data()` — §10.4.2, Table 10-2 (opID 0x000A). PAMS ==> AS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConfigResponse {
    /// `AS_index` — 1 byte. Index provided by the PAMS (0-255), §8.2.1.
    pub as_index: u8,
    /// `permanent_connection_requested` — 1 byte. Non-zero: a permanent
    /// TCP/IP link has been provisioned on the PAMS side.
    pub permanent_connection_requested: u8,
}

/// Fixed wire length of `config_response_data()`.
pub const CONFIG_RESPONSE_LEN: usize = 2;

/// `provisioning_response_data()` — §10.5.2, Table 10-4 (opID 0x000C). Empty body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProvisioningResponse;

/// `fault_request_data()` — §10.6.1, Table 10-5 (opID 0x000F). AS ==> PAMS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FaultRequest {
    /// `injector_IP_address` — 4 bytes. Zero if not using TCP/IP.
    pub injector_ip_address: u32,
    /// `injector_socket_number` — 2 bytes. Zero if not using TCP/IP.
    pub injector_socket_number: u16,
    /// `injector_service_name` — 32-byte NUL-terminated string field. Must
    /// match the `service_name` sent by the PAMS in `provisioning_request_data()`.
    pub injector_service_name: [u8; 32],
    /// `DPI_PID_index` — 2 bytes. May be zero if the Injector is otherwise
    /// unambiguously identified.
    pub dpi_pid_index: u16,
}

/// Fixed wire length of `fault_request_data()`.
pub const FAULT_REQUEST_LEN: usize = 40;

/// `fault_response_data()` — §10.6.2, Table 10-6 (opID 0x0010). Empty body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FaultResponse;

/// `AS_alive_request_data()` — §10.7.1, Table 10-7 (opID 0x0011). PAMS ==> AS.
/// Empty body; distinct from [`AliveRequest`] (opID 0x0003, AS ==> Injector,
/// which carries a `time()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AsAliveRequest;

/// `AS_alive_response_data()` — §10.7.2, Table 10-8 (opID 0x0012). AS ==> PAMS.
/// Empty body; distinct from [`AliveResponse`] (opID 0x0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AsAliveResponse;

// ── Parse / Serialize impls for the simple single-operation types ──

use crate::error::{Error, Result};
use crate::time::TIME_LEN;
use broadcast_common::{Parse, Serialize};

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

impl crate::traits::OperationDef<'_> for AliveRequest {
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

impl crate::traits::OperationDef<'_> for AliveResponse {
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

impl crate::traits::OperationDef<'_> for InjectResponse {
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

impl crate::traits::OperationDef<'_> for InjectCompleteResponse {
    const OP_ID: u16 = 0x0008;
    const NAME: &'static str = "INJECT_COMPLETE_RESPONSE";
}

// ── Parse / Serialize impls for the PAMS ⇔ AS messages (§10) ──

impl<'a> Parse<'a> for ConfigRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < CONFIG_REQUEST_LEN {
            return Err(Error::BufferTooShort {
                need: CONFIG_REQUEST_LEN,
                have: bytes.len(),
                what: "config_request_data",
            });
        }
        Ok(Self {
            as_ip_address: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            as_socket_number: u16::from_be_bytes([bytes[4], bytes[5]]),
            activeflag: bytes[6],
            protocol_version: bytes[7],
            last_as_index: bytes[8],
            last_injectorcount: u16::from_be_bytes([bytes[9], bytes[10]]),
            permanent_connection_requested: bytes[11],
        })
    }
}

impl Serialize for ConfigRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        CONFIG_REQUEST_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < CONFIG_REQUEST_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: CONFIG_REQUEST_LEN,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.as_ip_address.to_be_bytes());
        buf[4..6].copy_from_slice(&self.as_socket_number.to_be_bytes());
        buf[6] = self.activeflag;
        buf[7] = self.protocol_version;
        buf[8] = self.last_as_index;
        buf[9..11].copy_from_slice(&self.last_injectorcount.to_be_bytes());
        buf[11] = self.permanent_connection_requested;
        Ok(CONFIG_REQUEST_LEN)
    }
}

impl crate::traits::OperationDef<'_> for ConfigRequest {
    const OP_ID: u16 = 0x0009;
    const NAME: &'static str = "CONFIG_REQUEST";
}

impl<'a> Parse<'a> for ConfigResponse {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < CONFIG_RESPONSE_LEN {
            return Err(Error::BufferTooShort {
                need: CONFIG_RESPONSE_LEN,
                have: bytes.len(),
                what: "config_response_data",
            });
        }
        Ok(Self {
            as_index: bytes[0],
            permanent_connection_requested: bytes[1],
        })
    }
}

impl Serialize for ConfigResponse {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        CONFIG_RESPONSE_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < CONFIG_RESPONSE_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: CONFIG_RESPONSE_LEN,
                have: buf.len(),
            });
        }
        buf[0] = self.as_index;
        buf[1] = self.permanent_connection_requested;
        Ok(CONFIG_RESPONSE_LEN)
    }
}

impl crate::traits::OperationDef<'_> for ConfigResponse {
    const OP_ID: u16 = 0x000A;
    const NAME: &'static str = "CONFIG_RESPONSE";
}

impl_empty_body!(ProvisioningResponse, "PROVISIONING_RESPONSE", 0x000C);

impl<'a> Parse<'a> for FaultRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FAULT_REQUEST_LEN {
            return Err(Error::BufferTooShort {
                need: FAULT_REQUEST_LEN,
                have: bytes.len(),
                what: "fault_request_data",
            });
        }
        let mut injector_service_name = [0u8; 32];
        injector_service_name.copy_from_slice(&bytes[6..38]);
        Ok(Self {
            injector_ip_address: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            injector_socket_number: u16::from_be_bytes([bytes[4], bytes[5]]),
            injector_service_name,
            dpi_pid_index: u16::from_be_bytes([bytes[38], bytes[39]]),
        })
    }
}

impl Serialize for FaultRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        FAULT_REQUEST_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < FAULT_REQUEST_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: FAULT_REQUEST_LEN,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.injector_ip_address.to_be_bytes());
        buf[4..6].copy_from_slice(&self.injector_socket_number.to_be_bytes());
        buf[6..38].copy_from_slice(&self.injector_service_name);
        buf[38..40].copy_from_slice(&self.dpi_pid_index.to_be_bytes());
        Ok(FAULT_REQUEST_LEN)
    }
}

impl crate::traits::OperationDef<'_> for FaultRequest {
    const OP_ID: u16 = 0x000F;
    const NAME: &'static str = "FAULT_REQUEST";
}

impl_empty_body!(FaultResponse, "FAULT_RESPONSE", 0x0010);
impl_empty_body!(AsAliveRequest, "AS_ALIVE_REQUEST", 0x0011);
impl_empty_body!(AsAliveResponse, "AS_ALIVE_RESPONSE", 0x0012);

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
    /// opID 0x0009 — AS ==> PAMS, §10.4.1.
    ConfigRequest(ConfigRequest),
    /// opID 0x000A — PAMS ==> AS, §10.4.2.
    ConfigResponse(ConfigResponse),
    /// opID 0x000B — PAMS ==> AS, §10.5.1. Variable-length.
    ProvisioningRequest(crate::operations::provisioning_request::ProvisioningRequest),
    /// opID 0x000C — AS ==> PAMS, §10.5.2. Empty body.
    ProvisioningResponse(ProvisioningResponse),
    /// opID 0x000F — AS ==> PAMS, §10.6.1.
    FaultRequest(FaultRequest),
    /// opID 0x0010 — PAMS ==> AS, §10.6.2. Empty body.
    FaultResponse(FaultResponse),
    /// opID 0x0011 — PAMS ==> AS, §10.7.1. Empty body.
    AsAliveRequest(AsAliveRequest),
    /// opID 0x0012 — AS ==> PAMS, §10.7.2. Empty body.
    AsAliveResponse(AsAliveResponse),
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
            0x0009 => Ok(Self::ConfigRequest(ConfigRequest::parse(body)?)),
            0x000A => Ok(Self::ConfigResponse(ConfigResponse::parse(body)?)),
            0x000B => Ok(Self::ProvisioningRequest(
                crate::operations::provisioning_request::ProvisioningRequest::parse(body)?,
            )),
            0x000C => Ok(Self::ProvisioningResponse(ProvisioningResponse::parse(
                body,
            )?)),
            0x000F => Ok(Self::FaultRequest(FaultRequest::parse(body)?)),
            0x0010 => Ok(Self::FaultResponse(FaultResponse::parse(body)?)),
            0x0011 => Ok(Self::AsAliveRequest(AsAliveRequest::parse(body)?)),
            0x0012 => Ok(Self::AsAliveResponse(AsAliveResponse::parse(body)?)),
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
            Self::ConfigRequest(r) => r.serialized_len(),
            Self::ConfigResponse(r) => r.serialized_len(),
            Self::ProvisioningRequest(r) => r.serialized_len(),
            Self::ProvisioningResponse(_) => 0,
            Self::FaultRequest(r) => r.serialized_len(),
            Self::FaultResponse(_) => 0,
            Self::AsAliveRequest(_) => 0,
            Self::AsAliveResponse(_) => 0,
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
            Self::ConfigRequest(r) => r.serialize_into(buf),
            Self::ConfigResponse(r) => r.serialize_into(buf),
            Self::ProvisioningRequest(r) => r.serialize_into(buf),
            Self::ProvisioningResponse(r) => r.serialize_into(buf),
            Self::FaultRequest(r) => r.serialize_into(buf),
            Self::FaultResponse(r) => r.serialize_into(buf),
            Self::AsAliveRequest(r) => r.serialize_into(buf),
            Self::AsAliveResponse(r) => r.serialize_into(buf),
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
