# transmux 0.27.5

_Released 2026-10-18._

**Patch release.** `transmux` read the sample aspect ratio (SAR) out of every H.264 and H.265 SPS and then discarded it, so a player could not show anamorphic standard-definition video at the right shape: a 720x576 picture with SAR 16:11 or 64:45 came out squashed with bars at the sides. 0.27.5 keeps the SAR and exposes it on every video track as `TrackSpec::display`, derived once from the codec configuration so TS (batch and streaming), fMP4 and every other demux path carry it without per-path code. It also fixes `StreamingTsDemux` emitting `DemuxEvent::TrackUpdated` one picture too early. This is Phase 1a of the display-geometry design (`docs/display-geometry/DESIGN.md`).

## Why this version

- **Proposed version:** 0.27.4 -> **0.27.5** (patch). Every new public item is additive: the `ir::display` module, `TrackSpec::display` (a new field on `#[non_exhaustive]` `TrackSpec`, so downstream code can neither construct it with a literal nor match it exhaustively) and `TrackSpec::with_display`. **No field was added to `CodecConfig::Avc` / `Hevc`**, deliberately: `multimux` builds those variants with struct literals, so adding one would have forced a breaking 0.28 and a cascade through every dependent. Reading the geometry from the SPS inside the existing `avcC` / `hvcC` avoids that.
- **Caret epoch:** unchanged (0.27); dependents on `transmux = "0.27"` need no edits, and every workspace dependent builds unchanged.
- **No dependency changes.**

## Migration

- **Read the aspect ratio from the track.** `track.spec.display` is `Option<DisplayGeometry>` (`None` for audio and for video whose SPS cannot be parsed). `display.sample_aspect_ratio()` returns `Option<(u16, u16)>`: `None` means "unspecified" (idc 0, a reserved idc, or an `Extended_SAR` with a zero term), never a `0:0` sentinel. Apply it as the pixel aspect ratio when rendering (for example `kCMFormatDescriptionExtension_PixelAspectRatio` on Apple platforms): picture width `w` and height `h` display as `w * sar_w : h * sar_h`.
- **`TrackUpdated` now arrives one sample later.** If you worked around it arriving early (for example by buffering one picture or ignoring the first picture after the event), remove the workaround: the event now falls after the last sample of the old configuration and before the first sample of the new one, so a consumer applying it on receipt labels every picture with the configuration it was actually coded in. Audio is unchanged.
- **`display` follows config changes.** `TrackSpec::new` derives it from the first SPS in the `avcC` / `hvcC`, and it is re-derived on every `TrackUpdated`, so a mid-stream change of resolution or SAR reaches the consumer as a new `TrackSpec`. A source with better information can override it with `TrackSpec::with_display`.

## What changed

### Added

- **`transmux::ir::display`**: `DisplayGeometry` (coded picture size plus the SPS VUI aspect ratio), the typed `AspectRatioIdc` (ITU-T H.264 §E.2.1 Table E-1 and H.265 §E.3.1 Table E.1: idc 0 `Unspecified`, 1..=16 the named rows, 17..=254 `Reserved(u8)`, 255 `ExtendedSar { sar_width, sar_height }`; every variant has `name()` and `Display`) and `SampleAspectRatio` (both terms non-zero, with `reduced()`). `DisplayGeometry::from_avc_sps`, `from_hevc_sps` and `from_codec_config` are public.
- **`TrackSpec::display: Option<DisplayGeometry>`** and `TrackSpec::with_display`.
- The H.264 and H.265 VUI walks no longer discard `aspect_ratio_idc`, `sar_width` and `sar_height`.

### Fixed

- **`DemuxEvent::TrackUpdated` arrived one picture early.** Video samples are emitted one access unit behind the input, so pushing the event before the triggering access unit put it ahead of the previous (last old-config) sample. `StreamingTsDemux` now releases the pending sample first and then the event. Fixed against the public `h264-two-resolutions.ts` fixture (keyframe DTS 306000, 320x240 then 640x480).

## Verification

- **Real captures:** the Melodie TV capture parses to SAR 64:45 (`Extended_SAR`, idc 255) and the Krone capture to SAR 16:11 (idc 4), through both the batch and the streaming TS demuxers, and the SAR survives a CMAF mux then demux round trip. Both are 720x576 anamorphic services; the captures live in the private fixtures repository.
- **Spec vectors:** every `aspect_ratio_idc` value (1..=16, 255, 0, 17, 254, and `Extended_SAR` with a zero term) for H.264 and H.265. **The H.265 path has spec vectors only**: no real anamorphic HEVC capture exists yet.
- **Table E-1** was checked against the rendered pages of both specifications, not only the markdown transcription.
- **No golden was re-blessed:** the sweep goldens hash the `Debug` rendering of the whole IR, so they mask the new `display` field and every original hash is unchanged.
- Each new test was mutation-checked (reverting the fix makes it fail).

## Limits

- **Only the aspect ratio so far.** Cropping, the colour description (primaries, transfer, matrix, full range), a typed change event and per-sample tags are the next phases of the design; AFD (active format description) follows. The display geometry reports the coded size, not the cropped output.
- **A SAR change that keeps the picture shape** (for example 16:11 to 64:45, both 16:9) is visible as a new `TrackSpec` on `TrackUpdated`, not yet as a dedicated change event.
- **H.265 SAR is spec-vector only** (see above).

---

Published from tag `transmux-v0.27.5`.
