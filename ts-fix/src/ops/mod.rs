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

use alloc::vec::Vec;

pub(crate) mod continuity;

/// Observable state built up as the engine processes packets.
///
/// Repair operations receive a shared `&mut StreamModel` reference on every
/// `process` call.  v0.1 is a stub; later tasks add PAT/PMT programme state,
/// PID set, and the [`TimingContext`].
#[derive(Debug, Default)]
pub(crate) struct StreamModel {
    /// Number of TS packets seen so far (used by PCR timing interpolation).
    pub(crate) packet_count: u64,
    // Future tasks will add:
    //   pub(crate) pat: Option<...>,
    //   pub(crate) pmt: BTreeMap<u16, ...>,
    //   pub(crate) timing: TimingContext,
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
pub(crate) type BoxedOp = alloc::boxed::Box<dyn Op>;

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

/// Build the default identity pipeline (no repair operations).
pub(crate) fn identity_pipeline() -> Vec<BoxedOp> {
    alloc::vec![alloc::boxed::Box::new(IdentityOp)]
}
