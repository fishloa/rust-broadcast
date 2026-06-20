# scte104

ANSI/SCTE 104 2023 — Automation System to Compression System
Communications Applications Program Interface (API).

Parses and builds SCTE 104 messages: `single_operation_message` and
`multiple_operation_message` with all DPI operations from the opID
allocation tables (splice, time_signal, descriptor inserts,
segmentation, encryption, schedule, control words, and more).

## Usage

```rust
use scte104::{SingleOperationMessage, MultipleOperationMessage};
use scte104::operations::{
    AnyOperation, Operation, AnySingleOperation,
    splice_request::{SpliceRequest, SpliceInsertType},
    inject_section_data::InjectSectionData,
};
use scte104::time::Timestamp;
use dvb_common::{Parse, Serialize};

// Build a single_operation_message (basic response).
let msg = SingleOperationMessage::new_response(
    0x0007, 0x0000, 0xFFFF, 0, 1, 42, 0,
    AnySingleOperation::InjectResponse(
        scte104::operations::InjectResponse { message_number: 42 },
    ),
);
let bytes = msg.to_bytes();
let parsed = SingleOperationMessage::parse(&bytes).unwrap();
assert_eq!(msg, parsed);

// Build a multiple_operation_message with splice + time_signal.
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
];
let mom = MultipleOperationMessage::new(
    0, 1, 42, 0, 0,
    Timestamp::None,
    ops,
);
let bytes = mom.to_bytes();
let parsed = MultipleOperationMessage::parse(&bytes).unwrap();
assert_eq!(mom, parsed);
```

## Features

- `std` (default) — links the standard library.
- `serde` — derives `Serialize` on all types.

Without `std`, the crate is `#![no_std]` + `alloc` only.

## Examples

Run the bundled examples:

```sh
cargo run -p scte104 --example build_splice
cargo run -p scte104 --example multi_op_round_trip
```

## Coverage

All opID values from ANSI/SCTE 104 2023 Tables 8-3 and 8-4:

**Basic (single_operation_message):**
`general_response`, `init_request`, `init_response`, `alive_request`,
`alive_response`, `inject_response`, `inject_complete_response`.

**Normal/Supplemental/Control (multiple_operation_message):**
`inject_section_data`, `splice_request`, `splice_null_request`,
`start_schedule_download`, `time_signal_request`, `transmit_schedule`,
`component_mode_DPI`, `encrypted_DPI`, `insert_descriptor`,
`insert_DTMF_descriptor`, `insert_avail_descriptor`,
`insert_segmentation_descriptor`, `proprietary_command`,
`schedule_component_mode`, `schedule_definition`, `insert_tier`,
`insert_time_descriptor`, `insert_audio_descriptor`,
`insert_audio_provisioning`, `insert_alternate_break_duration`,
`delete_ControlWord`, `update_ControlWord`.

## License

MIT OR Apache-2.0
