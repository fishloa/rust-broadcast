#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p atsc3-route --example <name>`.\n"]
#![doc = "\n### `parse_route_fdt`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_route_fdt.rs")]
#![doc = "```\n\n### `build_route_media_fragment`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_route_media_fragment.rs")]
#![doc = "```"]

extern crate alloc;

pub mod codepoint;
mod error;
pub mod ext;
pub mod fec;
mod packet;

pub use codepoint::{Codepoint, CodepointSemantics, FormatId, FragMode};
pub use error::{Error, Result};
pub use ext::{HET_EXT_ROUTE_PRESENTATION_TIME, HET_EXT_TOL_24, HET_EXT_TOL_48};
pub use fec::{
    MAX_ESI, ROUTE_FEC_PAYLOAD_ID_LEN, RepairFecPayloadId, RouteFecPayloadId, SourceFecPayloadId,
};
pub use packet::{
    ROUTE_CCI_LEN, ROUTE_PSI_SOURCE, ROUTE_TOI_LEN, ROUTE_TSI_LEN, ROUTE_VERSION, RoutePacket,
};

// Re-export the underlying `rmt-flute` types this crate's public API is
// expressed in terms of, so a consumer of `RoutePacket` (which embeds an
// `rmt_flute::LctHeader`) never needs an explicit `rmt-flute` dependency of
// its own to build one.
pub use rmt_flute;
