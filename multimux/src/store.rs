//! Per-stream in-RAM rolling window of LL-HLS init/segments/parts, with a
//! `tokio::sync::watch` that signals new data for blocking playlist reloads.

use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::watch;
use transmux::hls::{LowLatencyConfig, MediaPlaylist, MediaSegment, OpenSegment, PartSpec};
use transmux::ll_hls::{PartInfo, SegmentInfo};

/// LL-HLS requires HLS protocol version 9 (RFC 8216bis §4.4.3.7/§4.4.3.8: the
/// `#EXT-X-PART-INF`/`#EXT-X-PART` directives this store always emits require
/// it).
const LL_HLS_VERSION: u8 = 9;

/// RFC 8216bis / Apple LL-HLS §4.4.3.7: `#EXT-X-SERVER-CONTROL`'s
/// `PART-HOLD-BACK` attribute MUST be at least 3x the part target duration
/// (`#EXT-X-PART-INF`'s `PART-TARGET`).
const PART_HOLD_BACK_MULTIPLIER: f64 = 3.0;

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
fn compute_max_live_parts(target_duration_secs: f64, part_target_ms: u32) -> usize {
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
}

/// In-RAM rolling window for one served LL-HLS stream.
pub struct StreamStore {
    inner: Mutex<Inner>,
    target_duration_secs: f64,
    part_target_ms: u32,
    max_live_parts: usize,
    progress_tx: watch::Sender<u64>,
}

impl StreamStore {
    /// New empty store; `window_segments` = full segments retained.
    pub fn new(target_duration_secs: f64, part_target_ms: u32, window_segments: usize) -> Self {
        // `send_modify` works even with zero receivers (it permits sending
        // values with no listeners), so the initial receiver half doesn't
        // need to be held anywhere — `subscribe()` mints new receivers off
        // the sender's retained value on demand.
        let (tx, _rx) = watch::channel(0u64);
        StreamStore {
            inner: Mutex::new(Inner {
                init: None,
                segments: VecDeque::new(),
                live_parts: Vec::new(),
                recent_parts: VecDeque::new(),
                window_segments,
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

    /// Render the LL-HLS media playlist for `track_id`.
    ///
    /// RFC 8216bis §4.4.4.9: an in-progress (not yet closed) segment MUST NOT
    /// be advertised with an `#EXTINF`/URI pair — that segment has no fetchable
    /// resource yet — it may only appear as trailing `#EXT-X-PART` lines.
    /// `transmux::hls::MediaPlaylist::open_segment` is exactly this
    /// representation: its parts render as trailing `#EXT-X-PART` lines with
    /// no `#EXTINF`/URI, so the in-progress segment's parts and the
    /// `#EXT-X-PRELOAD-HINT` for the next, not-yet-available part are both
    /// rendered by `to_m3u8()` itself — multimux only supplies the URI scheme
    /// (`part-<track>-<seq>.<idx>.m4s`) and the part metadata.
    pub fn media_playlist_m3u8(&self, track_id: u32) -> String {
        let g = self.inner.lock().unwrap();
        let media_sequence = g
            .segments
            .front()
            .map(|s| u64::from(s.segment_seq))
            .or_else(|| g.live_parts.first().map(|p| u64::from(p.segment_seq)))
            .unwrap_or(1);
        let segments: Vec<MediaSegment> = g
            .segments
            .iter()
            .map(|s| MediaSegment {
                uri: format!("seg-{track_id}-{}.m4s", s.segment_seq),
                duration: s.duration,
                discontinuous: false,
                parts: Vec::new(),
            })
            .collect();
        let part_target = f64::from(self.part_target_ms) / 1000.0;
        // The in-progress segment's live parts + the next (not yet available)
        // part's preload-hint URI.
        let open_seq = g.live_parts.first().map(|p| p.segment_seq);
        let open_segment = open_seq.map(|seq| {
            OpenSegment::new(
                g.live_parts
                    .iter()
                    .filter(|p| p.segment_seq == seq)
                    .map(|p| PartSpec {
                        uri: format!("part-{track_id}-{}.{}.m4s", p.segment_seq, p.part_index),
                        duration: p.duration,
                        independent: p.independent,
                    })
                    .collect(),
            )
        });
        let next_part_hint = open_seq.map(|seq| {
            let next_idx = g
                .live_parts
                .iter()
                .filter(|p| p.segment_seq == seq)
                .map(|p| p.part_index)
                .max()
                .map(|idx| idx + 1)
                .unwrap_or(0);
            format!("part-{track_id}-{seq}.{next_idx}.m4s")
        });
        let playlist = MediaPlaylist {
            version: LL_HLS_VERSION,
            target_duration: self.target_duration_secs.ceil() as u32,
            media_sequence,
            discontinuity_sequence: 0,
            segments,
            open_segment,
            endlist: false,
            extra_tags: vec![format!("#EXT-X-MAP:URI=\"init-{track_id}.mp4\"")],
            low_latency: Some(LowLatencyConfig {
                part_target,
                part_hold_back: part_target * PART_HOLD_BACK_MULTIPLIER,
                preload_hint_part: next_part_hint,
            }),
            iframes_only: false,
        };
        playlist.to_m3u8()
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
        let s = StreamStore::new(4.0, 500, 2);
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
    fn playlist_has_llhls_tags_and_parts() {
        let s = StreamStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_part(part(1, 0));
        s.add_part(part(1, 1));
        let m = s.media_playlist_m3u8(1);
        assert!(m.contains("#EXT-X-PART-INF"), "PART-INF present");
        assert!(
            m.contains("#EXT-X-SERVER-CONTROL"),
            "SERVER-CONTROL present"
        );
        assert!(m.contains("#EXT-X-PART"), "at least one PART");
        assert!(
            m.contains("part-1-1.0.m4s") || m.contains("part-1-1.1.m4s"),
            "part URI"
        );
    }

    #[test]
    fn open_segment_has_parts_but_no_extinf() {
        let s = StreamStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_part(part(1, 0));
        s.add_part(part(1, 1));
        let m = s.media_playlist_m3u8(1);
        // The in-progress segment's parts are advertised...
        assert!(m.contains("#EXT-X-PART"), "at least one PART line");
        assert!(m.contains("part-1-1.0.m4s"), "part 0 URI present");
        assert!(m.contains("part-1-1.1.m4s"), "part 1 URI present");
        // ...but RFC 8216bis §4.4.4.9: no premature #EXTINF/URI for the
        // not-yet-closed segment itself — "seg-1-1.m4s" must not appear
        // anywhere (it isn't fetchable; that segment hasn't been closed).
        assert!(
            !m.contains("seg-1-1.m4s"),
            "no full-segment URI for the open segment: {m}"
        );
        assert!(
            !m.contains("#EXTINF"),
            "no EXTINF for the open segment: {m}"
        );
    }

    #[test]
    fn final_part_fetchable_after_its_segment_closes() {
        // The segmenter emits a segment's final part and then closes the
        // segment in the same step. A preload-hint request for that final part
        // is typically in flight when the close happens, so it must remain
        // fetchable afterwards (from recent_parts) rather than 404 — the LL-HLS
        // preload-hint boundary bug.
        let s = StreamStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_part(part(1, 0));
        s.add_part(part(1, 1)); // .1 is this segment's final part
        s.add_segment(seg(1, 2)); // close segment 1 (moves its parts to recent_parts)
        assert_eq!(
            s.part_bytes(1, 1),
            Some(vec![1; 4]),
            "final part of a just-closed segment must still be individually fetchable"
        );
        assert_eq!(s.part_bytes(1, 0), Some(vec![0; 4]), "earlier parts too");
        // A genuinely-nonexistent part of the closed segment is still absent.
        assert_eq!(s.part_bytes(1, 9), None);
        // Closing does not resurrect parts into the rendered open segment: the
        // playlist advertises the whole segment, not its parts.
        let m = s.media_playlist_m3u8(1);
        assert!(
            m.contains("seg-1-1.m4s"),
            "closed segment rendered whole: {m}"
        );
        assert!(
            !m.contains("part-1-1."),
            "closed parts not rendered as open: {m}"
        );
    }

    #[test]
    fn recent_parts_bounded_across_many_closes() {
        // Closing many segments must not grow recent_parts unboundedly.
        let s = StreamStore::new(4.0, 500, 4);
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
        let s = StreamStore::new(4.0, 500, 4);
        let mut rx = s.subscribe();
        let before = *rx.borrow_and_update();
        s.add_part(part(1, 0));
        assert_ne!(*rx.borrow(), before, "watch value changed");
    }

    #[test]
    fn live_parts_capped_when_segment_never_closes() {
        // target_duration_secs=4.0, part_target_ms=500 -> cap =
        // ceil(4.0 / 0.5) + 4 margin = 12 (see compute_max_live_parts).
        let s = StreamStore::new(4.0, 500, 4);
        let cap = compute_max_live_parts(4.0, 500);
        assert_eq!(cap, 12, "sanity-check the expected cap for these params");
        s.set_init(vec![0; 4]);

        // Push far more parts than the cap into a single never-closed
        // segment (no add_segment call) — RAM must stay bounded.
        for i in 0..(cap as u32 * 5) {
            s.add_part(part(1, i));
        }
        assert_eq!(
            s.live_part_count(),
            cap,
            "live_parts must stay capped even though the segment never closed"
        );

        // The playlist must still render correctly from the capped parts:
        // only the most recent (highest-index) parts survive.
        let m = s.media_playlist_m3u8(1);
        assert!(m.contains("#EXT-X-PART"), "still has PART lines: {m}");
        let last_idx = cap as u32 * 5 - 1;
        assert!(
            m.contains(&format!("part-1-1.{last_idx}.m4s")),
            "most recent part must survive the cap: {m}"
        );
        let first_idx = cap as u32 * 5 - cap as u32;
        assert!(
            !m.contains(&format!("part-1-1.{}.m4s", first_idx - 1)),
            "an older part beyond the cap must have been dropped: {m}"
        );
    }
}
