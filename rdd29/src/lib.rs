#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p rdd29 --example <name>`.\n"]
#![doc = "\n### `build_atmos_frame`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_atmos_frame.rs")]
#![doc = "```\n\n### `parse_atmos_frame`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_atmos_frame.rs")]
#![doc = "```"]

extern crate alloc;

mod atmos_frame;
mod audio_data_dlc;
mod bed_definition;
pub mod distance;
mod element;
mod error;
mod frame_rate;
mod object_definition;
mod plex;
mod util;

pub use atmos_frame::{ATMOS_VERSION, AtmosFrame, BitDepth, SampleRate};
pub use audio_data_dlc::AudioDataDlc;
pub use bed_definition::{BedChannel, BedDefinition1, ChannelId};
pub use element::{
    AnyElement, ELEMENT_ID_ATMOS_FRAME, ELEMENT_ID_AUDIO_DATA_DLC, ELEMENT_ID_BED_DEFINITION1,
    ELEMENT_ID_OBJECT_DEFINITION1, ElementId,
};
pub use error::{Error, Result};
pub use frame_rate::FrameRate;
pub use object_definition::{
    AudioDescription, DecorCoefPrefix, MAX_KNOWN_ZONES, ObjectDefinition1, ObjectSpreadMode,
    PanInfo, PanSubBlock, ZoneGain, ZoneId,
};
pub use plex::PLEX_MAX_VALUE;
