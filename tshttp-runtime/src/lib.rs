#![cfg_attr(docsrs, feature(doc_cfg))]
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
/// Demux-free policy core of the session.
pub mod policy;

/// TS-over-HTTP pull session client (see DESIGN.md).
pub use client::TsHttpClient;
/// Session configuration (see DESIGN.md).
pub use config::{Config, ConfigError};
/// Action, event and starve-kind types (see DESIGN.md).
pub use events::{Action, Event, StarveKind};
/// Demux-free policy core (see DESIGN.md).
pub use policy::TsHttpPolicy;
/// Re-exported demux event type from transmux.
pub use transmux::DemuxEvent;
