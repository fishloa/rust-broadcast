//! `Retention` and `SegmentSink` — the hot/cold archive policy layered on
//! top of the segment log (plan step 3e;
//! `docs/superpowers/plans/2026-07-26-media-plane-implementation.md` Step 3e,
//! `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §1.2/§3).
//!
//! `#[cfg(feature = "std")]`, like [`crate::trunk`]/[`crate::egress`]: every
//! type here is built directly on [`crate::Trunk`]'s pinning
//! [`crate::trunk::SegmentCursor`], which is `std`-only for the reasons that
//! module's own docs give.
//!
//! # What this module deliberately does NOT reimplement
//!
//! Plan step 3b-ii already solved "a DVR/archive consumer must not miss a
//! segment, but the writer must never block" — see
//! [the DVR contradiction](crate::trunk#the-dvr-contradiction-losslessness-from-retention-not-back-pressure).
//! The resolution there was retention via a pinning
//! [`crate::trunk::SegmentCursor`] plus a caller-chosen
//! [`crate::trunk::ArchiveOverrun`] for the one moment the pin's guarantee
//! runs out. This module is that same tension **one layer further out**: a
//! cold-tier archive consumer ([`SegmentSink`]) can be slow or fail, and that
//! must not stall ingest either. Rather than inventing a second, parallel
//! "what happens when the archive can't keep up" enum, [`RetentionDriver`]
//! is built directly on [`crate::Trunk::pin_segments`] and threads the
//! caller's own [`ArchiveOverrun`] straight through — see
//! [`Retention::Tiered`]'s `on_overrun` field. **`ArchiveOverrun` governs
//! this layer verbatim, not a lookalike.**
//!
//! # The plan's `Tiered { hot, cold, cold_window }` sketch, and why only one
//! field is actually new
//!
//! The implementation plan names three fields; this module adds exactly
//! one new piece of state, `cold_window`, and says why the other two are not
//! separate fields at all rather than silently dropping them:
//!
//! - **`hot`** is [`ArchiveOverrun`] — the *already-existing* policy for what
//!   happens when the trunk's segment log (bounded by
//!   [`crate::trunk::TrunkConfig::segment_capacity`]) wants to evict an
//!   entry this driver has not yet consumed. Reused verbatim as
//!   [`Retention::Tiered::on_overrun`], not reinvented.
//! - **`cold`** is [`SegmentSink`] — not policy data at all. A sink is a
//!   caller-supplied, sans-IO *handle* (a trait object/generic parameter),
//!   not a `Copy`/`Debug`-able value a plain enum can hold. It is supplied
//!   separately, at [`RetentionDriver::new`] time, exactly the way
//!   [`crate::egress::PushEgress`]/[`crate::egress::SegmentEgress`] are
//!   supplied to whatever drains a cursor into them rather than being config
//!   data themselves.
//! - **`cold_window`** is the one genuinely new knob: how long a segment
//!   stays addressable in the cold tier after hand-off. See
//!   [Bounding the cold tier](#bounding-the-cold-tier-by-time-not-by-a-second-count-knob)
//!   for why a `Duration`, not another `NonZeroUsize`, is the right shape for
//!   it, and why it does not duplicate `segment_capacity`.
//!
//! # `SegmentSink` is sans-IO by construction
//!
//! [`SegmentSink::offer`] takes `&SegmentEntry`, returns [`SinkOutcome`]
//! synchronously, and touches nothing outside this crate's types — no
//! `std::fs`, no socket, no executor, no `async fn`. A real disk/object-store
//! adapter (Step 5 territory, per the plan) implements this trait and does
//! its actual I/O — buffering, retrying, spawning a write task — entirely
//! behind `offer`'s synchronous boundary; this crate has no way to know
//! (and does not need to know) whether "taken" means "written to disk
//! already" or "queued in the adapter's own channel". That is deliberately
//! the same non-committal contract [`crate::egress::PushEgress::send`] makes
//! for a transport it does not know the internals of either.
//!
//! # A failing or slow sink cannot stall ingest — and the pending hand-off
//! queue has a hard, structural bound of one
//!
//! [`SegmentWriter::publish_segment`](crate::trunk::SegmentWriter::publish_segment)
//! never references [`SegmentSink`] at all — a [`RetentionDriver`] is driven
//! by whatever caller owns it, on its own schedule, entirely decoupled from
//! the writer. That is a *structural* guarantee, not a race this module
//! defends against with a lock or a timeout: there is no code path from
//! `publish_segment` into `SegmentSink::offer`, so no mutation of this
//! module can make the writer block on a sink; the property is asserted by
//! there being nothing to mutate, and the test
//! `publish_segment_never_touches_the_sink` says so plainly rather than
//! inventing a timing assertion for something that cannot happen by
//! construction.
//!
//! What a slow/failing sink *does* affect is how fast [`RetentionDriver`]
//! drains its pin. [`RetentionDriver::drive`] holds **at most one**
//! [`SegmentEntry`] awaiting hand-off at a time ([`RetentionDriver::pending_len`]
//! is always `0` or `1`): it never polls the next segment off its pinning
//! cursor while one is stuck on [`SinkOutcome::Busy`]. Everything still
//! queued *behind* that stuck entry stays exactly where 3b-ii already put
//! it — inside the trunk's segment log, bounded by `segment_capacity`,
//! governed by the driver's own `on_overrun` the moment that bound is hit.
//! **No second queue, bounded or otherwise, was added for this** — the
//! existing pin mechanism already is the bounded backlog; this module adds
//! only the one-deep hand-off slot on top of it.
//!
//! # Bounding the cold tier by time, not by a second count knob
//!
//! [`RetentionDriver`] also keeps a small ledger of which sequence numbers
//! it has successfully handed off, so [`RetentionDriver::locate`] can answer
//! honestly (see the next section). That ledger is purged on every
//! [`RetentionDriver::drive`]/[`RetentionDriver::locate`] call: an entry
//! older than `cold_window` is dropped. A `Duration`, not a `NonZeroUsize`
//! count, is the correct shape here — unlike [`crate::trunk::TrunkConfig`]'s
//! ring capacities, `cold_window` is not a structural "how many slots exist"
//! bound (a zero `cold_window` is a legitimate, if degenerate, policy: write
//! through to cold storage and consider it immediately unaddressable —
//! there is no pathological "evicts everything the instant it is pushed"
//! failure mode the way a zero ring capacity has, so the `NonZeroUsize`
//! argument in `TrunkConfig`'s own docs does not apply here). The ledger's
//! size at any instant is bounded by how many segments a real recording
//! produces within `cold_window` — exactly the intended footprint of a DVR
//! window, the temporal analogue of `segment_capacity`'s count-based bound
//! on the hot ring, not an unbounded accumulation: it is purged, not merely
//! capped, so it cannot outlive its own window's worth of real segments even
//! under a sustained flood.
//!
//! # Does a cold segment stay addressable? Yes — "cold, ask the sink"
//!
//! Three answers were on the table (a write-only cold tier would make it
//! pointless as a catch-up/DVR backing store, so leaving this undecided was
//! not an option): report "cold, ask the sink"; query the sink
//! transparently from inside this crate; or declare cold reads out of scope
//! for 0.1. This module picks the first. [`SegmentSink`] has no read-side
//! method — sans-IO means this crate cannot know how to read from whatever
//! disk/object-store the real adapter uses (that adapter is Step 5's job),
//! so "query the sink transparently" is not available to a `no`-IO crate
//! without inventing a second, read-side trait this step has no consumer to
//! shape it against (exactly [`crate::egress`]'s own precedent for not
//! typing `ManifestSnapshot` before a real producer exists). "Out of scope"
//! was rejected because issue #746 (DVR/catch-up) is the entire reason this
//! tier exists — a cold tier nobody can resolve a catch-up request against
//! is not a smaller feature, it is a different, useless one.
//!
//! So: [`RetentionDriver::locate`] resolves a sequence number to one of
//! three honest [`SegmentLocation`]s — [`SegmentLocation::Hot`] (still, or
//! not yet, in the trunk's ordinary hot-ring path),
//! [`SegmentLocation::Cold`] (handed off to [`SegmentSink`] and still inside
//! `cold_window` — the caller must resolve the bytes through its own sink/
//! backing store; this crate never held them and cannot hand them back),
//! or [`SegmentLocation::Evicted`] (gapped before hand-off ever completed,
//! or aged out of the cold window since). See that method's own doc for the
//! exact decision table, including why it costs no new bookkeeping counter:
//! it reuses [`crate::Trunk::last_closed_segment`] for "has this sequence
//! number even been produced yet" rather than tracking a second, parallel
//! high-water mark.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;

use crate::trunk::{ArchiveOverrun, SegmentCursor, SegmentCursorItem, SegmentEntry, Trunk};

/// What happened when [`RetentionDriver::drive`] offered one [`SegmentEntry`]
/// to a [`SegmentSink`].
///
/// `#[non_exhaustive]`: the growth point for a later, more specific status
/// without a breaking change to every match arm in the workspace. Only two
/// variants today, deliberately: a sans-IO, synchronous `offer` call cannot
/// honestly distinguish "slow" from "permanently failing" (both look
/// identical from the caller's side — "not taken, right now"), so this type
/// does not pretend to — see [the module docs](self#segmentsink-is-sans-io-by-construction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SinkOutcome {
    /// The sink accepted hand-off of this segment. This says nothing about
    /// durability — a real adapter may still be buffering the write
    /// internally — only that this crate's obligation for this segment is
    /// discharged.
    Taken,
    /// The sink cannot accept this segment right now (its own internal
    /// queue is full, a backing store is unreachable, ...).
    /// [`RetentionDriver`] retries the **same** entry on the next
    /// [`RetentionDriver::drive`] call rather than dropping it or queuing a
    /// second one behind it — see
    /// [the module docs](self#a-failing-or-slow-sink-cannot-stall-ingest--and-the-pending-hand-off-queue-has-a-hard-structural-bound-of-one).
    Busy,
}

/// Where a finished segment's bytes go once they leave the trunk's hot
/// segment log — the cold-tier hand-off contract. Sans-IO: no filesystem, no
/// socket, no executor appears here. A real disk/object-store adapter (plan
/// Step 5) implements this trait and does its actual I/O entirely behind
/// [`SegmentSink::offer`]'s synchronous boundary.
pub trait SegmentSink: Send {
    /// Offer one segment for cold storage. Must return promptly — no
    /// blocking I/O, no `.await` — and report [`SinkOutcome::Busy`] rather
    /// than block if the underlying store cannot accept it right now.
    fn offer(&mut self, entry: &SegmentEntry) -> SinkOutcome;
}

/// The retention policy for a trunk's segment log: hot-only, or hot plus a
/// cold tier fed by a [`SegmentSink`]. See
/// [the module docs](self#the-plans-tiered--hot-cold-cold_window--sketch-and-why-only-one-field-is-actually-new)
/// for why only `cold_window` is new state; `on_overrun` is
/// [`ArchiveOverrun`] reused, and the sink itself is supplied separately at
/// [`RetentionDriver::new`], not carried by this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Retention {
    /// Segments live only in the trunk's segment log
    /// ([`crate::trunk::TrunkConfig::segment_capacity`] governs eviction;
    /// an ordinary [`crate::Trunk::subscribe_segments`] cursor sees ordinary
    /// [`SegmentCursorItem::Lagged`] on overflow). No cold tier: once
    /// evicted from hot, a segment is gone. The default — a policy with no
    /// [`SegmentSink`] to construct is the safe, do-nothing baseline.
    HotOnly,
    /// Hot ring unchanged, plus a cold tier: a [`RetentionDriver`] drains a
    /// pinning [`crate::Trunk::pin_segments`] cursor (governed by
    /// `on_overrun`, exactly [`ArchiveOverrun`]'s existing three-way trade)
    /// and offers every segment it consumes to a [`SegmentSink`]. A segment
    /// stays [`SegmentLocation::Cold`]-addressable for `cold_window` after
    /// hand-off, then ages out.
    Tiered {
        /// The pinning cursor's [`ArchiveOverrun`] — what happens when the
        /// hot ring wants to evict an entry the driver has not yet
        /// consumed. Exactly the 3b-ii policy, reused verbatim.
        on_overrun: ArchiveOverrun,
        /// How long a segment stays [`SegmentLocation::Cold`]-addressable
        /// after hand-off to the sink. See
        /// [Bounding the cold tier](self#bounding-the-cold-tier-by-time-not-by-a-second-count-knob)
        /// for why this is a `Duration`, not a count.
        cold_window: Duration,
    },
}

impl Default for Retention {
    /// [`Retention::HotOnly`] — no cold tier, no [`SegmentSink`] required.
    fn default() -> Self {
        Retention::HotOnly
    }
}

/// Where a [`RetentionDriver`] believes one segment currently is, resolved
/// by [`RetentionDriver::locate`]. See
/// [the module docs](self#does-a-cold-segment-stay-addressable-yes--cold-ask-the-sink)
/// for the design decision this type is the answer to.
///
/// `#[non_exhaustive]`: the growth point for a finer-grained answer later
/// (e.g. "cold, and here is which sink shard") without a breaking change to
/// every match arm in the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SegmentLocation {
    /// Not (yet) known to be evicted from the trunk's ordinary hot-ring
    /// path — either still resident there, guaranteed by this driver's pin,
    /// or not produced yet at all. **Not** a hard guarantee once this
    /// driver's pin has been torn down by [`ArchiveOverrun::Terminate`] —
    /// see [`RetentionDriver::locate`]'s own doc for the exact rule.
    Hot,
    /// No longer hot; handed off to this driver's [`SegmentSink`] and still
    /// inside `cold_window`. This crate does not hold the bytes — the
    /// caller must resolve them through its own sink/backing store.
    Cold,
    /// Genuinely gone: evicted from the hot ring before hand-off ever
    /// completed ([`ArchiveOverrun::Gap`]/[`ArchiveOverrun::Terminate`]
    /// fired first), or the hand-off succeeded but `cold_window` has since
    /// elapsed.
    Evicted,
}

/// One successfully-handed-off segment's ledger entry — purged once older
/// than `cold_window`; see
/// [Bounding the cold tier](self#bounding-the-cold-tier-by-time-not-by-a-second-count-knob).
struct ColdEntry {
    sequence_number: u32,
    handed_off_at: Timestamp,
}

/// Drains a pinning [`SegmentCursor`] into a [`SegmentSink`] — the
/// `Retention::Tiered` engine. Sans-IO, `Timestamp`-driven, no sleeping:
/// [`RetentionDriver::drive`] is a plain synchronous call a caller invokes
/// on its own schedule (a timer tick, a poll loop alongside `Stage::poll`),
/// exactly like every other pump in this crate.
pub struct RetentionDriver<S> {
    cursor: SegmentCursor,
    sink: S,
    cold_window: Duration,
    /// At most one entry: the hand-off queue's hard, structural bound. See
    /// [the module docs](self#a-failing-or-slow-sink-cannot-stall-ingest--and-the-pending-hand-off-queue-has-a-hard-structural-bound-of-one).
    in_flight: Option<SegmentEntry>,
    /// Ledger of handed-off, still-`cold_window`-fresh segments, oldest
    /// first (hand-offs happen in the pin's, and therefore the trunk's,
    /// publish order, so a plain front-to-back purge is correct).
    cold: VecDeque<ColdEntry>,
    /// Set once the pin's [`ArchiveOverrun::Terminate`] has fired — see
    /// [`RetentionDriver::locate`] for what this changes.
    terminated: bool,
    /// The highest `sequence_number` this driver's pin has actually drained
    /// off the cursor so far (via [`RetentionDriver::drive`]) — `None` until
    /// the first `Segment` item is polled. This, not
    /// [`crate::Trunk::last_closed_segment`], is what [`RetentionDriver::locate`]
    /// must compare a query against: a segment can be produced (and so
    /// reported by `last_closed_segment`) well before this driver's `drive`
    /// has ever run, while it is still resident and pin-protected in the
    /// hot ring — using `last_closed_segment` there conflated "produced" with
    /// "this driver has decided its fate", fabricating `Evicted` for a
    /// segment that is, in fact, still `Hot`.
    drained_up_to: Option<u32>,
}

impl<S: SegmentSink> RetentionDriver<S> {
    /// Build the driver `retention` implies. `None` for
    /// [`Retention::HotOnly`] — there is no cold tier to drive, so no
    /// [`SegmentSink`] is required and none is constructed.
    pub fn new(trunk: &Arc<Trunk>, retention: Retention, sink: S) -> Option<Self> {
        match retention {
            Retention::HotOnly => None,
            Retention::Tiered {
                on_overrun,
                cold_window,
            } => Some(RetentionDriver {
                cursor: trunk.pin_segments(on_overrun),
                sink,
                cold_window,
                in_flight: None,
                cold: VecDeque::new(),
                terminated: false,
                drained_up_to: None,
            }),
        }
    }

    /// Drain every segment currently ready on this driver's pinning cursor,
    /// offering each to the [`SegmentSink`] in order, and purge any cold
    /// ledger entry older than `cold_window`.
    ///
    /// Never blocks: [`SegmentSink::offer`] must not block by contract, and
    /// this method holds no lock of its own across the call. Bounded: the
    /// cursor can only ever be as far ahead as the trunk's segment log
    /// allows ([`crate::trunk::TrunkConfig::segment_capacity`]), so one call
    /// does finite work even under a sustained publish flood. Stops early,
    /// without polling further, the moment [`SinkOutcome::Busy`] fires —
    /// see [`RetentionDriver::pending_len`].
    pub fn drive(&mut self, now: Timestamp) {
        self.expire_cold(now);
        loop {
            if let Some(entry) = self.in_flight.take() {
                match self.sink.offer(&entry) {
                    SinkOutcome::Taken => {
                        self.cold.push_back(ColdEntry {
                            sequence_number: entry.sequence_number,
                            handed_off_at: now,
                        });
                    }
                    SinkOutcome::Busy => {
                        // Put it back and stop: do not poll the next segment
                        // off the pin while one is stuck. This is the entire
                        // bound on the pending hand-off queue.
                        self.in_flight = Some(entry);
                        return;
                    }
                }
            }
            match self.cursor.poll() {
                Some(SegmentCursorItem::Segment(entry)) => {
                    // As of right here this driver has decided this
                    // sequence number's fate (it will be offered to the
                    // sink next loop iteration) — advance `drained_up_to`
                    // now, not only once the hand-off outcome is known, so
                    // `locate` never reports `Hot` for a segment this driver
                    // has already moved past.
                    self.drained_up_to = Some(match self.drained_up_to {
                        Some(d) => d.max(entry.sequence_number),
                        None => entry.sequence_number,
                    });
                    self.in_flight = Some(entry);
                    // Loop back around to attempt hand-off immediately.
                }
                Some(SegmentCursorItem::Gap { .. }) | Some(SegmentCursorItem::Lagged { .. }) => {
                    // Nothing to hand off. These sequence numbers simply
                    // never enter `cold`. This loop continues immediately
                    // to the next `cursor.poll()` without returning, and the
                    // segment log always retains at least one entry once
                    // anything has ever been published (every capacity is
                    // non-zero), so the very next poll this same call makes
                    // yields the oldest still-resident `Segment` and folds
                    // the gapped range below it into `drained_up_to` via the
                    // arm above — `locate` needs no separate bookkeeping
                    // for exactly which numbers were skipped.
                }
                Some(SegmentCursorItem::Terminated) => {
                    self.terminated = true;
                    return;
                }
                None => return,
            }
        }
    }

    /// Segments currently awaiting hand-off: always `0` or `1`, by
    /// construction — see [`RetentionDriver::drive`]'s doc.
    pub fn pending_len(&self) -> usize {
        usize::from(self.in_flight.is_some())
    }

    /// Segments currently in the cold ledger (handed off and still inside
    /// `cold_window`) — a diagnostic, not a capacity: see
    /// [Bounding the cold tier](self#bounding-the-cold-tier-by-time-not-by-a-second-count-knob).
    pub fn cold_len(&self) -> usize {
        self.cold.len()
    }

    /// Resolve where segment `sequence_number` currently is. See
    /// [the module docs](self#does-a-cold-segment-stay-addressable-yes--cold-ask-the-sink)
    /// for the design story; this is the decision table:
    ///
    /// 1. In the cold ledger (handed off, still within `cold_window`) ⇒
    ///    [`SegmentLocation::Cold`].
    /// 2. Otherwise, if this driver's pin has been torn down
    ///    ([`ArchiveOverrun::Terminate`] already fired) ⇒
    ///    [`SegmentLocation::Evicted`] — this driver's guarantee has lapsed,
    ///    so it cannot honestly claim `Hot` for anything it is no longer
    ///    protecting, whether or not the trunk happens to still hold it.
    /// 3. Otherwise, if `sequence_number` is at or below
    ///    `drained_up_to` (this driver's own pin-drain progress) ⇒
    ///    [`SegmentLocation::Evicted`] — this driver has already decided
    ///    this segment's fate (offered it to the sink, or watched it be
    ///    gapped out from under the pin before it could), it is not in the
    ///    cold ledger, so it was either gapped before hand-off or handed off
    ///    and has since aged out of `cold_window`.
    /// 4. Otherwise ⇒ [`SegmentLocation::Hot`] — this driver has not yet
    ///    drained this far, whether because the segment has not been
    ///    produced yet or because [`Self::drive`] simply has not been
    ///    called since it was. **Not** [`crate::Trunk::last_closed_segment`]
    ///    (a segment can be produced, and so reported there, long before
    ///    this driver's `drive` has ever run, while still resident and
    ///    pin-protected in the hot ring — using that field here previously
    ///    fabricated `Evicted` for a segment this driver had not yet made
    ///    any decision about at all).
    pub fn locate(&mut self, sequence_number: u32, now: Timestamp) -> SegmentLocation {
        self.expire_cold(now);
        if self
            .cold
            .iter()
            .any(|c| c.sequence_number == sequence_number)
        {
            return SegmentLocation::Cold;
        }
        if self.terminated {
            return SegmentLocation::Evicted;
        }
        match self.drained_up_to {
            Some(drained) if sequence_number <= drained => SegmentLocation::Evicted,
            _ => SegmentLocation::Hot,
        }
    }

    /// Drop every cold-ledger entry whose `cold_window` has elapsed as of
    /// `now`. Entries are pushed in publish order, so a front-to-back purge
    /// is correct and this is the only place ledger entries are ever
    /// removed (no unbounded accumulation across any number of `drive`/
    /// `locate` calls).
    fn expire_cold(&mut self, now: Timestamp) {
        while let Some(front) = self.cold.front() {
            if front.handed_off_at.saturating_add(self.cold_window) <= now {
                self.cold.pop_front();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trunk::{NonMonotonicSequenceNumber, TrunkConfig};
    use std::num::NonZeroUsize;
    use std::sync::mpsc;
    use std::thread;
    use transmux::SegmentMeta;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    fn segment_entry(seq: u32) -> SegmentEntry {
        SegmentEntry::new(
            bytes::Bytes::from(vec![seq as u8; 4]),
            seq,
            Duration::from_secs(2),
            Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
            SegmentMeta {
                discontinuous: false,
            },
        )
    }

    /// A sink under full control of the test: `always_busy` toggles whether
    /// `offer` ever accepts, and every offered entry is recorded regardless,
    /// so a test can assert exactly what was (or was not) handed off.
    struct ScriptedSink {
        always_busy: bool,
        offered: Vec<u32>,
        taken: Vec<u32>,
    }

    impl ScriptedSink {
        fn new(always_busy: bool) -> Self {
            ScriptedSink {
                always_busy,
                offered: Vec::new(),
                taken: Vec::new(),
            }
        }
    }

    impl SegmentSink for ScriptedSink {
        fn offer(&mut self, entry: &SegmentEntry) -> SinkOutcome {
            self.offered.push(entry.sequence_number);
            if self.always_busy {
                SinkOutcome::Busy
            } else {
                self.taken.push(entry.sequence_number);
                SinkOutcome::Taken
            }
        }
    }

    // --- 1. A segment ages out of hot, is offered to the sink, stays
    //        `Cold` for `cold_window`, then releases -----------------------

    /// MUTATION VERIFIED: changing `RetentionDriver::expire_cold`'s
    /// condition from `<= now` to `< now` (off-by-one: an entry exactly at
    /// its expiry instant stays one tick longer than documented) makes this
    /// test's final `assert_eq!(driver.locate(1, at_expiry), SegmentLocation::Evicted)`
    /// fail — `locate` returns `SegmentLocation::Cold` instead, because the
    /// entry is still in `self.cold` at exactly `handed_off_at + cold_window`.
    /// Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn segment_is_cold_for_the_window_then_evicted() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(4), nz(8), nz(8)));
        let writer = trunk.segment_writer().unwrap();
        let retention = Retention::Tiered {
            on_overrun: ArchiveOverrun::Gap,
            cold_window: Duration::from_secs(10),
        };
        let mut driver = RetentionDriver::new(&trunk, retention, ScriptedSink::new(false))
            .expect("Tiered must build a driver");

        let handed_off_at = Timestamp::from_nanos(1_000_000_000);
        writer.publish_segment(segment_entry(1)).unwrap();
        driver.drive(handed_off_at);

        assert_eq!(driver.sink.offered, vec![1]);
        assert_eq!(driver.sink.taken, vec![1]);
        assert_eq!(
            driver.locate(1, handed_off_at),
            SegmentLocation::Cold,
            "just handed off: must be Cold, not Hot or Evicted"
        );

        // Strictly before the window elapses: still Cold.
        let before_expiry = handed_off_at.saturating_add(Duration::from_secs(9));
        assert_eq!(driver.locate(1, before_expiry), SegmentLocation::Cold);

        // At/after the window elapses: released.
        let at_expiry = handed_off_at.saturating_add(Duration::from_secs(10));
        assert_eq!(driver.locate(1, at_expiry), SegmentLocation::Evicted);
    }

    // --- 2. A failing sink does not stall `publish_segment` ---------------

    /// This is a *structural* property, not one a mutation can meaningfully
    /// break: `SegmentWriter::publish_segment` (see `src/trunk.rs`) has no
    /// reference to `SegmentSink`, `RetentionDriver`, or this module at all
    /// — there is no code path from the writer into a sink for a mutation to
    /// introduce a stall into. This test still exercises the real scenario
    /// (an always-`Busy` sink, many published segments) and asserts the
    /// writer completed every publish and the driver's own state stayed
    /// exactly as documented, rather than asserting a wall-clock timing
    /// bound that would be true even of a genuinely broken design running
    /// on a fast machine.
    #[test]
    fn publish_segment_never_touches_the_sink() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(4), nz(8), nz(8)));
        let writer = trunk.segment_writer().unwrap();
        let retention = Retention::Tiered {
            on_overrun: ArchiveOverrun::Gap,
            cold_window: Duration::from_secs(10),
        };
        let mut driver = RetentionDriver::new(&trunk, retention, ScriptedSink::new(true))
            .expect("Tiered must build a driver");

        // Every one of these calls returns (does not block) even though the
        // sink attached via `driver` never accepts anything.
        for seq in 1..=8u32 {
            writer.publish_segment(segment_entry(seq)).unwrap();
        }
        assert_eq!(trunk.segment_len(), 4, "hot ring still obeys its own cap");

        driver.drive(Timestamp::from_nanos(0));
        // Only the oldest surviving segment was ever offered — the driver
        // stopped polling further the instant it saw `Busy`.
        assert_eq!(driver.sink.offered, vec![5]);
        assert!(driver.sink.taken.is_empty());
    }

    // --- 3. A slow sink's pending hand-off queue is bounded: flooding
    //        cannot grow it past one -------------------------------------

    /// MUTATION VERIFIED (two independent mutations of the same `Busy` arm,
    /// each run separately, recompiled, confirmed failing, then reverted):
    ///
    /// 1. `SinkOutcome::Busy => { return; }` (dropping the stuck entry on
    ///    the floor instead of holding it for retry) makes
    ///    `assert_eq!(driver.pending_len(), 1, ..)` fail: `pending_len()`
    ///    reads `0` instead of `1`.
    /// 2. `SinkOutcome::Busy => { self.in_flight = None; continue; }`
    ///    (discarding the stuck entry *and* advancing to the next queued
    ///    segment, i.e. losing the entry while also moving on) fails the
    ///    same `pending_len` assertion the same way — confirming the
    ///    bound-of-one is what actually breaks, not merely which value
    ///    `offered` accumulates.
    #[test]
    fn slow_sink_pending_hand_off_queue_is_bounded_to_one() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(64), nz(8), nz(8)));
        let writer = trunk.segment_writer().unwrap();
        let retention = Retention::Tiered {
            on_overrun: ArchiveOverrun::Gap,
            cold_window: Duration::from_secs(10),
        };
        let mut driver = RetentionDriver::new(&trunk, retention, ScriptedSink::new(true))
            .expect("Tiered must build a driver");

        // Flood: 10 segments, always-Busy sink.
        for seq in 1..=10u32 {
            writer.publish_segment(segment_entry(seq)).unwrap();
        }
        driver.drive(Timestamp::from_nanos(0));

        assert_eq!(
            driver.pending_len(),
            1,
            "exactly one entry may be in flight, never more, never fewer once a segment exists"
        );
        assert_eq!(
            driver.sink.offered,
            vec![1],
            "only the single in-flight entry is ever offered while it is stuck; \
             a flood behind it must not grow the hand-off queue"
        );

        // Multiple further `drive` calls (simulating repeated polling by a
        // caller) retry the same stuck entry — `offered` grows with each
        // retry attempt — but never move on to a different sequence
        // number, and the in-flight slot never exceeds one.
        driver.drive(Timestamp::from_nanos(1));
        driver.drive(Timestamp::from_nanos(2));
        assert_eq!(driver.pending_len(), 1);
        assert!(
            driver.sink.offered.iter().all(|&seq| seq == 1),
            "only ever retries the single stuck entry, never advances to \
             the next queued segment while it is stuck: {:?}",
            driver.sink.offered
        );
    }

    // --- 4. `ArchiveOverrun` still governs at this layer: each variant ----

    /// `ArchiveOverrun::Gap` (default): the hot ring evicts out from under
    /// the driver's pin; the driver's own cursor reports the loss exactly
    /// as 3b-ii's `SegmentCursorItem::Gap`, and `locate` reflects it as
    /// `Evicted` (it was never handed off).
    ///
    /// MUTATION VERIFIED: changing the `Some(SegmentCursorItem::Gap { .. })`
    /// arm in `RetentionDriver::drive` to `panic!()` (simulating "this
    /// driver does not know how to survive a Gap") makes this test panic
    /// instead of completing — confirming the arm is load-bearing, not
    /// dead code. Recompiled and re-run to confirm the panic, then
    /// reverted.
    #[test]
    fn archive_overrun_gap_reports_loss_and_locate_reflects_it() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(2), nz(8), nz(8)));
        let writer = trunk.segment_writer().unwrap();
        let retention = Retention::Tiered {
            on_overrun: ArchiveOverrun::Gap,
            cold_window: Duration::from_secs(10),
        };
        let mut driver = RetentionDriver::new(&trunk, retention, ScriptedSink::new(false))
            .expect("Tiered must build a driver");

        // Segment log capacity 2: publishing 3 before the driver ever
        // drives evicts seq 1 out from under the pin.
        writer.publish_segment(segment_entry(1)).unwrap();
        writer.publish_segment(segment_entry(2)).unwrap();
        writer.publish_segment(segment_entry(3)).unwrap();

        driver.drive(Timestamp::from_nanos(0));

        // seq 1 was gapped, never offered; seq 2 and 3 made it to cold.
        assert_eq!(driver.sink.taken, vec![2, 3]);
        assert_eq!(
            driver.locate(1, Timestamp::from_nanos(0)),
            SegmentLocation::Evicted,
            "gapped before hand-off: genuinely gone, not Cold"
        );
        assert_eq!(
            driver.locate(2, Timestamp::from_nanos(0)),
            SegmentLocation::Cold
        );
    }

    /// `ArchiveOverrun::StallIngest`: `publish_segment` blocks until the
    /// driver's pin advances — proving the *writer* really does contend a
    /// slow archive consumer's pin when that policy is chosen (the one
    /// documented exception to "the writer never blocks" in the whole
    /// crate).
    ///
    /// No fresh mutation is recorded for this test: the blocking logic
    /// itself is `SegmentWriter::publish_segment`'s existing `must_wait`/
    /// `Condvar::wait` code in `src/trunk.rs`, entirely unmodified by this
    /// module — see that file's own
    /// `archive_overrun_stall_ingest_actually_blocks_the_writer`, which
    /// already carries the mutation transcript for that mechanism. This
    /// test's job is to prove the *integration*: that driving a
    /// `RetentionDriver`'s pinning cursor (rather than polling a bare
    /// `SegmentCursor` directly, as `trunk.rs`'s test does) is what
    /// releases the pin. That integration point is exercised for real
    /// above (the `recv_timeout` before `drive()` proves the writer is
    /// genuinely still blocked, not merely fast), so it is not a property a
    /// second mutation would add evidence for beyond what 3b-ii already
    /// recorded.
    #[test]
    fn archive_overrun_stall_ingest_blocks_writer_until_driver_advances() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(1), nz(8), nz(8)));
        let writer = Arc::new(trunk.segment_writer().unwrap());
        let retention = Retention::Tiered {
            on_overrun: ArchiveOverrun::StallIngest,
            cold_window: Duration::from_secs(10),
        };
        let mut driver = RetentionDriver::new(&trunk, retention, ScriptedSink::new(false))
            .expect("Tiered must build a driver");

        // Capacity 1: publishing the first segment fills the log; a second
        // publish would need to evict it, which StallIngest refuses to do
        // until the driver consumes the first.
        writer.publish_segment(segment_entry(1)).unwrap();

        let (done_tx, done_rx) = mpsc::channel();
        let blocked_writer = Arc::clone(&writer);
        let handle = thread::spawn(move || {
            blocked_writer.publish_segment(segment_entry(2)).unwrap();
            done_tx.send(()).unwrap();
        });

        assert!(
            done_rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "publish_segment must block: the driver has not consumed seq 1 yet"
        );

        // Draining the driver's pin (via `drive`, which polls the cursor
        // and hands the entry to the sink) releases the pin exactly like an
        // ordinary `SegmentCursor::poll` does in `trunk.rs`'s own
        // `archive_overrun_stall_ingest_actually_blocks_the_writer`.
        driver.drive(Timestamp::from_nanos(0));

        // Generous on purpose: the claim is "the writer unblocks once the pin
        // is drained", not "within N seconds". The unblock is observed across
        // a thread boundary, so a tight bound measures the machine's
        // scheduler. Note this `recv_timeout` also fails fast (Disconnected)
        // if the publishing thread panicked, which is the failure mode worth
        // catching -- a genuine `StallIngest` race -- and it does not wait
        // out the timeout to report it.
        done_rx
            .recv_timeout(Duration::from_secs(60))
            .expect("publish_segment must unblock once the driver drains its pin");
        handle.join().unwrap();

        // `taken` is asserted as a PREFIX here, not for exact equality.
        //
        // The single `drive` above both released the pin and polled the
        // cursor. The unblocked writer publishes seq 2 concurrently with that
        // poll, so whether this one `drive` also picks up seq 2 is decided by
        // thread scheduling: `[1]` and `[1, 2]` are BOTH correct outcomes.
        // Asserting `== vec![1]` made this test fail intermittently (four
        // observed failures) for a scheduling interleaving that is not a
        // defect — and it was misdiagnosed as a timing-bound problem first,
        // which a generous timeout could never have fixed.
        assert_eq!(
            driver.sink.taken.first().copied(),
            Some(1),
            "seq 1 must reach the sink first — it is what the driver drained \
             to release the pin"
        );

        // Draining to quiescence makes the end state deterministic, and
        // asserts something the old exact-equality never did: that seq 2
        // genuinely made it through after the stall, rather than being lost
        // when the writer unblocked.
        driver.drive(Timestamp::from_nanos(1));
        assert_eq!(
            driver.sink.taken,
            vec![1, 2],
            "both segments must reach the sink in order once the stall clears"
        );
    }

    /// `ArchiveOverrun::Terminate`: the driver's pin is torn down instead of
    /// gapping the recording or stalling ingest; `locate` conservatively
    /// reports `Evicted` for anything this driver can no longer promise,
    /// rather than fabricating `Hot`.
    ///
    /// MUTATION VERIFIED: changing `RetentionDriver::locate`'s
    /// `if self.terminated` check to `if false` (i.e. never treating a
    /// terminated driver specially) makes this test's final assertion
    /// fail: `driver.locate(3, ..)` — queried for a sequence number that
    /// has *not yet been produced*, so the fallback `last_closed_segment`
    /// branch alone would say `Hot` — returns `SegmentLocation::Hot`
    /// instead of the expected `SegmentLocation::Evicted`, a fabricated
    /// guarantee this driver can no longer back once its pin is gone.
    /// Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn archive_overrun_terminate_drops_pin_and_locate_stays_honest() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(1), nz(8), nz(8)));
        let writer = trunk.segment_writer().unwrap();
        let retention = Retention::Tiered {
            on_overrun: ArchiveOverrun::Terminate,
            cold_window: Duration::from_secs(10),
        };
        let mut driver = RetentionDriver::new(&trunk, retention, ScriptedSink::new(false))
            .expect("Tiered must build a driver");

        writer.publish_segment(segment_entry(1)).unwrap();
        writer.publish_segment(segment_entry(2)).unwrap(); // evicts seq 1's pin -> Terminate fires
        driver.drive(Timestamp::from_nanos(0));
        assert!(driver.terminated, "Terminate must have fired");

        // Query a sequence number that has *not yet been produced*
        // (`Trunk::last_closed_segment()` is still 2) — the fallback branch
        // of `locate` alone would say `Hot` for this (nothing says
        // otherwise: it simply has not happened yet), which is exactly the
        // case this test isolates: only the `terminated` check can turn
        // that into an honest `Evicted`, since this driver's pin is no
        // longer protecting *anything*, produced or not.
        assert_eq!(
            driver.locate(3, Timestamp::from_nanos(0)),
            SegmentLocation::Evicted,
            "once terminated, this driver cannot honestly claim Hot for \
             anything it is no longer protecting, produced or not"
        );
    }

    // --- 6. `locate` before `drive`: a produced-but-not-yet-drained -------
    // --- segment is Hot, not Evicted --------------------------------------

    /// MUTATION VERIFIED: reverting `RetentionDriver::locate`'s fallback
    /// branch from comparing against `self.drained_up_to` to comparing
    /// against `self.trunk.last_closed_segment()` (the pre-fix behaviour)
    /// makes the first assertion fail: `driver.locate(1, ..)`, called
    /// *before* `drive` has ever run, returns `SegmentLocation::Evicted`
    /// instead of `SegmentLocation::Hot` — `last_closed_segment()` reports
    /// 1 the instant it is published, so the old fallback treated "produced
    /// at all" as "gone", even though the segment is still sitting in the
    /// hot ring, still pin-protected, and this driver has not looked at it
    /// yet. Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn locate_is_hot_before_drive_and_reflects_the_real_outcome_after() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(4), nz(8), nz(8)));
        let writer = trunk.segment_writer().unwrap();
        let retention = Retention::Tiered {
            on_overrun: ArchiveOverrun::Gap,
            cold_window: Duration::from_secs(10),
        };
        let mut driver = RetentionDriver::new(&trunk, retention, ScriptedSink::new(false))
            .expect("Tiered must build a driver");

        writer.publish_segment(segment_entry(1)).unwrap();

        // Produced, but this driver's `drive` has not run yet: still Hot.
        assert_eq!(
            driver.locate(1, Timestamp::from_nanos(0)),
            SegmentLocation::Hot,
            "produced but not yet drained by this driver: still Hot, not Evicted"
        );

        driver.drive(Timestamp::from_nanos(0));

        // Now handed off to the sink: Cold.
        assert_eq!(
            driver.locate(1, Timestamp::from_nanos(0)),
            SegmentLocation::Cold
        );
    }

    // --- 7. A segment gapped before hand-off is Evicted even when no -----
    // --- later segment is ever drained to advance the pin's progress -----
    // --- some other way ---------------------------------------------------

    /// Companion to the `archive_overrun_gap_reports_loss_and_locate_reflects_it`
    /// test above, isolating `drained_up_to`'s update on the `Gap` arm
    /// itself: with segment-log capacity 1, publishing seq 1 then seq 2
    /// evicts seq 1 out from under the pin *before* any `drive` call, and
    /// the very next `cursor.poll()` in the same `drive` call yields seq 2
    /// as a real `Segment`, which is what folds the gapped seq 1 into
    /// `drained_up_to` — see `RetentionDriver::drive`'s `Gap` arm doc.
    #[test]
    fn locate_reports_evicted_for_a_gapped_segment() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(1), nz(8), nz(8)));
        let writer = trunk.segment_writer().unwrap();
        let retention = Retention::Tiered {
            on_overrun: ArchiveOverrun::Gap,
            cold_window: Duration::from_secs(10),
        };
        let mut driver = RetentionDriver::new(&trunk, retention, ScriptedSink::new(false))
            .expect("Tiered must build a driver");

        writer.publish_segment(segment_entry(1)).unwrap();
        writer.publish_segment(segment_entry(2)).unwrap(); // evicts seq 1's pin -> Gap fires

        driver.drive(Timestamp::from_nanos(0));

        assert_eq!(driver.sink.taken, vec![2]);
        assert_eq!(
            driver.locate(1, Timestamp::from_nanos(0)),
            SegmentLocation::Evicted,
            "gapped before hand-off: genuinely gone, not Hot or Cold"
        );
    }

    /// Adversarial-review finding: after a `SegmentWriter` re-issue, a
    /// restarted segmenter that (incorrectly) renumbers from 1 again would
    /// make every query keyed on `sequence_number` alone (`locate` included)
    /// resolve ambiguously against the OLD round's ledger for that same
    /// number. The root fix closes this at the source instead of patching
    /// every downstream query: `publish_segment` now rejects a
    /// `sequence_number` that is not strictly greater than the last one
    /// published, so the dangerous reuse can never reach `locate`'s ledger
    /// at all — a correctly re-issued writer resumes from
    /// `SegmentWriter::next_sequence_number`, which `locate` then resolves
    /// completely normally (produced, not yet drained: `Hot`).
    ///
    /// MUTATION VERIFIED: reverting `SegmentWriter::publish_segment`'s
    /// monotonicity check (allowing `w2.publish_segment(segment_entry(1))`
    /// to succeed again) makes this test's `unwrap_err()` panic instead —
    /// confirming the rejection is what actually prevents the reuse, not
    /// merely documented. Recompiled and re-run to confirm the failure,
    /// then reverted.
    #[test]
    fn segment_writer_reissue_rejects_reused_numbers_and_resumes_from_next_sequence_number() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(8), nz(8), nz(8)));
        let retention = Retention::Tiered {
            on_overrun: ArchiveOverrun::Gap,
            cold_window: Duration::from_secs(10),
        };
        let mut driver = RetentionDriver::new(&trunk, retention, ScriptedSink::new(false)).unwrap();
        {
            let w = trunk.segment_writer().unwrap();
            for s in 1..=5u32 {
                w.publish_segment(segment_entry(s)).unwrap();
            }
        } // dropped: re-issuable.
        driver.drive(Timestamp::from_nanos(0));

        let w2 = trunk.segment_writer().unwrap();

        // A restarted segmenter renumbering from 1 again is rejected
        // outright, not silently accepted with a stale/ambiguous ledger.
        let err = w2.publish_segment(segment_entry(1)).unwrap_err();
        assert_eq!(
            err,
            NonMonotonicSequenceNumber {
                attempted: 1,
                last_published: Some(5),
            }
        );

        // The correct resumption point, and what a correctly re-issued
        // writer must use instead:
        assert_eq!(w2.next_sequence_number(), 6);
        w2.publish_segment(segment_entry(6)).unwrap();

        let loc = driver.locate(6, Timestamp::from_nanos(0));
        assert_eq!(
            loc,
            SegmentLocation::Hot,
            "produced but not yet drained by this driver: still Hot, got {loc:?}"
        );
    }
}
