#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

extern crate alloc;

/// Client state machine implementation.
pub mod client;
/// Session configuration.
pub mod config;
/// Action, event and starve-kind types.
pub mod events;

/// TS-over-HTTP pull session client (see DESIGN.md).
pub use client::TsHttpClient;
/// Session configuration (see DESIGN.md).
pub use config::Config;
/// Action, event and starve-kind types (see DESIGN.md).
pub use events::{Action, Event, StarveKind};
/// Re-exported demux event type from transmux.
pub use transmux::DemuxEvent;
