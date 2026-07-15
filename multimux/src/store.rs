//! Per-stream in-RAM rolling window of LL-HLS init/segments/parts, with a
//! `tokio::sync::watch` that signals new data for blocking playlist reloads.

use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::watch;
use transmux::hls::{LowLatencyConfig, MediaPlaylist, MediaSegment, PartSpec};
use transmux::ll_hls::{PartInfo, SegmentInfo};

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
    progress_tx: watch::Sender<u64>,
    _progress_rx: watch::Receiver<u64>,
}

impl StreamStore {
    /// New empty store; `window_segments` = full segments retained.
    pub fn new(target_duration_secs: f64, part_target_ms: u32, window_segments: usize) -> Self {
        let (tx, rx) = watch::channel(0u64);
        StreamStore {
            inner: Mutex::new(Inner {
                init: None,
                segments: VecDeque::new(),
                live_parts: Vec::new(),
                window_segments,
            }),
            target_duration_secs,
            part_target_ms,
            progress_tx: tx,
            _progress_rx: rx,
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
    pub fn add_part(&self, part: PartInfo) {
        self.inner.lock().unwrap().live_parts.push(part);
        self.bump();
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

    /// The highest (segment seq, part index) currently available — used to
    /// resolve blocking `_HLS_msn`/`_HLS_part` requests.
    pub fn latest_progress(&self) -> (u32, u32) {
        let g = self.inner.lock().unwrap();
        let last_seg = g.segments.back().map(|s| s.segment_seq).unwrap_or(0);
        let last_part = g.live_parts.last().map(|p| p.part_index).unwrap_or(0);
        (
            g.live_parts
                .last()
                .map(|p| p.segment_seq)
                .unwrap_or(last_seg),
            last_part,
        )
    }

    /// Subscribe to the progress watch (value bumps on every new part/segment).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.progress_tx.subscribe()
    }

    /// Render the LL-HLS media playlist for `track_id`.
    pub fn media_playlist_m3u8(&self, track_id: u32) -> String {
        let g = self.inner.lock().unwrap();
        let media_sequence = g
            .segments
            .front()
            .map(|s| u64::from(s.segment_seq))
            .unwrap_or(1);
        let mut segments: Vec<MediaSegment> = g
            .segments
            .iter()
            .map(|s| MediaSegment {
                uri: format!("seg-{track_id}-{}.m4s", s.segment_seq),
                duration: s.duration,
                discontinuous: false,
                parts: Vec::new(),
            })
            .collect();
        // Attach the in-progress segment's live parts as a trailing (open) segment.
        if let Some(first) = g.live_parts.first() {
            let seq = first.segment_seq;
            let parts = g
                .live_parts
                .iter()
                .filter(|p| p.segment_seq == seq)
                .map(|p| PartSpec {
                    uri: format!("part-{track_id}-{}.{}.m4s", p.segment_seq, p.part_index),
                    duration: p.duration,
                    independent: p.independent,
                })
                .collect();
            segments.push(MediaSegment {
                uri: format!("seg-{track_id}-{seq}.m4s"),
                duration: self.target_duration_secs,
                discontinuous: false,
                parts,
            });
        }
        let part_target = f64::from(self.part_target_ms) / 1000.0;
        let playlist = MediaPlaylist {
            version: 9,
            target_duration: self.target_duration_secs.ceil() as u32,
            media_sequence,
            discontinuity_sequence: 0,
            segments,
            endlist: false,
            extra_tags: vec![format!("#EXT-X-MAP:URI=\"init-{track_id}.mp4\"")],
            low_latency: Some(LowLatencyConfig {
                part_target,
                part_hold_back: part_target * 3.0,
                preload_hint_part: None,
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
    fn watch_bumps_on_new_data() {
        let s = StreamStore::new(4.0, 500, 4);
        let mut rx = s.subscribe();
        let before = *rx.borrow_and_update();
        s.add_part(part(1, 0));
        assert_ne!(*rx.borrow(), before, "watch value changed");
    }
}
