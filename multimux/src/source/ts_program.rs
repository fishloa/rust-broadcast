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
//! # B5 + issue #781: mid-stream track changes reach the running segmenter
//!
//! Before this port, a PID declared only *after* `connect()`'s PMT wait
//! resolved was logged and silently dropped (issue #774). `ProgramTracker`
//! closed that by minting a new `ProgramId` for each mid-stream
//! `TrackAdded` — the track's samples were no longer dropped, but the new
//! program never got a segmenter (the brief's original defect: the segmenter
//! is built once at program-observation time from a one-shot track-spec
//! snapshot, issue #781).
//!
//! Now a mid-stream `TrackAdded` joins the **same** `ProgramId` as the
//! initial set — `track_program` maps it into the first-resolved program,
//! `track_specs` stores the spec, and the following `TracksResolved` emits a
//! `SessionEvent::TracksChanged` with the complete current track set. The
//! driver's existing `TracksChanged` handling applies it to the running
//! `Trunk` (bumping `track_generation`), and `drive_program_segmenters`
//! detects the generation change to admit the new track into (or rebuild)
//! the segmenter at the next segment boundary.
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

use std::collections::{HashMap, HashSet, VecDeque};
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
///
/// # MPTS support (issue #906)
///
/// Tracks are grouped by `TrackSpec::program_number`: each distinct
/// `program_number` gets its own [`ProgramId`], so an MPTS (multi-programme
/// transport stream) produces N `ProgramId`s for N `program_number`s.
/// Non-TS sources (`program_number: None`) collapse into one `ProgramId`
/// (unchanged from the pre-906 single-program behaviour).
///
/// Mid-stream `TrackAdded` (issue #781) is preserved PER PROGRAM: a track
/// added mid-stream joins the program keyed by its `program_number`.
pub(crate) struct ProgramTracker {
    pending: VecDeque<SessionEvent>,
    /// Tracks buffered before the first `TracksResolved`, grouped by
    /// `program_number`.
    resolving: HashMap<Option<u16>, Vec<TrackSpec>>,
    resolved_once: bool,
    /// Resolved programs keyed by `program_number`.
    programs: HashMap<Option<u16>, PerProgram>,
    /// Reverse map: `track_id → ProgramId` (for `Sample` routing).
    track_program: HashMap<u32, ProgramId>,
    /// Programs whose track set changed since last `TracksResolved`.
    changed: HashSet<ProgramId>,
    next_program_id: u32,
}

struct PerProgram {
    program_id: ProgramId,
    /// Complete current track specs for this program, keyed by `track_id`.
    track_specs: HashMap<u32, TrackSpec>,
}

impl ProgramTracker {
    /// Starts with [`SessionEvent::Established`] already queued — see the
    /// module doc.
    pub(crate) fn new() -> Self {
        ProgramTracker {
            pending: VecDeque::from(vec![SessionEvent::Established]),
            resolving: HashMap::new(),
            resolved_once: false,
            programs: HashMap::new(),
            track_program: HashMap::new(),
            changed: HashSet::new(),
            next_program_id: 0,
        }
    }

    pub(crate) fn handle(&mut self, event: DemuxEvent) {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                let pn = spec.program_number;
                if self.resolved_once {
                    // Issue #781 + #906: a track declared after initial
                    // resolution joins its program_number's program.
                    let program = self.find_or_create_program(pn);
                    self.track_specs_for_program(program)
                        .insert(spec.track_id, spec.clone());
                    self.track_program.insert(spec.track_id, program);
                    self.changed.insert(program);
                } else {
                    self.resolving.entry(pn).or_default().push(spec);
                }
            }
            DemuxEvent::TracksResolved { .. } => {
                if !self.resolved_once {
                    if !self.resolving.is_empty() {
                        self.resolved_once = true;
                        let groups: Vec<_> =
                            std::mem::take(&mut self.resolving).into_iter().collect();
                        for (pn, tracks) in groups {
                            if tracks.is_empty() {
                                continue;
                            }
                            let program = ProgramId(self.next_program_id);
                            self.next_program_id += 1;
                            let mut track_specs = HashMap::new();
                            for spec in &tracks {
                                self.track_program.insert(spec.track_id, program);
                                track_specs.insert(spec.track_id, spec.clone());
                            }
                            self.programs.insert(
                                pn,
                                PerProgram {
                                    program_id: program,
                                    track_specs,
                                },
                            );
                            self.pending
                                .push_back(SessionEvent::NewProgram { program, tracks });
                        }
                    }
                } else {
                    // Mid-stream TracksResolved: only emit TracksChanged for
                    // programs whose track set actually changed.
                    let dirty = std::mem::take(&mut self.changed);
                    for prog in self.programs.values() {
                        if !dirty.contains(&prog.program_id) {
                            continue;
                        }
                        let tracks: Vec<TrackSpec> = prog.track_specs.values().cloned().collect();
                        if !tracks.is_empty() {
                            self.pending.push_back(SessionEvent::TracksChanged {
                                program: prog.program_id,
                                tracks,
                            });
                        }
                    }
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
                // PID (issue #774): stop routing samples for it.
                if let Some(program) = self.track_program.remove(&track_id) {
                    if let Some(prog) = self.program_for_program_id_mut(program) {
                        prog.track_specs.remove(&track_id);
                    }
                    self.changed.insert(program);
                }
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

    /// Find or create the `ProgramId` for a given `program_number`.
    fn find_or_create_program(&mut self, pn: Option<u16>) -> ProgramId {
        if let Some(prog) = self.programs.get(&pn) {
            return prog.program_id;
        }
        let program = ProgramId(self.next_program_id);
        self.next_program_id += 1;
        self.programs.insert(
            pn,
            PerProgram {
                program_id: program,
                track_specs: HashMap::new(),
            },
        );
        // Mid-stream NewProgram — emitted before any TracksChanged so the
        // driver can mint a Trunk.
        self.pending.push_back(SessionEvent::NewProgram {
            program,
            tracks: Vec::new(),
        });
        program
    }

    fn track_specs_for_program(&mut self, program: ProgramId) -> &mut HashMap<u32, TrackSpec> {
        &mut self
            .program_for_program_id_mut(program)
            .expect("program must exist")
            .track_specs
    }

    fn program_for_program_id_mut(&mut self, program: ProgramId) -> Option<&mut PerProgram> {
        self.programs.values_mut().find(|p| p.program_id == program)
    }
}

/// The shared MPEG-2 TS [`IngestSession`]: no socket, no I/O — just a
/// [`StreamingTsDemux`] plus a `ProgramTracker`. Each transport module
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
/// default `poll_transmit`. `Request = Bytes` (media-plane round 3: every
/// `IngestSession` now names its own request type; a byte-stream source
/// always names `Bytes`, whether or not it ever actually sends one).
impl IngestSession for TsIngestSession {
    type Request = bytes::Bytes;
}

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

    /// **Issue #781:** a `TrackAdded` arriving *after* the initial program
    /// resolved goes to the **same** `ProgramId` — the track's samples join
    /// the running segmenter, rather than being siloed in a new, unserved
    /// program (the exact defect the brief describes).
    ///
    /// The `TrackAdded` itself emits nothing; the following `TracksResolved`
    /// emits `TracksChanged` with the complete current set.
    ///
    /// MUTATION-CHECKED: change `self.first_program.expect(...)` in the
    /// mid-stream `TrackAdded` arm back to minting a new `ProgramId` (the
    /// pre-781 behaviour) and this test's `assert_eq!(program, first)` fails:
    /// the sample routes to a new program, not the first one.
    #[test]
    fn late_track_added_joins_the_same_program_not_a_new_one() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::TrackAdded(track_spec(1)));
        tracker.handle(DemuxEvent::tracks_resolved(0));
        let _established = tracker.poll();
        let first = match tracker.poll() {
            Some(SessionEvent::NewProgram { program, .. }) => program,
            other => panic!("expected NewProgram, got {other:?}"),
        };

        // Mid-stream TrackAdded: emitted only when TracksResolved follows.
        tracker.handle(DemuxEvent::TrackAdded(track_spec(9)));
        assert!(
            tracker.poll().is_none(),
            "TrackAdded alone must not emit anything — TracksChanged waits for TracksResolved"
        );

        // TracksResolved after the addition: emits TracksChanged.
        tracker.handle(DemuxEvent::tracks_resolved(1));
        match tracker.poll() {
            Some(SessionEvent::TracksChanged { program, tracks }) => {
                assert_eq!(program, first, "TracksChanged must go to the same program");
                assert_eq!(tracks.len(), 2, "complete current set: both tracks");
                let ids: Vec<u32> = tracks.iter().map(|t| t.track_id).collect();
                assert!(ids.contains(&1));
                assert!(ids.contains(&9));
            }
            other => panic!("expected TracksChanged, got {other:?}"),
        }

        // Sample for the new track routes to the same program.
        tracker.handle(DemuxEvent::sample(9, sample_at(0xAA)));
        match tracker.poll() {
            Some(SessionEvent::Sample {
                program, track_id, ..
            }) => {
                assert_eq!(program, first);
                assert_eq!(track_id, 9);
            }
            other => panic!("expected Sample routed to the first program, got {other:?}"),
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

    /// A `TrackRemoved` mid-stream stops routing samples for that track and
    /// removes its spec from the complete set — the following `TracksResolved`
    /// emits `TracksChanged` without the removed track.
    #[test]
    fn removed_track_vanishes_from_tracks_changed() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::TrackAdded(track_spec(1)));
        tracker.handle(DemuxEvent::TrackAdded(track_spec(2)));
        tracker.handle(DemuxEvent::tracks_resolved(0));
        let _established = tracker.poll();
        let _new_program = tracker.poll();

        // Remove track 2.
        tracker.handle(DemuxEvent::track_removed(
            2,
            transmux::EventProvenance::default(),
        ));
        // TracksResolved after the removal.
        tracker.handle(DemuxEvent::tracks_resolved(1));
        match tracker.poll() {
            Some(SessionEvent::TracksChanged { tracks, .. }) => {
                assert_eq!(tracks.len(), 1);
                assert_eq!(tracks[0].track_id, 1);
            }
            other => panic!("expected TracksChanged with track 1 only, got {other:?}"),
        }

        // Sample for the removed track is dropped.
        tracker.handle(DemuxEvent::sample(2, sample_at(0xBB)));
        assert!(
            tracker.poll().is_none(),
            "sample for removed track must be dropped"
        );
    }

    /// **The segmenter gap, closed.** Step 5a round 1 reported that no
    /// producer could obtain a second `Trunk` write handle, so the
    /// `LlHlsSegmenter`/movie-timescale path the pre-5a `run_pipeline`
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
    /// This is deliberately a *test*, not production wiring: production
    /// wiring is `crate::source::segment::ProgramSegmenter`/
    /// `drive_program_segmenters` (issue #805 task 2b), which every
    /// driver-backed `run_*` entry point calls; it uses
    /// `transmux::VIDEO_CLOCK_RATE` for the same movie timescale this test
    /// hardcodes, and takes `target_duration_secs`/`part_target_ms` off the
    /// route exactly as this test's literal `1.0, 250` stand in for here.
    #[test]
    fn a_segmenter_can_hold_a_segment_writer_while_ingest_holds_the_sample_writer() {
        use super::test_support::{build_ts_bytes, handshake, track_spec, trunk_config};
        use broadcast_common::Timestamp;
        use media_plane::ingress::{IngestDriver, ProgramId};
        use media_plane::trunk::{PartEntry, SampleCursorItem, SegmentEntry};
        use std::time::Duration as StdDuration;
        use transmux::ll_hls::LlHlsSegmenter;
        use transmux::segmenter::SegmentMeta;

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
        let mut seg = LlHlsSegmenter::with_part_target(
            vec![track_spec(1)],
            transmux::VIDEO_CLOCK_RATE,
            1.0,
            250,
        )
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

    // --- MPTS tests (issue #906) ---

    /// Two distinct `program_number`s produce two distinct `ProgramId`s.
    #[test]
    fn mpts_two_programmes_produce_two_program_ids() {
        let mut tracker = ProgramTracker::new();
        // Program 1: track 1
        let mut spec1 = track_spec(1);
        spec1.program_number = Some(100);
        tracker.handle(DemuxEvent::TrackAdded(spec1));
        // Program 2: track 2
        let mut spec2 = track_spec(2);
        spec2.program_number = Some(200);
        tracker.handle(DemuxEvent::TrackAdded(spec2));
        // Resolve
        tracker.handle(DemuxEvent::tracks_resolved(0));
        let _established = tracker.poll();
        let mut new_programs = Vec::new();
        while let Some(event) = tracker.poll() {
            if let SessionEvent::NewProgram { program, tracks } = event {
                new_programs.push((program, tracks.len()));
            }
        }
        assert_eq!(new_programs.len(), 2, "MPTS: two distinct programmes");
        assert_ne!(new_programs[0].0, new_programs[1].0, "distinct ProgramIds");
        assert_eq!(new_programs[0].1, 1, "each programme has one track");
        assert_eq!(new_programs[1].1, 1, "each programme has one track");
    }

    /// A mid-stream `TrackAdded` with a specific `program_number` joins
    /// that programme's `TracksChanged`, not a new one.
    #[test]
    fn mpts_mid_stream_track_joins_correct_programme() {
        let mut tracker = ProgramTracker::new();
        // Program 100: track 1
        let mut spec1 = track_spec(1);
        spec1.program_number = Some(100);
        tracker.handle(DemuxEvent::TrackAdded(spec1));
        // Program 200: track 2
        let mut spec2 = track_spec(2);
        spec2.program_number = Some(200);
        tracker.handle(DemuxEvent::TrackAdded(spec2));
        tracker.handle(DemuxEvent::tracks_resolved(0));
        let _established = tracker.poll();
        let mut programs: HashMap<u16, ProgramId> = HashMap::new();
        while let Some(event) = tracker.poll() {
            if let SessionEvent::NewProgram { program, tracks } = event {
                if let Some(t) = tracks.first() {
                    if let Some(pn) = t.program_number {
                        programs.insert(pn, program);
                    }
                }
            }
        }

        // Mid-stream: add track 3 to programme 100
        let mut spec3 = track_spec(3);
        spec3.program_number = Some(100);
        tracker.handle(DemuxEvent::TrackAdded(spec3));
        tracker.handle(DemuxEvent::tracks_resolved(1));

        // TracksChanged for programme 100 must include both tracks;
        // programme 200 must NOT get a spurious TracksChanged.
        let prog100 = programs[&100];
        let prog200 = programs[&200];
        let mut saw_tracks_changed_100 = false;
        while let Some(event) = tracker.poll() {
            if let SessionEvent::TracksChanged { program, tracks } = event {
                assert_ne!(
                    program, prog200,
                    "programme 200 must NOT get a spurious TracksChanged"
                );
                if program == prog100 {
                    assert_eq!(tracks.len(), 2, "both tracks in programme 100");
                    let ids: Vec<u32> = tracks.iter().map(|t| t.track_id).collect();
                    assert!(ids.contains(&1));
                    assert!(ids.contains(&3));
                    saw_tracks_changed_100 = true;
                }
            }
        }
        assert!(
            saw_tracks_changed_100,
            "must see TracksChanged for programme 100"
        );

        // Sample for track 3 routes to programme 100
        tracker.handle(DemuxEvent::sample(3, sample_at(0xCC)));
        let sample_event = tracker.poll();
        match sample_event {
            Some(SessionEvent::Sample {
                program, track_id, ..
            }) => {
                assert_eq!(program, prog100);
                assert_eq!(track_id, 3);
            }
            other => panic!("expected Sample routed to programme 100, got {other:?}"),
        }
    }

    /// SPTS (`program_number: None`) works as before — all tracks
    /// collapse into one `ProgramId`.
    #[test]
    fn spts_none_programme_works_as_before() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::TrackAdded(track_spec(1)));
        tracker.handle(DemuxEvent::TrackAdded(track_spec(2)));
        tracker.handle(DemuxEvent::tracks_resolved(0));
        let _established = tracker.poll();
        match tracker.poll() {
            Some(SessionEvent::NewProgram { program, tracks }) => {
                assert_eq!(program, ProgramId(0));
                assert_eq!(tracks.len(), 2);
            }
            other => panic!("expected single NewProgram with 2 tracks, got {other:?}"),
        }

        // Mid-stream addition joins the same None programme
        let mut spec3 = track_spec(3);
        spec3.program_number = None;
        tracker.handle(DemuxEvent::TrackAdded(spec3));
        tracker.handle(DemuxEvent::tracks_resolved(1));
        match tracker.poll() {
            Some(SessionEvent::TracksChanged { program, tracks }) => {
                assert_eq!(program, ProgramId(0));
                assert_eq!(tracks.len(), 3, "all 3 tracks in TracksChanged");
            }
            other => panic!("expected TracksChanged for SPTS, got {other:?}"),
        }
    }
}
