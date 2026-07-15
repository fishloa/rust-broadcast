# multimux — Live RTSP → LL-HLS JIT Origin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native (Linux/macOS) executable — a thin client+server wrap over `rtsp-runtime` + `transmux` — that pulls live RTSP source(s) and serves each as **LL-HLS** from an in-process tokio+axum origin. (Issue #663.)

**Architecture:** One tokio task per configured route pulls RTSP (interleaved RTP over TCP via `rtsp-runtime::AsyncRtspClient`), depayloads to timed `Sample`s (`transmux::RtpStreamDepacketizer`), pushes them into a `transmux::LlHlsSegmenter`, and stores the resulting init/segments/parts in a per-stream in-RAM rolling window (`StreamStore`) that signals new data via a `tokio::sync::watch`. An axum origin reads the stores per request and renders LL-HLS playlists, blocking on the `watch` for `_HLS_msn`/`_HLS_part` reload requests. Muxing only — no transcode.

**Tech Stack:** Rust, std, tokio (multi-thread), axum 0.7, `transmux` 0.17 (path), `rtsp-runtime` 0.2 (path, `tokio` feature), `sdp-types` 0.1, serde/serde_json, clap 4 (behind `cli`).

## Global Constraints

- MSRV **1.86**, edition **2024**; build/test `--locked`. Adding axum/hyper/tower/sdp-types/serde_json re-resolves `Cargo.lock` — after adding deps, run `rustup run 1.86 cargo build -p multimux --locked` and, if a transitive dep pins > 1.86, pin it down (`cargo update -p <dep> --precise <ver>`) until the 1.86 build is green, then commit the lockfile. (See the workspace's MSRV discipline.)
- **New workspace member** `multimux` added to root `Cargo.toml` `[workspace] members`. No `[workspace.dependencies]` table exists — pin deps in `multimux/Cargo.toml` directly. Manifests keep manual column alignment.
- multimux is a **std application crate** (like `dvb-tools`) — NOT `no_std`. tokio/axum are non-optional. The `cli` feature gates only the binary (clap). The workspace `--no-default-features` build must still succeed (it just omits the CLI binary).
- Errors are structured `thiserror` (`MultimuxError`), matching workspace conventions. No `unwrap`/`panic` on I/O or parse paths.
- CLI follows `docs/CLI-STANDARD.md`: clap derive, named flags, auto `--help`/`--version`.
- Independent crate, own version starting at **0.1.0**. RELEASE-DOCS apply at release time (CHANGELOG, release note, README, ≥2 examples) — Task 9.
- v1 scope (from spec): **LL-HLS only**, RTSP-pull only, H.264(+AAC), live-only, RAM window, shared rolling window (no per-viewer sessions), no TLS/auth. DASH/LL-DASH, SRT/TS/file input, DVR = out of scope.

---

## File Structure

- `multimux/Cargo.toml` — new crate manifest.
- `multimux/src/lib.rs` — module wiring + re-exports; crate doc.
- `multimux/src/error.rs` — `MultimuxError` + `Result`.
- `multimux/src/config.rs` — `Config`, `Route`; JSON load + CLI-single-route build.
- `multimux/src/store.rs` — `StreamStore` (RAM rolling window + `watch` + playlist render + byte getters).
- `multimux/src/source/mod.rs` — `Source` trait + `TrackInit`; `MockSource` (test) lives here under `#[cfg(test)]` or a `testsupport` module.
- `multimux/src/source/sdp.rs` — pure SDP → `Vec<TrackInit>` parsing (unit-tested).
- `multimux/src/source/rtsp.rs` — `RtspSource` (drives `AsyncRtspClient`).
- `multimux/src/pipeline.rs` — per-route task: `Source` → `LlHlsSegmenter` → `StreamStore`.
- `multimux/src/origin/mod.rs` — axum `Router` + `AppState`.
- `multimux/src/origin/handlers.rs` — playlist + segment/part/init handlers (blocking reload).
- `multimux/src/bin/multimux.rs` — `cli`-gated binary (clap).
- `multimux/tests/origin_llhls.rs` — deterministic gate via `MockSource`.
- `multimux/examples/serve_mock.rs`, `multimux/examples/serve_rtsp.rs` — Task 9.

---

### Task 1: Crate scaffold + error type

**Files:**
- Create: `multimux/Cargo.toml`, `multimux/src/lib.rs`, `multimux/src/error.rs`
- Modify: `Cargo.toml` (root — add `"multimux"` to `[workspace] members`, keeping alignment)

**Interfaces:**
- Produces: `multimux::error::{MultimuxError, Result}` where `pub type Result<T> = core::result::Result<T, MultimuxError>;`

- [ ] **Step 1: Write the crate manifest**

Create `multimux/Cargo.toml`:

```toml
[package]
name         = "multimux"
version      = "0.1.0"
edition      = "2024"
rust-version = "1.86"
license      = "MIT OR Apache-2.0"
description  = "Live RTSP -> LL-HLS just-in-time repackaging HTTP origin (tokio + axum)."
repository   = "https://github.com/fishloa/rust-broadcast"
keywords     = ["rtsp", "hls", "ll-hls", "cmaf", "origin"]
categories   = ["multimedia", "network-programming"]

[dependencies]
transmux      = { path = "../transmux",     version = "0.17" }
rtsp-runtime  = { path = "../rtsp-runtime",  version = "0.2", features = ["tokio"] }
sdp-types     = "0.1"
tokio         = { version = "1", features = ["rt-multi-thread", "net", "io-util", "macros", "time", "sync"] }
axum          = "0.7"
serde         = { version = "1", features = ["derive"] }
serde_json    = "1"
thiserror     = "2"
clap          = { version = "4", features = ["derive"], optional = true }

[features]
default = ["cli"]
cli     = ["dep:clap"]

[[bin]]
name              = "multimux"
path              = "src/bin/multimux.rs"
required-features = ["cli"]

[dev-dependencies]
```

- [ ] **Step 2: Add to workspace members**

In the root `/Volumes/External/Projects/rust-broadcast/Cargo.toml`, add `"multimux"` to the `[workspace] members` array (preserve the existing formatting/alignment).

- [ ] **Step 3: Write the error type + lib skeleton**

Create `multimux/src/error.rs`:

```rust
//! Error type for multimux.

use thiserror::Error;

/// Errors from configuration, RTSP ingest, segmentation, or the HTTP origin.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MultimuxError {
    /// Config file could not be read or parsed.
    #[error("config: {0}")]
    Config(String),
    /// An RTSP/transport/SDP failure while pulling a source.
    #[error("source: {0}")]
    Source(String),
    /// A `transmux` segmentation/depayload error.
    #[error("transmux: {0}")]
    Transmux(#[from] transmux::Error),
    /// An I/O error (socket, bind).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// multimux result alias.
pub type Result<T> = core::result::Result<T, MultimuxError>;
```

Create `multimux/src/lib.rs`:

```rust
//! multimux — a live RTSP -> LL-HLS just-in-time repackaging HTTP origin.
//!
//! A thin client+server wrap over `rtsp-runtime` (RTSP pull) and `transmux`
//! (RTP depayload + LL-HLS CMAF segmentation): pull one or more live RTSP
//! sources and serve each as LL-HLS from an in-process tokio + axum origin.
//! Muxing only — samples are never transcoded.

pub mod config;
pub mod error;
pub mod origin;
pub mod pipeline;
pub mod source;
pub mod store;

pub use error::{MultimuxError, Result};
```

(The `mod` lines reference files created in later tasks; to keep Task 1 compiling on its own, create empty stub files `config.rs`, `origin/mod.rs`, `pipeline.rs`, `source/mod.rs`, `store.rs` each containing only a `//!` doc line. Later tasks replace the stubs.)

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p multimux --locked` then `rustup run 1.86 cargo build -p multimux --locked`
Expected: both PASS. If the 1.86 build fails on a transitive dep, pin it down and commit `Cargo.lock`.

- [ ] **Step 5: Commit**

```bash
git add multimux/Cargo.toml multimux/src/lib.rs multimux/src/error.rs Cargo.toml Cargo.lock
git commit -m "feat(multimux): crate scaffold + error type (#663)"
```

---

### Task 2: Config (CLI + optional JSON, routes model)

**Files:**
- Create/replace: `multimux/src/config.rs`
- Test: in-module `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub struct Route { pub name: String, pub rtsp_url: String }`
  - `pub struct Config { pub bind: String, pub target_duration_secs: f64, pub part_target_ms: u32, pub window_segments: usize, pub routes: Vec<Route> }`
  - `pub fn Config::from_json_file(path: &std::path::Path) -> Result<Config>`
  - `pub fn Config::validate(&self) -> Result<()>`
  - `Default for Config` (bind `0.0.0.0:8080`, target 4.0s, part 500ms, window 8, no routes)

- [ ] **Step 1: Write the failing test**

Replace `multimux/src/config.rs` with the doc + test:

```rust
//! multimux configuration: routes + segmentation/window/bind parameters.
//!
//! CLI-first with an optional JSON config file. A route maps one RTSP input
//! URL to a served stream name.

use crate::error::{MultimuxError, Result};
use serde::Deserialize;
use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_config_with_routes() {
        let json = r#"{
            "bind": "127.0.0.1:9000",
            "target_duration_secs": 2.0,
            "part_target_ms": 250,
            "window_segments": 6,
            "routes": [
                { "name": "cam1", "rtsp_url": "rtsp://host/stream1" },
                { "name": "cam2", "rtsp_url": "rtsp://host/stream2" }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.bind, "127.0.0.1:9000");
        assert_eq!(cfg.part_target_ms, 250);
        assert_eq!(cfg.routes.len(), 2);
        assert_eq!(cfg.routes[1].name, "cam2");
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_duplicate_stream_names() {
        let cfg = Config {
            routes: vec![
                Route { name: "x".into(), rtsp_url: "rtsp://a".into() },
                Route { name: "x".into(), rtsp_url: "rtsp://b".into() },
            ],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_no_routes() {
        assert!(Config::default().validate().is_err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p multimux --lib config`
Expected: FAIL — `Config`/`Route` not found.

- [ ] **Step 3: Implement config**

Add to `config.rs`:

```rust
/// One input→output route: an RTSP source URL served under `name`.
#[derive(Debug, Clone, Deserialize)]
pub struct Route {
    /// Served stream name (URL path segment).
    pub name: String,
    /// RTSP source URL to pull.
    pub rtsp_url: String,
}

/// multimux runtime configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `host:port` the HTTP origin binds.
    pub bind: String,
    /// Target full-segment duration (seconds).
    pub target_duration_secs: f64,
    /// LL-HLS part target (milliseconds).
    pub part_target_ms: u32,
    /// Rolling window depth (full segments retained in RAM).
    pub window_segments: usize,
    /// Input→output routes.
    pub routes: Vec<Route>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind: "0.0.0.0:8080".to_string(),
            target_duration_secs: 4.0,
            part_target_ms: 500,
            window_segments: 8,
            routes: Vec::new(),
        }
    }
}

impl Config {
    /// Load a JSON config file.
    pub fn from_json_file(path: &Path) -> Result<Config> {
        let bytes = std::fs::read(path)?;
        let cfg: Config = serde_json::from_slice(&bytes)
            .map_err(|e| MultimuxError::Config(format!("{path:?}: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject empty route sets, duplicate stream names, and nonsensical timing.
    pub fn validate(&self) -> Result<()> {
        if self.routes.is_empty() {
            return Err(MultimuxError::Config("no routes configured".into()));
        }
        if self.target_duration_secs <= 0.0 || self.part_target_ms == 0 || self.window_segments == 0
        {
            return Err(MultimuxError::Config("timing/window must be positive".into()));
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.routes {
            if !seen.insert(r.name.as_str()) {
                return Err(MultimuxError::Config(format!("duplicate stream name {:?}", r.name)));
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p multimux --lib config`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add multimux/src/config.rs
git commit -m "feat(multimux): config (routes, JSON + validation) (#663)"
```

---

### Task 3: StreamStore — RAM rolling window + watch + playlist render

**Files:**
- Create/replace: `multimux/src/store.rs`
- Test: in-module `#[cfg(test)]`

**Interfaces:**
- Consumes: `transmux::ll_hls::{PartInfo, SegmentInfo}`, `transmux::hls::{MediaPlaylist, MediaSegment, PartSpec, LowLatencyConfig}`.
- Produces:
  - `pub struct StreamStore` (behind `Arc`, interior-mutable via `std::sync::Mutex` + a `tokio::sync::watch`)
  - `StreamStore::new(target_duration_secs: f64, part_target_ms: u32, window_segments: usize) -> Self`
  - `set_init(&self, bytes: Vec<u8>)`
  - `add_part(&self, part: PartInfo)` / `add_segment(&self, seg: SegmentInfo)` (both bump the watch)
  - `init_bytes(&self) -> Option<Vec<u8>>`
  - `segment_bytes(&self, seq: u32) -> Option<Vec<u8>>`
  - `part_bytes(&self, seq: u32, part_index: u32) -> Option<Vec<u8>>`
  - `media_playlist_m3u8(&self, track_id: u32) -> String`
  - `subscribe(&self) -> tokio::sync::watch::Receiver<u64>` and `latest_progress(&self) -> (u32 /*last seg*/, u32 /*last part*/)`

> **Design:** the store holds `init: Option<Vec<u8>>`, `segments: VecDeque<SegmentInfo>` (capped at `window_segments`, evict front on overflow), and `live_parts: Vec<PartInfo>` (parts of the in-progress segment; cleared when that segment closes). The `watch<u64>` value is a monotonically-incremented counter bumped on every `add_part`/`add_segment`; blocking-reload handlers await a change. Single stream serves one video track (v1); the `track_id` argument selects segment/part URIs. Playlist URIs: `seg-{track}-{seq}.m4s`, `part-{track}-{seq}.{idx}.m4s`, `init-{track}.mp4`.

- [ ] **Step 1: Write the failing test**

Replace `multimux/src/store.rs` with doc + test:

```rust
//! Per-stream in-RAM rolling window of LL-HLS init/segments/parts, with a
//! `tokio::sync::watch` that signals new data for blocking playlist reloads.

use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::watch;
use transmux::hls::{LowLatencyConfig, MediaPlaylist, MediaSegment, PartSpec};
use transmux::ll_hls::{PartInfo, SegmentInfo};

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(seq: u32, parts: u32) -> SegmentInfo {
        SegmentInfo { bytes: vec![seq as u8; 8], duration: 4.0, segment_seq: seq, part_count: parts }
    }
    fn part(seq: u32, idx: u32) -> PartInfo {
        PartInfo { bytes: vec![idx as u8; 4], duration: 0.5, independent: idx == 0, segment_seq: seq, part_index: idx }
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
        assert!(m.contains("#EXT-X-SERVER-CONTROL"), "SERVER-CONTROL present");
        assert!(m.contains("#EXT-X-PART"), "at least one PART");
        assert!(m.contains("part-1-1.0.m4s") || m.contains("part-1-1.1.m4s"), "part URI");
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
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p multimux --lib store`
Expected: FAIL — `StreamStore` not found.

- [ ] **Step 3: Implement StreamStore**

Add to `store.rs`:

```rust
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
        g.segments.iter().find(|s| s.segment_seq == seq).map(|s| s.bytes.clone())
    }

    /// A part's bytes by (segment seq, part index) — searches live parts then
    /// any retained closed segment is not part-addressable (parts live only
    /// while their segment is in progress), so only `live_parts` is checked.
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
        (g.live_parts.last().map(|p| p.segment_seq).unwrap_or(last_seg), last_part)
    }

    /// Subscribe to the progress watch (value bumps on every new part/segment).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.progress_tx.subscribe()
    }

    /// Render the LL-HLS media playlist for `track_id`.
    pub fn media_playlist_m3u8(&self, track_id: u32) -> String {
        let g = self.inner.lock().unwrap();
        let media_sequence = g.segments.front().map(|s| u64::from(s.segment_seq)).unwrap_or(1);
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
```

> **Step 0 (read before coding):** confirm the exact field names/types of `transmux::hls::{MediaPlaylist, MediaSegment, PartSpec, LowLatencyConfig}` and `transmux::ll_hls::{PartInfo, SegmentInfo}` on main (they are pinned in this plan from an API scan, but verify — especially whether `MediaPlaylist` has an `extra_tags` field for the `EXT-X-MAP`, else set the map via whatever field carries it, or prepend it). The tests assert the observable m3u8 substrings, so adapt construction to whatever the real struct requires.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p multimux --lib store`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add multimux/src/store.rs
git commit -m "feat(multimux): StreamStore RAM window + watch + LL-HLS playlist render (#663)"
```

---

### Task 4: SDP → track init (pure parsing)

**Files:**
- Create: `multimux/src/source/sdp.rs`
- Create/replace: `multimux/src/source/mod.rs` (define `TrackInit`, `Source` trait, declare `mod sdp;`)
- Test: in-module `#[cfg(test)]` in `sdp.rs`

**Interfaces:**
- Produces:
  - `pub struct TrackInit { pub track_id: u32, pub kind: transmux::rtp::RtpMediaKind, pub config: transmux::pipeline::CodecConfig, pub clock_rate: u32, pub control: Option<String>, pub channel: u8 }`
  - `pub fn parse_sdp_tracks(sdp: &[u8]) -> Result<Vec<TrackInit>>` — assigns interleaved channels 0/2/4… (RTP; RTCP = +1) in media order; reads `a=rtpmap` (clock rate), `a=fmtp` (`sprop-parameter-sets=`/`config=`), `a=control` per media; builds `CodecConfig` via `transmux::{avc_config_from_sprop, aac_config_from_fmtp}`.
  - `pub trait Source { fn stream_name(&self) -> &str; }` (extended in Task 5 — keep minimal here).

> **Step 0 (read before coding):** open `rtsp-runtime/tests/integration.rs` for the exact `sdp_types::Session` / `MediaDescription` API (`session.medias`, `media.media`, `media.get_first_attribute_value("rtpmap"|"fmtp"|"control")`, and how the media's format/payload-type is exposed). Use the real accessors. For `avc_config_from_sprop`, extract just the `sprop-parameter-sets=<VALUE>` substring from the fmtp string; for AAC extract `config=<HEX>`. `CodecConfig::Avc` needs `width`/`height` (use 0 — unknown from SDP, matching transmux's own placeholder convention) wrapping the `AVCConfigurationBox` from `avc_config_from_sprop`.

- [ ] **Step 1: Write the failing test**

Create `multimux/src/source/sdp.rs`:

```rust
//! Parse an RTSP DESCRIBE SDP body into per-track init (codec config + clock
//! rate + control URL + assigned interleaved channel).

use crate::error::{MultimuxError, Result};
use transmux::rtp::RtpMediaKind;

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal H.264 SDP (one video media) with a real sprop-parameter-sets.
    const SDP: &[u8] = b"v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 0 RTP/AVP 96\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 packetization-mode=1; sprop-parameter-sets=Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==\r\n\
a=control:streamid=0\r\n";

    #[test]
    fn parses_h264_video_track() {
        let tracks = parse_sdp_tracks(SDP).unwrap();
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert!(matches!(t.kind, RtpMediaKind::H264));
        assert_eq!(t.clock_rate, 90_000);
        assert_eq!(t.channel, 0, "first media gets RTP channel 0");
        assert_eq!(t.control.as_deref(), Some("streamid=0"));
        assert_eq!(t.track_id, 1);
    }

    #[test]
    fn rejects_sdp_without_media() {
        let sdp = b"v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";
        assert!(parse_sdp_tracks(sdp).is_err());
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p multimux --lib source::sdp`
Expected: FAIL — `parse_sdp_tracks` / `TrackInit` not found.

- [ ] **Step 3: Implement — `TrackInit` + `Source` in `source/mod.rs`, parser in `sdp.rs`**

Replace `multimux/src/source/mod.rs`:

```rust
//! Ingest sources feeding the segmentation pipeline. v1 ships `RtspSource`;
//! the `Source` trait keeps ingest swappable (and lets tests drive a mock).

pub mod rtsp;
pub mod sdp;

use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;

/// Per-track init derived from the DESCRIBE SDP.
#[derive(Debug, Clone)]
pub struct TrackInit {
    /// 1-based track id used across the segmenter + playlist URIs.
    pub track_id: u32,
    /// Payload kind (H.264 / AAC).
    pub kind: RtpMediaKind,
    /// Codec config built from the SDP fmtp.
    pub config: CodecConfig,
    /// RTP clock rate (Hz) = IR timescale.
    pub clock_rate: u32,
    /// Per-media `a=control` URL suffix for SETUP.
    pub control: Option<String>,
    /// Interleaved RTP channel assigned to this media (RTCP = channel + 1).
    pub channel: u8,
}
```

(Include `sdp.rs`'s parser and — for Task 4 only — a placeholder `rtsp.rs` stub with just a `//!` doc so `pub mod rtsp;` compiles; Task 5 fills it.)

Add to `sdp.rs` the implementation:

```rust
use crate::source::TrackInit;
use sdp_types::Session;
use transmux::{aac_config_from_fmtp, avc_config_from_sprop};
use transmux::pipeline::CodecConfig;

/// Extract `key=<value>` from an fmtp attribute string (`;`/space separated).
fn fmtp_param<'a>(fmtp: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=");
    let idx = fmtp.find(&needle)? + needle.len();
    let rest = &fmtp[idx..];
    let end = rest.find([';', ' ', '\r', '\n']).unwrap_or(rest.len());
    Some(&rest[..end])
}

/// clock rate from `a=rtpmap:<pt> <enc>/<rate>[/<ch>]`.
fn rtpmap_clock_rate(rtpmap: &str) -> Option<u32> {
    let after_slash = rtpmap.split('/').nth(1)?;
    after_slash.split(['/', ' ']).next()?.trim().parse().ok()
}

/// Parse the DESCRIBE SDP body into per-track init, assigning interleaved
/// channels 0,2,4,… in media order.
pub fn parse_sdp_tracks(sdp: &[u8]) -> Result<Vec<TrackInit>> {
    let session =
        Session::parse(sdp).map_err(|e| MultimuxError::Source(format!("sdp parse: {e}")))?;
    let mut tracks = Vec::new();
    let mut track_id = 1u32;
    let mut channel = 0u8;
    for media in &session.medias {
        let fmtp = media.get_first_attribute_value("fmtp").ok().flatten();
        let rtpmap = media.get_first_attribute_value("rtpmap").ok().flatten();
        let control = media
            .get_first_attribute_value("control")
            .ok()
            .flatten()
            .map(|s| s.to_string());
        let clock_rate = rtpmap.and_then(rtpmap_clock_rate).unwrap_or(90_000);

        let (kind, config): (RtpMediaKind, CodecConfig) = match media.media.as_str() {
            "video" => {
                let fmtp = fmtp.ok_or_else(|| MultimuxError::Source("video media missing fmtp".into()))?;
                let sprop = fmtp_param(fmtp, "sprop-parameter-sets")
                    .ok_or_else(|| MultimuxError::Source("no sprop-parameter-sets".into()))?;
                let avc = avc_config_from_sprop(sprop)?;
                (RtpMediaKind::H264, CodecConfig::Avc { config: avc, width: 0, height: 0 })
            }
            "audio" => {
                let fmtp = fmtp.ok_or_else(|| MultimuxError::Source("audio media missing fmtp".into()))?;
                let cfg_hex = fmtp_param(fmtp, "config")
                    .ok_or_else(|| MultimuxError::Source("no AAC config=".into()))?;
                (RtpMediaKind::Aac, aac_config_from_fmtp(cfg_hex)?)
            }
            other => {
                return Err(MultimuxError::Source(format!("unsupported media {other:?}")));
            }
        };
        tracks.push(TrackInit { track_id, kind, config, clock_rate, control, channel });
        track_id += 1;
        channel = channel.saturating_add(2);
    }
    if tracks.is_empty() {
        return Err(MultimuxError::Source("SDP has no supported media".into()));
    }
    Ok(tracks)
}
```

> Verify in Step 0 the exact return type of `get_first_attribute_value` (the code assumes `Result<Option<&str>, _>`; adjust `.ok().flatten()` if it is `Option<&str>` directly). The tests pin observable behavior.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p multimux --lib source::sdp`
Expected: PASS (2 tests). If the committed base64 sprop in the test does not decode to a valid SPS, replace it with a known-good sprop (extract from `transmux/tests/rtp.rs`'s generated SDP or the fixture) — the SPS must be ≥4 bytes with valid profile/level bytes.

- [ ] **Step 5: Commit**

```bash
git add multimux/src/source/mod.rs multimux/src/source/sdp.rs
git commit -m "feat(multimux): SDP -> track init parsing (#663)"
```

---

### Task 5: RtspSource — drive AsyncRtspClient → depayload → Samples

**Files:**
- Create/replace: `multimux/src/source/rtsp.rs`
- Test: in-module `#[cfg(test)]` (the packet-routing logic, no socket) + one `#[ignore]` live test.

**Interfaces:**
- Consumes: `rtsp-runtime::io::AsyncRtspClient`, `rtsp-runtime::client::ClientEvent`, `rtsp-runtime::transport::{Transport, TransportSpec}`, `transmux::{RtpStreamDepacketizer, RtpStreamTrack}`, `crate::source::{TrackInit, sdp::parse_sdp_tracks}`, `transmux::pipeline::Sample`.
- Produces:
  - `pub struct RtspSource { name: String, url: String }` + `RtspSource::new(name, url)`
  - `pub struct RtspSession { pub tracks: Vec<TrackInit>, /* client + depacketizer + channel map */ }`
  - `impl RtspSource`: `async fn connect(&self) -> Result<RtspSession>` (connect → describe → parse SDP → setup each media interleaved → play)
  - `impl RtspSession`: `async fn next_samples(&mut self) -> Result<Option<Vec<(u32 /*track_id*/, Sample)>>>` (recv one interleaved frame; route RTP-channel→track; `depacketizer.push`; return emitted samples; `None` at stream end), and `fn track_specs(&self) -> Vec<transmux::pipeline::TrackSpec>`.
  - Extract the channel-routing decision into a pure `fn route_channel(channel: u8, tracks: &[TrackInit]) -> Option<u32>` so it is unit-testable without a socket.

> **Step 0 (read before coding):** open `rtsp-runtime/src/io.rs` + `transport.rs` + `client.rs` for exact signatures: `AsyncRtspClient::connect`, `describe`→`ClientEvent::Response{body}`, how to build a `Transport` from `TransportSpec::rtp_avp_tcp_interleaved(lo, hi)` (find the `Transport` wrapper/constructor — likely `Transport::from`/`Transport(vec![spec])`/a `single` ctor; use the real one), `setup(uri, &Transport)`, `negotiated_transport()`, `recv_interleaved() -> Result<Option<ClientEvent>>`, and `play`. Build per-media SETUP URLs from the base URL + each track's `control`. Assign each media interleaved channel `(2i, 2i+1)`. Map an incoming even `MediaData.channel` → the `TrackInit` with that `channel`; ignore odd (RTCP) channels in v1.

- [ ] **Step 1: Write the failing test (pure routing)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use transmux::pipeline::CodecConfig;
    use transmux::rtp::RtpMediaKind;
    use transmux::avc_config_from_sprop;

    fn video_track(channel: u8) -> TrackInit {
        // reuse a known-good sprop; any valid avcC works here.
        let avc = avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==").unwrap();
        TrackInit {
            track_id: 1, kind: RtpMediaKind::H264,
            config: CodecConfig::Avc { config: avc, width: 0, height: 0 },
            clock_rate: 90_000, control: None, channel,
        }
    }

    #[test]
    fn routes_even_channel_to_track_ignores_rtcp() {
        let tracks = vec![video_track(0)];
        assert_eq!(route_channel(0, &tracks), Some(1)); // RTP -> track 1
        assert_eq!(route_channel(1, &tracks), None);    // RTCP -> ignored
        assert_eq!(route_channel(4, &tracks), None);    // unknown
    }
}
```

- [ ] **Step 2: Run to verify fail** — `cargo test -p multimux --lib source::rtsp` → FAIL (`route_channel` missing).

- [ ] **Step 3: Implement `RtspSource`/`RtspSession` + `route_channel`.**

Write `route_channel` first (pure):

```rust
/// Map an interleaved RTP channel to its track id (even channels only; RTCP
/// odd channels return `None`).
pub fn route_channel(channel: u8, tracks: &[TrackInit]) -> Option<u32> {
    if channel % 2 != 0 {
        return None;
    }
    tracks.iter().find(|t| t.channel == channel).map(|t| t.track_id)
}
```

Then the async driver using the real `rtsp-runtime` API confirmed in Step 0. `connect` performs DESCRIBE → `parse_sdp_tracks(body)` → for each track SETUP with `TransportSpec::rtp_avp_tcp_interleaved(t.channel, t.channel + 1)` at the per-media control URL → PLAY; build the `RtpStreamDepacketizer` from `tracks.iter().map(|t| RtpStreamTrack::new(t.track_id, t.kind, t.config.clone(), t.clock_rate))`. `next_samples` calls `recv_interleaved()`, matches `Some(ClientEvent::MediaData{channel, data})`, and if `route_channel(channel, &tracks)` is `Some(track_id)`, returns `depacketizer.push(track_id, &data)?` zipped with `track_id`; other events → empty vec; `Ok(None)` at end of stream.

- [ ] **Step 4: Run to verify pass** — `cargo test -p multimux --lib source::rtsp` → PASS (routing test). Add an `#[ignore]` async test `live_rtsp_smoke` that connects to an env-var URL (`MULTIMUX_TEST_RTSP`) and pulls a few samples, skipped by default.

- [ ] **Step 5: Commit**

```bash
git add multimux/src/source/rtsp.rs
git commit -m "feat(multimux): RtspSource — DESCRIBE/SETUP/PLAY + interleaved depayload (#663)"
```

---

### Task 6: Pipeline task — Source → LlHlsSegmenter → StreamStore

**Files:**
- Create/replace: `multimux/src/pipeline.rs`
- Test: in-module `#[cfg(test)]` driving a `MockSource`.

**Interfaces:**
- Consumes: `crate::store::StreamStore`, `transmux::ll_hls::LlHlsSegmenter`, `transmux::pipeline::{Sample, TrackSpec}`, `crate::config::Config`.
- Produces:
  - `pub trait SampleSource { fn track_specs(&self) -> Vec<TrackSpec>; async fn next(&mut self) -> Result<Option<Vec<(u32, Sample)>>>; }` (async-trait-free: use `impl std::future::Future` return, or make it a concrete enum — simplest: define the pipeline over a concrete function/closure. Given async-in-trait on MSRV 1.86 is stable (edition 2024), an `async fn` in a trait is allowed).
  - `pub async fn run_pipeline(store: std::sync::Arc<StreamStore>, target_duration_secs: f64, part_target_ms: u32, specs: Vec<TrackSpec>, mut next: impl FnMut() -> ...)` — see note.

> **Design note:** to avoid async-trait complexity, model the pipeline as: build `LlHlsSegmenter::with_part_target(specs, movie_timescale, target_duration_secs, part_target_ms)`, set `store.set_init(seg.init_segment()?)`, then loop pulling `Vec<(track_id, Sample)>` batches, `segmenter.push(track_id, sample)` each, then drain `take_ready_parts()` → `store.add_part` and `take_ready_segments()` → `store.add_segment`. The "pull" is a boxed async closure `FnMut() -> Pin<Box<dyn Future<Output=Result<Option<Vec<(u32,Sample)>>>>>>` OR — cleaner — pass the concrete `RtspSession`/`MockSource` and call its `next_samples().await`. Choose the concrete approach: `run_pipeline` is generic over a `SampleSource` trait with an `async fn next_samples`. `movie_timescale`: use the video track's clock_rate (90000) or a fixed 90000.

- [ ] **Step 1: Write the failing test** — a `MockSource` yielding N pre-built video `Sample`s (construct via `transmux::pipeline::Sample::from_annexb` or `Sample::new`) across enough duration to close ≥1 segment; run the pipeline; assert `store.init_bytes().is_some()` and at least one segment or part landed (`store.media_playlist_m3u8(1)` contains `#EXT-X-PART` or a `seg-` URI).

- [ ] **Step 2: Run to verify fail.**

- [ ] **Step 3: Implement `SampleSource` trait + `run_pipeline`.** Provide `MockSource` in a `#[cfg(any(test, feature = "testsupport"))]` block (or `pub` in a `testsupport` module) so Task 8's integration test can reuse it.

- [ ] **Step 4: Run to verify pass.**

- [ ] **Step 5: Commit** — `feat(multimux): pipeline task (source -> segmenter -> store) (#663)`.

---

### Task 7: axum origin — router, handlers, blocking reload

**Files:**
- Create/replace: `multimux/src/origin/mod.rs`, create `multimux/src/origin/handlers.rs`
- Test: in-module `#[cfg(test)]` using `axum` handler calls against an `AppState` with a pre-populated `StreamStore` (no real socket needed — call handlers directly or via `tower::ServiceExt::oneshot` if `tower` is added; simplest: unit-test the handler functions directly with `State` + `Path`/`Query` extractors constructed in-test).

**Interfaces:**
- Produces:
  - `pub struct AppState { pub streams: std::collections::HashMap<String, std::sync::Arc<StreamStore>> }` (wrapped `Arc<AppState>`)
  - `pub fn router(state: std::sync::Arc<AppState>) -> axum::Router`
  - Handlers for: `GET /:stream/master.m3u8`, `/:stream/media.m3u8`, `/:stream/init-:track.mp4`, `/:stream/seg-:track-:seq.m4s`, `/:stream/part-:track-:seq.:idx.m4s`.

> **Design:** routes use axum 0.7 path syntax (`/:stream/...`). Content types: `application/vnd.apple.mpegurl` (m3u8), `video/mp4` (init), `video/iso.segment` or `video/mp4` (m4s). Unknown stream → 404. Missing bytes → 404. Master playlist: a minimal single-variant `#EXTM3U`/`#EXT-X-STREAM-INF` pointing at `media.m3u8`. **Blocking reload:** the `media.m3u8` handler reads optional `_HLS_msn` + `_HLS_part` query params; if the requested (msn, part) is beyond `store.latest_progress()`, `await` on `store.subscribe()` (`rx.changed().await`) in a loop until it is reached or a bounded timeout (`tokio::time::timeout`, e.g. `part_hold_back`-derived, cap 5s) fires, then render. Without the query params, render immediately.

- [ ] **Step 1: Write the failing test** — build an `AppState` with one `StreamStore` pre-filled (init + a couple parts via Task 3 APIs). Call the `media.m3u8` handler with no query → assert it returns 200 + body containing `#EXT-X-PART`. Call the segment handler for a present segment → 200 + bytes; for an absent one → 404. Call with an unknown stream → 404. (Construct extractors directly, e.g. `State(state)`, `Path((stream, ...))`, `Query(params)`.)

- [ ] **Step 2: Run to verify fail.**

- [ ] **Step 3: Implement router + handlers + blocking reload** (bounded `tokio::time::timeout` around the `watch` wait; on timeout, render current state rather than hang).

- [ ] **Step 4: Run to verify pass.**

- [ ] **Step 5: Commit** — `feat(multimux): axum origin + LL-HLS handlers + blocking reload (#663)`.

---

### Task 8: Integration gate — MockSource → pipeline → origin (deterministic)

**Files:**
- Create: `multimux/tests/origin_llhls.rs`

> The deterministic biting gate. Drive a `MockSource` (reuse Task 6's, exposed via a `testsupport` feature or `pub(crate)` test helper) built from a **real fixture**: demux `transmux`'s committed `h264_aac.ts`, packetize to RTP, and replay those packets through the pipeline — OR, simpler and still real, take the demuxed `Sample`s directly and feed them (the RTP round-trip is already gate-tested in transmux #700). Then spin the axum app (bind an ephemeral `127.0.0.1:0` port via `tokio::net::TcpListener`), run the pipeline task to close ≥2 segments with parts, and issue real HTTP requests with a minimal client (`tokio` TCP + hand-written GET, or add `hyper` as a dev-dep) to:

- [ ] **Step 1: Write the test** asserting:
  - `GET /{stream}/media.m3u8` → 200, body has `#EXT-X-PART`, `#EXT-X-PART-INF`, `#EXT-X-SERVER-CONTROL`.
  - a blocking `GET /{stream}/media.m3u8?_HLS_msn=<future>&_HLS_part=0` resolves (does not 404/hang) once the pipeline produces that part (drive the pipeline concurrently; assert the request completes after the part lands).
  - `GET /{stream}/init-1.mp4` → 200 and the bytes **pass `transmux::validate::validate_init_segment`** (no `Severity::Error`).
  - a `GET /{stream}/seg-1-<seq>.m4s` → 200 and bytes **pass `validate_media_segment`**.
  - unknown stream → 404.

- [ ] **Step 2: Run — `cargo test -p multimux --test origin_llhls` → PASS.** Fill real API where needed; if a genuine bug in Tasks 3/6/7 surfaces, fix it there, do not weaken assertions.

- [ ] **Step 3: Commit** — `test(multimux): deterministic LL-HLS origin integration gate (#663)`.

---

### Task 9: CLI binary + docs + examples + full gate

**Files:**
- Create: `multimux/src/bin/multimux.rs`, `multimux/README.md`, `multimux/CHANGELOG.md`, `multimux/examples/serve_mock.rs`, `multimux/examples/serve_rtsp.rs`, `docs/release-notes/multimux-0.1.0.md`
- Modify: `multimux/src/origin/mod.rs` (add a `serve(config)` entrypoint used by the bin)

**Interfaces:**
- Produces: `pub async fn serve(config: crate::config::Config) -> crate::Result<()>` — builds a `StreamStore` per route, spawns a pipeline task per route (each constructs an `RtspSource`), binds `config.bind`, serves `router(state)`.

- [ ] **Step 1: Implement the CLI** (clap derive, per `docs/CLI-STANDARD.md`): flags `--config <FILE>` (JSON) OR the single-route quick start `--rtsp <URL> --name <NAME>` plus `--bind`, `--target-duration`, `--part-ms`, `--window`. Build a `Config` (file if `--config`, else from flags), call `serve(config).await`. `#[tokio::main]`.

- [ ] **Step 2: Examples** — `serve_mock.rs` (runs the origin over a MockSource, no network; prints the URL) and `serve_rtsp.rs` (reads an RTSP URL from argv, serves it). Both must compile under `cargo build -p multimux --examples`.

- [ ] **Step 3: Docs** — `README.md` (what it is, quick start CLI, endpoint table, v1 scope/limits), `CHANGELOG.md` (`[0.1.0]`), `docs/release-notes/multimux-0.1.0.md`. Crate-root `//!` already covers the overview (Task 1).

- [ ] **Step 4: Full CI-exact gate (workspace):**

```bash
RUSTFLAGS="-D warnings" cargo build --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo build --workspace --no-default-features --locked
cargo test --workspace --all-features --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
rustup run 1.86 cargo build -p multimux --locked   # MSRV
```
Expected: all green. This is where axum-family MSRV pins are proven; fix the lockfile if 1.86 fails.

- [ ] **Step 5: Commit** — `feat(multimux): CLI binary + docs + examples (#663)`.

---

## Self-Review

**Spec coverage (against `docs/superpowers/specs/2026-07-14-multimux-design.md`):**
- Client+server wrap, glue-only → Tasks 4–7 consume transmux/rtsp-runtime, no spec logic. ✓
- RTSP pull via `AsyncRtspClient` → Task 5. ✓
- SDP→CodecConfig via transmux P2 helpers → Task 4. ✓
- Streaming depayload via P1 `RtpStreamDepacketizer` → Task 5. ✓
- LL-HLS only, parts + blocking reload → Tasks 3, 7. ✓
- RAM rolling window + `watch` → Task 3. ✓
- Routes model, CLI + optional JSON → Tasks 2, 9. ✓
- `Source` trait (in-crate, enables MockSource; no shared lib with #669) → Task 4/6. ✓
- Gate: LL-HLS playlist tags + blocking reload + fMP4 validate on a real fixture → Task 8. ✓
- v1 boundaries (no DASH/TLS/auth/sessions/DVR) — not built. ✓

**Type consistency:** `TrackInit`, `route_channel`, `StreamStore` methods, `RtpStreamTrack::new`, `LlHlsSegmenter::with_part_target`, `MediaPlaylist`/`PartSpec`/`LowLatencyConfig`, `validate_init_segment`/`validate_media_segment` — used consistently and match the API scan.

**Open verification points flagged (Step-0 reads):** exact `transmux::hls` struct fields incl. how `EXT-X-MAP` is carried (Task 3); `sdp_types` accessor return types (Task 4); `rtsp-runtime` `Transport` constructor + `recv_interleaved`/`negotiated_transport` signatures (Task 5); async-fn-in-trait shape for `SampleSource` (Task 6); axum 0.7 extractor construction in unit tests (Task 7); real-HTTP client choice for the gate (Task 8). Each task's Step 0 pins these to files; tests assert observable behavior so any correct wiring passes.

**Risk carried from #700 (documented, not re-solved here):** `RtpStreamDepacketizer` assumes in-arrival-order packets, low-delay H.264 (`composition_offset=0`), one AAC AU/packet — multimux feeds packets in `recv_interleaved` arrival order, satisfying the contract.
