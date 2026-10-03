//! Async/tokio stream adapters for DVB SI and T2-MI processing.
//!
//! This crate wraps the synchronous [`dvb_si::demux::SiDemux`] and
//! [`dvb_t2mi::pump::T2miPump`] as [`futures_core::Stream`] implementations,
//! quarantining `tokio` and `futures-core` away from the parser crates.
//!
//! # Streams
//!
//! - [`SectionStream`] — wraps [`dvb_si::demux::SiDemux`] over any
//!   [`tokio::io::AsyncRead`] byte source (file, TCP socket). Each item is an
//!   owned [`dvb_si::demux::SectionEvent`] (`'static`, no borrow of the read
//!   buffer).
//!
//! - [`T2miEventStream`] — wraps [`dvb_t2mi::pump::T2miPump`] over any
//!   [`tokio::io::AsyncRead`]. Each item is an owned
//!   [`dvb_t2mi::pump::T2miEvent`].
//!
//! - [`UdpSectionStream`] / [`UdpT2miStream`] (feature `udp`) — the same two
//!   pumps over a UDP/multicast socket, the dominant real-world DVB transport.
//!   Built with `bind_multicast` / `bind(&MulticastConfig)` (`socket2`:
//!   `SO_RCVBUF`, `SO_REUSEADDR`, multicast interface) or from an already-bound
//!   socket with `from_socket`.
//!
//! # Ownership and cancellation
//!
//! The adapter **owns** the read buffer and feeds bytes into the synchronous pump
//! on each `poll_next` call. Events are buffered in a small per-packet queue and
//! drained before the next read is attempted. There are no internal tasks or
//! spawning; cancellation is simply dropping the stream.
//!
//! # 188-byte TS framing and resync
//!
//! The adapter frames the source with a `tokio_util::codec::FramedRead` over
//! [`TsDecoder`], which performs 188-byte TS packet alignment via a sync-byte
//! (`0x47`) resync. The resync logic is implemented once in [`resync`] and
//! shared by every stream; UDP sources use [`TsDecoder::datagram`] so each
//! datagram is resynchronised independently.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `udp`   | on      | [`UdpSectionStream`] / [`UdpT2miStream`] and [`udp::MulticastConfig`] (`socket2` bind + join, `tokio_util::udp::UdpFramed` framing). |
//!
//! # MSRV
//!
//! `dvb-stream` **1.86** (mirrors the workspace). This crate is versioned and
//! released **independently** from the `dvb-si` / `dvb-t2mi` lockstep because
//! tokio's own MSRV moves faster.

// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-stream --example <name>`).\n"]
#![doc = "\n## `count_sections`\n\n```rust,ignore"]
#![doc = include_str!("../examples/count_sections.rs")]
#![doc = "```\n\n## `stream_stats`\n\n```rust,ignore"]
#![doc = include_str!("../examples/stream_stats.rs")]
#![doc = "```"]

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
