# ssai-runtime

[![crates.io](https://img.shields.io/crates/v/ssai-runtime.svg)](https://crates.io/crates/ssai-runtime)
[![docs.rs](https://img.shields.io/docsrs/ssai-runtime)](https://docs.rs/ssai-runtime)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Sans-IO SCTE-35 server-side ad insertion (SSAI) **session core**
([#929](https://github.com/fishloa/rust-broadcast/issues/929)): per-session
ad-break state, a pluggable ad-decision trait, splice-point conditioning against
real boundaries, and per-session HLS Interstitial playlist rendering.

## What it does / what it does not

**Does.** Decides *where* an ad break splices (aligning a SCTE-35 cue's nominal
instant to real segment or keyframe boundaries and reporting the actual drift),
remembers *which break each viewer session is in*, and renders *what that
viewer's HLS playlist should say* (an `EXT-X-DATERANGE
CLASS="com.apple.hls.interstitial"` tag over `broadcast_hls::MediaPlaylist`).
Used by a live origin (for example a `multimux` output adapter) that wants
per-session ad breaks without per-viewer media copies.

**Does not.**

- No HTTP client, no VAST/VMAP. Implementing `AdDecisionProvider` (and any
  network call it needs) is the caller's job.
- No media manipulation. Splicing the actual CMAF/TS bytes is `transmux`'s job
  (its splice/SSAI IR transforms); this crate only decides where and what the
  manifest says.
- No per-viewer media cursor. Every viewer in a break watches one of a small set
  of pre-conditioned ad assets and outside a break sees byte-identical primary
  content; what differs is a small `BreakState` record.
- No DASH MPD period insertion (HLS Interstitials only), no `multimux` HTTP
  wiring (an adapter is future work), no SCTE-35 parsing (the core takes plain
  tick counts; `scte35-splice` is a dev-dependency only).

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| IETF draft-pantos-hls-rfc8216bis, Appendix D (Interstitials) | the vendored draft dated 1 May 2026 (`specs/ietf_draft_pantos_hls_rfc8216bis.txt`), an Internet-Draft, not an RFC | `EXT-X-DATERANGE` Interstitial schema (D.2): `CLASS`, `X-ASSET-URI` / `X-ASSET-LIST` (URI only, JSON not fetched), `X-RESUME-OFFSET`, `X-PLAYOUT-LIMIT`, `X-SNAP`, `X-RESTRICT`; `START-DATE` required on the base playlist (RFC 8216bis section 4.4.5.1) | Partial | Skip-button control (D.3) and the asset-list JSON schema are not implemented. Attribute values are validated against tag injection |
| ANSI/SCTE 35 2023r1 | 2023r1 | Cue semantics only: `splice_insert()` out-of-network / break duration / event id as inputs (`BreakContext`), 33-bit `pts_time` wrap handling | Partial | The core takes ticks, not SCTE-35 wire types; decoding the cue is `scte35-splice` |

## Features

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Links `std` (and the siblings'). Off gives `#![no_std]` + `alloc`. |
| `serde` | no | `serde` derives on the decision and break-state types. |

## Quick start

```rust
use ssai_runtime::{
    AdBreakDecision, BreakState, InterstitialDateRange, SessionStore, condition_splice_point,
};

// A cue targets tick 900_000 (90 kHz); the real segment boundaries nearest it
// are at 897_000 and 904_500. Allow up to 5_000 ticks of drift.
let boundaries = [810_000_u64, 897_000, 904_500];
let point = condition_splice_point(900_000, &boundaries, 5_000).unwrap();
assert_eq!(point.snapped_pts, 897_000);
assert_eq!(point.delta_ticks, 3_000);

// Whatever ad-decision service the caller uses hands back one decision.
let decision = AdBreakDecision::single_asset("ad-1", "https://ads.example/ad-1.m3u8");

// Track the break for one viewer session ...
let mut store = SessionStore::new();
store
    .begin_break("viewer-42", BreakState::new(decision.clone(), point))
    .unwrap();
assert_eq!(store.len(), 1);

// ... and render the interstitial EXT-X-DATERANGE tag line for their playlist.
let tag = InterstitialDateRange::from_decision(&decision, "2026-01-01T00:00:10.000Z", Some(30.0))
    .to_tag_line()
    .unwrap();
assert!(tag.contains("CLASS=\"com.apple.hls.interstitial\""));
assert!(tag.contains("X-ASSET-URI=\"https://ads.example/ad-1.m3u8\""));
```

## What is implemented

- **`session`**: `SessionStore` maps a session id to one `BreakState` (the
  decision being rendered, plus the conditioned splice point it entered and
  resumed at): `begin_break`, `resume`, `get`, `end`, `len`.
- **`decision`**: `AdDecisionProvider`, the one extension point for "which ad
  to play" (no I/O); `AdBreakDecision` (`single_asset` / `asset_list`),
  `AssetSource`, `SnapMode`, `RestrictMode`, `BreakContext`.
- **`splice`**: `condition_splice_point` aligns a cue's target against candidate
  boundaries within a tolerance and reports the real delta and direction
  (`SnapDirection`); `condition_splice_point_wrapping` does the same on a
  33-bit wrapping clock (`PTS_MODULUS_33`), so a cue at `2^33 - 100` snaps to a
  boundary at `50`. Real cues are frequently not IDR-aligned (see below).
- **`playlist`**: `InterstitialDateRange` renders / parses the interstitial tag;
  `render_session_playlist` clones a base `MediaPlaylist` and appends the tag to
  `extra_tags`; `SessionPlaylistBase` renders the base once and splices the tag
  line per viewer with no per-viewer segment clone. Both refuse a base without
  `EXT-X-PROGRAM-DATE-TIME`.
- **`error`**: `Error` (`NoCandidates`, `NoAlignedBoundary`, `PtsOutOfRange`,
  `InvalidDuration`, ...).

### Splice-point conditioning: a real, non-IDR-aligned cue

`fixtures/scte35-ssai/` (Apache-2.0, a genuine DASH-IF `livesim2` capture; see its
`PROVENANCE.md`) carries a real `splice_insert()` whose nominal presentation time
is not on a video keyframe: the nearest keyframe lands 6000 ticks (67 ms) after
the cue, measured independently from the fragment's own `moof`/`traf`/`tfdt`/`trun`
boxes. `examples/condition_real_cue.rs` reproduces the measurement:

```sh
cargo run -p ssai-runtime --example condition_real_cue
```

```
cue presentation_time = 160767315900000 (90kHz ticks, absolute representation clock)
tolerance=3000 ticks (~33ms, one 30fps frame) -> Err(NoAlignedBoundary { .. nearest_delta_ticks: 6000 })
tolerance=10000 ticks (~111ms) -> ConditionedSplicePoint { .. delta_ticks: 6000, direction: After }
conditioning correctly measured the real, non-IDR-aligned gap: 6000 ticks (~66.7ms)
```

A caller picks its own tolerance; `condition_splice_point` never silently lies
about alignment that is not there.

## Examples

- `condition_real_cue`: splice-point conditioning against the real fixture's gap.
- `session_playlist`: decode a real cue, run it through an `AdDecisionProvider`,
  condition its splice point, track it in a `SessionStore`, and render one
  viewer's `EXT-X-DATERANGE` into their playlist while every other viewer's stays
  untouched (`cargo run -p ssai-runtime --example session_playlist`).

## Design notes

- Sans-IO and `no_std` + `alloc`, mirroring the `rtsp-runtime` / `hls-runtime`
  split: a driveable core with no sockets and no clock.
- Not a media cursor: `media-plane`'s "writer cost is O(N) in cursor count" rule
  concerns the shared media rings, not this crate.
- `#![forbid(unsafe_code)]`; structured `thiserror` errors; spec enums carry the
  workspace `name()` + `Display` label pair; ad-decision-supplied strings are
  rejected if they contain `"`, CR or LF (no tag injection).
- 33-bit clock arithmetic goes through `broadcast_common::clock33`
  (`tests/clock33_guard.rs` pins the one documented exception).

## Related crates

Depends on `broadcast-common` and `broadcast-hls` (playlist syntax); dev-depends
on `scte35-splice` and `mp4-emsg` for the real-fixture tests. Media splicing is
`transmux`; the HTTP-facing origin is `multimux`.

## Spec references

- HLS Interstitials: draft-pantos-hls-rfc8216bis Appendix D, transcribed at
  [`broadcast-hls/docs/interstitials.md`](../broadcast-hls/docs/interstitials.md).
- SCTE-35 `splice_insert()` / segmentation: ANSI/SCTE 35 2023r1, transcribed at
  [`scte35-splice/docs/`](../scte35-splice/docs/).

## MSRV and license

MSRV 1.95.0. Licensed under either of
[Apache License, Version 2.0](../LICENSE-APACHE) or
[MIT license](../LICENSE-MIT) at your option.
