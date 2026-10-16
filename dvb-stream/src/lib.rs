#![doc = include_str!("../README.md")]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-stream --example <name>`).\n"]
#![doc = "\n## `count_sections`\n\n```rust,ignore"]
#![doc = include_str!("../examples/count_sections.rs")]
#![doc = "```\n\n## `stream_stats`\n\n```rust,ignore"]
#![doc = include_str!("../examples/stream_stats.rs")]
#![doc = "```"]
#![cfg_attr(docsrs, feature(doc_cfg))]

// Drift-guard exemption (issue #806): this crate defines no `pub enum` at
// all (only stream adapter structs and `ResyncStats` below), so neither the
// `tests/label_coverage.rs` (#204 Display convention) nor the
// `tests/non_exhaustive_coverage.rs` (`#[non_exhaustive]`) drift guard has
// anything to police. Recorded in `broadcast-common`'s
// `tests/workspace_drift_guard_coverage.rs` exemption lists.

pub mod resync;
pub mod section_stream;
pub mod t2mi_stream;
mod ts_codec;
#[cfg(feature = "udp")]
pub mod udp;

pub use section_stream::SectionStream;
#[cfg(feature = "udp")]
pub use section_stream::UdpSectionStream;
pub use t2mi_stream::T2miEventStream;
#[cfg(feature = "udp")]
pub use t2mi_stream::UdpT2miStream;
pub use ts_codec::TsDecoder;

/// Statistics tracking resynchronisation events and discarded bytes in a TS
/// byte stream.
///
/// Returned by [`SectionStream::resync_stats`] and
/// [`T2miEventStream::resync_stats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResyncStats {
    /// Number of times the stream re-aligned on a new sync byte.
    pub resyncs: u64,
    /// Total bytes discarded due to resync alignment or mid-stream desync.
    pub bytes_discarded: u64,
    /// Number of mid-stream alignment losses detected (a packet whose first
    /// byte was not `0x47`).
    pub desyncs: u64,
}
