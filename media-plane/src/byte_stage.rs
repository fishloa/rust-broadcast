//! [`ByteStage`] — the pre-demux byte-to-byte drive contract.
//!
//! # Why this is a `Stage`, not a second trait
//!
//! Rev 2 of the media-plane spec (2026-07-26) originally specced `ByteStage`
//! as a standalone four-method trait:
//!
//! ```text
//! pub trait ByteStage: Send {
//!     fn feed(&mut self, input: &[u8], now: Instant) -> Result<(), StageError>;
//!     fn poll(&mut self) -> Option<Bytes>;
//!     fn next_deadline(&self) -> Option<Instant>;
//!     fn on_deadline(&mut self, now: Instant);
//! }
//! ```
//!
//! That predates plan Step 1, which shipped [`broadcast_common::Stage`] with
//! `type In<'a>` as a GAT for exactly this case — the module doc there reads
//! *"byte-stream stages use `&'a [u8]`; sample-consuming stages use an owned
//! typed input"*. Defining a second trait with the same shape would give the
//! plane two incompatible drive models for no reason, and (the rev-2 sketch's
//! actual defect) it silently dropped `finish()` (flush-at-EOF) and `demand()`
//! (back-pressure) — a byte stage consuming live network input needs both
//! exactly as much as a sample stage does. The spec's 2026-07-27 revision to
//! §1.1 corrects this: a byte stage **is** a `Stage` whose input is a borrowed
//! byte slice and whose output is [`Bytes`].
//!
//! # Which form was validated
//!
//! The revision flags that `for<'a>` combined with an associated-type equality
//! constraint (`Stage<In<'a> = &'a [u8]>` for *every* `'a`) is not always
//! accepted by the type system, and says to validate before building on it.
//! It was validated, standalone, against this workspace's actual
//! `broadcast_common::Stage` (not a toy trait) — compiled clean on stable and
//! on the pinned MSRV toolchain (`cargo +1.86 build`), including a generic
//! driver function taking `impl ByteStage` and a concrete implementor driven
//! through it (not just the empty trait/impl pair). **The alias form below is
//! the one shipped** — no fallback second trait was needed:
//!
//! ```
//! use broadcast_common::stage::{Stage, Timestamp};
//! use bytes::Bytes;
//! use media_plane::ByteStage;
//!
//! // Any `T: for<'a> Stage<In<'a> = &'a [u8], Out = Bytes> + Send` is
//! // automatically a `ByteStage` — no separate impl needed per stage.
//! fn accepts_byte_stage<S: ByteStage>(_s: &S) {}
//! ```
//!
//! # `finish()` and `demand()` are load-bearing, not decorative
//!
//! Both are on [`broadcast_common::Stage`] already, so `ByteStage` inherits
//! them for free — but they are called out here because the rev-2 sketch this
//! replaces omitted them, and a byte stage consuming remote input needs them
//! exactly as much as a sample stage: `finish()` flushes whatever a stage
//! (e.g. a re-framer) was holding back when the connection closes with a
//! partial tail still buffered; `demand()` is how a driver learns to stop
//! feeding a stage that is at its bound *before* it has to reject input.

use broadcast_common::Stage;
use bytes::Bytes;

/// Pre-demux, byte-to-byte, deadline-driven work: CAM descramble, TS
/// continuity/PCR repair, T2-MI/BBFrame inner-TS recovery, program-PID
/// filtering.
///
/// A blanket specialisation of [`broadcast_common::Stage`], not a new trait —
/// see the [module docs](self) for why, and for the exact form validated to
/// compile. Any `Stage` whose input is `&'a [u8]` for every `'a` and whose
/// output is [`Bytes`] is automatically a `ByteStage`.
pub trait ByteStage: for<'a> Stage<In<'a> = &'a [u8], Out = Bytes> + Send {}

impl<T> ByteStage for T where T: for<'a> Stage<In<'a> = &'a [u8], Out = Bytes> + Send {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use broadcast_common::stage::{Demand, Timestamp};

    /// Drives any [`ByteStage`] through `feed`/`poll`/`finish`, draining
    /// `poll()` after every `feed` (as the `Stage` contract requires) and
    /// again after `finish`. This is the generic driver both implementor
    /// tests below run through — it takes `impl ByteStage`, not a concrete
    /// type.
    fn drive<S: ByteStage>(stage: &mut S, inputs: &[&[u8]]) -> Result<Vec<Bytes>, S::Error> {
        let mut out = Vec::new();
        for (i, chunk) in inputs.iter().enumerate() {
            stage.feed(chunk, Timestamp::from_nanos(i as u64))?;
            while let Some(b) = stage.poll() {
                out.push(b);
            }
        }
        stage.finish()?;
        while let Some(b) = stage.poll() {
            out.push(b);
        }
        Ok(out)
    }

    // --- Implementor 1: trivial pass-through -----------------------------

    /// Hands each fed slice straight back out, unmodified, one `Bytes` per
    /// `feed`. The simplest possible `ByteStage`.
    struct PassThrough {
        pending: Option<Bytes>,
    }

    impl PassThrough {
        fn new() -> Self {
            PassThrough { pending: None }
        }
    }

    impl Stage for PassThrough {
        type In<'a> = &'a [u8];
        type Out = Bytes;
        type Error = core::convert::Infallible;

        fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<(), Self::Error> {
            self.pending = Some(Bytes::copy_from_slice(input));
            Ok(())
        }

        fn poll(&mut self) -> Option<Self::Out> {
            self.pending.take()
        }

        fn finish(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        fn next_deadline(&self) -> Option<Timestamp> {
            None
        }

        fn on_deadline(&mut self, _now: Timestamp) {}

        fn demand(&self) -> Demand {
            Demand::new(4096)
        }
    }

    #[test]
    fn passthrough_is_a_byte_stage_via_generic_driver() {
        fn accepts_byte_stage<S: ByteStage>(_s: &S) {}
        let stage = PassThrough::new();
        accepts_byte_stage(&stage);
    }

    #[test]
    fn passthrough_delivers_each_input_exactly_once() {
        let mut stage = PassThrough::new();
        let out = drive(&mut stage, &[b"abc", b"de", b"fghi"]).unwrap();
        assert_eq!(
            out,
            alloc::vec![
                Bytes::from_static(b"abc"),
                Bytes::from_static(b"de"),
                Bytes::from_static(b"fghi"),
            ]
        );
    }

    // --- Implementor 2: bounded fixed-size re-framer ---------------------

    /// Accumulates fed bytes and emits exactly-`chunk_size`-byte [`Bytes`]
    /// chunks, flushing a shorter partial tail on [`Stage::finish`].
    ///
    /// # Bound
    ///
    /// Two buffers exist: `partial` (bytes accumulated toward the next
    /// complete chunk — structurally bounded to `< chunk_size` because a
    /// complete chunk is cut off `partial` the instant one is available) and
    /// `queued` (complete chunks waiting for [`Stage::poll`] — the actual
    /// unbounded-growth risk, since a caller that never polls could otherwise
    /// grow `queued` without limit purely from feed-call count). `queued` is
    /// capped at `max_queued` chunks. [`Stage::feed`] computes, before
    /// touching any buffer, the maximum number of input bytes it could ever
    /// accept without needing more than `max_queued - queued.len()` new
    /// complete chunks; a call that would exceed that is rejected outright —
    /// none of its bytes are buffered — with [`FramerError::QueueFull`]. So
    /// total resident bytes across both buffers can never exceed
    /// `(max_queued + 1) * chunk_size - 1`, a fixed bound independent of how
    /// many times, or with how much data, `feed` is called. This is the
    /// project's standing "bound every buffer" rule applied to the shape a
    /// byte stage consuming network input actually has.
    struct FixedFramer {
        chunk_size: usize,
        max_queued: usize,
        partial: alloc::vec::Vec<u8>,
        queued: alloc::collections::VecDeque<Bytes>,
        finished: bool,
    }

    /// Errors [`FixedFramer::feed`] can return.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FramerError {
        /// `feed` was rejected outright because honouring it would require
        /// more than `max_queued` complete chunks to be waiting for `poll()`
        /// at once. No bytes from the rejected call were buffered.
        QueueFull { max_queued: usize },
        /// `feed` was called after `finish()`. Deliberate, not accidental:
        /// once a stage has been told there is no more input, accepting more
        /// input would either have to silently discard it or reopen a
        /// stage that already flushed its tail — both wrong, so it errors.
        FedAfterFinish,
    }

    impl FixedFramer {
        fn new(chunk_size: usize, max_queued: usize) -> Self {
            assert!(chunk_size > 0, "chunk_size must be > 0");
            assert!(max_queued > 0, "max_queued must be > 0");
            FixedFramer {
                chunk_size,
                max_queued,
                partial: alloc::vec::Vec::new(),
                queued: alloc::collections::VecDeque::new(),
                finished: false,
            }
        }
    }

    impl Stage for FixedFramer {
        type In<'a> = &'a [u8];
        type Out = Bytes;
        type Error = FramerError;

        fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<(), Self::Error> {
            if self.finished {
                return Err(FramerError::FedAfterFinish);
            }
            let remaining_capacity = self.max_queued - self.queued.len();
            // Room left in `partial` before it would complete another chunk,
            // plus room for `remaining_capacity` more complete chunks.
            let max_acceptable =
                remaining_capacity * self.chunk_size + (self.chunk_size - 1 - self.partial.len());
            if input.len() > max_acceptable {
                return Err(FramerError::QueueFull {
                    max_queued: self.max_queued,
                });
            }
            self.partial.extend_from_slice(input);
            while self.partial.len() >= self.chunk_size {
                let chunk: alloc::vec::Vec<u8> = self.partial.drain(..self.chunk_size).collect();
                self.queued.push_back(Bytes::from(chunk));
            }
            Ok(())
        }

        fn poll(&mut self) -> Option<Self::Out> {
            self.queued.pop_front()
        }

        fn finish(&mut self) -> Result<(), Self::Error> {
            if !self.finished {
                if !self.partial.is_empty() {
                    let tail = core::mem::take(&mut self.partial);
                    self.queued.push_back(Bytes::from(tail));
                }
                self.finished = true;
            }
            Ok(())
        }

        fn next_deadline(&self) -> Option<Timestamp> {
            None
        }

        fn on_deadline(&mut self, _now: Timestamp) {}

        fn demand(&self) -> Demand {
            if self.queued.len() >= self.max_queued {
                Demand::saturated()
            } else {
                let remaining_capacity = self.max_queued - self.queued.len();
                let want = remaining_capacity * self.chunk_size
                    + (self.chunk_size - 1 - self.partial.len());
                Demand::new(want)
            }
        }
    }

    #[test]
    fn framer_is_a_byte_stage_via_generic_driver() {
        fn accepts_byte_stage<S: ByteStage>(_s: &S) {}
        let stage = FixedFramer::new(4, 8);
        accepts_byte_stage(&stage);
    }

    #[test]
    fn framer_emits_multiple_outputs_from_one_feed_with_no_loss_or_duplication() {
        let mut stage = FixedFramer::new(4, 8);
        // One feed call, 8 bytes, chunk_size 4: exactly two chunks must be
        // available from `poll` immediately, *before* `finish` is ever
        // called — proving `poll` returning multiple outputs per `feed`,
        // not a tail flush wearing a disguise.
        stage.feed(b"abcdefgh", Timestamp::ZERO).unwrap();
        assert_eq!(stage.poll(), Some(Bytes::from_static(b"abcd")));
        assert_eq!(stage.poll(), Some(Bytes::from_static(b"efgh")));
        assert_eq!(stage.poll(), None);

        // finish() must add nothing further: the partial buffer is already
        // empty, so there is no tail left to flush.
        stage.finish().unwrap();
        assert_eq!(stage.poll(), None);
    }

    #[test]
    fn framer_finish_flushes_partial_tail() {
        let mut stage = FixedFramer::new(4, 8);
        let out = drive(&mut stage, &[b"abc"]).unwrap();
        // Below chunk_size: nothing until finish, then exactly the tail.
        assert_eq!(out, alloc::vec![Bytes::from_static(b"abc")]);
    }

    #[test]
    fn framer_finish_is_idempotent() {
        let mut stage = FixedFramer::new(4, 8);
        stage.feed(b"ab", Timestamp::ZERO).unwrap();
        stage.finish().unwrap();
        assert_eq!(stage.poll(), Some(Bytes::from_static(b"ab")));
        assert_eq!(stage.poll(), None);
        // Second finish: no error, and critically no extra/duplicate output.
        stage.finish().unwrap();
        assert_eq!(stage.poll(), None);
    }

    #[test]
    fn framer_feed_after_finish_errors_and_is_deliberate() {
        let mut stage = FixedFramer::new(4, 8);
        stage.feed(b"ab", Timestamp::ZERO).unwrap();
        stage.finish().unwrap();
        assert_eq!(stage.poll(), Some(Bytes::from_static(b"ab")));

        let err = stage.feed(b"cd", Timestamp::ZERO).unwrap_err();
        assert_eq!(err, FramerError::FedAfterFinish);
        // Nothing was buffered from the rejected feed.
        assert_eq!(stage.poll(), None);
    }

    #[test]
    fn framer_buffer_bound_holds_under_flood_and_demand_saturates() {
        let chunk_size = 4;
        let max_queued = 3;
        let mut stage = FixedFramer::new(chunk_size, max_queued);

        // Fill to capacity in one feed call: exactly `max_queued` chunks.
        stage
            .feed(&alloc::vec![0u8; chunk_size * max_queued], Timestamp::ZERO)
            .unwrap();
        assert_eq!(stage.demand(), Demand::saturated());

        // Flood: many further feeds, each far larger than the bound, none
        // of which are polled in between. Every one must be rejected.
        for _ in 0..1_000 {
            let err = stage
                .feed(&alloc::vec![0u8; chunk_size * 64], Timestamp::ZERO)
                .unwrap_err();
            assert_eq!(err, FramerError::QueueFull { max_queued });
        }

        // The flood never actually grew the buffer past the bound: draining
        // now yields exactly `max_queued` chunks, not one more.
        let mut drained = 0usize;
        while stage.poll().is_some() {
            drained += 1;
        }
        assert_eq!(drained, max_queued);
    }
}
