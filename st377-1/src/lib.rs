//! SMPTE ST 377-1:2019 Material Exchange Format (MXF): KLV framing, Partition
//! Pack, Primer Pack, the Header Metadata Sets of an OP1a file, and the
//! Random Index Pack, `no_std` + `alloc`.
//!
//! The wire structures follow the curated transcription at
//! `st377-1/docs/st377-1.md` (from
//! `https://pub.smpte.org/latest/st377-1/st377-1-2019.pdf`); cite that file,
//! not this comment, as the field-semantics oracle. See also
//! `docs/st378-op1a.md` for the OP1a transcription.
//!
//! # What it does / what it does not
//!
//! MXF is the professional file-interchange wrapper for audio/video. This
//! crate reads and writes the **structure** of an MXF file: KLV triplets, the
//! partition layout and the structural metadata describing the contents. It is
//! the workspace's first file-based-interchange crate.
//!
//! It does **not**:
//!
//! - decode essence: Essence Container payload bytes stay opaque
//!   [`KlvItem`]s;
//! - type Essence Descriptors (F.2 to F.6 and per-codec registrations):
//!   [`SourcePackage::descriptor`] is a bare [`StrongRef`] this crate can
//!   neither resolve nor build a target for;
//! - decode Index Table contents, DM Segments / DM Source Clips or
//!   Application Metadata Sets (identified via [`StructuralSetKind`], kept as
//!   generic [`LocalSet`]s);
//! - assemble a file: nothing computes cross-Partition offsets,
//!   `HeaderByteCount`/`IndexByteCount`, or a [`RandomIndexPack`] pointing at
//!   real offsets;
//! - implement other Operational Patterns or Essence Container mappings.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope / clauses | Status | Notes |
//! |---|---|---|---|---|
//! | SMPTE ST 377-1, MXF File Format Specification | 2019 (vendored; revision of 377-1-2009) | §6.3 KLV/BER, §7.1-7.4 Partition Pack, §9.2 Primer Pack, §9.3 local sets, Table 17 Set identification, §12 Random Index Pack, Annex A root sets, Annex B/E structural sets | Partial | typed vs opaque split below |
//! | SMPTE ST 378, MXF OP1a | 2004 (378M-2004, vendored) | [`op1a`] Operational Pattern UL identification and qualifier bits | Partial | UL helpers only; the minimum-file `EssenceDescriptor` requirement is not met |
//! | Essence-container mapping specs (ST 379 family) | not vendored | none | Not implemented | essence stays opaque |
//!
//! Typed Header Metadata Sets (dedicated struct, field-by-field,
//! byte-identical re-serialization):
//!
//! - Root sets (Annex A): [`Preface`], [`Identification`], [`ContentStorage`],
//!   [`EssenceContainerData`].
//! - Packages: [`MaterialPackage`], [`SourcePackage`].
//! - Tracks: [`TimelineTrack`], [`EventTrack`], [`StaticTrack`].
//! - Components: [`Sequence`], [`SourceClip`], [`TimecodeComponent`],
//!   [`FillerComponent`].
//!
//! Typed packs: [`PartitionPack`] ([`PartitionKind`] x [`PartitionStatus`]),
//! [`PrimerPack`], [`RandomIndexPack`] ([`PartitionLocation`]).
//!
//! Identified only ([`StructuralSetKind`] name + generic [`LocalSet`], bytes
//! preserved): DM Segment, DM Source Clip, Package Marker Object, the File /
//! Generic Picture / CDCI / RGBA / Generic Sound / Generic Data / Multiple
//! Descriptors, Network and Text Locators, Application Plug-In and Referenced
//! Objects, and `Unknown([u8; 2])` for any Set not in Table 17.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | yes | Link the standard library; without it the crate is `#![no_std]` + `alloc`. |
//! | `serde` | no | `serde::Serialize`/`Deserialize` derives on public types. |
//!
//! # What is implemented
//!
//! - **KLV**: [`KlvItem`], [`walk_klv_items`], [`collect_klv_items`],
//!   fill-item helpers ([`is_fill_item_key`]) and [`BerLength`], which records
//!   the on-wire BER length width so re-serialization is byte-identical even
//!   for the fixed-width long forms real encoders use (issue #1047).
//! - **Packs**: [`PartitionPack`], [`PrimerPack`], [`RandomIndexPack`].
//! - **Local sets**: [`LocalSet`] / [`LocalSetItem`] ([`ItemLengthMode`]) and
//!   [`StructuralSetKind`] (Table 17).
//! - **Value types**: [`Auid`], [`PackageId`], [`StrongRef`], [`Rational`],
//!   [`MxfTimestamp`], [`ProductVersion`], [`UlBytes`], UTF-16BE helpers.
//! - **[`op1a`]**: `OP1A_UL_PREFIX`, `Op1aQualifier`, `is_op1a`, `op1a_ul`.
//! - Validated against a real ffmpeg-muxed OP1a file
//!   (`tests/fixtures/op1a_mpeg2_pcm.mxf`; all 25 of its partition packs
//!   round-trip byte-identically) and a small synthetic file.
//!
//! ## OP1a support is structural-metadata-only (issue #937)
//!
//! The typed OP1a Header Metadata Sets parse and round-trip losslessly, but two
//! gaps remain by design:
//!
//! - **No Essence Descriptor type.** A minimal OP1a file needs the File Package
//!   to carry an `EssenceDescriptor` (`docs/st378-op1a.md` §6.5/§8). The real
//!   fixture carries an MPEG Video Descriptor and a Wave Audio Descriptor, both
//!   defined by sibling essence-container-mapping specs, which this crate does
//!   not anticipate.
//! - **No file assembler.** Every offset/byte-count field in
//!   `tests/round_trip.rs`'s `full_op1a_structure_builds_and_round_trips` is a
//!   hardcoded placeholder, not a computed value. A full implementation would
//!   need a typed `EssenceDescriptor` family plus a writer that lays out
//!   Partitions, tracks running offsets, backpatches the byte counts and emits a
//!   `RandomIndexPack` from the real offsets: a second, comparably-sized
//!   project, tracked separately.
//!
//! # Design notes
//!
//! - Symmetric `Parse`/`Serialize` with byte-identical round-trips; equality
//!   ignores the serialization-form field `len_size`, and a freshly built value
//!   defaults to [`BerLength::Minimal`].
//! - [`Preface::identifications`] is an `Option` so an absent property is
//!   distinguished from an empty batch.
//! - Public enums are `#[non_exhaustive]` with `name()` + `Display` labels.
//! - `no_std` + `alloc`; one `thiserror` enum, [`Error`]. MSRV 1.95.0; licence
//!   MIT OR Apache-2.0.
//!
//! # Related crates
//!
//! Depends only on `broadcast-common`. Sibling "container, not codec" crates:
//! `st337` and `rdd29`.
//!
//! # Examples
//!
//! Build a Partition Pack and round-trip it:
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use st377_1::{BerLength, PartitionKind, PartitionPack, PartitionStatus};
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
//!     len_size: BerLength::Minimal,
//! };
//! let bytes = pack.try_to_bytes().unwrap();
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
mod filler_component;
mod identification;
mod klv;
mod local_set;
pub mod op1a;
mod package;
mod partition;
mod preface;
mod primer;
mod random_index_pack;
mod sequence;
mod sets;
mod source_clip;
mod timecode_component;
mod track;
mod types;

pub use ber::BerLength;
pub use content_storage::ContentStorage;
pub use error::{Error, Result};
pub use essence_container_data::EssenceContainerData;
pub use filler_component::FillerComponent;
pub use identification::Identification;
pub use klv::{
    FILL_ITEM_KEY_PREFIX, FILL_ITEM_KEY_SUFFIX, KlvItem, collect_klv_items, is_fill_item_key,
    walk_klv_items,
};
pub use local_set::{ItemLengthMode, LocalSet, LocalSetItem, StructuralSetKind, is_local_set_key};
pub use package::{MaterialPackage, SourcePackage};
pub use partition::{PartitionKind, PartitionPack, PartitionStatus};
pub use preface::Preface;
pub use preface::VERSION_1_3;
pub use primer::PrimerPack;
pub use random_index_pack::{PartitionLocation, RandomIndexPack};
pub use sequence::Sequence;
pub use sets::InterchangeObjectFields;
pub use source_clip::SourceClip;
pub use timecode_component::TimecodeComponent;
pub use track::{EventTrack, StaticTrack, TimelineTrack};
pub use types::{
    Auid, MxfTimestamp, PRODUCT_VERSION_LEN, PackageId, ProductVersion, RATIONAL_LEN, Rational,
    ReleaseType, StrongRef, TIMESTAMP_LEN, UlBytes, decode_utf16_be, encode_utf16_be,
    parse_uid_batch, serialize_uid_batch,
};

// Re-exported so downstream code can build owned local-set item lists for
// dark/private extensions without depending on this crate's internal
// module layout.
pub use sets::LocalSetOwnedItem;
