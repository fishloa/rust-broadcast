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
pub mod input_log;
pub mod jitter;
pub mod policy;
pub mod stall_memory;
pub mod time;
pub mod tracks;
pub mod window;

pub use config::{ControllerConfig, JitterConfig, PolicyConfig, US_PER_S};
pub use controller::BufferController;
pub use error::{ConfigField, Error};
pub use events::{Event, EventRecord, Hint, Reason, State, Stats, kind};
pub use jitter::{DeliveryJitter, JitterEstimate, SampleOutcome};
pub use stall_memory::StallMemorySnapshot;
pub use time::{
    Clock27, duration_add, duration_between, duration_micros, duration_sub, micros_as_duration,
};
pub use tracks::{COALESCE, DEFAULT_TRACK, MAX_TRACKS};

use broadcast_common::clock33::{PCR_EXT_MODULUS, WRAP_33BIT};

/// C-ABI revision of the plain-data surface ([`EventRecord`], the `u32`
/// codes of [`State`]/[`Reason`]/[`Hint`], the event [`kind`] codes). Bumped
/// when any of them changes shape or meaning. Version 2 added the
/// `track` / `track_valid` pair to [`EventRecord`] (40 bytes, was 32).
pub const ABI_VERSION: u32 = 2;

/// A 90 kHz PTS (33-bit, wrapped) as a playback-domain position: `PTS x 300`,
/// exact and wrap-consistent (one PTS wrap is one `PCR_WRAP_27MHZ`).
///
/// # Errors
/// [`broadcast_common::clock33::ClockOutOfRange`] when the product leaves the
/// 27 MHz clock: `PTS x 300` is always below `2^33 x 300`, so only a caller's
/// own value at the top of the 33-bit range can be over.
pub fn pts90k_to_clock27(pts: u64) -> Result<Clock27, broadcast_common::clock33::ClockOutOfRange> {
    let ticks = (pts % WRAP_33BIT) * PCR_EXT_MODULUS;
    Clock27::from_27mhz(ticks)
}
