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
//!   eight are PAMS⇔AS messages, §10, `private/specs-verified/scte/scte104/ansi_scte_104/pams_operations.md`).
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

use self::provisioning_request::SERVICE_NAME_LEN;
pub use any::AnyOperation;

// Shared wire layout constants (ANSI/SCTE 104 2023 §8/§10).
const PROVIDER_AVAIL_ID_LEN: usize = core::mem::size_of::<u32>();
/// Byte width of a 16-bit field.
pub(crate) const U16_LEN: usize = core::mem::size_of::<u16>();
/// Byte width of a 32-bit field.
pub(crate) const U32_LEN: usize = core::mem::size_of::<u32>();

/// Copy `N` big-endian bytes at `offset` into an array.
pub(crate) fn be<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N]> {
    bytes
        .get(offset..offset + N)
        .and_then(|s| s.first_chunk::<N>())
        .copied()
        .ok_or(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: offset + N,
                have: bytes.len(),
                what: "fixed-width big-endian field",
            },
        ))
}

/// Fixed wire length of `inject_complete_response`.
const INJECT_COMPLETE_RESPONSE_LEN: usize = 2;

// `config_request_data()` field offsets (§10.4.1, Table 10-1).
const AS_IP_ADDRESS_OFFSET: usize = 0;
const AS_SOCKET_NUMBER_OFFSET: usize = 4;
const ACTIVE_FLAG_OFFSET: usize = 6;
const PROTOCOL_VERSION_OFFSET: usize = 7;
const LAST_AS_INDEX_OFFSET: usize = 8;
const LAST_INJECTOR_COUNT_OFFSET: usize = 9;
const PERMANENT_CONNECTION_REQUESTED_OFFSET: usize = 11;

// `fault_request_data()` field offsets (§10.6.1, Table 10-5).
const FAULT_IP_ADDRESS_OFFSET: usize = 0;
const FAULT_SOCKET_NUMBER_OFFSET: usize = 4;
const FAULT_SERVICE_NAME_OFFSET: usize = 6;
const FAULT_DPI_PID_INDEX_OFFSET: usize = 38;

/// Reject an operation body whose length is not exactly what the parsed typed
/// structure re-serializes to (#1103 W4). The typed parsers read what they
/// need and ignore the rest, so without this check a body with trailing bytes
/// parses "successfully" and then silently loses those bytes on re-serialize.
pub(crate) fn require_exact_len(
    body_len: usize,
    typed_len: usize,
    what: &'static str,
) -> Result<()> {
    if body_len == typed_len {
        Ok(())
    } else {
        Err(Error::LengthOverflow {
            declared: body_len,
            available: typed_len,
            what,
        })
    }
}

/// Shared wire body of the `count:u8 + provider_avail_id[count]` operations.
pub(crate) fn parse_provider_avail_ids(
    bytes: &[u8],
    what_count: &'static str,
    what_ids: &'static str,
) -> Result<Vec<u32>> {
    let Some(&count) = bytes.first() else {
        return Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 1,
                have: 0,
                what: what_count,
            },
        ));
    };
    let count = usize::from(count);
    let need = 1 + count * PROVIDER_AVAIL_ID_LEN;
    need_in(bytes, need, what_ids)?;
    bytes[1..need]
        .chunks_exact(PROVIDER_AVAIL_ID_LEN)
        .map(|c| be::<U32_LEN>(c, 0).map(u32::from_be_bytes))
        .collect::<Result<Vec<u32>>>()
}

pub(crate) fn provider_avail_ids_len(ids: &[u32]) -> usize {
    1 + ids.len() * PROVIDER_AVAIL_ID_LEN
}

pub(crate) fn write_provider_avail_ids(
    ids: &[u32],
    buf: &mut [u8],
    what_count: &'static str,
) -> Result<usize> {
    let need = provider_avail_ids_len(ids);
    need_out(buf, need)?;
    buf[0] = broadcast_core::len::fit_u8(ids.len(), what_count)?;
    for (chunk, &id) in buf[1..need]
        .chunks_exact_mut(PROVIDER_AVAIL_ID_LEN)
        .zip(ids)
    {
        chunk.copy_from_slice(&id.to_be_bytes());
    }
    Ok(need)
}

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

// ── PAMS ⇔ AS messages (§10, Tables 10-1/10-2/10-5, private/specs-verified/scte/scte104/ansi_scte_104/pams_operations.md) ──

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
    pub injector_service_name: [u8; SERVICE_NAME_LEN],
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

use alloc::vec::Vec;
use broadcast_core::len::{need_in, need_out};

use crate::error::{Error, Result};
use crate::time::TIME_LEN;
use broadcast_core::{Parse, Serialize};

macro_rules! impl_empty_body {
    ($ty:ident, $what:literal, $oid:expr) => {
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

/// Operation ids of the empty-body operation types (ANSI/SCTE 104 2023
/// Tables 8-3 and 8-4).
const GENERAL_RESPONSE_OP_ID: u16 = 0x0000;
const INIT_REQUEST_OP_ID: u16 = 0x0001;
const INIT_RESPONSE_OP_ID: u16 = 0x0002;
const PROVISIONING_RESPONSE_OP_ID: u16 = 0x000C;
const FAULT_RESPONSE_OP_ID: u16 = 0x0010;
const AS_ALIVE_REQUEST_OP_ID: u16 = 0x0011;
const AS_ALIVE_RESPONSE_OP_ID: u16 = 0x0012;

impl_empty_body!(GeneralResponse, "GENERAL_RESPONSE", GENERAL_RESPONSE_OP_ID);
impl_empty_body!(InitRequest, "INIT_REQUEST", INIT_REQUEST_OP_ID);
impl_empty_body!(InitResponse, "INIT_RESPONSE", INIT_RESPONSE_OP_ID);

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
        need_in(bytes, 1, "inject_response message_number")?;
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
            return Err(Error::OutputBufferTooSmall(
                broadcast_core::error::OutputBufferTooSmall { need: 1, have: 0 },
            ));
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
        need_in(
            bytes,
            INJECT_COMPLETE_RESPONSE_LEN,
            "inject_complete_response",
        )?;
        Ok(Self {
            message_number: bytes[0],
            cue_message_count: bytes[1],
        })
    }
}

impl Serialize for InjectCompleteResponse {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        INJECT_COMPLETE_RESPONSE_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, INJECT_COMPLETE_RESPONSE_LEN)?;
        buf[0] = self.message_number;
        buf[1] = self.cue_message_count;
        Ok(INJECT_COMPLETE_RESPONSE_LEN)
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
        need_in(bytes, CONFIG_REQUEST_LEN, "config_request_data")?;
        Ok(Self {
            as_ip_address: u32::from_be_bytes(be::<U32_LEN>(bytes, AS_IP_ADDRESS_OFFSET)?),
            as_socket_number: u16::from_be_bytes(be::<U16_LEN>(bytes, AS_SOCKET_NUMBER_OFFSET)?),
            activeflag: bytes[ACTIVE_FLAG_OFFSET],
            protocol_version: bytes[PROTOCOL_VERSION_OFFSET],
            last_as_index: bytes[LAST_AS_INDEX_OFFSET],
            last_injectorcount: u16::from_be_bytes(be::<U16_LEN>(
                bytes,
                LAST_INJECTOR_COUNT_OFFSET,
            )?),
            permanent_connection_requested: bytes[PERMANENT_CONNECTION_REQUESTED_OFFSET],
        })
    }
}

impl Serialize for ConfigRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        CONFIG_REQUEST_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, CONFIG_REQUEST_LEN)?;
        buf[0..4].copy_from_slice(&self.as_ip_address.to_be_bytes());
        buf[4..6].copy_from_slice(&self.as_socket_number.to_be_bytes());
        buf[ACTIVE_FLAG_OFFSET] = self.activeflag;
        buf[PROTOCOL_VERSION_OFFSET] = self.protocol_version;
        buf[LAST_AS_INDEX_OFFSET] = self.last_as_index;
        buf[9..11].copy_from_slice(&self.last_injectorcount.to_be_bytes());
        buf[PERMANENT_CONNECTION_REQUESTED_OFFSET] = self.permanent_connection_requested;
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
        need_in(bytes, CONFIG_RESPONSE_LEN, "config_response_data")?;
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
        need_out(buf, CONFIG_RESPONSE_LEN)?;
        buf[0] = self.as_index;
        buf[1] = self.permanent_connection_requested;
        Ok(CONFIG_RESPONSE_LEN)
    }
}

impl crate::traits::OperationDef<'_> for ConfigResponse {
    const OP_ID: u16 = 0x000A;
    const NAME: &'static str = "CONFIG_RESPONSE";
}

impl_empty_body!(
    ProvisioningResponse,
    "PROVISIONING_RESPONSE",
    PROVISIONING_RESPONSE_OP_ID
);

impl<'a> Parse<'a> for FaultRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, FAULT_REQUEST_LEN, "fault_request_data")?;
        let mut injector_service_name = [0u8; SERVICE_NAME_LEN];
        injector_service_name.copy_from_slice(
            &bytes[FAULT_SERVICE_NAME_OFFSET..FAULT_SERVICE_NAME_OFFSET + SERVICE_NAME_LEN],
        );
        Ok(Self {
            injector_ip_address: u32::from_be_bytes(be::<U32_LEN>(bytes, FAULT_IP_ADDRESS_OFFSET)?),
            injector_socket_number: u16::from_be_bytes(be::<U16_LEN>(
                bytes,
                FAULT_SOCKET_NUMBER_OFFSET,
            )?),
            injector_service_name,
            dpi_pid_index: u16::from_be_bytes(be::<U16_LEN>(bytes, FAULT_DPI_PID_INDEX_OFFSET)?),
        })
    }
}

impl Serialize for FaultRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        FAULT_REQUEST_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, FAULT_REQUEST_LEN)?;
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

impl_empty_body!(FaultResponse, "FAULT_RESPONSE", FAULT_RESPONSE_OP_ID);
impl_empty_body!(AsAliveRequest, "AS_ALIVE_REQUEST", AS_ALIVE_REQUEST_OP_ID);
impl_empty_body!(
    AsAliveResponse,
    "AS_ALIVE_RESPONSE",
    AS_ALIVE_RESPONSE_OP_ID
);

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
        let parsed = Self::dispatch_unchecked(op_id, body)?;
        // Exact length (#1103 W4): trailing bytes would be lost on re-serialize.
        require_exact_len(body.len(), parsed.body_len(), "single operation body")?;
        Ok(parsed)
    }

    fn dispatch_unchecked(op_id: u16, body: &'a [u8]) -> Result<Self> {
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
                need_out(buf, body.len())?;
                buf[..body.len()].copy_from_slice(body);
                Ok(body.len())
            }
        }
    }
}
