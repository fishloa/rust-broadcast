# scte104

[![crates.io](https://img.shields.io/crates/v/scte104.svg)](https://crates.io/crates/scte104)
[![docs.rs](https://img.shields.io/docsrs/scte104)](https://docs.rs/scte104)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Parser and builder for ANSI/SCTE 104 2023 (Automation System to Compression System
Communications API): `single_operation_message` and `multiple_operation_message`
with every operation in the opID allocation tables.

```toml
[dependencies]
scte104 = "0.5"
```

## What it does / what it does not

SCTE 104 is the protocol an automation (playout) system uses to tell a
compression / injection system to insert SCTE 35 (DPI) cueing into the outgoing
transport stream. This crate gives you the wire messages: typed operations, the
message framing, and the variable-length timestamp. Use it to build an
automation-side sender or a compression-side receiver / monitor.

It does **not** implement the transport (the spec's TCP/serial connection handling,
session state machines, alive/timeout timers), and it does not turn a SCTE 104
message into a SCTE 35 `splice_info_section`: the SCTE 35 bytes themselves are the
`scte35-splice` crate's job (the `inject_section_data` operation carries an opaque
SCTE 35 section as bytes).

## Standards supported

| Standard | Edition | Scope / clauses | Status | Notes |
|---|---|---|---|---|
| ANSI/SCTE 104 Automation System to Compression System Communications API | 2023 (checked against `private/specs-md/ansi_scte_104_2023.md`) | §8.2.2 / Table 8-1 `single_operation_message`; §8.2.3 / Table 8-2 `multiple_operation_message`; Table 8-3 (15 basic operations, §9 and §10); Table 8-4 (22 Normal / Supplemental / Control operations); §12.4 `Time`; §12.5 `Timestamp` | Full for the opID tables | Unknown opIDs fall through to `Unknown { op_id, body }` (Typed-opaque, round-trips). |

### Operation coverage

| Group | Operations | Status |
|---|---|---|
| Basic (single_operation_message, 15) | `general_response`, `init_request`, `init_response`, `alive_request`, `alive_response`, `inject_response`, `inject_complete_response`, `config_request`, `config_response`, `provisioning_request`, `provisioning_response`, `fault_request`, `fault_response`, `AS_alive_request`, `AS_alive_response` | Full (typed) |
| Normal / Supplemental (multiple_operation_message, 20) | `inject_section_data`, `splice_request`, `splice_null_request`, `start_schedule_download`, `time_signal_request`, `transmit_schedule`, `component_mode_DPI`, `encrypted_DPI`, `insert_descriptor`, `insert_DTMF_descriptor`, `insert_avail_descriptor`, `insert_segmentation_descriptor`, `proprietary_command`, `schedule_component_mode`, `schedule_definition`, `insert_tier`, `insert_time_descriptor`, `insert_audio_descriptor`, `insert_audio_provisioning`, `insert_alternate_break_duration` | Full (typed); `insert_descriptor` / `proprietary_command` payloads are opaque by definition |
| Control (2) | `delete_ControlWord`, `update_ControlWord` | Full (typed) |

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | on | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | off | `serde::Serialize` on all types (no Deserialize). |

## Quick start

```rust
use scte104::{SingleOperationMessage, MultipleOperationMessage, operations::{AnyOperation, Operation, splice_request::{SpliceRequest, SpliceInsertType}}};
use broadcast_common::{Parse, Serialize};

// Build a splice_request single_operation_message (basic response wrap).
let msg = SingleOperationMessage::new_request(
    0x0101, 0, 1, 42, 0,
    scte104::operations::AnySingleOperation::Unknown { op_id: 0x0101, body: &[] },
);
let bytes = msg.try_to_bytes().unwrap();

// Build a multiple_operation_message with splice + insert_descriptor
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
    scte104::time::Timestamp::None,
    ops,
);
let bytes = mom.try_to_bytes().unwrap();
```

Runnable examples: `cargo run -p scte104 --example build_splice` and
`--example multi_op_round_trip`.

## What is implemented

- `SingleOperationMessage` / `MultipleOperationMessage`: framing, derived
  `message_size()` accessor (no stored size field that could disagree with the data).
- `operations::AnyOperation` (22 multi-operation types) and `AnySingleOperation`
  (15 basic types), each generated from a single declarative list with a drift test
  pinning every opID literal to its type constant.
- `time::Timestamp` (none / UTC / VITC / GPI, typed payloads) and `time::Time`
  (8-byte GPS-epoch time used by `alive_request` / `alive_response`).
- `insert_audio_descriptor`, `insert_audio_provisioning` and
  `insert_alternate_break_duration` are fully typed from Tables 9-33 to 9-36.

## Design notes

- Zero-copy `Parse<'a>` / `Serialize` symmetry; parse then serialize is
  byte-identical (round-trip invariant).
- Operation bodies must be exactly the length the typed structure re-serializes to:
  trailing bytes are rejected, never silently dropped.
- `Error` is `#[non_exhaustive]`. `no_std` + `alloc` without `std`.

## Related crates

Depends only on `broadcast-common`. Downstream of this protocol sits `scte35-splice`
(the SCTE 35 message the compression system emits); `playout-runtime` builds SCTE 35
emission points on that crate.

## MSRV and licence

Rust **1.95.0**. MIT OR Apache-2.0.
