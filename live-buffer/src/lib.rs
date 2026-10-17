#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
// The README is the single text for crates.io and docs.rs (docs/DOC-STANDARD.md rule 1).

extern crate alloc;

pub mod config;
pub mod controller;
pub mod error;
pub mod events;
pub mod histogram;
pub mod jitter;
pub mod policy;
pub mod stall_memory;
pub mod tracks;
pub mod window;

pub use config::{ControllerConfig, JitterConfig, Micros, PolicyConfig, Ticks27, US_PER_S};
pub use controller::BufferController;
pub use error::{ConfigField, Error};
pub use events::{Event, EventRecord, Hint, Reason, State, Stats, kind};
pub use jitter::{DeliveryJitter, JitterEstimate, SampleOutcome};
pub use stall_memory::StallMemorySnapshot;
pub use tracks::{COALESCE_US, DEFAULT_TRACK, MAX_TRACKS};

use broadcast_common::clock33::{PCR_EXT_MODULUS, PCR_WRAP_27MHZ, WRAP_33BIT};

/// C-ABI revision of the plain-data surface ([`EventRecord`], the `u32`
/// codes of [`State`]/[`Reason`]/[`Hint`], the event [`kind`] codes). Bumped
/// when any of them changes shape or meaning. Version 2 added the
/// `track` / `track_valid` pair to [`EventRecord`] (40 bytes, was 32).
pub const ABI_VERSION: u32 = 2;

/// A 90 kHz PTS (33-bit, wrapped) as 27 MHz ticks in the playback domain:
/// `PTS x 300`, exact and wrap-consistent (one PTS wrap is one
/// `PCR_WRAP_27MHZ`).
#[must_use]
pub fn pts90k_to_27mhz(pts: u64) -> Ticks27 {
    (pts % WRAP_33BIT) * PCR_EXT_MODULUS
}

/// The 27 MHz wrap modulus of the PCR / playback domain, re-exported so a
/// caller shifting positions by a constant offset stays wrap-correct.
pub const WRAP_27MHZ: u64 = PCR_WRAP_27MHZ;
