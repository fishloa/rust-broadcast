//! Per-stream, protocol-neutral in-RAM rolling window of the segmenter's
//! init/segments/parts, with a runtime-agnostic change notification
//! ([`event_listener::Event`]) so a blocking-reload wait works under any
//! async runtime — not `tokio::sync::watch`.
//!
//! [`MediaStore`] holds bytes + timing only — no playlist/manifest syntax.
//! Rendering a manifest (LL-HLS `#EXT-M3U8`) is [`super::media_playlist_m3u8`]'s
//! concern, layered on top.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use event_listener::{Event, EventListener};
use transmux::ll_hls::{PartInfo, SegmentInfo};
use transmux::pipeline::TrackSpec;

/// Minimum live-parts cap, regardless of the timing-derived bound — keeps a
/// usable window even for pathologically small `target_duration_secs`/
/// `part_target_ms` configs.
const MIN_MAX_LIVE_PARTS: usize = 8;

/// Safety margin (in parts) added on top of one nominal segment's worth of
/// parts, absorbing jitter in part sizes/timing without needing an exact fit.
const MAX_LIVE_PARTS_SAFETY_MARGIN: usize = 4;

/// Bound on `Inner::live_parts` derived from segment timing: roughly one
/// nominal segment's worth of parts (`target_duration_secs / part_target`),
/// plus a small safety margin, floored at [`MIN_MAX_LIVE_PARTS`].
///
/// Without a cap, a segment that never closes (GOP much longer than
/// `target_duration_secs`, or a source that stops sending keyframes) would
/// grow `live_parts` unboundedly — `add_segment`'s clear-on-close only trims
/// it when a segment *does* close. This bound keeps RAM use flat regardless;
/// an LL-HLS playlist need only advertise the most recent parts of the open
/// segment (RFC 8216bis has no requirement to retain every part ever
/// produced for an in-progress segment).
pub(crate) fn compute_max_live_parts(target_duration_secs: f64, part_target_ms: u32) -> usize {
    let part_target_secs = f64::from(part_target_ms) / 1000.0;
    let nominal_parts = if part_target_secs > 0.0 {
        (target_duration_secs / part_target_secs).ceil() as usize
    } else {
        0
    };
    (nominal_parts + MAX_LIVE_PARTS_SAFETY_MARGIN).max(MIN_MAX_LIVE_PARTS)
}

struct Inner {
    init: Option<Vec<u8>>,
    segments: VecDeque<SegmentInfo>,
    live_parts: Vec<PartInfo>,
    /// Parts of *just-closed* segments, kept briefly (bounded, oldest-evicted)
    /// after `add_segment` moves them out of `live_parts`. They are no longer
    /// rendered in the playlist (the segment is advertised as a whole `seg-…`),
    /// but stay **fetchable** so an in-flight LL-HLS preload-hint request for a
    /// segment's *final* part still resolves: the segmenter emits that final
    /// part and closes the segment in the same pipeline step, so without this
    /// the part is evicted microseconds after it appears — before the blocked
    /// part request can wake — and every segment boundary 404s its hinted part.
    recent_parts: VecDeque<PartInfo>,
    window_segments: usize,
    /// Current ingest health, set by the feeding pipeline's supervisor (e.g.
    /// `multimux::origin::supervisor`) and read by adapters/metrics.
    health: HealthState,
    /// The largest `SegmentInfo.duration` ever seen by `add_segment`, over
    /// the whole lifetime of this store (never reset when the window slides
    /// or a segment is evicted). RFC 8216bis §4.4.3.1 requires
    /// `#EXT-X-TARGETDURATION` to be at least the rounded duration of every
    /// Media Segment ever advertised — since a real segment can exceed the
    /// *configured* target duration (the segmenter cuts on the next keyframe
    /// after the target, not exactly at it), an all-time max is the only way
    /// to guarantee the MUST holds for every segment this store has ever
    /// produced, including ones already evicted from the window.
    max_segment_duration: f64,
    /// The track specs the feeding pipeline built its segmenter from (issue
    /// #663 P4) — set once via [`MediaStore::set_track_specs`], before the
    /// first sample lands. Protocol-neutral (any `Output` can read it), but
    /// only [`crate`]'s DASH sibling in `multimux::output::dash` actually
    /// needs it today: a DASH `Representation` must advertise a real RFC 6381
    /// `codecs` string, which the LL-HLS playlist never needs.
    track_specs: Vec<TrackSpec>,
}

/// Ingest health of the pipeline feeding a [`MediaStore`], set by the
/// caller's supervisor loop (e.g. `multimux::origin::supervisor::supervise`)
/// as it connects/reconnects the source.
///
/// `Failed` is reserved for an unrecoverable connect error class; a
/// well-behaved supervisor never gives up on a route (sources like cameras
/// come back), so in practice it cycles `Connecting` -> `Live` <->
/// `Reconnecting`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    /// Never yet connected; the supervisor's first connect attempt is in
    /// flight.
    Connecting,
    /// Connected and actively receiving media.
    Live,
    /// Lost the source (connect failure, pipeline error, or source EOF) and
    /// the supervisor is retrying with backoff.
    Reconnecting,
    /// Unrecoverable — the supervisor has given up on this route.
    Failed,
}

impl HealthState {
    /// The spec/field-enum label (workspace #204 convention): a stable,
    /// lowercase token per state, suitable for logs/metrics.
    pub fn name(&self) -> &'static str {
        match self {
            HealthState::Connecting => "connecting",
            HealthState::Live => "live",
            HealthState::Reconnecting => "reconnecting",
            HealthState::Failed => "failed",
        }
    }
}

impl std::fmt::Display for HealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// One closed segment's identity/timing — a protocol-neutral snapshot entry
/// returned by [`MediaStore::window_segments`] (issue #663 P4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentWindowEntry {
    /// This segment's sequence number — matches the `seg-{track}-{seq}.m4s`
    /// filename [`MediaStore::resolve_resource`](super::MediaStore::resolve_resource)
    /// serves.
    pub segment_seq: u32,
    /// This segment's actual duration, in seconds.
    pub duration_secs: f64,
}

/// In-RAM rolling window for one served stream: bytes + timing, shared by
/// every adapter serving that stream.
pub struct MediaStore {
    inner: Mutex<Inner>,
    target_duration_secs: f64,
    part_target_ms: u32,
    max_live_parts: usize,
    /// Monotonic version bumped by every mutation (`add_*`/`set_init`/
    /// `set_health` that actually changes health) — never reset, only ever
    /// grows (wrapping on overflow, which at any realistic mutation rate
    /// never happens within a process lifetime).
    progress_version: AtomicU64,
    /// Runtime-agnostic wakeup: `listen()` registers a listener *before*
    /// returning (so a `notify` racing the caller's next check is never
    /// missed — the standard `event-listener` idiom), and every mutation
    /// calls `notify(usize::MAX)` to wake every parked waiter.
    progress_event: Event,
    /// Wall-clock time this store was constructed — used as the live
    /// presentation's `availabilityStartTime` anchor (issue #663 P4; see
    /// [`Self::created_at`]). Not bumped/mutated after construction, so it
    /// needs no lock.
    created_at: SystemTime,
}

impl MediaStore {
    /// New empty store; `window_segments` = full segments retained.
    pub fn new(target_duration_secs: f64, part_target_ms: u32, window_segments: usize) -> Self {
        MediaStore {
            inner: Mutex::new(Inner {
                init: None,
                segments: VecDeque::new(),
                live_parts: Vec::new(),
                recent_parts: VecDeque::new(),
                window_segments,
                health: HealthState::Connecting,
                max_segment_duration: 0.0,
                track_specs: Vec::new(),
            }),
            target_duration_secs,
            part_target_ms,
            max_live_parts: compute_max_live_parts(target_duration_secs, part_target_ms),
            progress_version: AtomicU64::new(0),
            progress_event: Event::new(),
            created_at: SystemTime::now(),
        }
    }

    fn bump(&self) {
        self.progress_version.fetch_add(1, Ordering::SeqCst);
        self.progress_event.notify(usize::MAX);
    }

    /// Store the fMP4 init segment.
    pub fn set_init(&self, bytes: Vec<u8>) {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).init = Some(bytes);
        self.bump();
    }

    /// Append a completed part to the in-progress segment.
    ///
    /// Caps `live_parts` at `max_live_parts` worth of entries, dropping the
    /// *oldest* live part(s) first if the cap is exceeded — this bounds RAM
    /// use even if the current segment never closes (see
    /// `compute_max_live_parts`).
    pub fn add_part(&self, part: PartInfo) {
        let mut g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.live_parts.push(part);
        while g.live_parts.len() > self.max_live_parts {
            g.live_parts.remove(0);
        }
        drop(g);
        self.bump();
    }

    /// Count of currently-retained live parts (test accessor for the
    /// `live_parts` cap).
    #[cfg(test)]
    pub(crate) fn live_part_count(&self) -> usize {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).live_parts.len()
    }

    /// Close a full segment into the window (evicting the oldest). Its
    /// in-progress parts move out of `live_parts` into a bounded `recent_parts`
    /// buffer — still fetchable (so an in-flight preload-hint request for the
    /// segment's final part resolves) but no longer rendered as open parts.
    /// `recent_parts` is capped like `live_parts`, oldest-first.
    pub fn add_segment(&self, seg: SegmentInfo) {
        let mut g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let seq = seg.segment_seq;
        g.max_segment_duration = g.max_segment_duration.max(seg.duration);
        let (closed, still_live): (Vec<PartInfo>, Vec<PartInfo>) =
            core::mem::take(&mut g.live_parts)
                .into_iter()
                .partition(|p| p.segment_seq <= seq);
        g.live_parts = still_live;
        for p in closed {
            g.recent_parts.push_back(p);
        }
        while g.recent_parts.len() > self.max_live_parts {
            g.recent_parts.pop_front();
        }
        g.segments.push_back(seg);
        while g.segments.len() > g.window_segments {
            g.segments.pop_front();
        }
        drop(g);
        self.bump();
    }

    /// The fMP4 init segment bytes, if present — the one accessor kept
    /// public beyond `add_*`/`set_*`/`health`/`listen`, since callers (e.g.
    /// `multimux`'s pipeline/supervisor tests) commonly need to assert media
    /// has actually landed without going through `resolve_resource`.
    pub fn init_bytes(&self) -> Option<Vec<u8>> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).init.clone()
    }

    /// A full segment's bytes by sequence number.
    pub(crate) fn segment_bytes(&self, seq: u32) -> Option<Vec<u8>> {
        let g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.segments
            .iter()
            .find(|s| s.segment_seq == seq)
            .map(|s| s.bytes.clone())
    }

    /// A part's bytes by (segment seq, part index). Checks the in-progress
    /// segment's `live_parts` first, then the just-closed `recent_parts` — the
    /// latter so an LL-HLS client's in-flight preload-hint request for a
    /// segment's final part still resolves after `add_segment` closed it. Parts
    /// older than the `recent_parts` bound are no longer individually
    /// addressable (only the whole segment is).
    pub(crate) fn part_bytes(&self, seq: u32, part_index: u32) -> Option<Vec<u8>> {
        let g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let matches = |p: &&PartInfo| p.segment_seq == seq && p.part_index == part_index;
        g.live_parts
            .iter()
            .find(matches)
            .or_else(|| g.recent_parts.iter().find(matches))
            .map(|p| p.bytes.clone())
    }

    /// `(in-progress segment seq, count of live parts available for it)` —
    /// used to resolve blocking `_HLS_msn`/`_HLS_part` requests.
    ///
    /// The second value is a **count**, not the last part's index: the
    /// blocking-reload resolver treats "part `P` ready" as `count > P` (0
    /// means no parts of the in-progress segment are available yet).
    pub(crate) fn latest_progress(&self) -> (u32, u32) {
        let g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let last_closed_seg = g.segments.back().map(|s| s.segment_seq).unwrap_or(0);
        let in_progress_seg = g
            .live_parts
            .last()
            .map(|p| p.segment_seq)
            .unwrap_or(last_closed_seg);
        let part_count = g
            .live_parts
            .iter()
            .filter(|p| p.segment_seq == in_progress_seg)
            .count() as u32;
        (in_progress_seg, part_count)
    }

    /// The current monotonic progress version — bumped by every mutation.
    /// Mostly useful for a caller wanting to detect "did anything change"
    /// without registering a listener (e.g. a cheap pre-check).
    pub fn progress_version(&self) -> u64 {
        self.progress_version.load(Ordering::SeqCst)
    }

    /// Register for the next change notification. **Register before
    /// re-checking the condition you're waiting on** (see this module's
    /// `super`-level doc for the wait-loop shape) — `event-listener`
    /// guarantees any `notify` call that happens after `listen()` returns
    /// will wake this listener, so there is no missed-wakeup race as long as
    /// the re-check happens after `listen()`, not before.
    ///
    /// The returned [`EventListener`] is a plain `Future<Output = ()>` — any
    /// async runtime (or none, via its blocking `.wait()`) can drive it; this
    /// is what keeps `server` runtime-agnostic (unlike a
    /// `tokio::sync::watch::Receiver`, which only ever paired with tokio).
    pub fn listen(&self) -> EventListener {
        self.progress_event.listen()
    }

    /// Set the route's ingest health. Bumps the progress notification
    /// **only when the state actually changes**, so a caller blocked on
    /// [`Self::listen`] (e.g. an LL-HLS blocking playlist reload) wakes on a
    /// health transition too, not just new media.
    pub fn set_health(&self, state: HealthState) {
        let mut g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if g.health != state {
            g.health = state;
            drop(g);
            self.bump();
        }
    }

    /// The current ingest health (default [`HealthState::Connecting`] until
    /// the supervisor sets it).
    pub fn health(&self) -> HealthState {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).health
    }

    /// The full-segment target duration, in seconds, this store was built
    /// with — timing configuration a manifest renderer needs (e.g. LL-HLS's
    /// `#EXT-X-TARGETDURATION`, or a DASH `Output`'s `minimumUpdatePeriod`/
    /// `timeShiftBufferDepth`). `pub` (not `pub(crate)`) since issue #663 P4:
    /// `multimux::output::dash` needs it too, not just this crate's own
    /// `engine`.
    pub fn target_duration_secs(&self) -> f64 {
        self.target_duration_secs
    }

    /// The part target duration, in milliseconds, this store was built with.
    /// `pub` for the same cross-`Output` reason as
    /// [`Self::target_duration_secs`].
    pub fn part_target_ms(&self) -> u32 {
        self.part_target_ms
    }

    /// Wall-clock time this store was constructed (issue #663 P4) — used as
    /// a live DASH presentation's `availabilityStartTime` anchor. An
    /// approximation (the *route*'s start time, not the first segment's
    /// exact cut time — the first segment typically closes
    /// `target_duration_secs` or so later), acceptable for a manifest
    /// attribute that only needs to establish a consistent, monotonic
    /// timeline, not wall-clock precision.
    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    /// Store the track specs the feeding pipeline built its segmenter from —
    /// called once, before the first sample is pushed. See
    /// `Inner::track_specs` for why this exists (DASH's `codecs` string
    /// needs real codec identity; LL-HLS never reads this).
    pub fn set_track_specs(&self, specs: Vec<TrackSpec>) {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).track_specs = specs;
    }

    /// The track specs set by [`Self::set_track_specs`], empty if never
    /// called (e.g. in a test that only exercises playlist rendering).
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).track_specs.clone()
    }

    /// Snapshot of the closed segments currently retained in the rolling
    /// window, oldest first — enough for a manifest renderer to enumerate
    /// fetchable segments (issue #663 P4: DASH's `SegmentTemplate`/
    /// `$Number$` addressing) without depending on the LL-HLS-specific
    /// playlist rendering in the playlist renderer.
    pub fn window_segments(&self) -> Vec<SegmentWindowEntry> {
        self.inner
            .lock()
            .unwrap()
            .segments
            .iter()
            .map(|s| SegmentWindowEntry {
                segment_seq: s.segment_seq,
                duration_secs: s.duration,
            })
            .collect()
    }

    /// The largest `SegmentInfo.duration` ever seen by [`Self::add_segment`],
    /// `0.0` if no segment has closed yet. The playlist renderer combines
    /// this with [`Self::target_duration_secs`] to compute a
    /// spec-conformant `#EXT-X-TARGETDURATION` (RFC 8216bis §4.4.3.1).
    pub(crate) fn max_segment_duration(&self) -> f64 {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).max_segment_duration
    }

    /// The sequence number of the most-recently-closed segment, `0` if none
    /// has closed yet. Unlike [`Self::latest_progress`]'s first element
    /// (which reflects the *in-progress* segment once any of its parts have
    /// landed), this is specifically the last **closed** segment — used to
    /// implement RFC 8216bis §6.2.5.2's bare-`_HLS_msn` blocking-reload
    /// semantics, which must wait for segment `msn` to be a fully-present
    /// Media Segment, not merely an in-progress one with live parts.
    pub(crate) fn last_closed_segment_seq(&self) -> u32 {
        self.inner
            .lock()
            .unwrap()
            .segments
            .back()
            .map(|s| s.segment_seq)
            .unwrap_or(0)
    }

    /// Run `f` against a consistent snapshot of the closed `segments` and the
    /// in-progress segment's `live_parts`, taken under a single lock
    /// acquisition — used by the playlist renderer, which needs both
    /// collections together.
    pub(crate) fn with_segments_and_parts<R>(
        &self,
        f: impl FnOnce(&VecDeque<SegmentInfo>, &[PartInfo]) -> R,
    ) -> R {
        let g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&g.segments, &g.live_parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use event_listener::Listener;

    fn seg(seq: u32, parts: u32) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![seq as u8; 8],
            duration: 4.0,
            segment_seq: seq,
            part_count: parts,
        }
    }
    fn part(seq: u32, idx: u32) -> PartInfo {
        PartInfo {
            bytes: vec![idx as u8; 4],
            duration: 0.5,
            independent: idx == 0,
            segment_seq: seq,
            part_index: idx,
        }
    }

    #[test]
    fn window_evicts_oldest_and_serves_bytes() {
        let s = MediaStore::new(4.0, 500, 2);
        s.set_init(vec![0xAA; 10]);
        s.add_segment(seg(1, 8));
        s.add_segment(seg(2, 8));
        s.add_segment(seg(3, 8)); // evicts seq 1
        assert!(s.segment_bytes(1).is_none(), "seq 1 evicted");
        assert!(s.segment_bytes(2).is_some());
        assert!(s.segment_bytes(3).is_some());
        assert_eq!(s.init_bytes().unwrap(), vec![0xAA; 10]);
    }

    #[test]
    fn recent_parts_bounded_across_many_closes() {
        // Closing many segments must not grow recent_parts unboundedly.
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        let cap = compute_max_live_parts(4.0, 500);
        for seq in 1..=20u32 {
            for idx in 0..4u32 {
                s.add_part(part(seq, idx));
            }
            s.add_segment(seg(seq, 4));
        }
        // Only the most recent ~cap parts remain individually fetchable; a very
        // old one has been evicted from recent_parts.
        assert!(s.part_bytes(1, 0).is_none(), "old closed part evicted");
        assert!(
            s.part_bytes(20, 3).is_some(),
            "most-recent closed part retained (within the {cap}-part bound)"
        );
    }

    #[test]
    fn progress_version_bumps_on_new_data() {
        let s = MediaStore::new(4.0, 500, 4);
        let before = s.progress_version();
        s.add_part(part(1, 0));
        assert_ne!(s.progress_version(), before, "progress version changed");
    }

    #[test]
    fn listen_wakes_on_new_data() {
        // Proves `listen()` actually observes a `notify()` triggered by a
        // mutation — not just that the version counter moves (that's
        // `progress_version_bumps_on_new_data`). Registers first (the
        // documented no-missed-wakeup ordering), then mutates, then blocks on
        // the listener with `EventListener::wait` (bounded so a broken wakeup
        // fails the test instead of hanging it).
        let s = MediaStore::new(4.0, 500, 4);
        let listener = s.listen();
        s.add_part(part(1, 0));
        assert!(
            listener
                .wait_deadline(std::time::Instant::now() + std::time::Duration::from_secs(2))
                .is_some(),
            "listener must wake within 2s of add_part"
        );
    }

    #[test]
    fn health_defaults_to_connecting() {
        let s = MediaStore::new(4.0, 500, 4);
        assert_eq!(s.health(), HealthState::Connecting);
    }

    #[test]
    fn set_health_updates_and_bumps_progress_only_on_change() {
        let s = MediaStore::new(4.0, 500, 4);
        let before = s.progress_version();

        // No-op: setting the same state again must not bump.
        s.set_health(HealthState::Connecting);
        assert_eq!(
            s.progress_version(),
            before,
            "unchanged state does not bump progress"
        );

        s.set_health(HealthState::Live);
        assert_eq!(s.health(), HealthState::Live);
        assert_ne!(
            s.progress_version(),
            before,
            "state change bumps progress so blocked readers wake"
        );

        let mid = s.progress_version();
        s.set_health(HealthState::Reconnecting);
        assert_eq!(s.health(), HealthState::Reconnecting);
        assert_ne!(s.progress_version(), mid);
    }

    #[test]
    fn max_segment_duration_tracks_lifetime_max_not_just_current_window() {
        let s = MediaStore::new(4.0, 500, 2);
        s.set_init(vec![0; 4]);
        assert_eq!(s.max_segment_duration(), 0.0, "nothing closed yet");

        let mut over = seg(1, 8);
        over.duration = 4.0;
        s.add_segment(over);
        assert_eq!(s.max_segment_duration(), 4.0);

        // A real segment that overshoots the configured target (the
        // segmenter cuts on the next keyframe after the target, so this is
        // routine, not pathological).
        let mut over = seg(2, 8);
        over.duration = 7.5;
        s.add_segment(over);
        assert_eq!(s.max_segment_duration(), 7.5);

        // Window slides (window_segments=2 evicts seq 1 and seq 2 eventually)
        // but the lifetime max must NOT reset/shrink back down.
        let mut small = seg(3, 8);
        small.duration = 3.0;
        s.add_segment(small); // evicts seq 1 from the window
        let mut small2 = seg(4, 8);
        small2.duration = 3.0;
        s.add_segment(small2); // evicts seq 2 from the window
        assert!(
            s.segment_bytes(2).is_none(),
            "seq 2 (the 7.5s segment) evicted from the window"
        );
        assert_eq!(
            s.max_segment_duration(),
            7.5,
            "lifetime max must survive window eviction"
        );
    }

    #[test]
    fn last_closed_segment_seq_tracks_the_newest_close() {
        let s = MediaStore::new(4.0, 500, 4);
        assert_eq!(s.last_closed_segment_seq(), 0, "nothing closed yet");
        s.add_segment(seg(1, 8));
        assert_eq!(s.last_closed_segment_seq(), 1);
        s.add_segment(seg(2, 8));
        assert_eq!(s.last_closed_segment_seq(), 2);
    }

    #[test]
    fn health_state_name_and_display_agree() {
        for (state, label) in [
            (HealthState::Connecting, "connecting"),
            (HealthState::Live, "live"),
            (HealthState::Reconnecting, "reconnecting"),
            (HealthState::Failed, "failed"),
        ] {
            assert_eq!(state.name(), label);
            assert_eq!(state.to_string(), label);
        }
    }

    // --- issue #663 P4: DASH-facing accessors ---

    #[test]
    fn track_specs_round_trip_and_default_empty() {
        use transmux::pipeline::CodecConfig;

        let s = MediaStore::new(4.0, 500, 4);
        assert!(
            s.track_specs().is_empty(),
            "no specs set yet -> empty, not a panic/placeholder"
        );

        let spec = TrackSpec::new(
            1,
            90_000,
            CodecConfig::Vp8 {
                width: 0,
                height: 0,
            },
        );
        s.set_track_specs(vec![spec.clone()]);
        let got = s.track_specs();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].track_id, spec.track_id);
        assert_eq!(got[0].timescale, spec.timescale);
    }

    #[test]
    fn window_segments_reflects_closed_segments_oldest_first_and_evicts() {
        let s = MediaStore::new(4.0, 500, 2);
        assert!(s.window_segments().is_empty(), "nothing closed yet");

        s.add_segment(seg(1, 4));
        s.add_segment(seg(2, 4));
        let window = s.window_segments();
        assert_eq!(
            window.iter().map(|e| e.segment_seq).collect::<Vec<_>>(),
            vec![1, 2],
            "oldest first"
        );
        assert_eq!(window[0].duration_secs, 4.0);

        s.add_segment(seg(3, 4)); // evicts seq 1 (window_segments == 2)
        assert_eq!(
            s.window_segments()
                .iter()
                .map(|e| e.segment_seq)
                .collect::<Vec<_>>(),
            vec![2, 3],
            "eviction reflected in the snapshot"
        );
    }

    #[test]
    fn created_at_is_set_at_construction() {
        let before = SystemTime::now();
        let s = MediaStore::new(4.0, 500, 4);
        let after = SystemTime::now();
        assert!(s.created_at() >= before && s.created_at() <= after);
    }
}
