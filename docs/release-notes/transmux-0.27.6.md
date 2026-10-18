# transmux 0.27.6

_Released 2026-10-19._

**Patch release.** A video track now reports how its signal is coloured. `TrackSpec::display` (added in 0.27.5 for the sample aspect ratio) gains the SPS VUI `video_signal_type`: the video format, the full-range flag and the colour description (primaries, transfer characteristics, matrix coefficients), for H.264 and H.265. A player that converts Y'CbCr to RGB itself needs exactly these four values and otherwise has to guess the matrix (BT.601 or BT.709) from the picture size. This is Phase 1b of the display-geometry design (`docs/display-geometry/DESIGN.md`).

## Why this version

- **Proposed version:** 0.27.5 -> **0.27.6** (patch). Everything new is additive: new types in `ir::display` and new fields and accessors on `DisplayGeometry`, which is `#[non_exhaustive]`, so downstream code can neither construct it with a literal nor match it exhaustively. No `CodecConfig` variant changed.
- **Caret epoch:** unchanged (0.27); dependents on `transmux = "0.27"` need no edits, and every workspace dependent builds unchanged.
- **No dependency changes.**

## Migration

- **Read the signal from the track.** `track.spec.display` (an `Option<DisplayGeometry>`) now has `video_signal: Option<VideoSignalType>`. It is `None` when the stream does not signal a video signal type, and it records what was *signalled*, not what the spec infers. `VideoSignalType { video_format, full_range, colour }` carries `colour: Option<ColourDescription { primaries, transfer, matrix }>`, which is `None` when `colour_description_present_flag` is 0.
- **Unsignalled is not "BT.709".** When the VUI omits the colour description, the spec infers primaries, transfer and matrix 2 ("unspecified"), video format 5 and limited range. Use the accessors for those inferred defaults, and apply your own policy (for example "HD without a signal is BT.709, SD is BT.601") on top. Transport streams carry no `colr` box, so the SPS is the only source.
- **`DisplayGeometry::full_range()`** returns the signalled flag, or `false` (limited range) when none was signalled. `video_format()` and `colour_description()` are the matching convenience accessors.
- **Reserved code points are kept.** Each of the four enums has a lossless `Reserved(u8)` arm, so a code point the spec reserves (or defines in a newer revision) round-trips unchanged. Match with a wildcard arm.

## What changed

### Added

- **`VideoFormat`** (H.264 Table E-2 / H.265 Table E.2), **`ColourPrimaries`** (E-3 / E.3), **`TransferCharacteristics`** (E-4 / E.4) and **`MatrixCoefficients`** (E-5 / E.5), each with `from_wire`, `code`, `name` (the spec's own label) and `Display`.
- **`ColourDescription`** and **`VideoSignalType`**, plus `DisplayGeometry::{overscan_appropriate, video_signal, full_range, video_format, colour_description}` and the constants for the inferred defaults.
- The H.264 and H.265 VUI walks now read `overscan_info_present_flag`, `overscan_appropriate_flag` and the whole `video_signal_type` group in the specified order.
- `transmux/docs/codec/vui-colour-tables.md`: the verified code-point tables with their source pages.

## Verification

- **Spec tables:** every row of Tables E-2 to E-5 (H.264) and E.2 to E.5 (H.265) was checked against the rendered PDF page, not only the markdown transcription, because the markdown conversion had lost rows (E-4 rows 2 and 3, E-5 rows 0 to 15 merged into one paragraph). The H.264 and H.265 tables are identical except that `video_format` 6 and 7 mean "interpret as 5" in H.265.
- **Real captures, against an independent oracle** (`ffprobe` and `ffmpeg -bsf:v trace_headers`): Melodie TV signals component video, limited range, primaries/transfer/matrix 5/5/5 (BT.470 B/G); Krone signals no video signal type; a full-range capture signals full range with no colour description; two more captures signal 1/1/1 (BT.709) and 5/5/5. Each is asserted through the batch and the streaming TS demuxers, and the colour fields survive a CMAF mux then demux round trip.
- **Spec vectors:** the field order (overscan before video signal type), absent and present signal types, reserved code points, and every wire code point 0 to 255 round-tripping through each enum.
- **No golden was re-blessed:** the sweep goldens mask the `display` field and every original hash is unchanged.
- Each new test was mutation-checked (ignoring `colour_description_present_flag`, swapping the primaries and transfer reads, swapping the overscan and signal-type order, dropping the colour read, inverting full range, and mislabelling a code point all make a test fail).

## Limits

- **H.265 colour is spec-vector only.** No real HEVC capture in the fixture set signals a colour description; the HEVC path shares the tables and is tested with constructed VUI bit patterns.
- **Cropping, a typed change event, per-sample tags and AFD are later phases.** A mid-stream change of colour reaches the consumer today as a new `TrackSpec` on `TrackUpdated`.

---

Published from tag `transmux-v0.27.6`.
