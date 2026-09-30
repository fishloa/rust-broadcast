# `fixtures/dash/` — DASH MPD fixtures

- `manifest.mpd` + the `init-stream*.m4s` / `chunk-stream*-*.m4s` segments — a
  real ffmpeg-generated DASH presentation (`ffmpeg -i fixtures/ts/h264_aac.ts
  -c copy -f dash`), one video and one audio `AdaptationSet`, each
  `Representation` carrying its own `SegmentTemplate` +
  `SegmentTimeline`. Used by `transmux/tests/dash.rs`, `dash_mpd.rs`,
  `dash_parse.rs`.

- `manifest-inheritance.mpd` — **derived** from `manifest.mpd` above (same
  segments, same codec/geometry/timing values; nothing invented), restructured
  into the `SegmentTemplate`-inheritance layouts the standard itself uses:

  | level | declares | inherits |
  | --- | --- | --- |
  | Period | `BaseURL`, `@timescale`, `@startNumber`, `@presentationTimeOffset`, `SegmentTimeline` | — |
  | video AdaptationSet | `@media` | `@timescale`, `@startNumber`, `SegmentTimeline` |
  | video Representation | `@initialization` | all of the above |
  | audio AdaptationSet | `@timescale`, `@startNumber`, `@media` | `SegmentTimeline` |
  | audio Representation | `@initialization`, `SegmentTimeline` | `@timescale`, `@startNumber`, `@media` |

  This is ISO/IEC 23009-1 §5.3.9.1's attribute-by-attribute inheritance: Annex
  G.13's shape (a `Representation` `SegmentTemplate` declaring one attribute
  and inheriting the rest from the `AdaptationSet`) plus G.12's (the level
  above carrying `@media` while the level below carries `@timescale`). The
  audio `Representation` additionally carries its own `SegmentTimeline`, which
  must override the Period's (the "lowest level that declares one wins"
  element-level rule). MP4Box and ffmpeg only ever emit
  `Representation`-level templates, and DASH-IF livesim2 rejects any other
  shape outright, so no real-world capture of this layout could be pulled —
  hence a derivation from the real file rather than a fabricated one.
  Released under the workspace licence.
