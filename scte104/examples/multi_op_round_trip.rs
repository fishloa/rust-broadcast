//! Build a multiple_operation_message with several operations and round-trip.
//!
//! This example constructs a message with a splice_request followed by a
//! time_signal_request (≥2 operations), serializes it, then parses it back.

use broadcast_common::{Parse, Serialize};
use scte104::MultipleOperationMessage;
use scte104::operations::{
    AnyOperation, Operation,
    splice_request::{SpliceInsertType, SpliceRequest},
    time_signal_request::TimeSignalRequest,
};
use scte104::time::Timestamp;

fn main() {
    // Build operations: splice + time_signal.
    let ops = vec![
        Operation {
            op_id: 0x0101,
            data: AnyOperation::SpliceRequest(SpliceRequest {
                splice_insert_type: SpliceInsertType::SpliceStartNormal,
                splice_event_id: 42,
                unique_program_id: 1,
                pre_roll_time: 5000,
                break_duration: 300,
                avail_num: 0,
                avails_expected: 0,
                auto_return_flag: 1,
                not_an_entry_flag: 0,
            }),
        },
        Operation {
            op_id: 0x0104,
            data: AnyOperation::TimeSignalRequest(TimeSignalRequest {
                pre_roll_time: 2000,
            }),
        },
    ];

    // Build the multi-op message.
    let msg = MultipleOperationMessage::new(
        0,               // protocol_version
        1,               // AS_index
        42,              // message_number
        0,               // DPI_PID_index
        0,               // SCTE35_protocol_version
        Timestamp::None, // immediate processing
        ops,
    );

    // Serialize.
    let bytes = msg.to_bytes();
    println!(
        "Serialized {} bytes with {} ops",
        bytes.len(),
        msg.operations.len()
    );

    // Round-trip.
    let parsed = MultipleOperationMessage::parse(&bytes).unwrap();
    assert_eq!(msg, parsed);
    assert_eq!(parsed.operations.len(), 2);
    println!("Round-trip OK: 2 operations preserved");

    // Verify operations are correctly parsed.
    for op in &parsed.operations {
        println!("  opID {:#06x}: {}", op.op_id, op.data.name());
    }

    // Demonstrate mutation detection.
    let mut msg2 = msg.clone();
    msg2.message_number = 99;
    assert_ne!(msg.to_bytes(), msg2.to_bytes());
    println!("Mutation changes output: OK");
}
