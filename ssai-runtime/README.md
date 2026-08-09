# ssai-runtime

[![crates.io](https://img.shields.io/crates/v/ssai-runtime.svg)](https://crates.io/crates/ssai-runtime)
[![docs.rs](https://img.shields.io/docsrs/ssai-runtime)](https://docs.rs/ssai-runtime)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Sans-IO SCTE-35 server-side ad insertion (SSAI) **session core** (issue
[#929](https://github.com/fishloa/rust-broadcast/issues/929)). `no_std` +
`alloc`, mirroring the `rtsp-runtime`/`hls-runtime` split: a driveable core
with no HTTP, no ad-decision client, and no per-viewer media cursor.

## What's here

- **[`session`]** — per-session ad-break state: `SessionStore` maps a session
  id to one small [`session::BreakState`] record (the decision being
  rendered, plus the conditioned splice point it entered/resumed at). This is
  deliberately **not** a media cursor: every viewer in a break watches one of
  a small set of ad assets, and outside a break sees byte-identical primary
  content — see the issue's design-decision comment. `media-plane`'s
  "writer cost is O(N) in cursor count" rule is about the shared media rings,
  not this.
- **[`decision`]** — [`decision::AdDecisionProvider`], the one pluggable
  extension point for "which ad to play". VAST/VMAP (or anything else) is
  entirely the implementor's concern: the trait performs no I/O, and this
  crate has no HTTP client. [`decision::AdBreakDecision`] models the HLS
  Interstitials attributes this crate implements (`X-ASSET-URI`/
  `X-ASSET-LIST`, `X-RESUME-OFFSET`, `X-PLAYOUT-LIMIT`, `X-SNAP`,
  `X-RESTRICT` — draft-pantos-hls-rfc8216bis Appendix D §D.2).
- **[`splice`]** — [`splice::condition_splice_point`]: aligns a cue's nominal
  target instant against real candidate boundaries (segment starts, or
  sync-sample/IDR timestamps), reporting the actual delta rather than
  assuming alignment. Real cues frequently are **not** IDR-aligned — see
  [Splice-point conditioning](#splice-point-conditioning-a-real-non-idr-aligned-cue) below.
- **[`playlist`]** — [`playlist::InterstitialDateRange`] renders/parses the
  `EXT-X-DATERANGE CLASS="com.apple.hls.interstitial"` tag (Appendix D),
  and [`playlist::render_session_playlist`] clones a base
  `broadcast_hls::MediaPlaylist` and appends that tag to
  `MediaPlaylist::extra_tags` — the one difference between a viewer's
  playlist inside a break and everyone else's.

## What this crate is **not**

- **No HTTP client, no VAST/VMAP.** Implementing `AdDecisionProvider` (and
  doing whatever network call that needs) is the caller's job.
- **No media manipulation.** Splicing the actual CMAF/TS bytes at the
  conditioned splice point is `transmux`'s job (its splice/SSAI IR
  transforms); this crate only decides *where* that point is and *what* the
  per-session manifest should say about it.
- **No per-viewer media cursor** — see [`session`] above.
- **No `multimux` HTTP wiring** — an adapter is future work, not this crate.

## Splice-point conditioning: a real, non-IDR-aligned cue

`fixtures/scte35-ssai/` (Apache-2.0, a genuine DASH-IF `livesim2` capture;
see its `PROVENANCE.md`) carries a real `splice_insert()` whose nominal
presentation time is **not** on a video keyframe: the nearest keyframe lands
6000 ticks (67 ms) *after* the cue, measured independently straight from the
fragment's own `moof`/`traf`/`tfdt`/`trun` boxes. That's real-world encoder
behaviour, not a fixture bug, and `ssai-runtime/examples/condition_real_cue.rs`
reproduces the measurement rather than asserting it:

```sh
cargo run -p ssai-runtime --example condition_real_cue
```

```
cue presentation_time = 160767315900000 (90kHz ticks, absolute representation clock)
tolerance=3000 ticks (~33ms, one 30fps frame) -> Err(NoAlignedBoundary { .. nearest_delta_ticks: 6000 })
tolerance=10000 ticks (~111ms) -> ConditionedSplicePoint { .. delta_ticks: 6000, direction: After }
conditioning correctly measured the real, non-IDR-aligned gap: 6000 ticks (~66.7ms)
```

A caller picks its own tolerance: a live low-latency splice might need a
tight bound (and correctly reject this cue), while a VOD packager splicing at
a GOP boundary can accept the drift. `condition_splice_point` never silently
lies about alignment that isn't there.

## Examples

- `condition_real_cue` — splice-point conditioning against the real fixture's
  measured 67 ms gap (above).
- `session_playlist` — the full walkthrough: decode a real cue, run it
  through an `AdDecisionProvider`, condition its splice point, track it in a
  `SessionStore`, and render one viewer's `EXT-X-DATERANGE` into their
  playlist while every other viewer's stays untouched.

```sh
cargo run -p ssai-runtime --example session_playlist
```

## Install

```toml
[dependencies]
ssai-runtime = "0.1"
```

## Spec references

- HLS Interstitials: draft-pantos-hls-rfc8216bis Appendix D (+ Appendix F),
  transcribed at [`broadcast-hls/docs/interstitials.md`](../broadcast-hls/docs/interstitials.md).
- SCTE-35 `splice_insert()` / segmentation: ANSI/SCTE 35 2023r1, transcribed
  at [`scte35-splice/docs/`](../scte35-splice/docs/).

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or
[MIT license](../LICENSE-MIT) at your option.
