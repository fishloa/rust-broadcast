//! Single operation message framing — ANSI/SCTE 104 2023 §8.2.2, Table 8-1.
//!
//! Carries one "basic" operation (request or response) with metadata fields
//! (result, protocol_version, AS_index, message_number, DPI_PID_index).

use crate::error::{Error, Result};
use crate::operations::AnySingleOperation;
use dvb_common::{Parse, Serialize};

/// Wire constants.
const HEADER_LEN: usize = 13; // opID(2) + messageSize(2) + result(2) + result_ext(2) + protocol_version(1) + AS_index(1) + message_number(1) + DPI_PID_index(2)

/// `single_operation_message()` — §8.2.2, Table 8-1.
///
/// A variable-length structure carrying one "basic" operation. `data` is the
/// parsed operation body; `data_raw` is retained for unknown opIDs so the
/// message round-trips byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SingleOperationMessage<'a> {
    /// `opID` (2 bytes, big-endian) — identifies the operation (Table 8-3).
    pub op_id: u16,
    /// `messageSize` — total size of this structure in bytes (includes header).
    pub message_size: u16,
    /// `result` — result code (§14). For requests it is `0xFFFF`.
    pub result: u16,
    /// `result_extension` — normally `0xFFFF` unless sending additional result info.
    pub result_extension: u16,
    /// `protocol_version` — shall be zero (`0x00`).
    pub protocol_version: u8,
    /// `AS_index` — identifies the AS instance (§8.2.1).
    pub as_index: u8,
    /// `message_number` — unique message identifier.
    pub message_number: u8,
    /// `DPI_PID_index` — DPI PID index (§8.2.1).
    pub dpi_pid_index: u16,
    /// The parsed operation body (if `op_id` is recognized).
    pub data: AnySingleOperation<'a>,
}

const RESULT_REQUEST: u16 = 0xFFFF;

impl<'a> SingleOperationMessage<'a> {
    /// Construct a new single_operation_message for a basic request.
    ///
    /// `message_size` is computed automatically from the fields + operation body.
    #[must_use]
    pub fn new_request(
        op_id: u16,
        protocol_version: u8,
        as_index: u8,
        message_number: u8,
        dpi_pid_index: u16,
        data: AnySingleOperation<'a>,
    ) -> Self {
        let message_size = (HEADER_LEN + data.body_len()) as u16;
        Self {
            op_id,
            message_size,
            result: RESULT_REQUEST,
            result_extension: RESULT_REQUEST,
            protocol_version,
            as_index,
            message_number,
            dpi_pid_index,
            data,
        }
    }

    /// Construct a new single_operation_message for a basic response.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new_response(
        op_id: u16,
        result: u16,
        result_extension: u16,
        protocol_version: u8,
        as_index: u8,
        message_number: u8,
        dpi_pid_index: u16,
        data: AnySingleOperation<'a>,
    ) -> Self {
        let message_size = (HEADER_LEN + data.body_len()) as u16;
        Self {
            op_id,
            message_size,
            result,
            result_extension,
            protocol_version,
            as_index,
            message_number,
            dpi_pid_index,
            data,
        }
    }
}

impl<'a> Parse<'a> for SingleOperationMessage<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "single_operation_message header",
            });
        }
        let op_id = u16::from_be_bytes([bytes[0], bytes[1]]);
        let message_size = u16::from_be_bytes([bytes[2], bytes[3]]);
        let result = u16::from_be_bytes([bytes[4], bytes[5]]);
        let result_extension = u16::from_be_bytes([bytes[6], bytes[7]]);
        let protocol_version = bytes[8];
        let as_index = bytes[9];
        let message_number = bytes[10];
        let dpi_pid_index = u16::from_be_bytes([bytes[11], bytes[12]]);

        let body_start = HEADER_LEN;
        let body_len = (message_size as usize).saturating_sub(HEADER_LEN);
        if bytes.len() < body_start + body_len {
            return Err(Error::BufferTooShort {
                need: body_start + body_len,
                have: bytes.len(),
                what: "single_operation_message body",
            });
        }
        let body = &bytes[body_start..body_start + body_len];
        let data = AnySingleOperation::dispatch(op_id, body)?;

        Ok(Self {
            op_id,
            message_size,
            result,
            result_extension,
            protocol_version,
            as_index,
            message_number,
            dpi_pid_index,
            data,
        })
    }
}

impl Serialize for SingleOperationMessage<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.message_size as usize
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..2].copy_from_slice(&self.op_id.to_be_bytes());
        buf[2..4].copy_from_slice(&self.message_size.to_be_bytes());
        buf[4..6].copy_from_slice(&self.result.to_be_bytes());
        buf[6..8].copy_from_slice(&self.result_extension.to_be_bytes());
        buf[8] = self.protocol_version;
        buf[9] = self.as_index;
        buf[10] = self.message_number;
        buf[11..13].copy_from_slice(&self.dpi_pid_index.to_be_bytes());
        self.data.serialize_body_into(&mut buf[HEADER_LEN..need])?;
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::GeneralResponse;

    #[test]
    fn round_trip_general_response() {
        let msg = SingleOperationMessage::new_response(
            0x0000,
            0x0000,
            0xFFFF,
            0,
            1,
            42,
            0,
            AnySingleOperation::GeneralResponse(GeneralResponse),
        );
        let bytes = msg.to_bytes();
        let back = SingleOperationMessage::parse(&bytes).unwrap();
        assert_eq!(msg, back);
        assert!(matches!(back.data, AnySingleOperation::GeneralResponse(_)));
    }

    #[test]
    fn round_trip_init_request() {
        let msg = SingleOperationMessage::new_request(
            0x0001,
            0,
            1,
            7,
            0,
            AnySingleOperation::InitRequest(crate::operations::InitRequest),
        );
        let bytes = msg.to_bytes();
        let back = SingleOperationMessage::parse(&bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn unknown_op_id_round_trips() {
        // Build a message with an unknown op_id
        let raw_body = [0xAA, 0xBB];
        let msg = SingleOperationMessage::new_request(
            0xDEAD,
            0,
            1,
            99,
            0,
            AnySingleOperation::Unknown {
                op_id: 0xDEAD,
                body: &raw_body,
            },
        );
        let bytes = msg.to_bytes();
        let back = SingleOperationMessage::parse(&bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let msg = SingleOperationMessage::new_response(
            0x0000,
            0x0000,
            0xFFFF,
            0,
            1,
            42,
            0,
            AnySingleOperation::GeneralResponse(GeneralResponse),
        );
        let bytes = msg.to_bytes();
        let mut msg2 = msg.clone();
        msg2.message_number = 99;
        assert_ne!(msg2.to_bytes(), bytes);
    }
}
