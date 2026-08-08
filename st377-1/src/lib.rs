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
//! - [`MaterialPackage`], [`SourcePackage`] — the two concrete Package
//!   kinds (Annex E / B.1), carrying Package UID, dates, and Track
//!   references.
//! - [`TimelineTrack`], [`EventTrack`], [`StaticTrack`] — the three Track
//!   kinds (B.12/B.13/B.14), wrapping a Sequence reference plus timing
//!   properties.
//! - [`Sequence`] — the ordered component collection inside every Track
//!   (B.9).
//! - [`SourceClip`] — a component referencing a span of Source Package
//!   essence (B.10).
//! - [`TimecodeComponent`] — a component carrying a timecode reference
//!   (B.17).
//! - [`FillerComponent`] — a gap placeholder inside a Sequence (B.11).
//! - [`op1a`] — OP1a Operational Pattern UL helpers (ST 378).
//! - [`RandomIndexPack`] — the optional file-trailer Partition index (§12).
//!
//! **Out of scope entirely**: Essence Container payload bytes (the actual
//! audio/video/data samples) — carried opaquely via [`KlvItem`], never
//! decoded, the same boundary as `st337`'s `burst_payload`/`rdd29`'s
//! `AudioDataDLC`. Index Table *contents*, Descriptors (F.*), DM Segments/
//! Source Clips (B.32-B.33), and Application Metadata Sets (C.*) are
//! identified via [`StructuralSetKind`] but not individually typed — see
//! `docs/st377-1.md`.
//!
//! ## OP1a support is structural-metadata-only (issue #937)
//!
//! [`op1a`] plus [`MaterialPackage`]/[`SourcePackage`]/[`TimelineTrack`]/
//! [`EventTrack`]/[`StaticTrack`]/[`Sequence`]/[`SourceClip`]/
//! [`TimecodeComponent`]/[`FillerComponent`] parse and byte-losslessly
//! round-trip every OP1a Header Metadata Set this crate types (see
//! `docs/st378-op1a.md`), and are validated against a real `ffmpeg`-muxed
//! OP1a file in `tests/fixture_real_op1a.rs`. Two things this does **not**
//! add up to:
//!
//! - **No Essence Descriptor type.** `docs/st378-op1a.md`'s minimum OP1a
//!   file requires the File Package to carry an `EssenceDescriptor`
//!   (§6.5/§8), but this crate has no typed representation of any
//!   Descriptor (F.2-F.6) — [`SourcePackage::descriptor`] is a bare
//!   [`StrongRef`], a 16-byte Instance UID this crate can neither resolve
//!   nor build a target for. Doing so properly would mean typing not just
//!   ST 377-1's own generic Descriptor Sets but the per-essence-kind
//!   registrations that actually appear on the wire (this crate's real
//!   fixture carries an MPEG Video Descriptor and a Wave Audio Descriptor,
//!   both defined by *sibling* essence-container-mapping specs, not
//!   ST 377-1 itself) — exactly the ecosystem-anticipation problem the
//!   Scope section above already declines to take on.
//! - **No file assembler.** Nothing in this crate computes cross-Partition
//!   byte offsets (`ThisPartition`/`PreviousPartition`/`FooterPartition`),
//!   `HeaderByteCount`/`IndexByteCount`, or builds a [`RandomIndexPack`]
//!   that actually points at the Partitions it describes.
//!   [`PartitionPack`], [`PrimerPack`], the typed Header Metadata Sets, and
//!   [`RandomIndexPack`] each parse and serialize correctly in isolation,
//!   but nothing stitches them into one valid, playable OP1a file —
//!   confirm this yourself in `tests/round_trip.rs`'s
//!   `full_op1a_structure_builds_and_round_trips`: every offset/byte-count
//!   field there is a hardcoded placeholder (`0`, or `9999` for the
//!   `RandomIndexPack` byte offset), not a computed value.
//!
//! A full implementation would need, at minimum: a typed `EssenceDescriptor`
//! family (File/Generic Picture/CDCI/RGBA/Generic Sound/Generic Data/
//! Multiple, F.2-F.6) plus a way to plug in essence-kind-specific
//! descriptors from sibling specs; and a writer that lays out Partitions in
//! order, tracks running byte offsets as it serializes each one, backpatches
//! `HeaderByteCount`/`IndexByteCount`/`ThisPartition`/`PreviousPartition`/
//! `FooterPartition`, and emits a `RandomIndexPack` from the real offsets.
//! That is a second, comparably-sized project; tracked separately rather
//! than attempted here.
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
