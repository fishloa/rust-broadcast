#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

extern crate alloc;

pub mod client;
pub mod config;
pub mod events;

pub use client::TsHttpClient;
pub use config::Config;
pub use events::{Action, Event, StarveKind};
pub use transmux::DemuxEvent;
