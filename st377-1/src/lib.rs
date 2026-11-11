#![doc = include_str!("../README.md")]
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

pub use ber::{BerLength, ber_length_size_for, decode_ber_length, encode_ber_length_as};
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
