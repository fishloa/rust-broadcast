//! # scte104 — ANSI/SCTE 104 2023 Automation↔Compression DPI signalling
//!
//! SCTE 104 is the message protocol an automation system uses to tell a
//! compression/injection system to insert SCTE 35 (DPI) cueing into the outgoing
//! Transport Stream. This crate parses + builds the SCTE 104 messages:
//! `single_operation_message` and `multiple_operation_message` and the DPI
//! operations they carry (splice, time_signal, insert-descriptor, segmentation,
//! …).
//!
//! ANSI/SCTE 104 2023 — Automation System to Compression System Communications
//! Applications Program Interface (API).
//!
//! Depends only on `broadcast-common` and is `#![no_std]` (+ `alloc`).
//!
//! ## What it does / what it does not
//!
//! Gives you the wire messages: typed operations, message framing and the
//! variable-length timestamp, for an automation-side sender or a compression-side
//! receiver / monitor. It does **not** implement the transport (connection
//! handling, session state machines, timers) and does not turn a message into a
//! SCTE 35 section (`inject_section_data` carries SCTE 35 bytes opaquely; use the
//! `scte35-splice` crate for those).
//!
//! ## Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | ANSI/SCTE 104 Automation System to Compression System Communications API | 2023 | §8.2.2 / §8.2.3 framing, Table 8-3 (15 basic ops), Table 8-4 (22 ops), §12.4 `Time`, §12.5 `Timestamp` | Full for the opID tables; unknown opIDs Typed-opaque (`Unknown`) |
//!
//! ## Feature flags
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `std` | on | link `std`; without it `no_std` + `alloc` |
//! | `serde` | off | `serde::Serialize` on all types |
//!
//! ## Coverage
//!
//! - [`SingleOperationMessage`] — single-operation framing (§8.2.2, Table 8-1)
//!   with basic request/response operations.
//! - [`MultipleOperationMessage`] — multi-operation framing (§8.2.3, Table 8-2)
//!   with Normal, Supplemental, and Control operations.
//! - All 15 Table 8-3 operations (basic request/response, including the
//!   `general_response`/`init_*`/`alive_*`/`inject_*` messages of §9 and the
//!   `config_*`/`provisioning_*`/`fault_*`/`AS_alive_*` PAMS⇔AS messages of
//!   §10) and all 22 Table 8-4 operations (Normal/Supplemental/Control):
//!   splice, time_signal, splice_null, descriptor inserts, segmentation,
//!   encryption, schedule, control words, proprietary commands, and more.
//! - [`Timestamp`](time::Timestamp) (§12.5): variable-length (none/UTC/VITC/GPI)
//!   with typed payloads.
//! - [`Time`](time::Time) (§12.4): 8-byte GPS-epoch timestamp used in
//!   alive_request/response.
//! - [`AnyOperation`](operations::AnyOperation): unified dispatch enum with
//!   a drift test pinning opID literals to type constants.
//!
//! ## Quick start
//!
//! ```
//! use scte104::{SingleOperationMessage, MultipleOperationMessage, operations::{AnyOperation, Operation, splice_request::{SpliceRequest, SpliceInsertType}}};
//! use broadcast_common::{Parse, Serialize};
//!
//! // Build a splice_request single_operation_message (basic response wrap).
//! let msg = SingleOperationMessage::new_request(
//!     0x0101, 0, 1, 42, 0,
//!     scte104::operations::AnySingleOperation::Unknown { op_id: 0x0101, body: &[] },
//! );
//! let bytes = msg.try_to_bytes().unwrap();
//!
//! // Build a multiple_operation_message with splice + insert_descriptor
//! let ops = vec![
//!     Operation {
//!         op_id: 0x0101,
//!         data: AnyOperation::SpliceRequest(SpliceRequest {
//!             splice_insert_type: SpliceInsertType::SpliceStartNormal,
//!             splice_event_id: 42,
//!             unique_program_id: 1,
//!             pre_roll_time: 5000,
//!             break_duration: 300,
//!             avail_num: 0,
//!             avails_expected: 0,
//!             auto_return_flag: 1,
//!             not_an_entry_flag: 0,
//!         }),
//!     },
//! ];
//! let mom = MultipleOperationMessage::new(
//!     0, 1, 42, 0, 0,
//!     scte104::time::Timestamp::None,
//!     ops,
//! );
//! let bytes = mom.try_to_bytes().unwrap();
//! ```
//!
//! ## Design notes and related crates
//!
//! Zero-copy `Parse`/`Serialize` symmetry with a byte-identical round trip;
//! operation bodies must be exactly the typed length (trailing bytes are
//! rejected); [`Error`] is `#[non_exhaustive]`. Depends only on `broadcast-common`;
//! its output feeds `scte35-splice` (via a compression system). MSRV 1.95.0;
//! MIT OR Apache-2.0.
//!
//! ## Examples
//!
//! Two runnable examples ship with this crate (`cargo run -p scte104 --example <name>`).
//!
//! ```rust,ignore
#![doc = include_str!("../examples/build_splice.rs")]
//! ```
//!
//! ```rust,ignore
#![doc = include_str!("../examples/multi_op_round_trip.rs")]
//! ```
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

pub mod error;
pub mod multi;
pub mod operations;
pub mod single;
pub mod time;
pub mod traits;

pub use error::{Error, Result};
pub use multi::MultipleOperationMessage;
pub use single::SingleOperationMessage;
