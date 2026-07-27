//! [`TsIngestSession`] — the shared MPEG-2 TS ingest session (plan step 5a).
//!
//! Every TS-carrying ingest source in this crate — [`crate::source::ts_udp`]
//! (UDP datagrams), [`crate::source::ts_http`] (a chunked HTTP body),
//! [`crate::source::srt`] (SRT payloads) — differs *only* in its transport.
//! Once the bytes are in hand, all three do the identical thing: feed
//! [`transmux::StreamingTsDemux`] and translate its
//! [`transmux::DemuxEvent`]s into [`SessionEvent`]s. Before this port each
//! kept its own copy of that `while let Some(ev) = demux.poll_event()` drain
//! (three near-identical ~60-line blocks, which had already drifted — see
//! the differing `TrackAdded` log wording); this module is the single copy,
//! and the three transports keep only their socket/stream handling.
//!
//! This is exactly the consolidation `media_plane::ingress`'s module docs
//! anticipate ("this module's job is to ... own the feed/poll/deadline/
//! dispatch loop so no protocol has to reimplement it"): the plane owns the
//! driver loop, and this owns the one TS-specific translation the plane
//! cannot know about.
//!
//! # B5: the mid-stream `NewProgram` these sources used to drop
//!
//! Before this port, a PID declared only *after* `connect()`'s PMT wait
//! resolved was logged and silently dropped (issue #774) — cited directly in
//! `media_plane::ingress`'s own module docs as "the gap `NewProgram`
//! generalises". [`ProgramTracker`] closes it: the *first*
//! [`transmux::DemuxEvent::TracksResolved`] mints `ProgramId(0)` from every
//! track collected up to that point, and **any**
//! [`transmux::DemuxEvent::TrackAdded`] arriving after that mints a **new**
//! `ProgramId` (1, 2, ...) instead of being dropped.
//!
//! This is a deliberate simplification, not full MPTS `program_number`
//! support: `transmux::DemuxEvent` does not carry `program_number` today
//! (`media_plane::ingress`'s own docs record this as finding B5's root
//! cause), so "a track declared after the stream's initial program resolved"
//! is treated as a new program rather than being mapped to its real
//! PMT-declared `program_number` — the latter needs `program_number`
//! threaded through `transmux`'s IR first (future work, not this port).
//! What this *does* deliver is the mechanism
//! [`media_plane::ingress::IngestDriver`] was built for: a `NewProgram`
//! announced mid-session, on an already-live connection, mints a fresh
//! `Trunk` exactly like one announced at the start.
//!
//! # Why `Established` is queued at construction
//!
//! None of the three TS transports has a *media-level* handshake to
//! negotiate once its transport is up (UDP: a local bind; HTTP: the response
//! headers are already in; SRT: the SRT handshake completed in the socket
//! layer). They are the "purely local operation with nothing to negotiate"
//! case `media_plane::ingress`'s own `ScriptedSession` test precedent
//! documents, so [`SessionEvent::Established`] is queued up front rather
//! than gated on the PMT — establishment is a per-*connection* fact, and the
//! PMT resolving is a per-*program* one that `NewProgram` already carries
//! (see `SessionEvent::Established`'s "Why not called `TracksResolved`").

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{IngestSession, ProgramId, SessionEvent};
use media_plane::trunk::RetentionClass;

use transmux::pipeline::TrackSpec;
use transmux::{DemuxEvent, StreamingTsDemux};

/// Translates [`transmux::DemuxEvent`]s into [`SessionEvent`]s and tracks
/// which [`ProgramId`] owns each `track_id` — a plain, byte-free state
/// machine (no socket, no demuxer), so the B5 mid-stream-`NewProgram`
/// behaviour is unit-testable by constructing [`DemuxEvent`]s directly (via
/// their own `#[non_exhaustive]` constructors), without needing a hand-built
/// MPTS byte stream.
pub(crate) struct ProgramTracker {
    pending: VecDeque<SessionEvent>,
    /// [`DemuxEvent::TrackAdded`] specs collected before the first
    /// [`DemuxEvent::TracksResolved`] — becomes `ProgramId(0)`'s track set.
    resolving: Vec<TrackSpec>,
    resolved_once: bool,
    track_program: HashMap<u32, ProgramId>,
    next_program_id: u32,
}

impl ProgramTracker {
    /// Starts with [`SessionEvent::Established`] already queued — see the
    /// module doc.
    pub(crate) fn new() -> Self {
        ProgramTracker {
            pending: VecDeque::from(vec![SessionEvent::Established]),
            resolving: Vec::new(),
            resolved_once: false,
            track_program: HashMap::new(),
            next_program_id: 0,
        }
    }

    pub(crate) fn handle(&mut self, event: DemuxEvent) {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                if self.resolved_once {
                    // B5: a track declared only after the initial program
                    // resolved — see the module doc.
                    let program = ProgramId(self.next_program_id);
                    self.next_program_id += 1;
                    self.track_program.insert(spec.track_id, program);
                    self.pending.push_back(SessionEvent::NewProgram {
                        program,
                        tracks: vec![spec],
                    });
                } else {
                    self.resolving.push(spec);
                }
            }
            DemuxEvent::TracksResolved { .. } => {
                if !self.resolved_once && !self.resolving.is_empty() {
                    self.resolved_once = true;
                    let program = ProgramId(self.next_program_id);
                    self.next_program_id += 1;
                    let tracks = std::mem::take(&mut self.resolving);
                    for spec in &tracks {
                        self.track_program.insert(spec.track_id, program);
                    }
                    self.pending
                        .push_back(SessionEvent::NewProgram { program, tracks });
                }
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(&program) = self.track_program.get(&track_id) {
                    self.pending.push_back(SessionEvent::Sample {
                        program,
                        track_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
                // A sample for a track never announced (or since removed)
                // is dropped — mirrors the pre-5a `known_track_ids` check.
            }
            DemuxEvent::TrackRemoved { track_id, .. } => {
                // A mid-stream PMT version bump dropped a previously-live
                // PID (issue #774): stop routing samples for it. No
                // `SessionEvent` for this yet — `SessionEvent` has no
                // `TrackRemoved`/`ProgramEnded` variant (`#[non_exhaustive]`,
                // deliberately not added speculatively; see its own doc).
                self.track_program.remove(&track_id);
            }
            DemuxEvent::TrackUpdated(_) | DemuxEvent::TrackAbandoned { .. } => {
                // Metadata-only / pre-resolution events; nothing routes on
                // them yet (mirrors the pre-5a tracing-only handling).
            }
            _ => {}
        }
    }

    pub(crate) fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }
}

/// The shared MPEG-2 TS [`IngestSession`]: no socket, no I/O — just a
/// [`StreamingTsDemux`] plus a [`ProgramTracker`]. Each transport module
/// (`ts_udp`/`ts_http`/`srt`) owns the real socket/stream and feeds this.
pub struct TsIngestSession {
    demux: StreamingTsDemux,
    tracker: ProgramTracker,
}

impl TsIngestSession {
    /// Construct a fresh session — performs no I/O.
    pub fn new() -> Self {
        TsIngestSession {
            demux: StreamingTsDemux::new(),
            tracker: ProgramTracker::new(),
        }
    }
}

impl Default for TsIngestSession {
    fn default() -> Self {
        Self::new()
    }
}

impl Stage for TsIngestSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    /// TS demuxing itself cannot fail (`StreamingTsDemux::feed` is
    /// infallible — a corrupt packet is resynced past, not surfaced as an
    /// error). Every failure mode of a TS ingest route (a dead socket, a
    /// read stall, an HTTP status) lives at the I/O layer in the transport
    /// module, outside this sans-IO session entirely.
    type Error = Infallible;

    fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
        self.demux.feed(input);
        while let Some(event) = self.demux.poll_event() {
            self.tracker.handle(event);
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.tracker.poll()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(crate::source::MAX_TS_READ)
    }
}

/// Nothing to send back on any of the three TS transports — takes the
/// default `poll_transmit`.
impl IngestSession for TsIngestSession {}

/// Shared test fixtures for the three TS transports — a real `TsMux`-muxed
/// byte stream plus the `TrunkConfig`/`HandshakePolicy` each transport's
/// loopback test needs. Lives here (rather than being copy-pasted into
/// `ts_udp`/`ts_http`/`srt`'s own `mod tests`) for the same
/// one-copy reason the session itself does.
#[cfg(test)]
pub(crate) mod test_support {
    use media_plane::ingress::HandshakePolicy;
    use media_plane::trunk::TrunkConfig;

    use broadcast_common::Timestamp;
    use std::num::NonZeroUsize;
    use transmux::TsMux;
    use transmux::media::Track;
    use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    pub(crate) fn trunk_config() -> TrunkConfig {
        TrunkConfig::new(nz(64), nz(16), nz(8), nz(8), nz(8))
    }

    /// A handshake deadline far enough out that it never fires — these
    /// transports' `Established` is queued at construction.
    pub(crate) fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
    }

    pub(crate) fn track_spec(track_id: u32) -> TrackSpec {
        let avc = transmux::avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==")
            .expect("valid sprop");
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Avc {
                config: avc,
                width: 0,
                height: 0,
            },
        )
    }

    /// Builds a real (not hand-faked) single-track muxed TS byte stream via
    /// the workspace's own `transmux::TsMux` packager — the "real fixture,
    /// not inline bytes" discipline this project requires, since a
    /// hand-built TS risks missing real PSI/PES framing quirks.
    pub(crate) fn build_ts_bytes(track_id: u32, nal_byte: u8, count: u32) -> Vec<u8> {
        use broadcast_common::Package;
        let spec = track_spec(track_id);
        let frame_dur = 90_000 / 30;
        let samples: Vec<Sample> = (0..count)
            .map(|i| {
                let nal = [0x65u8, nal_byte, (i % 256) as u8];
                let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                data.extend_from_slice(&nal);
                Sample::new(
                    data,
                    Some(i64::from(i) * i64::from(frame_dur)),
                    Some(i64::from(i) * i64::from(frame_dur)),
                    Some(frame_dur),
                    i == 0,
                )
            })
            .collect();
        let track = Track::new(spec, samples);
        let media = transmux::media::Media::new(vec![track], 90_000);
        TsMux::default().package(&media).expect("mux to TS")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::track_spec;
    use super::*;
    use transmux::pipeline::Sample;

    fn sample_at(nal: u8) -> Sample {
        Sample::new(vec![0x65, nal], Some(0), Some(0), Some(3000), true)
    }

    #[test]
    fn first_tracks_resolved_mints_program_zero() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::TrackAdded(track_spec(1)));
        tracker.handle(DemuxEvent::tracks_resolved(0));
        assert!(matches!(tracker.poll(), Some(SessionEvent::Established)));
        match tracker.poll() {
            Some(SessionEvent::NewProgram { program, tracks }) => {
                assert_eq!(program, ProgramId(0));
                assert_eq!(tracks.len(), 1);
            }
            other => panic!("expected NewProgram(0), got {other:?}"),
        }
    }

    /// The B5 property: a `TrackAdded` arriving *after* the initial program
    /// resolved mints a **second** `ProgramId`, not a dropped/logged event —
    /// the exact bug (issue #774's `TrackAdded`-drop) `NewProgram` closes.
    ///
    /// MUTATION-CHECKED: change the `if self.resolved_once` branch's
    /// `ProgramId(self.next_program_id)` to always mint `ProgramId(0)` and
    /// this test's `assert_ne!` fails: both programs compare equal.
    #[test]
    fn late_track_added_mints_a_second_program_not_a_drop() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::TrackAdded(track_spec(1)));
        tracker.handle(DemuxEvent::tracks_resolved(0));
        let _established = tracker.poll();
        let first = match tracker.poll() {
            Some(SessionEvent::NewProgram { program, .. }) => program,
            other => panic!("expected NewProgram, got {other:?}"),
        };

        tracker.handle(DemuxEvent::TrackAdded(track_spec(9)));
        let second = match tracker.poll() {
            Some(SessionEvent::NewProgram { program, tracks }) => {
                assert_eq!(tracks[0].track_id, 9);
                program
            }
            other => panic!("expected a second NewProgram, got {other:?}"),
        };
        assert_ne!(
            first, second,
            "a late-declared track must mint a NEW program, not be folded into the first"
        );

        tracker.handle(DemuxEvent::sample(9, sample_at(0xAA)));
        match tracker.poll() {
            Some(SessionEvent::Sample {
                program, track_id, ..
            }) => {
                assert_eq!(program, second);
                assert_eq!(track_id, 9);
            }
            other => panic!("expected Sample routed to the second program, got {other:?}"),
        }
    }

    /// A `Sample` for a track never announced (or already `TrackRemoved`) is
    /// dropped, not panicked on or misrouted.
    #[test]
    fn sample_for_unannounced_track_is_dropped() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::sample(42, sample_at(0x01)));
        assert!(matches!(tracker.poll(), Some(SessionEvent::Established)));
        assert!(
            tracker.poll().is_none(),
            "no event for an unannounced track's sample"
        );
    }

    /// **The segmenter gap, closed.** Step 5a round 1 reported that no
    /// producer could obtain a second `Trunk` write handle, so the
    /// `LlHlsSegmenter`/`MOVIE_TIMESCALE` path the pre-5a `run_pipeline`
    /// owned was structurally impossible to express: `Trunk::writer()` was
    /// single-take for the *whole* `Trunk`, and `IngestDriver` claims it
    /// per-program to publish samples.
    ///
    /// `Trunk::segment_writer()` (the ring-group split) fixes exactly that,
    /// and this test is the proof — not an assertion in a doc comment. It
    /// drives the **real** chain: a real `TsMux`-muxed TS stream → the real
    /// `TsIngestSession` → a real `IngestDriver` (holding the `TrunkWriter`)
    /// → a real `SampleCursor` → a real `LlHlsSegmenter` → a
    /// `SegmentWriter` **taken from the same `Trunk` the driver is already
    /// writing samples into** → `Trunk::part_bytes`/`last_closed_segment`.
    ///
    /// Both handles are held live simultaneously across the whole loop; if
    /// the split had not landed, `segment_writer()` would return `None` and
    /// the `expect` below would fail — which is what makes this a real
    /// check rather than a restatement.
    ///
    /// This is deliberately a *test*, not production wiring: which component
    /// owns the segmenter (and where `MOVIE_TIMESCALE`/target-duration
    /// per-route policy is configured) is step 5b's call, since it is the
    /// egress side that consumes segments. What this pins down is that the
    /// plane no longer *prevents* it.
    #[test]
    fn a_segmenter_can_hold_a_segment_writer_while_ingest_holds_the_sample_writer() {
        use super::test_support::{build_ts_bytes, handshake, track_spec, trunk_config};
        use broadcast_common::Timestamp;
        use media_plane::ingress::{IngestDriver, ProgramId};
        use media_plane::trunk::{PartEntry, SampleCursorItem, SegmentEntry};
        use std::time::Duration as StdDuration;
        use transmux::ll_hls::LlHlsSegmenter;
        use transmux::segmenter::SegmentMeta;

        /// The CMAF movie timescale the pre-5a `pipeline::run_pipeline`
        /// hardcoded. Still a constant here (this is a test), but it is now
        /// a *parameter of the segmenter component*, which is what makes it
        /// per-route-configurable at all.
        const MOVIE_TIMESCALE: u32 = 90_000;

        let mut driver = IngestDriver::new(
            TsIngestSession::new(),
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );

        // Feed a real muxed TS stream; the driver mints program 0's Trunk
        // and takes its TrunkWriter internally.
        let ts_bytes = build_ts_bytes(1, 0xAB, 90);
        driver.feed(&ts_bytes, Timestamp::ZERO);
        let trunk = driver
            .trunk(ProgramId(0))
            .cloned()
            .expect("program 0 resolved from the muxed TS");

        // The sample/event writer is genuinely already taken — this is what
        // made the segmenter impossible before the ring-group split, and
        // asserting it here is what stops this test from silently degrading
        // into "a Trunk nobody else was writing to".
        assert!(
            trunk.writer().is_none(),
            "the IngestDriver must already hold this Trunk's sample writer"
        );

        // THE POINT: a second, independent write handle on the SAME Trunk
        // the driver is already publishing samples into.
        let segment_writer = trunk
            .segment_writer()
            .expect("the segments+parts ring group has its own single-take writer");
        assert!(
            trunk.segment_writer().is_none(),
            "the segment writer is itself single-take, on its own flag"
        );

        // A segmenter consumes samples off a cursor and produces
        // segments/parts. One cursor, per `Trunk::subscribe`'s
        // single-digit-readers-by-design contract.
        let mut cursor = trunk.subscribe();
        let mut seg =
            LlHlsSegmenter::with_part_target(vec![track_spec(1)], MOVIE_TIMESCALE, 1.0, 250)
                .expect("segmenter builds from the resolved track spec");

        // Drive more media through so the cursor (which starts from *now*)
        // actually observes samples.
        let more = build_ts_bytes(1, 0xCD, 90);
        driver.feed(&more, Timestamp::from_nanos(1));

        let mut pushed = 0usize;
        while let Some(item) = cursor.poll() {
            if let SampleCursorItem::Timed { track_id, sample } = item {
                seg.push(track_id, sample)
                    .expect("segmenter accepts sample");
                pushed += 1;
            }
        }
        assert!(pushed > 0, "the cursor must have observed real samples");
        seg.flush().expect("flush the trailing partial segment");

        let mut parts_published = 0usize;
        for part in seg.take_ready_parts() {
            segment_writer.publish_part(PartEntry::new(
                part.bytes,
                part.segment_seq,
                part.part_index,
                StdDuration::from_secs_f64(part.duration),
                part.independent,
            ));
            parts_published += 1;
        }
        let mut segments_published = 0usize;
        for segment in seg.take_ready_segments() {
            segment_writer.publish_segment(SegmentEntry::new(
                segment.bytes,
                segment.segment_seq,
                StdDuration::from_secs_f64(segment.duration),
                Timestamp::ZERO,
                SegmentMeta {
                    discontinuous: false,
                },
            ));
            segments_published += 1;
        }

        assert!(
            segments_published > 0,
            "the segmenter must have produced at least one closed segment"
        );
        assert!(
            trunk.last_closed_segment().is_some(),
            "a published segment must be visible via Trunk::last_closed_segment"
        );
        assert_eq!(
            trunk.segment_len(),
            segments_published,
            "every published segment must be in the Trunk's segment log"
        );
        if parts_published > 0 {
            assert!(
                trunk.part_len() > 0,
                "published parts must be visible in the Trunk's live-part log"
            );
        }
    }

    /// A `TrackRemoved` (mid-stream PMT version bump, issue #774) stops
    /// routing that track's samples rather than forwarding stale media.
    ///
    /// MUTATION-CHECKED: delete the `self.track_program.remove(&track_id)`
    /// line and this test's final assertion fails — a `Sample` for the
    /// removed track would still be routed.
    #[test]
    fn removed_track_stops_routing_its_samples() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::TrackAdded(track_spec(1)));
        tracker.handle(DemuxEvent::tracks_resolved(0));
        let _established = tracker.poll();
        let _new_program = tracker.poll();

        // Before removal: the sample routes.
        tracker.handle(DemuxEvent::sample(1, sample_at(0xAA)));
        assert!(matches!(tracker.poll(), Some(SessionEvent::Sample { .. })));

        tracker.handle(DemuxEvent::track_removed(1, Default::default()));
        tracker.handle(DemuxEvent::sample(1, sample_at(0xBB)));
        assert!(
            tracker.poll().is_none(),
            "a removed track's samples must no longer be routed"
        );
    }
}
