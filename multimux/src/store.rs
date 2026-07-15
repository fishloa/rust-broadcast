//! Per-stream in-RAM rolling window of LL-HLS init/segments/parts, with a
//! `tokio::sync::watch` that signals new data for blocking playlist reloads.

use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::watch;
use transmux::hls::{LowLatencyConfig, MediaPlaylist, MediaSegment};
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

    /// Close a full segment into the window (evicting the oldest), clearing the
    /// in-progress parts belonging to it.
    pub fn add_segment(&self, seg: SegmentInfo) {
        let mut g = self.inner.lock().unwrap();
        g.live_parts.retain(|p| p.segment_seq > seg.segment_seq);
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

    /// A part's bytes by (segment seq, part index) — parts live only while their
    /// segment is in progress, so only `live_parts` is checked (a closed
    /// segment's parts are no longer individually addressable, only the whole
    /// segment is).
    pub fn part_bytes(&self, seq: u32, part_index: u32) -> Option<Vec<u8>> {
        let g = self.inner.lock().unwrap();
        g.live_parts
            .iter()
            .find(|p| p.segment_seq == seq && p.part_index == part_index)
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
    /// `transmux::hls::MediaPlaylist::to_m3u8` renders one `#EXTINF`+URI per
    /// `MediaSegment` unconditionally (parts, when present, are rendered
    /// *before* that segment's `#EXTINF`, never as a substitute for it), so it
    /// has no representation for an EXTINF-less trailing partial segment.
    /// Rather than push a fabricated `MediaSegment` (which is what produced
    /// the RFC violation this fixes), we render the CLOSED segments only via
    /// `to_m3u8()`, then append the in-progress segment's `#EXT-X-PART` lines
    /// and an `#EXT-X-PRELOAD-HINT` for the next, not-yet-available part as
    /// raw strings.
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
        // part's preload-hint URI, computed before we know the closed-segment
        // playlist string so both can be appended after it.
        let open_seq = g.live_parts.first().map(|p| p.segment_seq);
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
            endlist: false,
            extra_tags: vec![format!("#EXT-X-MAP:URI=\"init-{track_id}.mp4\"")],
            low_latency: Some(LowLatencyConfig {
                part_target,
                part_hold_back: part_target * PART_HOLD_BACK_MULTIPLIER,
                // The preload hint is appended manually below, alongside the
                // open segment's part lines, so `to_m3u8()` itself must not
                // also render one.
                preload_hint_part: None,
            }),
            iframes_only: false,
        };
        let mut m3u8 = playlist.to_m3u8();
        if let Some(seq) = open_seq {
            for p in g.live_parts.iter().filter(|p| p.segment_seq == seq) {
                m3u8.push_str(&format!(
                    "#EXT-X-PART:DURATION={},URI=\"part-{track_id}-{}.{}.m4s\"",
                    format_secs(p.duration),
                    p.segment_seq,
                    p.part_index,
                ));
                if p.independent {
                    m3u8.push_str(",INDEPENDENT=YES");
                }
                m3u8.push('\n');
            }
        }
        if let Some(uri) = next_part_hint {
            m3u8.push_str(&format!("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"{uri}\"\n"));
        }
        m3u8
    }
}

/// Format a non-negative seconds value with up to three decimal places,
/// trailing zeros trimmed (`0.5`, `1.334`, `6`) — mirrors the private
/// `format_secs` in `transmux::hls` so the manually-appended `#EXT-X-PART`
/// lines for the in-progress segment render identically to the ones
/// `MediaPlaylist::to_m3u8` produces for closed segments (that helper isn't
/// public, so it can't be reused directly).
fn format_secs(v: f64) -> String {
    let millis = (v * 1000.0 + 0.5) as u64;
    let whole = millis / 1000;
    let frac = millis % 1000;
    if frac == 0 {
        return format!("{whole}");
    }
    let mut f = format!("{frac:03}");
    while f.ends_with('0') {
        f.pop();
    }
    format!("{whole}.{f}")
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
