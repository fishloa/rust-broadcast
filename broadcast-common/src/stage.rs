//! [`Stage`] — the incremental-drive contract every streaming stage in the
//! workspace will adopt (media-plane migration step 1).
//!
//! # Why
//!
//! Today the workspace has four incompatible "keep feeding me bytes/samples"
//! APIs, each shaped slightly differently:
//!
//! - `transmux::StreamingTsDemux::feed` returns `()` — output is pulled through
//!   a separate accessor.
//! - `transmux::StreamingFlvDemux::feed` returns `Result<(), Error>` — but still
//!   no inline output.
//! - `transmux::StreamingTsHlsSegmenter::push` returns the completed segment
//!   inline from the feed call itself.
//! - `hls_runtime`'s `LlHlsSegmenter` splits draining into two separate
//!   methods, `take_ready_parts` and `take_ready_segments`.
//!
//! None of these agree on whether output comes back from `feed`, from a
//! separate pull, or from two separate pulls — and none of them carry a clock,
//! which blocks deadline-driven stages (rate-scheduled SI emission, RTCP
//! timeout, segment-boundary timers) from ever sharing a driver loop with the
//! byte-shovelling stages. `Stage` unifies the shape: push input in with
//! [`Stage::feed`], pull typed output out with [`Stage::poll`] (repeatable,
//! decoupled from `feed`), signal end-of-input with [`Stage::finish`], and let
//! time-driven work happen via [`Stage::next_deadline`] / [`Stage::on_deadline`]
//! — all without requiring a full media IR or any concrete codec type.
//!
//! # `In`: a per-implementor input type, not hardcoded bytes
//!
//! The container-demux family's real input is bytes (`&[u8]`), but a
//! sample-consuming stage's real input is a typed `(track_id, Sample)` — there
//! is no useful byte encoding of a `Sample` that any caller wants, so forcing
//! `feed(&[u8], _)` on a segmenter would mean either inventing a fake wire
//! format nobody consumes or silently discarding real structure. [`Stage::In`]
//! is a generic associated type precisely so each implementor states its own
//! honest input shape — `&'a [u8]` for byte-stream stages, `(u32, Sample)` for
//! the segmenters — while [`Stage::Out`]/[`Stage::Error`] stay per-implementor
//! too. This is what lets one driver loop, generic only over `S: Stage`, span
//! both families (see `transmux/tests/it/stage.rs`'s `drive` helper).
//!
//! This module defines the trait plus its two small supporting types; it does
//! not migrate any existing implementor (that is the workspace's media-plane
//! migration step 2 onward — see `docs/superpowers/specs/2026-07-26-media-plane-architecture.md`).
//!
//! # Clock: why a clock parameter at all
//!
//! An audit of the wider workspace found a clockless `Stage` would only fit the
//! container-demux family. Several existing drive loops already take a clock
//! and cannot be expressed without one:
//!
//! - `dvb_conformance::ConformanceMonitor::feed(pkt, t: Duration)` — TR 101 290
//!   timing indicators need packet arrival time.
//! - `mpeg_ts::mux::SiMux::poll_into(now: Duration, out)` — PSI/SI re-emission
//!   is rate-scheduled, not event-driven.
//! - `media_doctor::WatchState::feed_datagram(payload, clock: Duration)` —
//!   datagram-loss/jitter watch state needs wall-clock deltas.
//!
//! So the clock is on the trait from the start, not bolted on later.
//!
//! # `Timestamp`, not `std::time::Instant`
//!
//! `broadcast-common` is `#![no_std]` without the `std` feature, and
//! `std::time::Instant` cannot exist in that build. [`Timestamp`] is a plain
//! `u64` microsecond count from an epoch the *driver* chooses (not the stage).
//! A driver that does have `std` can derive one from a pair of `Instant`s via
//! [`Timestamp::from_instant`]. All arithmetic on `Timestamp` saturates instead
//! of panicking, since a stage must never crash on a clock that runs backwards
//! or wraps.

use core::time::Duration;

use crate::units::{MICROS_PER_MILLI, NANOS_PER_MICRO};

/// Monotonic microseconds from an arbitrary epoch chosen by the driver.
///
/// Microseconds, not nanoseconds: this is a host's monotonic "now" and the
/// wall clock read beside it, never a media clock (those are
/// [`crate::clock33`]'s 90 kHz / 27 MHz values), and a microsecond resolves
/// every use here — a deadline, a reconnect instant, an inactivity timeout.
/// The epoch (what `0` means) is entirely up to whatever is driving the
/// [`Stage`] — stages must treat `Timestamp` as an opaque, monotonically
/// non-decreasing counter and never assume it relates to wall-clock time.
///
/// All arithmetic saturates rather than panicking: a [`Stage`] must not crash
/// because a driver's clock underflowed or overflowed.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Default)]
pub struct Timestamp(pub u64);

impl Timestamp {
    /// The zero timestamp — whatever epoch the driver has chosen.
    pub const ZERO: Timestamp = Timestamp(0);

    /// Construct from a raw nanosecond count, truncating to whole
    /// microseconds (a driver with only a nanosecond reading).
    pub const fn from_nanos(nanos: u64) -> Self {
        Timestamp(nanos / NANOS_PER_MICRO)
    }

    /// Construct from a whole-microsecond count.
    pub const fn from_micros(micros: u64) -> Self {
        Timestamp(micros)
    }

    /// The raw nanosecond count, from the whole microseconds this holds.
    ///
    /// Saturates at `u64::MAX` for a reading past 584 942 years; no host
    /// clock gets there.
    pub const fn as_nanos(self) -> u64 {
        self.0.saturating_mul(NANOS_PER_MICRO)
    }

    /// Construct from a whole-millisecond count, saturating at `u64::MAX`
    /// microseconds.
    pub const fn from_millis(millis: u64) -> Self {
        Timestamp(millis.saturating_mul(MICROS_PER_MILLI))
    }

    /// The raw microsecond count since the driver's chosen epoch.
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// The raw microsecond count in whole milliseconds, rounded down.
    pub const fn as_millis(self) -> u64 {
        self.0 / MICROS_PER_MILLI
    }

    /// `self - other`, saturating at zero instead of underflowing.
    pub const fn saturating_sub(self, other: Timestamp) -> Duration {
        Duration::from_micros(self.0.saturating_sub(other.0))
    }

    /// `self + nanos`, in nanoseconds: the result is the whole microsecond
    /// the nanosecond reading falls in (rounded up), saturating at `u64::MAX`.
    pub const fn saturating_add_nanos(self, nanos: u64) -> Self {
        let micros = nanos.div_ceil(NANOS_PER_MICRO);
        Timestamp(self.0.saturating_add(micros))
    }

    /// `self + duration`, rounded up to the whole microsecond the span ends
    /// in (a 100 ms timeout added to microsecond `t` is the deadline at which
    /// `now - t >= 100 ms` first holds), saturating at `u64::MAX` instead of
    /// overflowing.
    pub fn saturating_add(self, duration: Duration) -> Self {
        let micros = u64::try_from(duration.as_nanos().div_ceil(u128::from(NANOS_PER_MICRO)))
            .unwrap_or(u64::MAX);
        Timestamp(self.0.saturating_add(micros))
    }
}

#[cfg(feature = "std")]
impl Timestamp {
    /// Derive a [`Timestamp`] from a `std::time::Instant` pair: `now - base`,
    /// expressed as microseconds since `base`, rounded up so two different
    /// instants never collapse onto the same reading.
    ///
    /// `base` is whatever fixed instant the driver picked as its epoch (e.g.
    /// "when this stage was constructed"); every subsequent call converts an
    /// `Instant` to a `Timestamp` on the same epoch. This is a `std`-only
    /// convenience so `std` callers are not forced to hand-roll the
    /// subtraction; `no_std` drivers construct `Timestamp` directly.
    pub fn from_instant(base: std::time::Instant, now: std::time::Instant) -> Self {
        let elapsed = now.saturating_duration_since(base);
        let micros = u64::try_from(elapsed.as_nanos().div_ceil(u128::from(NANOS_PER_MICRO)))
            .unwrap_or(u64::MAX);
        Timestamp(micros)
    }
}

/// A stage's hint about how much more input it can usefully accept right now.
///
/// This is advisory backpressure, not enforcement: a driver may call
/// [`Stage::feed`] with more than `want_bytes`, or while `saturated` is `true`,
/// and a well-behaved stage must still handle it correctly (buffering,
/// erroring, or blocking as its own contract dictates) — `Demand` only lets a
/// cooperative driver avoid doing so needlessly.
#[non_exhaustive]
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Demand {
    /// How many more bytes this stage would like handed to [`Stage::feed`]
    /// before it is called again. `0` carries no meaning beyond "no
    /// particular preference"; it is not a demand for zero bytes.
    pub want_bytes: usize,
    /// `true` if the stage is currently full and would prefer not to receive
    /// more input until it has been polled / has had time to drain.
    pub saturated: bool,
}

impl Demand {
    /// Build a [`Demand`] requesting `want_bytes` more input, not saturated.
    pub const fn new(want_bytes: usize) -> Self {
        Demand {
            want_bytes,
            saturated: false,
        }
    }

    /// A [`Demand`] indicating the stage currently wants no more input.
    pub const fn saturated() -> Self {
        Demand {
            want_bytes: 0,
            saturated: true,
        }
    }

    /// [`Demand::saturated`] when `full`, otherwise the "no preference"
    /// [`Demand::default`].
    pub const fn saturated_if(full: bool) -> Self {
        if full {
            Self::saturated()
        } else {
            Demand::new(0)
        }
    }
}

/// The incremental-drive shape every streaming stage in the workspace adopts.
///
/// A `Stage` consumes input via [`feed`](Stage::feed) (the shape of that input
/// is [`In`](Stage::In), chosen per implementor — see the [module docs](self)),
/// produces typed output via repeated [`poll`](Stage::poll) calls (decoupled
/// from `feed` — a single `feed` may unlock zero, one, or many outputs), is
/// told there is no more input via [`finish`](Stage::finish), and may need to
/// act purely on the passage of time via [`next_deadline`](Stage::next_deadline)
/// / [`on_deadline`](Stage::on_deadline) (e.g. rate-scheduled re-emission with
/// no new input at all). [`demand`](Stage::demand) lets a driver ask before
/// feeding more.
///
/// See the [module docs](self) for the four divergent APIs this contract
/// unifies and the reasoning behind the `Timestamp` clock parameter.
///
/// # Drive loop
///
/// A typical driver loop:
///
/// ```
/// use broadcast_common::stage::{Demand, Stage, Timestamp};
/// use std::collections::VecDeque;
///
/// /// A stage that reverses each fed chunk and hands it back on poll.
/// struct Reverser {
///     queue: VecDeque<Vec<u8>>,
/// }
///
/// impl Stage for Reverser {
///     type In<'a> = &'a [u8];
///     type Out = Vec<u8>;
///     type Error = core::convert::Infallible;
///
///     fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<(), Self::Error> {
///         let mut chunk = input.to_vec();
///         chunk.reverse();
///         self.queue.push_back(chunk);
///         Ok(())
///     }
///
///     fn poll(&mut self) -> Option<Self::Out> {
///         self.queue.pop_front()
///     }
///
///     fn finish(&mut self) -> Result<(), Self::Error> {
///         Ok(())
///     }
///
///     fn next_deadline(&self) -> Option<Timestamp> {
///         None
///     }
///
///     fn on_deadline(&mut self, _now: Timestamp) {}
///
///     fn demand(&self) -> Demand {
///         Demand::new(4096)
///     }
/// }
///
/// let mut stage = Reverser { queue: Default::default() };
/// let mut outputs = Vec::new();
///
/// // feed → drain everything poll() currently has ready → repeat.
/// stage.feed(b"abc", Timestamp::from_nanos(0)).unwrap();
/// stage.feed(b"de", Timestamp::from_nanos(1_000)).unwrap();
/// while let Some(out) = stage.poll() {
///     outputs.push(out);
/// }
///
/// // No more input: flush anything the stage was holding back.
/// stage.finish().unwrap();
/// while let Some(out) = stage.poll() {
///     outputs.push(out);
/// }
///
/// assert_eq!(outputs, vec![vec![b'c', b'b', b'a'], vec![b'e', b'd']]);
/// ```
pub trait Stage {
    /// The shape of input this stage consumes via [`feed`](Stage::feed).
    ///
    /// A generic associated type, not a hardcoded `&[u8]`, so each
    /// implementor states its own honest input: byte-stream stages use
    /// `&'a [u8]`; sample-consuming stages (e.g. a segmenter) use an owned
    /// typed input such as `(u32, Sample)` that does not need to borrow
    /// anything, and can simply not use the `'a` parameter. See the
    /// [module docs](self) for why this is a GAT rather than a second
    /// `feed`-like method or an invented byte encoding.
    type In<'a>;
    /// The type of output this stage produces, pulled via [`poll`](Stage::poll).
    type Out;
    /// The error type this stage returns from [`feed`](Stage::feed) and
    /// [`finish`](Stage::finish).
    type Error;

    /// Feed more input into the stage at time `now`.
    ///
    /// May unlock output retrievable via subsequent [`poll`](Stage::poll)
    /// calls; a single `feed` call does not itself return output.
    fn feed(&mut self, input: Self::In<'_>, now: Timestamp) -> Result<(), Self::Error>;

    /// Pull one unit of ready output, if any is available.
    ///
    /// Callers should drain this in a `while let Some(_) = poll()` loop after
    /// every [`feed`](Stage::feed)/[`finish`](Stage::finish)/
    /// [`on_deadline`](Stage::on_deadline) call, since any of them may unlock
    /// more than one output.
    fn poll(&mut self) -> Option<Self::Out>;

    /// Signal that no more input will ever be fed.
    ///
    /// Lets the stage flush any output it was withholding (e.g. waiting for a
    /// boundary that will now never arrive). Idempotent: calling it more than
    /// once must not error or reprocess.
    fn finish(&mut self) -> Result<(), Self::Error>;

    /// The next point in time (on the same clock as [`feed`](Stage::feed)'s
    /// `now`) at which this stage has time-driven work to do, if any.
    ///
    /// A driver should call [`on_deadline`](Stage::on_deadline) once `now` has
    /// reached this value, even if no new input has arrived. `None` means the
    /// stage has nothing scheduled and only reacts to `feed`/`finish`, which
    /// is the default.
    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    /// Let the stage act purely on the passage of time (no new bytes), e.g. a
    /// rate-scheduled re-emission or a timeout.
    ///
    /// May unlock output retrievable via subsequent [`poll`](Stage::poll)
    /// calls, exactly like [`feed`](Stage::feed). The default does nothing, for
    /// a stage whose [`next_deadline`](Stage::next_deadline) is always `None`.
    fn on_deadline(&mut self, _now: Timestamp) {}

    /// A hint about how much more input this stage would like right now.
    ///
    /// Advisory only — see [`Demand`]'s docs for what a driver may and may not
    /// assume from it. The default is [`Demand::default`], no preference.
    fn demand(&self) -> Demand {
        Demand::default()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_clock_conversions_truncate_or_round_up_at_the_microsecond() {
        // `from_nanos` truncates: a driver with only a nanosecond reading lands on the
        // microsecond it is in.
        assert_eq!(Timestamp::from_nanos(1_999), Timestamp::from_micros(1));
        assert_eq!(
            Timestamp::from_nanos(1_999_999),
            Timestamp::from_micros(1_999)
        );
        // `saturating_add` rounds the span up: the deadline is the first microsecond at which
        // the whole span has passed, so `now - start >= span` holds there.
        assert_eq!(
            Timestamp::from_micros(10).saturating_add(Duration::from_nanos(1)),
            Timestamp::from_micros(11)
        );
        assert_eq!(
            Timestamp::from_micros(10).saturating_add(Duration::from_micros(100)),
            Timestamp::from_micros(110)
        );
        assert_eq!(
            Timestamp::from_micros(10).saturating_add_nanos(2_000),
            Timestamp::from_micros(12)
        );
        assert_eq!(
            Timestamp::from_micros(10).saturating_sub(Timestamp::from_micros(4)),
            Duration::from_micros(6)
        );
    }

    #[test]
    fn the_clock_conversions_round_trip_whole_units() {
        assert_eq!(Timestamp::from_micros(1_500).as_micros(), 1_500);
        assert_eq!(Timestamp::from_millis(1_500).as_millis(), 1_500);
        assert_eq!(
            Timestamp::from_millis(1_500).as_nanos(),
            1_500 * NANOS_PER_MICRO * 1_000
        );
        // A finer reading rounds down to the coarser unit, never up past the instant.
        assert_eq!(Timestamp::from_nanos(1_999).as_micros(), 1);
        assert_eq!(Timestamp::from_nanos(1_999_999).as_millis(), 1);
        // A count that would leave `u64` nanoseconds saturates instead of wrapping.
        assert_eq!(Timestamp::from_millis(u64::MAX).as_nanos(), u64::MAX);
        assert_eq!(Timestamp::from_micros(u64::MAX).as_nanos(), u64::MAX);
    }

    #[test]
    fn saturated_if_selects_saturated_or_default() {
        assert_eq!(
            super::Demand::saturated_if(true),
            super::Demand::saturated()
        );
        assert_eq!(super::Demand::saturated_if(false), super::Demand::default());
    }

    use super::*;
    use alloc::collections::VecDeque;
    use alloc::vec::Vec;
    use core::convert::Infallible;

    // A minimal `Stage` implementor using only `core`/`alloc` (no `std`),
    // proving the trait is genuinely usable from a `no_std` crate. It counts
    // fed bytes and, once a configured threshold is crossed OR a deadline
    // fires, emits the running total.
    struct ByteCounter {
        total: u64,
        threshold: u64,
        emitted_up_to: u64,
        pending: VecDeque<u64>,
        deadline: Option<Timestamp>,
        finished: bool,
    }

    impl ByteCounter {
        fn new(threshold: u64, deadline: Option<Timestamp>) -> Self {
            ByteCounter {
                total: 0,
                threshold,
                emitted_up_to: 0,
                pending: VecDeque::new(),
                deadline,
                finished: false,
            }
        }

        fn maybe_emit(&mut self) {
            if self.total.saturating_sub(self.emitted_up_to) >= self.threshold {
                self.pending.push_back(self.total);
                self.emitted_up_to = self.total;
            }
        }
    }

    impl Stage for ByteCounter {
        type In<'a> = &'a [u8];
        type Out = u64;
        type Error = Infallible;

        fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<(), Self::Error> {
            self.total += input.len() as u64;
            self.maybe_emit();
            Ok(())
        }

        fn poll(&mut self) -> Option<Self::Out> {
            self.pending.pop_front()
        }

        fn finish(&mut self) -> Result<(), Self::Error> {
            if !self.finished && self.total > self.emitted_up_to {
                self.pending.push_back(self.total);
                self.emitted_up_to = self.total;
            }
            self.finished = true;
            Ok(())
        }

        fn next_deadline(&self) -> Option<Timestamp> {
            self.deadline
        }

        fn on_deadline(&mut self, now: Timestamp) {
            if Some(now) >= self.deadline && self.total > self.emitted_up_to {
                self.pending.push_back(self.total);
                self.emitted_up_to = self.total;
                self.deadline = None;
            }
        }

        fn demand(&self) -> Demand {
            if self.total.saturating_sub(self.emitted_up_to) >= self.threshold {
                Demand::saturated()
            } else {
                Demand::new(self.threshold as usize)
            }
        }
    }

    #[test]
    fn no_std_implementor_drives_via_feed_poll() {
        let mut stage = ByteCounter::new(4, None);
        let mut outs: Vec<u64> = Vec::new();

        stage.feed(&[1, 2], Timestamp::from_nanos(0)).unwrap();
        assert_eq!(stage.poll(), None); // below threshold, nothing yet

        stage
            .feed(&[3, 4, 5], Timestamp::from_nanos(1_000))
            .unwrap();
        while let Some(out) = stage.poll() {
            outs.push(out);
        }
        assert_eq!(outs, alloc::vec![5]); // crossed threshold at total=5

        stage.finish().unwrap();
        // Nothing left unflushed since the last emission caught everything.
        assert_eq!(stage.poll(), None);
    }

    #[test]
    fn no_std_implementor_finish_flushes_remainder() {
        let mut stage = ByteCounter::new(100, None);
        stage.feed(&[1, 2, 3], Timestamp::from_nanos(0)).unwrap();
        assert_eq!(stage.poll(), None); // well below threshold

        stage.finish().unwrap();
        assert_eq!(stage.poll(), Some(3));
        assert_eq!(stage.poll(), None);
    }

    #[test]
    fn no_std_implementor_on_deadline_fires_without_new_input() {
        let deadline = Timestamp::from_nanos(5_000);
        let mut stage = ByteCounter::new(100, Some(deadline));
        stage.feed(&[1, 2, 3], Timestamp::from_nanos(0)).unwrap();
        assert_eq!(stage.next_deadline(), Some(deadline));
        assert_eq!(stage.poll(), None);

        stage.on_deadline(deadline);
        assert_eq!(stage.poll(), Some(3));
        assert_eq!(stage.next_deadline(), None);
    }

    #[test]
    fn demand_default_and_constructors() {
        let d = Demand::default();
        assert_eq!(d.want_bytes, 0);
        assert!(!d.saturated);

        let want = Demand::new(1024);
        assert_eq!(want.want_bytes, 1024);
        assert!(!want.saturated);

        let full = Demand::saturated();
        assert_eq!(full.want_bytes, 0);
        assert!(full.saturated);
    }

    #[test]
    fn timestamp_arithmetic_saturates_instead_of_panicking() {
        let zero = Timestamp::ZERO;
        let small = Timestamp::from_micros(5);
        let big = Timestamp::from_micros(u64::MAX);

        // Subtracting a larger timestamp from a smaller one saturates to 0,
        // never underflows/panics.
        assert_eq!(
            small.saturating_sub(Timestamp::from_micros(100)),
            Duration::from_micros(0)
        );
        assert_eq!(
            Timestamp::from_micros(100).saturating_sub(small),
            Duration::from_micros(95)
        );

        // Adding past u64::MAX saturates rather than wrapping/panicking.
        assert_eq!(
            big.saturating_add_nanos(10),
            Timestamp::from_micros(u64::MAX)
        );
        assert_eq!(zero.saturating_add_nanos(10), Timestamp::from_micros(1));

        // Duration-based saturating_add, including a Duration whose
        // nanoseconds exceed u64::MAX.
        assert_eq!(
            zero.saturating_add(Duration::from_nanos(10)),
            Timestamp::from_micros(1)
        );
        assert_eq!(
            big.saturating_add(Duration::from_secs(1)),
            Timestamp::from_micros(u64::MAX)
        );
        let huge = Duration::from_secs(u64::MAX);
        assert_eq!(zero.saturating_add(huge), Timestamp::from_micros(u64::MAX));
    }

    #[test]
    fn timestamp_ordering_and_default() {
        assert!(Timestamp::from_micros(1) < Timestamp::from_micros(2));
        assert_eq!(Timestamp::default(), Timestamp::ZERO);
    }

    #[cfg(feature = "std")]
    #[test]
    fn from_instant_std_convenience() {
        let base = std::time::Instant::now();
        let later = base + Duration::from_millis(5);
        let ts = Timestamp::from_instant(base, later);
        assert_eq!(ts, Timestamp::from_micros(5_000));

        // A `now` before `base` saturates to zero rather than panicking.
        let earlier = base.checked_sub(Duration::from_millis(1)).unwrap_or(base);
        let ts2 = Timestamp::from_instant(base, earlier);
        assert_eq!(ts2, Timestamp::ZERO);
    }
}
