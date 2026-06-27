//! DVB Targeted Advertising — binary SCTE 35 profile (ETSI TS 103 752-1 V1.2.1).
//!
//! ETSI TS 103 752-1 ("Carriage and signalling of placement opportunity
//! information in DVB Transport Streams", the DVB-TA Part 1 deliverable) defines
//! a typed DVB profile over ANSI/SCTE 35. It does **not** re-define the SCTE 35
//! wire format; instead it (1) profiles how the base structures
//! (`splice_info_section()`, `splice_insert()`, `time_signal()`,
//! `segmentation_descriptor()`) are constrained for Digital Advertising
//! Substitution (DAS), and (2) adds a small amount of NEW binary syntax. This
//! module implements only the new/profiled binary syntax; the base SCTE 35
//! structures it references are reused from this crate (not reimplemented).
//!
//! Layouts are transcribed in `scte35-splice/docs/dvb_ta/` and cited per module.
//!
//! ## Contents
//!
//! - [`das_descriptor`] — `DVB_DAS_descriptor()` (§5.3.5.16, tag `0xF0`): a
//!   DVB-private splice descriptor carried in the base `splice_info_section()`
//!   descriptor loop; integrated into
//!   [`AnySpliceDescriptor`](crate::descriptors::AnySpliceDescriptor).
//! - [`compact`] — the Compact SCTE 35 Encoding Format (§8.3.3): a binary
//!   alternative for low-capacity watermark carriage
//!   ([`compact::CompactScte35`] / `compact_time_signal()` /
//!   `compact_splice_insert()`).
//! - [`stream_event`] — `DSM-CC_stream_event_payload_binary()` (§6.3.1): the
//!   binary payload wrapping (or referencing) a full SCTE 35 section for DSM-CC
//!   stream-event carriage, plus the RFC 4648 base-64 helper.
//! - [`profiling`] — typed helpers over base SCTE 35 for the PPO/DPO
//!   `segmentation_type_id` constraints (§5.3.4–5.3.5; no new wire syntax).
//!
//! ## What is referenced vs. new
//!
//! The base SCTE 35 structures (`splice_info_section()`, `splice_insert()`,
//! `time_signal()`, `segmentation_descriptor()`, the CRC-32 and the
//! `segmentation_type_id` / `segmentation_upid_type` tables) are **reused** from
//! the crate's existing modules. Only the four items above are new in this
//! profile.

pub mod compact;
pub mod das_descriptor;
pub mod profiling;
pub mod stream_event;

pub use compact::{CompactDas, CompactScte35, CompactSpliceInsert, CompactTimeSignal};
pub use das_descriptor::{DvbDasDescriptor, EquivalentSegmentationType, DVB_IDENTIFIER};
pub use profiling::{validate_po_segmentation, PlacementOpportunity, ProfileViolation};
pub use stream_event::{
    base64_encode, PrivateData, Scte35Carriage, StreamEventPayload, TimelineType,
};
