//! [`ByteMerge`] — the one bounded multi-input primitive in the byte layer.
//!
//! # Why this exists, and why it is the only one
//!
//! Rev 1 of the media-plane architecture claimed there was no multi-input
//! shape below the IR layer; rev 2 conceded that was already false — ST
//! 2022-7 hitless (2-input) and RIST bonding (N-input) are on the roadmap,
//! and rev 1 had *cited* 2022-7 to justify one-writer-per-`Trunk` while
//! ignoring that its own inputs are multiple. `ByteMerge` is rev 2's honest
//! answer: **N byte sources reduce to one byte stream at exactly one place**,
//! the byte layer, via exactly one primitive. Every layer above this one
//! (demux, IR transforms, `Trunk`) stays strictly single-input — if a third
//! multi-input shape is ever needed above the byte layer, that is a sign this
//! design is wrong and a real DAG is the answer, not a reason to bolt another
//! ad-hoc merge point onto some other layer.
//!
//! # Messages, not a byte soup
//!
//! **`ByteMerge` operates on discrete messages (one [`bytes::Bytes`] unit per
//! [`ByteMerge::feed`] call), not an undelimited byte stream.** Both policies
//! implemented here need message boundaries to make sense of: [`MergePolicy::FirstArrival`]
//! interleaves whole messages (there is no meaningful "first arrival" of a
//! byte offset inside an undelimited stream), and [`MergePolicy::Failover`]
//! forwards or drops whole messages depending on which source is currently
//! active. A future deduplicating policy needs this even more directly — you
//! cannot recognise two copies of the same datagram by comparing byte
//! ranges, only by comparing whole messages (or, for ST 2022-7 specifically,
//! RTP sequence numbers inside them; see below). The API shape enforces this
//! itself: [`ByteMerge::feed`] takes one `Bytes` per call, not a `&[u8]`
//! slice a caller could concatenate several messages into.
//!
//! # `Hitless2022_7` is deliberately absent, not stubbed
//!
//! SMPTE ST 2022-7 seamless switching dedupes two identical RTP streams by
//! comparing RTP sequence numbers, selecting whichever copy of each sequence
//! number arrives first and discarding the duplicate. Raw `Bytes` cannot
//! express that: telling two arrivals apart as "the same sequence number"
//! needs an RTP header parse and per-stream sequence-number bookkeeping this
//! layer does not have (and should not grow just to make room for a variant —
//! see the "no correct producer" note below). That work lands with #752,
//! where it gets a real implementation. Until then, this crate's
//! [`MergePolicy`] simply does not have a `Hitless2022_7` variant — it is not
//! present as an empty/unimplemented arm, a `todo!()`, or a doc-only stub.
//! This project has already been bitten more than once by shipping a variant
//! with no correct producer behind it; the fix here is to not create another,
//! rather than to add one and hope nobody constructs it. [`MergePolicy`] is
//! `#[non_exhaustive]` specifically so adding `Hitless2022_7` later is
//! additive, not a breaking change to every match arm in the workspace.
//!
//! # Bounding
//!
//! Both types in the byte layer eat remote input directly, and this
//! project's unbounded-allocation incidents have all been in code doing
//! exactly that. `ByteMerge` holds two kinds of state, both bounded
//! independently of call volume:
//!
//! - **Per-source state** (`last_seen`) is a fixed-size array sized at
//!   construction (`num_sources`) — flooding one source with any number of
//!   `feed` calls updates one entry in place, it never grows the array.
//! - **The output queue** is capped at `max_queued`; once full,
//!   [`ByteMerge::feed`] rejects the call outright with
//!   [`MergeError::QueueFull`] rather than growing past the cap or silently
//!   evicting (unlike [`crate::ByteTap`] — a merge feeds a demux pipeline
//!   that is expected to apply its own back-pressure via `demand()`
//!   upstream, so there is no "producer must never see a rejection" contract
//!   here the way there is for a tap).

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use broadcast_common::stage::Timestamp;
use bytes::Bytes;
use core::time::Duration;
use thiserror::Error;

/// Identifies one of the `N` sources feeding a [`ByteMerge`].
///
/// `ByteMerge` has no notion of what a source *is* — a UDP socket, an RTP
/// session, a T2-MI PLP — only that it is one of `0..num_sources`, assigned
/// by whoever constructs the merge and calls [`ByteMerge::feed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub usize);

/// Policy [`ByteMerge`] uses to reduce `N` sources to one output stream.
///
/// `#[non_exhaustive]`: see the [module docs](self) on why `Hitless2022_7`
/// is deliberately not a variant here yet, and why that makes this attribute
/// load-bearing rather than decorative — adding it in #752 must not break
/// every exhaustive `match` in the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergePolicy {
    /// Forward every message from every source, in the order [`ByteMerge::feed`]
    /// is called, with no preference and no deduplication. The plain fan-in
    /// case: whichever source's message arrives first is emitted first.
    FirstArrival,
    /// Prefer `primary`; forward `secondary`'s messages only while `primary`
    /// is considered silent.
    ///
    /// # Silence detection
    ///
    /// `primary` is silent once `silence_timeout` has elapsed since the last
    /// message `primary` produced, checked via [`ByteMerge::on_deadline`]
    /// (there is no background timer — this crate is sans-IO, like
    /// [`crate::ByteStage`], so a driver must call `on_deadline` itself; see
    /// [`ByteMerge::next_deadline`]). A single message from `primary` — even
    /// one that arrives close to the timeout — resets the silence clock, so
    /// a merge does not switch away from a primary that is merely running
    /// late rather than actually down. Before `primary` has produced its
    /// first message at all, the merge treats it as active by default and
    /// reports no deadline: a fresh merge has no evidence primary is
    /// unhealthy, only that it has not started yet, and those are not the
    /// same thing.
    ///
    /// # Switch-back rule
    ///
    /// **The instant `primary` produces a message again, it immediately
    /// reclaims active status** — that message is forwarded, and any
    /// subsequent `secondary` traffic is dropped until `primary` goes silent
    /// again. This is chosen because "prefer a primary source" only means
    /// something if the merge actually prefers it whenever it is alive, and
    /// no separate hold-down timer is needed to avoid flapping on the
    /// switch-*back* direction: the switch *away* from primary already
    /// required a full `silence_timeout` of no traffic, so a real message
    /// arriving is evidence the source recovered, not jitter — the flapping
    /// this project cares about (switching away on a single late packet) is
    /// what the silence-clock reset above already prevents.
    Failover {
        /// The source [`ByteMerge`] prefers whenever it is producing.
        primary: SourceId,
        /// The source forwarded only while `primary` is silent.
        secondary: SourceId,
        /// How long `primary` must be silent before `secondary` takes over.
        silence_timeout: Duration,
    },
    // `Hitless2022_7` intentionally NOT a variant — see the module docs.
}

/// Errors [`ByteMerge::feed`] can return.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeError {
    /// `source` is not one of `0..num_sources` for this merge.
    #[error("source {source_id:?} is out of range for a merge with {num_sources} sources")]
    UnknownSource {
        /// The out-of-range source that was fed.
        source_id: SourceId,
        /// How many sources this merge was constructed with.
        num_sources: usize,
    },
    /// The bounded output queue already holds `max_queued` messages awaiting
    /// [`ByteMerge::poll`]; the call was rejected outright and nothing from
    /// it was buffered.
    #[error("merge output queue full: {max_queued} messages already queued for poll()")]
    QueueFull {
        /// The configured queue bound.
        max_queued: usize,
    },
}

/// The one bounded multi-input primitive in the byte layer: `N` byte sources
/// reduced to one output stream. See the [module docs](self).
pub struct ByteMerge {
    policy: MergePolicy,
    num_sources: usize,
    /// Last arrival time seen from each source, indexed by `SourceId.0`.
    /// Fixed-size at construction — bounded per-source state (module docs).
    last_seen: Vec<Option<Timestamp>>,
    /// Which source `Failover` currently forwards; meaningless (unused)
    /// under `FirstArrival`.
    active: usize,
    queue: VecDeque<(Bytes, Timestamp)>,
    max_queued: usize,
}

impl ByteMerge {
    /// Construct a merge for `num_sources` sources (`0..num_sources`),
    /// applying `policy`, with its output queue bounded to `max_queued`
    /// messages.
    ///
    /// Panics if `num_sources == 0`, `max_queued == 0`, or (for
    /// [`MergePolicy::Failover`]) `primary`/`secondary` are not both within
    /// `0..num_sources` — all three are construction-time configuration
    /// mistakes, not remote input, so they panic rather than returning a
    /// `Result` a caller could ignore.
    pub fn new(policy: MergePolicy, num_sources: usize, max_queued: usize) -> Self {
        assert!(num_sources > 0, "ByteMerge num_sources must be > 0");
        assert!(max_queued > 0, "ByteMerge max_queued must be > 0");
        let active = match &policy {
            MergePolicy::Failover {
                primary, secondary, ..
            } => {
                assert!(
                    primary.0 < num_sources && secondary.0 < num_sources,
                    "ByteMerge Failover primary/secondary must be within 0..num_sources"
                );
                primary.0
            }
            MergePolicy::FirstArrival => 0,
        };
        ByteMerge {
            policy,
            num_sources,
            last_seen: vec![None; num_sources],
            active,
            queue: VecDeque::new(),
            max_queued,
        }
    }

    /// How many sources this merge accepts `feed` calls from.
    pub fn num_sources(&self) -> usize {
        self.num_sources
    }

    /// Messages currently queued, awaiting [`ByteMerge::poll`]. Never
    /// exceeds the `max_queued` bound this merge was constructed with.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// `true` if no messages are currently queued.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Feed one discrete message from `source`, observed at `at`.
    ///
    /// Whether it is forwarded to [`ByteMerge::poll`] depends on the policy:
    /// every message is forwarded under [`MergePolicy::FirstArrival`]; under
    /// [`MergePolicy::Failover`] only the currently-active source's messages
    /// are (see that variant's docs for exactly when that is, and the
    /// switch-back rule).
    ///
    /// Returns [`MergeError::UnknownSource`] if `source` is out of range, or
    /// [`MergeError::QueueFull`] if the output queue is already at its bound
    /// — in both cases nothing from this call is buffered.
    pub fn feed(&mut self, source: SourceId, msg: Bytes, at: Timestamp) -> Result<(), MergeError> {
        if source.0 >= self.num_sources {
            return Err(MergeError::UnknownSource {
                source_id: source,
                num_sources: self.num_sources,
            });
        }
        self.last_seen[source.0] = Some(at);

        let forward = match &self.policy {
            MergePolicy::FirstArrival => true,
            MergePolicy::Failover {
                primary, secondary, ..
            } => {
                if source.0 == primary.0 {
                    // Switch-back rule: primary reclaims active status the
                    // instant it produces a message.
                    self.active = primary.0;
                    true
                } else if source.0 == secondary.0 {
                    self.active == secondary.0
                } else {
                    // Neither named source: Failover only defines behaviour
                    // for its two named sources, so a third source's traffic
                    // is silently uninvolved rather than an error.
                    false
                }
            }
        };

        if forward {
            if self.queue.len() >= self.max_queued {
                return Err(MergeError::QueueFull {
                    max_queued: self.max_queued,
                });
            }
            self.queue.push_back((msg, at));
        }
        Ok(())
    }

    /// Pull the next merged message, in the order it was forwarded by
    /// [`ByteMerge::feed`].
    pub fn poll(&mut self) -> Option<(Bytes, Timestamp)> {
        self.queue.pop_front()
    }

    /// When [`ByteMerge::on_deadline`] should next be called to check a
    /// [`MergePolicy::Failover`] silence timeout.
    ///
    /// `None` under [`MergePolicy::FirstArrival`] (arrival-driven, no clock
    /// needed) and under `Failover` before `primary` has produced its first
    /// message (see that variant's docs).
    pub fn next_deadline(&self) -> Option<Timestamp> {
        match &self.policy {
            MergePolicy::FirstArrival => None,
            MergePolicy::Failover {
                primary,
                silence_timeout,
                ..
            } => self.last_seen[primary.0].map(|t| t.saturating_add(*silence_timeout)),
        }
    }

    /// Drive time-based transitions: under [`MergePolicy::Failover`], switch
    /// to `secondary` if `primary` has been silent for at least
    /// `silence_timeout` as of `now`. A no-op under [`MergePolicy::FirstArrival`].
    pub fn on_deadline(&mut self, now: Timestamp) {
        if let MergePolicy::Failover {
            primary,
            secondary,
            silence_timeout,
        } = &self.policy
        {
            if let Some(last) = self.last_seen[primary.0] {
                if now.saturating_sub(last) >= *silence_timeout {
                    self.active = secondary.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FirstArrival` must interleave two sources exactly in the order
    /// `feed` was called, regardless of which source each call came from.
    #[test]
    fn first_arrival_interleaves_two_sources_in_arrival_order() {
        let mut merge = ByteMerge::new(MergePolicy::FirstArrival, 2, 8);
        let a = SourceId(0);
        let b = SourceId(1);

        merge
            .feed(a, Bytes::from_static(b"a0"), Timestamp::from_nanos(0))
            .unwrap();
        merge
            .feed(b, Bytes::from_static(b"b0"), Timestamp::from_nanos(1))
            .unwrap();
        merge
            .feed(a, Bytes::from_static(b"a1"), Timestamp::from_nanos(2))
            .unwrap();
        merge
            .feed(b, Bytes::from_static(b"b1"), Timestamp::from_nanos(3))
            .unwrap();

        assert_eq!(
            merge.poll(),
            Some((Bytes::from_static(b"a0"), Timestamp::from_nanos(0)))
        );
        assert_eq!(
            merge.poll(),
            Some((Bytes::from_static(b"b0"), Timestamp::from_nanos(1)))
        );
        assert_eq!(
            merge.poll(),
            Some((Bytes::from_static(b"a1"), Timestamp::from_nanos(2)))
        );
        assert_eq!(
            merge.poll(),
            Some((Bytes::from_static(b"b1"), Timestamp::from_nanos(3)))
        );
        assert_eq!(merge.poll(), None);
    }

    /// `Failover` must not switch before the silence timeout, must not flap
    /// on a single late (but still-within-timeout) primary message, must
    /// switch once the timeout genuinely elapses, and must switch straight
    /// back the instant primary is heard from again.
    #[test]
    fn failover_switches_after_timeout_not_on_single_late_message_then_switches_back() {
        let primary = SourceId(0);
        let secondary = SourceId(1);
        let silence_timeout = Duration::from_millis(100);
        let mut merge = ByteMerge::new(
            MergePolicy::Failover {
                primary,
                secondary,
                silence_timeout,
            },
            2,
            8,
        );

        // t=0: primary speaks. Active is primary; forwarded.
        merge
            .feed(primary, Bytes::from_static(b"p0"), Timestamp::from_nanos(0))
            .unwrap();
        assert_eq!(
            merge.poll(),
            Some((Bytes::from_static(b"p0"), Timestamp::from_nanos(0)))
        );

        // While primary is active, secondary's traffic is dropped, not queued.
        merge
            .feed(
                secondary,
                Bytes::from_static(b"s-dropped"),
                Timestamp::from_nanos(10),
            )
            .unwrap();
        assert_eq!(merge.poll(), None);

        // t=50ms: well within the 100ms timeout — must not switch.
        merge.on_deadline(Timestamp::from_nanos(50_000_000));

        // t=90ms: primary speaks again — "a single late message" arriving
        // close to (but before) the timeout. This MUST reset the silence
        // clock rather than merely being noted: if it didn't, the next
        // on_deadline at t=150ms (60ms after t=90ms, well under the 100ms
        // timeout) would incorrectly see 150ms since t=0 and switch.
        merge
            .feed(
                primary,
                Bytes::from_static(b"p-late"),
                Timestamp::from_nanos(90_000_000),
            )
            .unwrap();
        assert_eq!(
            merge.poll(),
            Some((
                Bytes::from_static(b"p-late"),
                Timestamp::from_nanos(90_000_000)
            ))
        );

        // t=150ms: only 60ms since the t=90ms reset — must NOT have switched.
        merge.on_deadline(Timestamp::from_nanos(150_000_000));
        merge
            .feed(
                secondary,
                Bytes::from_static(b"s-still-dropped"),
                Timestamp::from_nanos(150_000_000),
            )
            .unwrap();
        assert_eq!(
            merge.poll(),
            None,
            "must not have flapped to secondary on a single late primary message"
        );

        // t=200ms: 110ms since t=90ms — genuinely silent past the timeout.
        merge.on_deadline(Timestamp::from_nanos(200_000_000));
        merge
            .feed(
                secondary,
                Bytes::from_static(b"s0"),
                Timestamp::from_nanos(200_000_000),
            )
            .unwrap();
        assert_eq!(
            merge.poll(),
            Some((
                Bytes::from_static(b"s0"),
                Timestamp::from_nanos(200_000_000)
            )),
            "must have switched to secondary once genuinely silent past the timeout"
        );

        // Primary returns: reclaims active status immediately (switch-back
        // rule), and secondary is dropped again from this point on.
        merge
            .feed(
                primary,
                Bytes::from_static(b"p-back"),
                Timestamp::from_nanos(210_000_000),
            )
            .unwrap();
        assert_eq!(
            merge.poll(),
            Some((
                Bytes::from_static(b"p-back"),
                Timestamp::from_nanos(210_000_000)
            ))
        );
        merge
            .feed(
                secondary,
                Bytes::from_static(b"s-dropped-again"),
                Timestamp::from_nanos(220_000_000),
            )
            .unwrap();
        assert_eq!(merge.poll(), None);
    }

    /// Per-source state must be bounded: flooding one source far beyond the
    /// queue cap must not grow `num_sources()`, and the output queue itself
    /// must stay capped, rejecting the overflow outright.
    #[test]
    fn per_source_state_and_output_queue_are_bounded_under_flood() {
        let max_queued = 4;
        let mut merge = ByteMerge::new(MergePolicy::FirstArrival, 2, max_queued);

        let mut full_errors = 0usize;
        for i in 0..10_000u64 {
            match merge.feed(
                SourceId(0),
                Bytes::from_static(b"x"),
                Timestamp::from_nanos(i),
            ) {
                Ok(()) => {}
                Err(MergeError::QueueFull { max_queued: cap }) => {
                    assert_eq!(cap, max_queued);
                    full_errors += 1;
                }
                Err(other) => panic!("unexpected error: {other:?}"),
            }
            assert!(
                merge.len() <= max_queued,
                "queue exceeded its bound mid-flood"
            );
        }

        // Fixed per-source state never grew from the flood.
        assert_eq!(merge.num_sources(), 2);
        // The queue is exactly full (never more), and the flood was mostly
        // rejected once it was.
        assert_eq!(merge.len(), max_queued);
        assert!(full_errors > 0, "flood should have hit the queue bound");

        let mut drained = 0usize;
        while merge.poll().is_some() {
            drained += 1;
        }
        assert_eq!(drained, max_queued);
    }

    #[test]
    fn unknown_source_is_rejected() {
        let mut merge = ByteMerge::new(MergePolicy::FirstArrival, 2, 4);
        let err = merge
            .feed(SourceId(5), Bytes::from_static(b"x"), Timestamp::ZERO)
            .unwrap_err();
        assert_eq!(
            err,
            MergeError::UnknownSource {
                source_id: SourceId(5),
                num_sources: 2,
            }
        );
        assert!(merge.is_empty());
    }

    #[test]
    #[should_panic(expected = "num_sources must be > 0")]
    fn zero_sources_panics() {
        let _ = ByteMerge::new(MergePolicy::FirstArrival, 0, 4);
    }

    #[test]
    #[should_panic(expected = "max_queued must be > 0")]
    fn zero_max_queued_panics() {
        let _ = ByteMerge::new(MergePolicy::FirstArrival, 2, 0);
    }

    #[test]
    #[should_panic(expected = "within 0..num_sources")]
    fn failover_out_of_range_source_panics() {
        let _ = ByteMerge::new(
            MergePolicy::Failover {
                primary: SourceId(0),
                secondary: SourceId(9),
                silence_timeout: Duration::from_millis(1),
            },
            2,
            4,
        );
    }
}
