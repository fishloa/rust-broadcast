//! MPEG-2 TS repair / remux — container-layer operations, no codec parsing.
//!
//! `ts-fix` provides a **builder-driven streaming engine** that feeds 188-byte
//! TS packets in and emits repaired packets out.  Repair operations are opt-in
//! via builder methods; the engine owns and enforces the canonical ordering.
//!
//! # Operations
//!
//! | Operation | Builder method | What it does |
//! |---|---|---|
//! | Continuity repair | [`repair_continuity`](TsFixBuilder::repair_continuity) | Renumber per-PID continuity counters (§2.4.3.3). |
//! | PID filter / service extract | [`filter_pids`](TsFixBuilder::filter_pids) | Keep specified PIDs or extract a single programme by `program_number`. |
//! | PAT/PMT regeneration | [`regen_psi`](TsFixBuilder::regen_psi) | Rebuild PAT from observed PMT PIDs on flush. |
//! | PCR restamp | [`restamp_pcr`](TsFixBuilder::restamp_pcr) | Recompute PCR values on the PCR PID (§2.4.3.5). |
//! | PCR-discontinuity honor | [`honor_pcr_discontinuity`](TsFixBuilder::honor_pcr_discontinuity) | Set `discontinuity_indicator` on genuine, unflagged PCR breaks (TR 101 290 §5.2.2 2.3b) without rewriting values. |
//! | Stuffing | [`stuffing`](TsFixBuilder::stuffing) | Drop null packets or pad to a target packet rate. |
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
//! use ts_fix::{PcrRestamp, PidFilter, Stuffing, TsFix};
//!
//! let mut engine = TsFix::builder()
//!     .repair_continuity()
//!     .filter_pids(PidFilter::keep([0x0100, 0x0101]))
//!     .regen_psi()
//!     .restamp_pcr(PcrRestamp::interpolate())
//!     .stuffing(Stuffing::drop_nulls())
//!     .build()
//!     .unwrap();
//! ```
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) — §2.4.3.2 (TS packet), §2.4.3.3
//! (adaptation field / continuity counter), §2.4.3.4 (PCR), §2.4.4 (PSI).

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod discontinuity;
pub mod error;
pub mod pes;

mod engine;
mod ops;

use ops::OpKind;

pub use error::Error;
pub use ops::pcr_restamp::PcrRestamp;
pub use ops::pid_filter::PidFilter;
pub use ops::stuffing::Stuffing;

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
    /// Ops paired with their `OpKind` for canonical ordering at `build()` time.
    ops: alloc::vec::Vec<(OpKind, ops::BoxedOp)>,
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
    ///
    /// The engine applies operations in the canonical ordering:
    /// filter_pids → regen_psi → repair_continuity → restamp_pcr →
    /// honor_pcr_discontinuity → stuffing. The `build()` method sorts ops by
    /// this order regardless of the order in which builder methods were
    /// called.
    pub fn build(mut self) -> Result<TsFix, Error> {
        // If no ops were configured, install the identity no-op so the engine
        // always has something to call.  This keeps `engine::Engine::push`
        // simple and ensures zero-op builds are provably correct.
        if self.ops.is_empty() {
            self.ops = alloc::vec![(OpKind::Identity, alloc::boxed::Box::new(ops::IdentityOp))];
        }

        // Sort by canonical ordering, then discard the OpKind tag.
        self.ops.sort_by_key(|(kind, _)| *kind);
        let ops: alloc::vec::Vec<ops::BoxedOp> = self.ops.into_iter().map(|(_, op)| op).collect();

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
        self.ops.push((
            OpKind::Continuity,
            alloc::boxed::Box::new(ops::continuity::ContinuityOp::new()),
        ));
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
        self.ops.push((
            OpKind::PidFilter,
            alloc::boxed::Box::new(ops::pid_filter::PidFilterOp::new(cfg)),
        ));
        self
    }

    /// Enable PAT/PMT regeneration.
    ///
    /// Rebuilds the Program Association Table (PAT) to be consistent with the
    /// actual programs present in the stream output. This is particularly useful
    /// after [`filter_pids`](Self::filter_pids) to ensure the PAT lists only the
    /// programs that survived the filter.
    ///
    /// The engine observes PAT sections as packets pass through, collecting the
    /// program → PMT PID mappings. On flush (end of stream), it emits a
    /// freshly-generated PAT listing exactly the observed programs.
    ///
    /// # Example — filter to one service, then regenerate PAT
    ///
    /// ```rust,no_run
    /// use ts_fix::{TsFix, PidFilter};
    ///
    /// let mut engine = TsFix::builder()
    ///     .filter_pids(PidFilter::service(1))
    ///     .regen_psi()
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn regen_psi(mut self) -> Self {
        self.ops.push((
            OpKind::PsiRegen,
            alloc::boxed::Box::new(ops::psi_regen::PsiRegenOp::new()),
        ));
        self
    }

    /// Enable PCR restamping.
    ///
    /// Recomputes the 42-bit Program Clock Reference on the PCR PID using a
    /// timing model (ISO/IEC 13818-1 §2.4.3.5). Two modes:
    ///
    /// - [`PcrRestamp::interpolate`] — interpolate PCRs between observed anchors.
    /// - [`PcrRestamp::from_bitrate`] — recompute from a fixed bitrate.
    ///
    /// PCR values are written in-place via mpeg-ts editors; the adaptation field
    /// layout is preserved.
    ///
    /// # Example — restore a plausible PCR timeline
    ///
    /// ```rust,no_run
    /// use ts_fix::{TsFix, PcrRestamp};
    ///
    /// let mut engine = TsFix::builder()
    ///     .restamp_pcr(PcrRestamp::interpolate())
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn restamp_pcr(mut self, cfg: PcrRestamp) -> Self {
        self.ops.push((
            OpKind::PcrRestamp,
            alloc::boxed::Box::new(ops::pcr_restamp::PcrRestampOp::new(cfg)),
        ));
        self
    }

    /// Enable PCR-discontinuity **honor** mode (#562).
    ///
    /// The alternative to [`restamp_pcr`](Self::restamp_pcr): leaves every
    /// timestamp byte — including the PCR field itself — untouched, and
    /// instead sets `discontinuity_indicator` (ISO/IEC 13818-1 §2.4.3.5) on
    /// packets where a genuine, unflagged PCR break exists.
    ///
    /// "Genuine, unflagged" means the PCR delta exceeds the ETSI TR 101 290
    /// v1.4.1 §5.2.2 Table 5.0b indicator 2.3b
    /// (`PCR_discontinuity_indicator_error`) threshold with
    /// `discontinuity_indicator == 0` — reused verbatim from
    /// [`dvb_conformance::ConformanceMonitor`], never re-derived here. A
    /// packet that already carries `discontinuity_indicator == 1` is a legal
    /// break and is left alone.
    ///
    /// # Example — flag genuine PCR defects without rewriting values
    ///
    /// ```rust,no_run
    /// use ts_fix::TsFix;
    ///
    /// let mut engine = TsFix::builder()
    ///     .honor_pcr_discontinuity()
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn honor_pcr_discontinuity(mut self) -> Self {
        self.ops.push((
            OpKind::PcrHonor,
            alloc::boxed::Box::new(ops::pcr_honor::PcrHonorOp::new()),
        ));
        self
    }

    /// Enable null packet stuffing or drop.
    ///
    /// Two modes:
    ///
    /// - [`Stuffing::drop_nulls`] — strip all null packets (PID 0x1FFF)
    ///   from the output.
    /// - [`Stuffing::pad_to`] — insert null packets to reach a target
    ///   packet rate (e.g. `pad_to(2.0)` doubles the output packet count).
    ///
    /// # Example — drop all null packets
    ///
    /// ```rust,no_run
    /// use ts_fix::{TsFix, Stuffing};
    ///
    /// let mut engine = TsFix::builder()
    ///     .stuffing(Stuffing::drop_nulls())
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn stuffing(mut self, cfg: Stuffing) -> Self {
        self.ops.push((
            OpKind::Stuffing,
            alloc::boxed::Box::new(ops::stuffing::StuffingOp::new(cfg)),
        ));
        self
    }
}
