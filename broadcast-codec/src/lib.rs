#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p broadcast-codec --example <name>`.\n"]
#![doc = "\n### `pcm_passthrough`\n\n```rust,ignore"]
#![doc = include_str!("../examples/pcm_passthrough.rs")]
#![doc = "```\n\n### `drain_contract`\n\n```rust,ignore"]
#![doc = include_str!("../examples/drain_contract.rs")]
#![doc = "```\n\n### `registry`\n\n```rust,ignore"]
#![doc = include_str!("../examples/registry.rs")]
#![doc = "```"]

extern crate alloc;

mod audio;
mod availability;
mod codec;
mod descriptor;
mod direction;
mod encoder_config;
mod error;
mod fallback;
mod frame;
mod inventory;
mod packet;
mod padding_error;
mod preference;
mod preference_error;
mod profile;
mod profile_error;
mod provider;
mod provider_info;
mod registry;
mod registry_error;
mod rejection;
mod sample_format;
mod traits;
mod trim;
mod video;

pub use audio::{AudioFrame, AudioSamples, ChannelLayout, SampleLayout};
pub use availability::{Availability, Probe};
// A consumer builds `CodecParameters::new` with this type, so re-export it
// rather than make every adapter depend on `broadcast-core` for one name.
pub use broadcast_core::timescale::Timescale;
pub use codec::{AudioParameters, CodecId, CodecParameters, ConfigurationRecord, VideoParameters};
pub use descriptor::CodecDescriptor;
pub use direction::Direction;
pub use encoder_config::EncoderConfig;
pub use error::CodecError;
pub use fallback::Fallback;
pub use frame::Frame;
pub use inventory::{InventoryEntry, render_inventory};
pub use packet::Packet;
pub use padding_error::PaddingError;
pub use preference::{Preference, parse_preferences};
pub use preference_error::PreferenceError;
pub use profile::CodecProfile;
pub use profile_error::ProfileError;
pub use provider::CodecProvider;
pub use provider_info::ProviderInfo;
pub use registry::{Registry, Selector};
pub use registry_error::RegistryError;
pub use rejection::ProfileRejection;
pub use sample_format::SampleFormat;
pub use traits::{Decoder, Encoder};
pub use trim::TrimStage;
pub use video::{
    ColourDescription, ContentLightLevel, ItuT35, MasteringDisplay, PixelFormat, VideoFrame,
    VideoPlane, VideoStorage,
};
