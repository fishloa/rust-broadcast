//! Multiple operation message framing — ANSI/SCTE 104 2023 §8.2.3, Table 8-2.
//!
//! Carries one or more "Normal", "Control", or "Supplemental" operations with
//! a shared timestamp.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::operations::{AnyOperation, Operation};
use crate::time::Timestamp;
use broadcast_common::{Parse, Serialize};

/// Wire constants.
const HEADER_LEN: usize = 10; // reserved(2) + messageSize(2) + protocol_version(1) + AS_index(1) + message_number(1) + DPI_PID_index(2) + SCTE35_protocol_version(1)
const RESERVED: u16 = 0xFFFF;

/// `multiple_operation_message()` — §8.2.3, Table 8-2.
///
/// A variable-length structure carrying one or more operations (Normal,
/// Control, Supplemental). The `timestamp()` field provides processing time.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MultipleOperationMessage<'a> {
    /// `messageSize` — total size of this structure in bytes.
    pub message_size: u16,
    /// `protocol_version` — shall be zero (`0x00`).
    pub protocol_version: u8,
    /// `AS_index` — identifies the AS instance (§8.2.1).
    pub as_index: u8,
    /// `message_number` — unique message identifier.
    pub message_number: u8,
    /// `DPI_PID_index` — DPI PID index (§8.2.1).
    pub dpi_pid_index: u16,
    /// `SCTE35_protocol_version` — version of resulting SCTE 35 sections.
    pub scte35_protocol_version: u8,
    /// `timestamp()` — processing time (§12.5).
    pub timestamp: Timestamp,
    /// Parsed operations in order.
    pub operations: Vec<Operation<'a>>,
}

impl<'a> MultipleOperationMessage<'a> {
    /// Construct a new `multiple_operation_message`.
    ///
    /// `message_size` is computed automatically.
    #[must_use]
    pub fn new(
        protocol_version: u8,
        as_index: u8,
        message_number: u8,
        dpi_pid_index: u16,
        scte35_protocol_version: u8,
        timestamp: Timestamp,
        operations: Vec<Operation<'a>>,
    ) -> Self {
        // Accumulate in usize (never panics/wraps here, unlike raw u16 `+=`,
        // #1129); `serialize_into` independently recomputes and validates
        // this fits the wire field before it trusts the stored value.
        let mut message_size: usize = HEADER_LEN;
        message_size += timestamp.serialized_len();
        message_size += 1; // num_ops
        for op in &operations {
            message_size += 4; // opID(2) + data_length(2)
            message_size += op.body_len();
        }
        // Saturate rather than wrap when it doesn't fit; `serialize_into`
        // recomputes the true size and rejects it rather than trusting this.
        let message_size = u16::try_from(message_size).unwrap_or(u16::MAX);
        Self {
            message_size,
            protocol_version,
            as_index,
            message_number,
            dpi_pid_index,
            scte35_protocol_version,
            timestamp,
            operations,
        }
    }
}

impl<'a> Parse<'a> for MultipleOperationMessage<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "multiple_operation_message header",
            });
        }
        let reserved = u16::from_be_bytes([bytes[0], bytes[1]]);
        if reserved != RESERVED {
            return Err(Error::ReservedSet {
                field: "reserved",
                expected: RESERVED,
                got: reserved,
            });
        }
        let message_size = u16::from_be_bytes([bytes[2], bytes[3]]);
        let protocol_version = bytes[4];
        let as_index = bytes[5];
        let message_number = bytes[6];
        let dpi_pid_index = u16::from_be_bytes([bytes[7], bytes[8]]);
        let scte35_protocol_version = bytes[9];

        let mut pos = HEADER_LEN;
        let timestamp = Timestamp::parse(&bytes[pos..])?;
        pos += timestamp.serialized_len();

        if bytes.len() < pos + 1 {
            return Err(Error::BufferTooShort {
                need: pos + 1,
                have: bytes.len(),
                what: "num_ops",
            });
        }
        let num_ops = bytes[pos] as usize;
        pos += 1;

        let mut operations = Vec::with_capacity(num_ops);
        for i in 0..num_ops {
            if bytes.len() < pos + 4 {
                return Err(Error::BufferTooShort {
                    need: pos + 4,
                    have: bytes.len(),
                    what: "operation opID+data_length",
                });
            }
            let op_id = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
            let data_length = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
            pos += 4;
            if bytes.len() < pos + data_length {
                return Err(Error::BufferTooShort {
                    need: pos + data_length,
                    have: bytes.len(),
                    what: "operation data",
                });
            }
            let body = &bytes[pos..pos + data_length];
            pos += data_length;
            let any_op = AnyOperation::dispatch(op_id, body)?;
            operations.push(Operation {
                op_id,
                data: any_op,
            });
            // Silence unused variable warning on the index
            let _ = i;
        }

        // Verify message_size matches
        let expected_size = pos as u16;
        if message_size != expected_size {
            return Err(Error::LengthOverflow {
                declared: message_size as usize,
                available: bytes.len(),
                what: "multiple_operation_message messageSize mismatch",
            });
        }

        Ok(Self {
            message_size,
            protocol_version,
            as_index,
            message_number,
            dpi_pid_index,
            scte35_protocol_version,
            timestamp,
            operations,
        })
    }
}

impl<'a> MultipleOperationMessage<'a> {
    /// Recomputes the true wire size from the operations, in `usize`
    /// (audit run-09 S4-W1): the stored `message_size` field is a cached
    /// value `new()` fills in, which goes stale the moment a caller mutates
    /// `operations` directly (the field is `pub`) — `serialized_len()` and
    /// `serialize_into` must each derive the real size themselves rather
    /// than trust it, or a stale `message_size` under-reports the buffer a
    /// caller needs to allocate.
    fn actual_size(&self) -> usize {
        let mut size = HEADER_LEN + self.timestamp.serialized_len() + 1;
        for op in &self.operations {
            size += 4 + op.body_len();
        }
        size
    }
}

impl Serialize for MultipleOperationMessage<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.actual_size()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let actual_size = self.actual_size();
        let message_size =
            broadcast_common::len::fit_u16(actual_size, "multiple_operation_message.messageSize")?;

        // Use the freshly-computed size, not `serialized_len()` (which
        // trusts the stored `message_size` field), so a stale field can
        // never make this buffer-size check pass when the real body does
        // not fit.
        let need = actual_size;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..2].copy_from_slice(&RESERVED.to_be_bytes());
        buf[2..4].copy_from_slice(&message_size.to_be_bytes());
        buf[4] = self.protocol_version;
        buf[5] = self.as_index;
        buf[6] = self.message_number;
        buf[7..9].copy_from_slice(&self.dpi_pid_index.to_be_bytes());
        buf[9] = self.scte35_protocol_version;

        let mut pos = HEADER_LEN;
        self.timestamp.serialize_into(&mut buf[pos..])?;
        pos += self.timestamp.serialized_len();

        buf[pos] = broadcast_common::len::fit_u8(
            self.operations.len(),
            "multiple_operation_message.num_ops",
        )?;
        pos += 1;

        for op in &self.operations {
            buf[pos..pos + 2].copy_from_slice(&op.op_id.to_be_bytes());
            let data_len = broadcast_common::len::fit_u16(op.body_len(), "operation.data_length")?;
            buf[pos + 2..pos + 4].copy_from_slice(&data_len.to_be_bytes());
            pos += 4;
            op.data
                .serialize_body_into(&mut buf[pos..pos + data_len as usize])?;
            pos += data_len as usize;
        }

        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::{
        insert_descriptor::InsertDescriptor,
        splice_request::{SpliceInsertType, SpliceRequest},
        time_signal_request::TimeSignalRequest,
    };
    use alloc::vec;

    #[test]
    fn round_trip_single_splice_request() {
        let ops = vec![Operation {
            op_id: 0x0101,
            data: AnyOperation::SpliceRequest(SpliceRequest {
                splice_insert_type: SpliceInsertType::SpliceStartNormal,
                splice_event_id: 0x0000_0042,
                unique_program_id: 1,
                pre_roll_time: 5000,
                break_duration: 300,
                avail_num: 0,
                avails_expected: 0,
                auto_return_flag: 1,
                not_an_entry_flag: 0,
            }),
        }];
        let msg = MultipleOperationMessage::new(0, 1, 42, 0, 0, Timestamp::None, ops);
        let bytes = msg.to_bytes();
        let back = MultipleOperationMessage::parse(&bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn round_trip_multiple_operations() {
        let ops = vec![
            Operation {
                op_id: 0x0101,
                data: AnyOperation::SpliceRequest(SpliceRequest {
                    splice_insert_type: SpliceInsertType::SpliceStartNormal,
                    splice_event_id: 1,
                    unique_program_id: 1,
                    pre_roll_time: 1000,
                    break_duration: 300,
                    avail_num: 0,
                    avails_expected: 0,
                    auto_return_flag: 0,
                    not_an_entry_flag: 0,
                }),
            },
            Operation {
                op_id: 0x0108,
                data: AnyOperation::InsertDescriptor(InsertDescriptor {
                    descriptor_images: alloc::vec![&[0xAB, 0x02, 0x01, 0x02][..]],
                }),
            },
            Operation {
                op_id: 0x0104,
                data: AnyOperation::TimeSignalRequest(TimeSignalRequest {
                    pre_roll_time: 2000,
                }),
            },
        ];
        let msg = MultipleOperationMessage::new(0, 1, 42, 0, 0, Timestamp::None, ops);
        let bytes = msg.to_bytes();
        let back = MultipleOperationMessage::parse(&bytes).unwrap();
        assert_eq!(msg, back);
        assert_eq!(back.operations.len(), 3);
    }

    /// S4-W2 (#1129): `new()` used to accumulate `message_size` in raw `u16`
    /// arithmetic, panicking in debug builds (`attempt to add with
    /// overflow`) or wrapping in release for a large operation body.
    /// Confirmed pre-fix: this exact case panicked in a debug build.
    #[test]
    fn two_large_ops_do_not_panic_new() {
        use crate::operations::proprietary_command::ProprietaryCommand;
        let ops = vec![
            Operation {
                op_id: 0xFFFF,
                data: AnyOperation::ProprietaryCommand(ProprietaryCommand {
                    proprietary_id: 1,
                    proprietary_command: 0,
                    proprietary_data: &[0u8; 40_000],
                }),
            },
            Operation {
                op_id: 0xFFFF,
                data: AnyOperation::ProprietaryCommand(ProprietaryCommand {
                    proprietary_id: 1,
                    proprietary_command: 0,
                    proprietary_data: &[0u8; 40_000],
                }),
            },
        ];
        let _ = MultipleOperationMessage::new(0, 1, 1, 0, 0, Timestamp::None, ops);
    }

    #[test]
    fn new_does_not_panic_on_oversized_body() {
        use crate::operations::proprietary_command::ProprietaryCommand;
        let ops = vec![Operation {
            op_id: 0xFFFF,
            data: AnyOperation::ProprietaryCommand(ProprietaryCommand {
                proprietary_id: 1,
                proprietary_command: 0,
                proprietary_data: &[0u8; 70_000],
            }),
        }];
        // Must not panic.
        let msg = MultipleOperationMessage::new(0, 1, 1, 0, 0, Timestamp::None, ops);
        // The real body does not fit u16, so serializing it must error
        // rather than emit a misframed messageSize.
        assert!(msg.try_to_bytes().is_err());
    }

    /// S4-W3 (#1129): `num_ops` is an 8-bit field but used to be written
    /// with `self.operations.len() as u8`, wrapping to 0 for 256+ operations.
    #[test]
    fn over_255_operations_rejected_not_wrapped() {
        use crate::operations::splice_null_request::SpliceNullRequest;
        let ops: Vec<_> = (0..256)
            .map(|_| Operation {
                op_id: 0x0101,
                data: AnyOperation::SpliceNullRequest(SpliceNullRequest),
            })
            .collect();
        let msg = MultipleOperationMessage::new(0, 1, 1, 0, 0, Timestamp::None, ops);
        assert!(msg.try_to_bytes().is_err());
    }

    /// S4-W1 (#1103): `serialized_len()` used to return the *stored*
    /// `message_size` field, which goes stale the moment a caller appends
    /// to the `pub operations` vec after construction (`new()` only fills
    /// it in once). It must recompute from the operations instead.
    #[test]
    fn serialized_len_reflects_operations_mutated_after_construction() {
        use crate::operations::splice_null_request::SpliceNullRequest;
        let mut msg = MultipleOperationMessage::new(
            0,
            1,
            1,
            0,
            0,
            Timestamp::None,
            vec![Operation {
                op_id: 0x0101,
                data: AnyOperation::SpliceNullRequest(SpliceNullRequest),
            }],
        );
        let len_before = msg.serialized_len();
        msg.operations.push(Operation {
            op_id: 0x0101,
            data: AnyOperation::SpliceNullRequest(SpliceNullRequest),
        });
        let len_after = msg.serialized_len();
        assert_eq!(
            len_after,
            len_before + 4,
            "serialized_len must grow by the appended op's opID+data_length \
             header (SpliceNullRequest has an empty body), not stay stale"
        );
        // And it must actually match what serialize_into needs.
        let bytes = msg.to_bytes();
        assert_eq!(bytes.len(), len_after);
    }

    #[test]
    fn mutate_field_changes_output() {
        let ops = vec![Operation {
            op_id: 0x0101,
            data: AnyOperation::SpliceRequest(SpliceRequest {
                splice_insert_type: SpliceInsertType::SpliceStartNormal,
                splice_event_id: 1,
                unique_program_id: 1,
                pre_roll_time: 1000,
                break_duration: 300,
                avail_num: 0,
                avails_expected: 0,
                auto_return_flag: 0,
                not_an_entry_flag: 0,
            }),
        }];
        let msg = MultipleOperationMessage::new(0, 1, 42, 0, 0, Timestamp::None, ops);
        let bytes = msg.to_bytes();
        let mut msg2 = msg.clone();
        msg2.message_number = 99;
        assert_ne!(msg2.to_bytes(), bytes);
    }
}
