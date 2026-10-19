# transmux 0.27.7

_Released 2026-10-19._

**Patch release.** A player can now be told, at the exact picture where it happens, that the shape of the video changed. Standard-definition channels switch between a programme and a broadcaster slate (for example sample aspect ratio 16:11 to 1:1, or 16:11 to 64:45) and, until now, a consumer only found out from a timestamp jump. 0.27.7 adds `DemuxEvent::DisplayChanged`, delivered through the same event stream as `TrackUpdated`, and a per-access-unit `Sample::display` tag that survives the batch `Media` path, IR transforms and the `media-plane` Trunk. This is Phase 2 of the display-geometry design (`docs/display-geometry/DESIGN.md`) and builds on the sample aspect ratio (0.27.5) and colour description (0.27.6).

## Why this version

- **Proposed version:** 0.27.6 -> **0.27.7** (patch). Everything new is additive: a new variant on the `#[non_exhaustive]` `DemuxEvent`, a new field on the `#[non_exhaustive]` `Sample`, and new `#[non_exhaustive]` types `DisplayInfo` and `DisplayChangeSet`. A downstream `match` on `DemuxEvent` already needs a wildcard arm; every workspace dependent (`multimux`, `hls-runtime`, `media-plane`, `media-doctor`, `compliance-probe`, ...) builds unchanged.
- **Caret epoch:** unchanged (0.27); dependents on `transmux = "0.27"` need no edits.
- **No dependency changes.**

## Migration

- **Apply `DisplayChanged` where it arrives.** For one video track the order is: the last sample of the old geometry, then (when the codec configuration bytes changed) `TrackUpdated`, then `DisplayChanged`, then the first sample of the new geometry. There is **no old-geometry video sample after the event**, so a player that stamps each frame at decode-submit applies the new shape from exactly the first affected picture. The event carries that picture's own `dts`, `pts` and `random_access` (true on an H.264 IDR or an H.265 IDR/BLA), the full new `Arc<DisplayInfo>`, and a `DisplayChangeSet` naming what changed.
- **Level-triggered start.** Every video track that has display state gets one `DisplayChanged { changed.initial }` right after its `TrackAdded` and before its first sample, so a consumer has one code path: apply `DisplayChanged`.
- **A SAR-only change raises the event.** Changing the sample aspect ratio while keeping the display aspect ratio (idc 4 `16:11` to `Extended_SAR` `64:45`, both 16:9 at 720 wide) sets `changed.aspect_ratio`. Changes are detected by comparing the **parsed** geometry, never the parameter-set bytes: an identical SPS re-sent at every IDR emits nothing, and an SPS whose bytes change but whose geometry does not produces `TrackUpdated` only.
- **Per-sample tag.** `sample.display` is `Some(Arc<DisplayInfo>)` on every video sample from the TS demuxers (batch `TsDemux` and `StreamingTsDemux`). The first sample after a change carries the same `Arc` as the event before it (`Arc::ptr_eq`). `None` means "this demuxer does not track display state", never "unchanged"; the Program Stream, FLV, RTP and fMP4 demuxers leave it `None` in this release.
- **A change takes effect at a random access point.** SPS-derived fields can only change at an IDR (H.264) or IRAP (H.265). A non-conforming stream that changes an active SPS in the middle of a coded video sequence gets the event at the first access unit that references the new content, with `random_access == false`.

## What changed

### Added

- **`DemuxEvent::DisplayChanged { track_id, display, changed, dts, pts, random_access, provenance }`** and the `DemuxEvent::display_changed(..)` constructor.
- **`Sample::display: Option<Arc<DisplayInfo>>`** and `Sample::with_display`.
- **`ir::display::DisplayInfo { geometry }`** (`DisplayInfo::changes_from(previous)`) and **`DisplayChangeSet { initial, aspect_ratio, output_rect, default_display_rect, video_signal, overscan }`**.
- The TS demuxers track SPS and PPS **by id** and advance the geometry at the access unit whose slices first reference the changed SPS (slice `pic_parameter_set_id` -> PPS `seq_parameter_set_id` -> SPS), not at the access unit that merely carries the NAL. A slice that references a PPS or SPS not yet received keeps the previous state; the change is raised on the first access unit that resolves it.

## Verification

- **Three real slate switches, checked against an independent `ffprobe` oracle:** the stimmungsgarten capture 16:11 to 1:1 (a still-image slate on a new timeline), the stimmungsgarten capture 1:1 to 16:11 (back to the programme), and the krone capture 16:11 to 64:45. Each gives exactly one switch event; no old-geometry video sample follows it; and the event's `dts`, `pts` and `random_access` equal those of the first new-geometry sample (for the 16:11 to 1:1 slate: `dts` and `pts` 10 780 874 677, an IDR). The stimmungsgarten slate references a PPS that has not arrived yet at its first IDR, so the change lands on the next one; that case is pinned on the real capture.
- **Single-geometry captures** (Melodie TV, Krone, and the six-track `test-092.ts`) produce exactly one initial event per track and nothing else, which is the "identical SPS re-sent at every IDR emits nothing" case on real data.
- **`media-plane`** carries `Sample::display` through a Trunk unchanged (pinned by a test).
- **Spec vectors:** a new SPS id sent ahead of its use, a geometry-neutral SPS change, a mid-CVS change, an unseen PPS or SPS, and the H.265 id chain.
- **No golden was re-blessed:** the sweep goldens mask the `display` fields and every original hash is unchanged. The one allocation budget that moved (`ps_demux`, 658 to 659) is the extra pointer-sized field on `Sample`, not extra payload copies.
- Each ordering and timing assertion was mutation-checked (event after the sample, no event, timing taken from the wrong access unit, and `random_access` taken from the wrong access unit all make a test fail).

## Limits

- **`TrackUpdated` is still emitted twice per switch** on these captures: a pre-existing effect of the track re-probe, already present in 0.27.6 and tracked as issue #1380. `DisplayChanged` is emitted exactly once.
- **The unseen-PPS path is covered by the real captures** but has no isolated test that kills its mutation.
- **H.265 is synthetic-only:** no real HEVC capture exercises the change event.
- **Not yet covered:** the Program Stream, FLV, RTP and fMP4 in-band (`avc3`/`hev1`) demuxers do not set `Sample::display`; crop and AFD (active format description) are later phases.

---

Published from tag `transmux-v0.27.7`.
