//! Internal operation abstraction — NOT part of the public API.
//!
//! The public API surfaces operations exclusively through [`crate::TsFixBuilder`]
//! builder methods (e.g. `repair_continuity()`, `filter_pids()`, …).  There is
//! intentionally **no** public `enum Operation` or `trait Operation` — adding a new
//! operation in v0.2/v0.3 is a purely additive builder-method change and can never
//! cause a breaking change for callers who pattern-match on a public Operation enum.
//!
//! # Extension contract
//!
//! To add a new operation:
//! 1. Create `ops/<name>.rs` and implement `Op` for a private struct.
//! 2. Add a builder method to [`crate::TsFixBuilder`] that pushes the boxed op.
//! 3. Register the op in [`crate::engine`]'s fixed ordering table if it has a
//!    positional relationship with existing ops.
//!
//! No existing public signatures change.

use alloc::boxed::Box;

pub(crate) mod continuity;
pub(crate) mod pcr_restamp;
pub(crate) mod pid_filter;
pub(crate) mod psi_regen;
pub(crate) mod stuffing;

/// Observable state built up as the engine processes packets.
///
/// Repair operations receive a shared `&mut StreamModel` reference on every
/// `process` call.  v0.1 is a stub; later tasks add PAT/PMT programme state,
/// PID set, and the [`TimingContext`].
#[derive(Debug, Default)]
pub(crate) struct StreamModel {
    /// Number of TS packets seen so far (used by PCR timing interpolation).
    pub(crate) packet_count: u64,
    /// Timing context — 27 MHz clock, last PCR anchor, etc.
    /// Used by PCR restamp (v0.1) and will be reused by PTS/DTS-wrap (v0.2).
    pub(crate) timing: TimingContext,
    // Future tasks will add:
    //   pub(crate) pat: Option<...>,
    //   pub(crate) pmt: BTreeMap<u16, ...>,
}

// — TimingContext ───────────────────────────────────────────────────────────

/// Forward-compat timing model for TS-level clock reconstruction.
///
/// Holds the 27 MHz clock state and last-anchor information needed by PCR
/// restamp (v0.1) and, in v0.2+, PTS/DTS wrap-around repair.
///
/// # Design rationale
///
/// This is deliberately NOT PCR-specific — it lives in `StreamModel` and stores
/// the stream's 27 MHz clock model.  v0.2's PTS/DTS-wrap op will read and
/// update the same context to unroll 33-bit wrap on presentation timestamps
/// without duplicating clock state.
///
/// PCR-specific configuration (mode, target bitrate) lives in
/// [`super::pcr_restamp::PcrRestamp`], not here.
#[derive(Debug, Clone, Default)]
pub(crate) struct TimingContext {
    /// Accumulated 27 MHz clock ticks (monotonic, may wrap beyond 2^33).
    pub(crate) clock_27mhz: u64,
    /// The last PCR value we wrote (for interpolation mode).
    pub(crate) last_pcr_base: u64,
    pub(crate) last_pcr_ext: u16,
    /// Whether we have seen a PCR anchor yet.
    pub(crate) has_anchor: bool,
    /// Packet index at which `last_pcr` was observed.
    pub(crate) anchor_packet_index: u64,
    /// Packet index of the *previous* PCR (for rate calculation).
    pub(crate) prev_packet_index: u64,
    /// Inter-packet bitrate in bytes/s, derived from two consecutive PCRs.
    pub(crate) interpolated_bitrate: Option<f64>,
}

/// Private operation trait — sealed inside this module.
///
/// `process` is called once per incoming packet.  The op may:
/// - emit the packet unchanged (identity / pass-through),
/// - emit a modified copy,
/// - suppress the packet entirely (return without calling `out`), or
/// - emit additional packets (e.g. null stuffing).
///
/// `flush` is called once at end-of-stream; the op may emit buffered output.
///
/// # Why `alloc::vec::Vec<u8>` rather than `&[u8]`?
///
/// Packet mutation (CC renumbering, PCR restamping) requires an owned buffer.
/// Passing `[u8; 188]` by value would fix the size at the trait boundary, which
/// would prevent future PES-level ops that need to reassemble across packets.
/// Using `Vec<u8>` keeps the boundary general while remaining cheap (each call
/// pushes at most a handful of 188-byte chunks).
pub(crate) trait Op: Send {
    /// Process one 188-byte TS packet and emit zero or more output packets.
    ///
    /// `model` carries stream state that ops may observe and update.
    /// `packet` is the raw 188-byte slice (sync byte already validated by the engine).
    /// `out` receives each output packet (always 188 bytes; ops must not change size).
    fn process(&mut self, packet: &[u8], model: &mut StreamModel, out: &mut dyn FnMut(&[u8]));

    /// Flush any internally buffered state at end-of-stream.
    fn flush(&mut self, model: &mut StreamModel, out: &mut dyn FnMut(&[u8]));
}

/// A boxed, heap-allocated operation.  The engine holds an ordered `Vec<BoxedOp>`.
pub(crate) type BoxedOp = Box<dyn Op>;

/// Canonical operation-kind discriminant used by the builder to enforce
/// engine ordering (filter → regen_psi → cc_repair → pcr_restamp → stuffing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum OpKind {
    PidFilter,
    PsiRegen,
    Continuity,
    PcrRestamp,
    Stuffing,
    Identity,
}

// ── Identity pass-through (the v0.1 no-op) ──────────────────────────────────

/// Identity operation: forwards every packet unchanged and emits nothing on flush.
///
/// Used by the engine when no ops have been registered so that `TsFix` built
/// with `TsFixBuilder::build()` is a pure pass-through.  This also serves as a
/// reference implementation for the `Op` trait.
pub(crate) struct IdentityOp;

impl Op for IdentityOp {
    #[inline]
    fn process(&mut self, packet: &[u8], _model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        out(packet);
    }

    #[inline]
    fn flush(&mut self, _model: &mut StreamModel, _out: &mut dyn FnMut(&[u8])) {
        // Nothing buffered.
    }
}
