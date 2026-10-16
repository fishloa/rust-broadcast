//! # scte35-splice — ANSI/SCTE 35 2023r1 splice information
//!
//! Spec-cited parser **and builder** for the SCTE 35 Digital Program Insertion
//! cueing message (`splice_info_section`, table_id `0xFC`), with the workspace's
//! symmetric [`Parse`](broadcast_common::Parse)/[`Serialize`](broadcast_common::Serialize)
//! discipline: every wire type round-trips byte-for-byte.
//!
//! The implemented edition is **ANSI/SCTE 35 2023r1** (the single-document
//! edition; SCTE has since split the standard into 35-1 / 35-2). Layouts are
//! transcribed in `scte35-splice/docs/scte_35.md` and cited per module.
//!
//! ## What it does / what it does not
//!
//! **Does:** parse and build `splice_info_section`s byte-exactly: typed commands,
//! typed splice descriptors, segmentation UPID / type-id tables, decoded 90 kHz
//! time accessors, and the DVB Targeted Advertising SCTE 35 profile
//! ([`dvb_ta`]). **Does not:** demux or transport anything (feed it the section
//! bytes), decrypt encrypted sections (the encrypted region is kept raw and
//! round-trips), make ad decisions, implement SCTE 104 (see the `scte104` crate)
//! or the SCTE 35-2 XML/JSON mappings.
//!
//! ## Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | ANSI/SCTE 35 Digital Program Insertion Cueing Message | 2023r1 | §9.6 section, Table 7 commands, Table 16 descriptors, Tables 21-25 | Full (clear); encrypted payload Typed-opaque |
//! | ETSI TS 103 752-1 DVB Targeted Advertising | V1.2.1 | §5.3.5.16 DAS descriptor, §8.3.3 compact forms, §6.3.1 stream-event payload, §5.3.4-5.3.5 profiling | Full, one open ambiguity |
//! | ISO/IEC 13818-1 CRC-32/MPEG-2 | n/a | section `CRC_32` | Full |
//!
//! | SCTE 35 item | Code | Status |
//! |---|---|---|
//! | 6 commands (0x00, 0x04, 0x05, 0x06, 0x07, 0xFF) | [`commands`] | Full; others to `AnyCommand::Unknown` (Typed-opaque) |
//! | 5 descriptors (0x00-0x04) | [`descriptors`] | Full; others to `AnySpliceDescriptor::Unknown` (Typed-opaque) |
//! | `DVB_DAS_descriptor` 0xF0, compact `time_signal`/`splice_insert` | [`dvb_ta`] | Full |
//!
//! **Open question, not guessed:** ETSI TS 103 752-1 Table 1 (stand-alone
//! `DVB_DAS_descriptor()`, type in the low nibble) and Table 8
//! (`compact_splice_insert()`, type in the high nibble) disagree on the nibble
//! order of the same body. [`dvb_ta::DasNibbleOrder`] selects either reading via
//! `parse_with_order` / `serialize_into_with_order`; defaults are Table 1 for the
//! descriptor and Table 8 for the compact form.
//!
//! ## Feature flags
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `std` | on | link `std`; without it the crate is `no_std` + `alloc` |
//! | `serde` | on | Serialize-only `serde::Serialize` on all types |
//!
//! ## Coverage
//!
//! - [`SpliceInfoSection`] — the full §9.6 header (protocol_version, the
//!   encryption flags, 33-bit `pts_adjustment`, `cw_index`, 12-bit `tier`,
//!   `splice_command_length`/type, descriptor loop, CRC_32). Encrypted sections
//!   are kept raw and round-trip losslessly; clear sections expose typed
//!   commands and descriptors.
//! - Commands ([`commands`]): `splice_null`, `splice_schedule`, `splice_insert`,
//!   `time_signal`, `bandwidth_reservation`, `private_command`, unified by
//!   [`AnyCommand`](commands::AnyCommand).
//! - Splice descriptors ([`descriptors`]): `avail`, `DTMF`, `segmentation`
//!   (with [`SegmentationTypeId`](descriptors::SegmentationTypeId) /
//!   [`SegmentationUpidType`](descriptors::SegmentationUpidType)), `time`,
//!   `audio`, unified by
//!   [`AnySpliceDescriptor`](descriptors::AnySpliceDescriptor) with a raw
//!   fall-through.
//! - Typed UPID sub-structures: [`descriptors::Mpu`] (§10.3.3.3, Table 24)
//!   and [`descriptors::MidUpid`] (§10.3.3.4, Table 25) decoded on demand via
//!   [`descriptors::SegmentationDescriptor::mpu`] /
//!   [`descriptors::SegmentationDescriptor::mid`].
//! - Decoded accessors: 90 kHz fields → [`core::time::Duration`]
//!   (`pts_time`, `break_duration`, `pts_adjustment`).
//!
//! ## Quick start
//!
//! ```
//! use scte35_splice::{SpliceInfoSection, commands::AnyCommand};
//! use broadcast_common::{Parse, Serialize};
//!
//! // A minimal time_signal() section with no descriptors, built and emitted.
//! let ts = scte35_splice::commands::TimeSignal {
//!     splice_time: scte35_splice::time::SpliceTime::with_pts(0x0_0012_3456),
//! };
//! let section = SpliceInfoSection::new_clear(AnyCommand::TimeSignal(ts), &[]);
//! let bytes = section.try_to_bytes().unwrap();
//! assert_eq!(bytes[0], 0xFC); // table_id
//!
//! // ...and parsed straight back.
//! let parsed = SpliceInfoSection::parse(&bytes).unwrap();
//! assert!(matches!(parsed.clear.as_ref().unwrap().command, AnyCommand::TimeSignal(_)));
//! ```
//!
//! ## dvb-si integration
//!
//! SCTE 35 sections ride on a PID labelled in the PMT by a registration
//! descriptor carrying the format_identifier `"CUEI"` (which dvb-si already
//! parses). Once you have the `0xFC` section bytes, route them here:
//!
//! ```
//! use scte35_splice::SpliceInfoSection;
//! use broadcast_common::{Parse, Serialize};
//!
//! // A splice_null() section produced by this crate stands in for bytes a
//! // dvb-si demux would hand you from the SCTE 35 PID.
//! let section = SpliceInfoSection::new_clear(
//!     scte35_splice::commands::AnyCommand::SpliceNull(Default::default()),
//!     &[],
//! );
//! let on_the_wire = section.try_to_bytes().unwrap();
//!
//! // table_id 0xFC marks it as a splice_info_section; parse it.
//! assert_eq!(on_the_wire[0], scte35_splice::section::TABLE_ID);
//! let parsed = SpliceInfoSection::parse(&on_the_wire).unwrap();
//! assert_eq!(parsed.descriptors().count(), 0);
//! ```
//!
//! ## Reserved-bit & CRC policy
//!
//! Reserved bits are written as `1` on serialize (the spec's convention) and
//! ignored on parse. `splice_info_section` uses the MPEG-2 CRC-32
//! ([`broadcast_common::crc32_mpeg2`], §9.6.1): parse verifies it, serialize
//! recomputes it. [`Error`] is `#[non_exhaustive]`; serializers reject over-range
//! fields with `Error::FieldOverflow` rather than masking them.
//!
//! ## Related crates and MSRV
//!
//! Depends on `broadcast-common`; used by `timed-metadata`, `ssai-runtime` and
//! `playout-runtime`; `dvb-si` finds the SCTE 35 PID. MSRV 1.95.0; licence
//! MIT OR Apache-2.0.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p scte35-splice --example <name>`).\n"]
#![doc = "\n## `parse_splice_insert`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_splice_insert.rs")]
#![doc = "```\n\n## `round_trip_and_descriptors`\n\n```rust,ignore"]
#![doc = include_str!("../examples/round_trip_and_descriptors.rs")]
#![doc = "```"]

extern crate alloc;

pub mod commands;
pub mod descriptors;
pub mod dvb_ta;
pub mod error;
pub mod section;
pub mod time;
pub mod traits;

pub use error::{Error, Result};
pub use section::{ClearPayload, SpliceInfoSection};
pub use traits::{CommandDef, SpliceDescriptorDef};
