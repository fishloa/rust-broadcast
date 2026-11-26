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
#![doc = "```"]

extern crate alloc;

mod audio;
mod codec;
mod error;
mod frame;
mod packet;
mod traits;
mod video;

pub use audio::{AudioFrame, AudioSamples, ChannelLayout, SampleLayout};
pub use codec::{AudioParameters, CodecId, CodecParameters, VideoParameters};
pub use error::CodecError;
pub use frame::Frame;
pub use packet::Packet;
pub use traits::{Decoder, Encoder};
pub use video::{ColourDescription, PixelFormat, VideoFrame, VideoPlane, VideoStorage};
