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
//! Both streams are also constructable from a UDP multicast socket (the dominant
//! real-world DVB transport) via the `bind_multicast` constructor when the `udp`
//! feature is enabled.
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
//! The adapter reads raw bytes from the `AsyncRead` source and performs 188-byte
//! TS packet alignment via a sync-byte (`0x47`) resync on the read buffer. The
//! resync logic is implemented once in [`resync`] and shared by both streams.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `udp`   | on      | UDP/multicast constructors (`bind_multicast`) via `tokio::net::UdpSocket`. |
//!
//! # MSRV
//!
//! `dvb-stream` **1.81** (mirrors the workspace). This crate is versioned and
//! released **independently** from the `dvb-si` / `dvb-t2mi` lockstep because
//! tokio's own MSRV moves faster.

pub mod resync;
pub mod section_stream;
pub mod t2mi_stream;

pub use section_stream::SectionStream;
pub use t2mi_stream::T2miEventStream;

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
