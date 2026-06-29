//! Streaming repair engine — internal plumbing between [`crate::TsFix`] and the
//! operation pipeline.
//!
//! The engine owns:
//! - an ordered `Vec<BoxedOp>` of private repair operations,
//! - a [`crate::ops::StreamModel`] that ops may read and update,
//! - per-pipeline chaining logic (runs ops in series: each op's output feeds
//!   the next op's input).
//!
//! # Operation ordering
//!
//! The engine applies ops in the fixed order in which they were pushed.
//! [`crate::TsFixBuilder`] is responsible for inserting ops at the correct
//! position (the builder's `build()` method sorts by the canonical order table):
//!
//! ```text
//! filter_pids → regen_psi → repair_continuity → restamp_pcr → stuffing
//! ```
//!
//! This ordering is an implementation detail; callers register ops via builder
//! methods and the engine enforces the canonical sequence.

use alloc::vec::Vec;

use crate::error::Error;
use crate::ops::{BoxedOp, StreamModel};
use mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};

/// The core streaming engine.
///
/// Not constructed directly — use [`crate::TsFixBuilder::build`].
pub(crate) struct Engine {
    /// Ordered pipeline of repair operations.
    ops: Vec<BoxedOp>,
    /// Shared stream state (PAT/PMT/PID set + timing context).
    model: StreamModel,
}

impl Engine {
    /// Create an engine with the given pipeline.
    pub(crate) fn new(ops: Vec<BoxedOp>) -> Self {
        Self {
            ops,
            model: StreamModel::default(),
        }
    }

    /// Validate a raw packet slice (188 bytes, sync byte `0x47`).
    ///
    /// Returns `Err` for short or desynchronised input.
    fn validate(packet: &[u8]) -> Result<(), Error> {
        if packet.len() != TS_PACKET_SIZE {
            return Err(Error::ShortPacket { len: packet.len() });
        }
        if packet[0] != TS_SYNC_BYTE {
            return Err(Error::NoSyncByte { found: packet[0] });
        }
        Ok(())
    }

    /// Feed one validated 188-byte packet through the operation pipeline.
    ///
    /// Calls `out` for every packet that survives the pipeline.  Returns `Err`
    /// if the packet is malformed (wrong length or missing sync byte); the caller
    /// decides how to handle malformed input.
    pub(crate) fn push(&mut self, packet: &[u8], mut out: impl FnMut(&[u8])) -> Result<(), Error> {
        Self::validate(packet)?;
        self.model.packet_count += 1;

        // Chain ops: each op's output is the next op's input.
        // v0.1 has exactly one op (IdentityOp), so this is a direct call.
        // When multiple ops are added in later tasks, staging_in/staging_out
        // buffers will collect each op's emitted packets and feed them forward.
        //
        // The two-buffer staging approach keeps the per-op interface simple
        // (`process` calls `out` for each emitted packet) and does not require
        // any heap allocation for the common single-op case — the closure
        // captures the final `out` directly.

        if self.ops.is_empty() {
            out(packet);
            return Ok(());
        }

        if self.ops.len() == 1 {
            // Fast path: no intermediate buffering needed.
            let op = &mut self.ops[0];
            let model = &mut self.model;
            op.process(packet, model, &mut out);
            return Ok(());
        }

        // Multi-op chain: stage_a holds packets produced by op[n], fed into op[n+1].
        let mut stage_a: Vec<[u8; TS_PACKET_SIZE]> = Vec::new();
        let mut stage_b: Vec<[u8; TS_PACKET_SIZE]> = Vec::new();

        // Seed stage_a with the incoming packet.
        let mut arr = [0u8; TS_PACKET_SIZE];
        arr.copy_from_slice(packet);
        stage_a.push(arr);

        let model = &mut self.model;
        let ops = &mut self.ops;
        let last = ops.len() - 1;

        for (i, op) in ops.iter_mut().enumerate() {
            stage_b.clear();
            if i < last {
                for pkt in stage_a.drain(..) {
                    op.process(&pkt, model, &mut |emitted: &[u8]| {
                        let mut buf = [0u8; TS_PACKET_SIZE];
                        buf.copy_from_slice(emitted);
                        stage_b.push(buf);
                    });
                }
                core::mem::swap(&mut stage_a, &mut stage_b);
            } else {
                // Last op: write directly to caller's `out`.
                for pkt in stage_a.drain(..) {
                    op.process(&pkt, model, &mut |emitted: &[u8]| {
                        out(emitted);
                    });
                }
            }
        }

        Ok(())
    }

    /// Flush buffered state at end of stream.
    ///
    /// Each op's flush output is fed forward through the remaining ops.
    pub(crate) fn finish(&mut self, mut out: impl FnMut(&[u8])) {
        if self.ops.is_empty() {
            return;
        }

        if self.ops.len() == 1 {
            let op = &mut self.ops[0];
            let model = &mut self.model;
            op.flush(model, &mut out);
            return;
        }

        // Multi-op flush: propagate flushed packets through the tail of the chain.
        let model = &mut self.model;
        let ops = &mut self.ops;
        let last = ops.len() - 1;

        let mut stage_a: Vec<[u8; TS_PACKET_SIZE]> = Vec::new();

        for (i, op) in ops.iter_mut().enumerate() {
            // Flush op[i] into stage_a.
            op.flush(model, &mut |emitted: &[u8]| {
                let mut buf = [0u8; TS_PACKET_SIZE];
                buf.copy_from_slice(emitted);
                stage_a.push(buf);
            });

            if i == last {
                for pkt in stage_a.drain(..) {
                    out(&pkt);
                }
            }
            // v0.1: flush output from earlier ops flowing into later ops is handled
            // generically in later tasks when buffering ops are introduced.  For now
            // IdentityOp.flush is a no-op so stage_a is always empty after the last op.
        }
    }
}
