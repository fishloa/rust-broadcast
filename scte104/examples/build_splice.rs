//! Build and serialize a single_operation_message with a splice_request.
//!
//! This example constructs a basic response (inject_response),
//! serializes it, then parses it back to verify round-trip integrity.

use dvb_common::{Parse, Serialize};
use scte104::operations::{AnySingleOperation, InjectResponse};
use scte104::SingleOperationMessage;

fn main() {
    // Build an inject_response single_operation_message.
    let msg = SingleOperationMessage::new_response(
        0x0007, // opID = inject_response
        0x0000, // result = success
        0xFFFF, // result_extension
        0,      // protocol_version
        1,      // AS_index
        42,     // message_number
        0,      // DPI_PID_index
        AnySingleOperation::InjectResponse(InjectResponse { message_number: 42 }),
    );

    // Serialize.
    let bytes = msg.to_bytes();
    println!("Serialized {} bytes: {:02x?}", bytes.len(), bytes);

    // Round-trip: parse back.
    let parsed = SingleOperationMessage::parse(&bytes).unwrap();
    assert_eq!(msg, parsed);
    println!("Round-trip OK: message_number={}", parsed.message_number);

    // Build a second message with an unknown opID (raw body preserved).
    let raw_body = [0xCA, 0xFE];
    let msg2 = SingleOperationMessage::new_response(
        0xDEAD,
        0x0000,
        0xFFFF,
        0,
        1,
        99,
        0,
        AnySingleOperation::Unknown {
            op_id: 0xDEAD,
            body: &raw_body,
        },
    );
    let bytes2 = msg2.to_bytes();
    let parsed2 = SingleOperationMessage::parse(&bytes2).unwrap();
    assert_eq!(msg2, parsed2);
    println!("Unknown opID round-trip OK");
}
