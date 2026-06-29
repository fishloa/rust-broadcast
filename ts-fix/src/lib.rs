//! MPEG-2 TS repair / remux — container-layer operations, no codec parsing.
//!
//! `ts-fix` provides a **builder-driven streaming engine** that feeds 188-byte
//! TS packets in and emits repaired packets out.  Repair operations are opt-in
//! via builder methods; the engine owns and enforces the canonical ordering.
//!
//! # Forward compatibility
//!
//! The public API is designed so that adding a new repair operation in a future
//! minor release is a **purely additive** change:
//!
//! - There is no public `enum Operation` (adding a variant is breaking).
//! - There is no public `trait Operation` (locking the contract before all ops'
//!   needs are known).
//! - Operations are exposed exclusively through [`TsFixBuilder`] methods.
//! - All configuration and error enums are `#[non_exhaustive]`.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use ts_fix::{TsFix, Error};
//!
//! fn repair(input: &[u8]) -> Result<Vec<u8>, Error> {
//!     let mut engine = TsFix::builder().build()?;
//!     let mut output = Vec::with_capacity(input.len());
//!     for chunk in input.chunks(188) {
//!         engine.push(chunk, |pkt| output.extend_from_slice(pkt));
//!     }
//!     engine.finish(|pkt| output.extend_from_slice(pkt));
//!     Ok(output)
//! }
//! ```
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) — §2.4.3.2 (TS packet), §2.4.3.3
//! (adaptation field / continuity counter), §2.4.3.4 (PCR), §2.4.4 (PSI).

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod error;

mod engine;
mod ops;

pub use error::Error;
pub use ops::pid_filter::PidFilter;

/// A repair / remux engine for MPEG-2 TS byte streams.
///
/// Constructed via [`TsFix::builder`] → [`TsFixBuilder::build`].
///
/// Feed 188-byte TS packets one at a time with [`push`](TsFix::push); call
/// [`finish`](TsFix::finish) at end-of-stream to flush any buffered state.
pub struct TsFix {
    engine: engine::Engine,
}

impl TsFix {
    /// Create a new builder for configuring a [`TsFix`] engine.
    pub fn builder() -> TsFixBuilder {
        TsFixBuilder::new()
    }

    /// Feed one 188-byte TS packet into the engine.
    ///
    /// `out` is called once per emitted packet (may be called zero or more times
    /// if an op suppresses or multiplies packets).
    ///
    /// Returns `Err` if `packet` is not exactly 188 bytes or lacks the `0x47`
    /// sync byte (ISO/IEC 13818-1 §2.4.3.2).
    pub fn push(&mut self, packet: &[u8], out: impl FnMut(&[u8])) -> Result<(), Error> {
        self.engine.push(packet, out)
    }

    /// Flush any internally buffered state at end-of-stream.
    ///
    /// Must be called after the last [`push`](TsFix::push) to ensure that
    /// buffering operations (e.g. PCR interpolation) emit their final packets.
    pub fn finish(&mut self, out: impl FnMut(&[u8])) {
        self.engine.finish(out);
    }
}

/// Builder for [`TsFix`].
///
/// Each repair operation is opt-in via a dedicated method.  Methods that
/// correspond to later tasks are listed here for documentation purposes but will
/// be implemented in subsequent releases — attempting to call them will cause a
/// compile error until they ship.
///
/// # Forward-compat guarantee
///
/// Adding a new builder method in v0.2/v0.3 is an additive change.  Callers who
/// construct `TsFix::builder().build()?` (with no additional methods) will
/// compile and behave identically across versions.
pub struct TsFixBuilder {
    /// Ordered list of configured operations.
    ///
    /// The engine enforces canonical ordering at `build()` time; callers do not
    /// need to call methods in the correct order.
    ops: alloc::vec::Vec<ops::BoxedOp>,
}

impl TsFixBuilder {
    fn new() -> Self {
        Self {
            ops: alloc::vec::Vec::new(),
        }
    }

    /// Build the configured engine.
    ///
    /// When no operations have been registered the engine is an **identity
    /// pass-through**: every packet is emitted unchanged.
    pub fn build(self) -> Result<TsFix, Error> {
        // If no ops were configured, install the identity no-op so the engine
        // always has something to call.  This keeps `engine::Engine::push`
        // simple and ensures zero-op builds are provably correct.
        let ops = if self.ops.is_empty() {
            ops::identity_pipeline()
        } else {
            self.ops
        };

        Ok(TsFix {
            engine: engine::Engine::new(ops),
        })
    }

    /// Enable continuity counter repair.
    ///
    /// Renumbers the 4-bit `continuity_counter` per PID to a correct monotonic
    /// sequence (mod 16), respecting the ISO/IEC 13818-1 §2.4.3.3 rule that the
    /// counter increments **only** on payload-bearing packets.
    pub fn repair_continuity(mut self) -> Self {
        self.ops
            .push(alloc::boxed::Box::new(ops::continuity::ContinuityOp::new()));
        self
    }

    /// Enable PID filtering / service extraction.
    ///
    /// Two modes:
    ///
    /// - [`PidFilter::keep`] — pass only packets whose PID is in the supplied
    ///   set.  PAT PID 0x0000 is always implicitly included.
    /// - [`PidFilter::service`] — observe the live PAT/PMT and keep exactly
    ///   the PIDs that belong to the given program_number
    ///   (PAT + PMT PID + PCR PID + all ES PIDs); everything else is dropped.
    ///
    /// # Example — extract service 1 from a multi-program mux
    ///
    /// ```rust,no_run
    /// use ts_fix::{TsFix, PidFilter};
    ///
    /// let mut engine = TsFix::builder()
    ///     .filter_pids(PidFilter::service(1))
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn filter_pids(mut self, cfg: PidFilter) -> Self {
        self.ops
            .push(alloc::boxed::Box::new(ops::pid_filter::PidFilterOp::new(
                cfg,
            )));
        self
    }

    // ── Future operation methods (stubs document the planned API surface) ────
    //
    // These will be added in later tasks.  They are NOT present in v0.1 — they
    // appear here only as comments so reviewers can confirm the builder surface
    // is stable and additive.
    //
    //   pub fn regen_psi(self) -> Self                   // Task 4
    //   pub fn stuffing(self, cfg: Stuffing) -> Self     // Task 5
    //   pub fn restamp_pcr(self, cfg: PcrRestamp) -> Self // Task 6
    //
    // Each adds a builder method and a corresponding ops/<name>.rs module.
    // No existing public signature changes.
}
