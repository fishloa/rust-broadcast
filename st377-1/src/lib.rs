//! SMPTE ST 377-1:2019 "Material Exchange Format (MXF) — File Format
//! Specification".
//!
//! This crate implements exactly the wire structure described in the
//! curated spec transcription at `st377-1/docs/st377-1.md` (fetched
//! directly from `https://pub.smpte.org/latest/st377-1/st377-1-2019.pdf`) —
//! cite that file, not this doc comment, as the field-semantics oracle. It
//! also documents in detail this crate's scope decision: MXF is a huge
//! ecosystem spec (Operational Patterns, Essence Container mappings, DM/
//! Application Metadata plug-ins, per-essence-kind Descriptors all live in
//! sibling documents this crate does not attempt to anticipate), so this
//! first pass fully types the format's own backbone and the four Root
//! Metadata Sets every real MXF file has, and falls back to an identified-
//! but-generic passthrough for everything else — see `docs/st377-1.md`'s
//! "Scope decision for this crate" section for the full breakdown with
//! spec citations.
//!
//! - [`KlvItem`] — the generic KLV (Key-Length-Value) triplet (§6.3) every
//!   other structure in an MXF file rides on; [`walk_klv_items`] /
//!   [`collect_klv_items`] walk a sequence of them.
//! - [`PartitionPack`] — the Header/Body/Footer Partition Pack (§7.1-§7.4,
//!   Tables 4-8): [`PartitionKind`] + [`PartitionStatus`] plus every Table 5
//!   field.
//! - [`PrimerPack`] — the per-Partition local-tag lookup table (§9.2).
//! - [`LocalSet`] — the generic "local set" KLV-lite framing (§9.3) used by
//!   every Header Metadata Set; [`StructuralSetKind`] identifies which Set
//!   a given instance is (Table 17), even for the many Sets this crate does
//!   not deeply type.
//! - [`Preface`], [`Identification`], [`ContentStorage`],
//!   [`EssenceContainerData`] — the four Root Metadata Sets (Annex A) every
//!   real MXF file has exactly one/more of, decoded field-by-field.
//! - [`RandomIndexPack`] — the optional file-trailer Partition index (§12).
//!
//! **Out of scope entirely**: Essence Container payload bytes (the actual
//! audio/video/data samples) — carried opaquely via [`KlvItem`], never
//! decoded, the same boundary as `st337`'s `burst_payload`/`rdd29`'s
//! `AudioDataDLC`. Index Table *contents* and every Operational-Pattern- or
//! essence-kind-specific Set (Packages/Tracks/Sequences/Descriptors, DM/
//! Application Metadata) are identified via [`StructuralSetKind`] but not
//! individually typed — see `docs/st377-1.md`.
//!
//! Depends only on `broadcast-common`. `#![no_std]` + `alloc` when the
//! `std` feature is disabled.
//!
//! # Examples
//!
//! Parse a Partition Pack and walk its Header Metadata:
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use st377_1::{PartitionKind, PartitionPack, PartitionStatus};
//!
//! let pack = PartitionPack {
//!     kind: PartitionKind::Header,
//!     status: PartitionStatus::ClosedComplete,
//!     major_version: 1,
//!     minor_version: 3,
//!     kag_size: 512,
//!     this_partition: 0,
//!     previous_partition: 0,
//!     footer_partition: 0,
//!     header_byte_count: 0,
//!     index_byte_count: 0,
//!     index_sid: 0,
//!     body_offset: 0,
//!     body_sid: 0,
//!     operational_pattern: [0u8; 16],
//!     essence_containers: Vec::new(),
//! };
//! let bytes = pack.to_bytes();
//! assert_eq!(PartitionPack::parse(&bytes).unwrap(), pack);
//! ```
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync
// with the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p st377-1 --example <name>`.\n"]
#![doc = "\n### `parse_partition`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_partition.rs")]
#![doc = "```\n\n### `build_preface`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_preface.rs")]
#![doc = "```"]

extern crate alloc;

mod ber;
mod content_storage;
mod error;
mod essence_container_data;
mod identification;
mod klv;
mod local_set;
mod partition;
mod preface;
mod primer;
mod random_index_pack;
mod sets;
mod types;

pub use content_storage::ContentStorage;
pub use error::{Error, Result};
pub use essence_container_data::EssenceContainerData;
pub use identification::Identification;
pub use klv::{
    FILL_ITEM_KEY_PREFIX, FILL_ITEM_KEY_SUFFIX, KlvItem, collect_klv_items, is_fill_item_key,
    walk_klv_items,
};
pub use local_set::{ItemLengthMode, LocalSet, LocalSetItem, StructuralSetKind, is_local_set_key};
pub use partition::{PartitionKind, PartitionPack, PartitionStatus};
pub use preface::Preface;
pub use preface::VERSION_1_3;
pub use primer::PrimerPack;
pub use random_index_pack::{PartitionLocation, RandomIndexPack};
pub use sets::InterchangeObjectFields;
pub use types::{
    Auid, MxfTimestamp, PRODUCT_VERSION_LEN, PackageId, ProductVersion, ReleaseType, TIMESTAMP_LEN,
    UlBytes, decode_utf16_be, encode_utf16_be, parse_uid_batch, serialize_uid_batch,
};

// Re-exported so downstream code can build owned local-set item lists for
// dark/private extensions without depending on this crate's internal
// module layout.
pub use sets::LocalSetOwnedItem;
