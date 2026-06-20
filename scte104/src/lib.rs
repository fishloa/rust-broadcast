//! ANSI/SCTE 104 2023 — Automation System to Compression System Communications
//! Applications Program Interface (API).
//!
//! SCTE 104 is the message protocol an automation system uses to tell a
//! compression/injection system to insert SCTE 35 (DPI) cueing into the outgoing
//! Transport Stream. This crate parses + builds the SCTE 104 messages:
//! `single_operation_message` and `multiple_operation_message` and the DPI
//! operations they carry (splice, time_signal, insert-descriptor, segmentation,
//! …).
//!
//! Depends only on `dvb-common` and is `#![no_std]` (+ `alloc`).
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

// Implemented by story #260 (delegated): message framing + the operation set
// from the op_id table (ANSI/SCTE 104 2023 §10–§13), with symmetric Serialize.
