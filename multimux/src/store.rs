//! Per-stream, protocol-neutral in-RAM rolling window of the segmenter's
//! init/segments/parts, with a `tokio::sync::watch` that signals new data for
//! blocking playlist/manifest reloads.
//!
//! [`MediaStore`] holds bytes + timing only — no playlist/manifest syntax.
//! Rendering a manifest (LL-HLS `#EXT-M3U8`, DASH `MPD`, …) is an
//! [`crate::output::Output`] concern layered on top; see
//! [`crate::output::llhls`] for the LL-HLS renderer that used to live here.

use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::watch;
use transmux::ll_hls::{PartInfo, SegmentInfo};

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
    /// Current ingest health, set by the route's supervisor (see
    /// `crate::origin::supervisor`) and read by outputs/metrics.
    health: HealthState,
}

/// Ingest health of the route feeding a [`MediaStore`], set by the
/// supervisor loop (`crate::origin::supervisor::supervise`) as it
/// connects/reconnects the source.
///
/// `Failed` is reserved for an unrecoverable connect error class; the
/// default supervisor never gives up on a route (sources like cameras come
/// back), so in practice it cycles `Connecting` -> `Live` <-> `Reconnecting`.
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

/// In-RAM rolling window for one served stream: bytes + timing, shared by
/// every [`crate::output::Output`] serving that stream (LL-HLS, DASH, …).
pub struct MediaStore {
    inner: Mutex<Inner>,
    target_duration_secs: f64,
    part_target_ms: u32,
    max_live_parts: usize,
    progress_tx: watch::Sender<u64>,
}

impl MediaStore {
    /// New empty store; `window_segments` = full segments retained.
    pub fn new(target_duration_secs: f64, part_target_ms: u32, window_segments: usize) -> Self {
        // `send_modify` works even with zero receivers (it permits sending
        // values with no listeners), so the initial receiver half doesn't
        // need to be held anywhere — `subscribe()` mints new receivers off
        // the sender's retained value on demand.
        let (tx, _rx) = watch::channel(0u64);
        MediaStore {
            inner: Mutex::new(Inner {
                init: None,
                segments: VecDeque::new(),
                live_parts: Vec::new(),
                recent_parts: VecDeque::new(),
                window_segments,
                health: HealthState::Connecting,
            }),
            target_duration_secs,
            part_target_ms,
            max_live_parts: compute_max_live_parts(target_duration_secs, part_target_ms),
            progress_tx: tx,
        }
    }

    fn bump(&self) {
        self.progress_tx.send_modify(|v| *v = v.wrapping_add(1));
    }

    /// Store the fMP4 init segment.
    pub fn set_init(&self, bytes: Vec<u8>) {
        self.inner.lock().unwrap().init = Some(bytes);
        self.bump();
    }

    /// Append a completed part to the in-progress segment.
    ///
    /// Caps `live_parts` at `max_live_parts` worth of entries, dropping the
    /// *oldest* live part(s) first if the cap is exceeded — this bounds RAM
    /// use even if the current segment never closes (see
    /// `compute_max_live_parts`).
    pub fn add_part(&self, part: PartInfo) {
        let mut g = self.inner.lock().unwrap();
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
    pub fn live_part_count(&self) -> usize {
        self.inner.lock().unwrap().live_parts.len()
    }

    /// Close a full segment into the window (evicting the oldest). Its
    /// in-progress parts move out of `live_parts` into a bounded `recent_parts`
    /// buffer — still fetchable (so an in-flight preload-hint request for the
    /// segment's final part resolves) but no longer rendered as open parts.
    /// `recent_parts` is capped like `live_parts`, oldest-first.
    pub fn add_segment(&self, seg: SegmentInfo) {
        let mut g = self.inner.lock().unwrap();
        let seq = seg.segment_seq;
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

    /// The fMP4 init segment bytes, if present.
    pub fn init_bytes(&self) -> Option<Vec<u8>> {
        self.inner.lock().unwrap().init.clone()
    }

    /// A full segment's bytes by sequence number.
    pub fn segment_bytes(&self, seq: u32) -> Option<Vec<u8>> {
        let g = self.inner.lock().unwrap();
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
    pub fn part_bytes(&self, seq: u32, part_index: u32) -> Option<Vec<u8>> {
        let g = self.inner.lock().unwrap();
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
    /// The second value is a **count**, not the last part's index: a future
    /// blocking-reload resolver should treat "part `P` ready" as
    /// `count > P` (0 means no parts of the in-progress segment are
    /// available yet).
    pub fn latest_progress(&self) -> (u32, u32) {
        let g = self.inner.lock().unwrap();
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

    /// Subscribe to the progress watch (value bumps on every new part/segment).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.progress_tx.subscribe()
    }

    /// Set the route's ingest health. Bumps the progress watch **only when
    /// the state actually changes**, so a client/output blocked on
    /// `subscribe()` (e.g. an LL-HLS blocking playlist reload) wakes on a
    /// health transition too, not just new media.
    pub fn set_health(&self, state: HealthState) {
        let mut g = self.inner.lock().unwrap();
        if g.health != state {
            g.health = state;
            drop(g);
            self.bump();
        }
    }

    /// The current ingest health (default [`HealthState::Connecting`] until
    /// the supervisor sets it).
    pub fn health(&self) -> HealthState {
        self.inner.lock().unwrap().health
    }

    /// The full-segment target duration, in seconds, this store was built
    /// with — timing configuration an [`crate::output::Output`] renderer
    /// needs (e.g. LL-HLS's `#EXT-X-TARGETDURATION`).
    pub(crate) fn target_duration_secs(&self) -> f64 {
        self.target_duration_secs
    }

    /// The part target duration, in milliseconds, this store was built with.
    pub(crate) fn part_target_ms(&self) -> u32 {
        self.part_target_ms
    }

    /// Run `f` against a consistent snapshot of the closed `segments` and the
    /// in-progress segment's `live_parts`, taken under a single lock
    /// acquisition — used by output renderers (e.g. LL-HLS's media playlist
    /// renderer) that need both collections together.
    pub(crate) fn with_segments_and_parts<R>(
        &self,
        f: impl FnOnce(&VecDeque<SegmentInfo>, &[PartInfo]) -> R,
    ) -> R {
        let g = self.inner.lock().unwrap();
        f(&g.segments, &g.live_parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn watch_bumps_on_new_data() {
        let s = MediaStore::new(4.0, 500, 4);
        let mut rx = s.subscribe();
        let before = *rx.borrow_and_update();
        s.add_part(part(1, 0));
        assert_ne!(*rx.borrow(), before, "watch value changed");
    }

    #[test]
    fn health_defaults_to_connecting() {
        let s = MediaStore::new(4.0, 500, 4);
        assert_eq!(s.health(), HealthState::Connecting);
    }

    #[test]
    fn set_health_updates_and_bumps_watch_only_on_change() {
        let s = MediaStore::new(4.0, 500, 4);
        let mut rx = s.subscribe();
        let before = *rx.borrow_and_update();

        // No-op: setting the same state again must not bump.
        s.set_health(HealthState::Connecting);
        assert_eq!(*rx.borrow(), before, "unchanged state does not bump watch");

        s.set_health(HealthState::Live);
        assert_eq!(s.health(), HealthState::Live);
        assert_ne!(
            *rx.borrow_and_update(),
            before,
            "state change bumps watch so blocked readers wake"
        );

        let mid = *rx.borrow();
        s.set_health(HealthState::Reconnecting);
        assert_eq!(s.health(), HealthState::Reconnecting);
        assert_ne!(*rx.borrow(), mid);
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
}
