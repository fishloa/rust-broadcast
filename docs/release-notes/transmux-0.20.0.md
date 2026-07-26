# transmux 0.20.0

**Breaking: Major IR consolidation to enable splice/DVR/pacing**, plus three latent demux bugs fixed and absolute timing model adoption across all formats.

## Why This Release

The IR could not express absolute time — `Sample` timing was relative (`summed duration`), anchored on a per-track `start_decode_time` that many sources (FLV, WebM, MPEG-PS, RTMP, RTP) left at `0`. This prevented three critical use cases:

- **Splice/ad-stitch:** can't identify a sample's wall-clock position to inject an ad.
- **DVR/seek:** can't rebase a sample's timeline across independent TS captures.
- **Pacing/deadline:** can't express "send this sample at wall-clock T".

This release fixes the IR to carry absolute, unwrapped timestamps — once, at the demux edge — so downstream callers (splice, DVR, pacing, live origin) can work with real time.

## Breaking Changes

### IR Consolidated into `transmux::ir`

All IR types (`Media`, `Track`, `TrackSpec`, `Sample`, `SampleFlags`, `Provenance`, `DemuxEvent`) moved from the crate root into a new `transmux::ir` module and re-exported at the crate root for backwards compatibility **within a single crate** — but external crates must update their imports:

| Old | New |
|---|---|
| `transmux::Media` | `transmux::ir::Media` (crate-root re-export ok for local use) |
| `transmux::Sample` | `transmux::ir::Sample` (crate-root re-export ok for local use) |
| `transmux::DemuxEvent` | `transmux::ir::event::DemuxEvent` (de-TS-ified; see below) |

### Sample: `data: Bytes` + Absolute Timing

`Sample` is now:

```rust
pub struct Sample {
    pub data: bytes::Bytes,                    // was: &'a [u8]
    pub dts: Option<i64>,                      // absolute ticks in track timescale, was: duration-summed
    pub pts: Option<i64>,                      // absolute ticks in track timescale, was: duration-summed
    pub duration: Option<u32>,                 // as before
    pub flags: SampleFlags,                    // is_sync moved here (was a bool field)
    pub provenance: Option<Provenance>,        // debug-only wire_dts/wire_pts for round-trip
}
```

**Migration:**

- `Sample::new(data, dts, pts, duration, is_sync)` now accepts `impl Into<Bytes>` for `data` (so `&[u8]`, `Vec<u8>`, or `Bytes` work; zero-copy for `Bytes`).
- `pts`/`dts` are now `Option<i64>`, not `u32`. Callers must unwrap or provide defaults. Unwrapping happens once at the demux edge; downstream processing is decoupled from source format.
- `composition_offset` is no longer a field — derive it via `sample.composition_offset()` (`pts - dts`), which round-trips fMP4 `ctts` byte-identically.
- `is_sync` moved into `SampleFlags` — use `sample.flags.is_sync` or `sample.flags = SampleFlags::SYNC`.

### Absolute Time: No More `SourceTiming`

`SourceTiming` is deleted (it was write-only; the crate's own docs admitted "all mux paths ignore this field"). Wire-level debug info survives as `Provenance { wire_dts, wire_pts }` — optional debug-only side-fields on every sample, populated by demuxers that want to preserve the original wire stamps for round-trip verification.

**Migration:** Remove `SourceTiming` usage. If you need the original wire timestamps for debugging, check `sample.provenance`.

### Rebase: `SourceTiming` → `Provenance`; Wrap-Unwrapping Centralized

`rebase::unroll_33bit_wraps` / `rebase::MPEG_TS_WRAP` are removed. Wrap-unwrapping now happens once at the demux edge (33-bit MPEG-2 Systems §2.4.3.7, 32-bit RTP RFC 3550 §5.1), so re-folding and unwrapping again was an anti-pattern.

`rebase_to_zero` / `apply_offset` / `insert_discontinuity_gap` now shift every sample's absolute `dts`/`pts` in lockstep with `Track::start_decode_time` (and never fabricate a timestamp for a `None`-timed sample).

### DemuxEvent: Moved + De-TS-ified

`DemuxEvent` moved to `transmux::ir::event::DemuxEvent` and lost its TS-specific variant:

- `DemuxEvent::TrackUpdated` removed (TS-only; see the `ts_demux` module docs, "Streaming TS track layout changes").
- All other variants (`TrackAdded`, `Sample`, `Discontinuity`) remain; callers using the pull API (`poll_event`) are unaffected if they use `if let` or exhaustive match (since `TrackUpdated` removal fixes a struct-literal construction gap in earlier versions).

### New: `CodecConfig::Subtitle`

`CodecConfig` gained a new variant for structured subtitle formats:

```rust
CodecConfig::Subtitle { format: SubtitleFormat }
```

`SubtitleFormat` (`#[non_exhaustive]`, `name()`/`Display` per #204) carries:
- `DvbBitmap` — DVB bitmap subtitles (ETSI EN 300 743, `CodecConfig::Data` on TS path today).
- `Teletext` — DVB teletext subtitles.
- `Fmp4Stpp` — TTML/IMSC (ISO/IEC 14496-30 §7.2).
- `Fmp4Wvtt` — WebVTT (§9.2).

`Fmp4Demux` now demuxes `stpp` and `wvtt` ISOBMFF sample entries into this variant. TS and PES paths for DVB subtitles remain on the roadmap.

**There is no re-mux path yet** — `build_trak` rejects `CodecConfig::Subtitle` with `Error::UnsupportedCodec` (tracked as #753).

### DemuxEvent Error Path: Silent Drops Replaced with Typed Errors

Two silent drops are now typed errors (found only after the additions above, so real `stpp`/`wvtt`/`ac-4` streams no longer hit them):

- `Fmp4Demux` no longer silently skips a track whose sample entry it cannot reconstruct; it returns `Error::UnsupportedSampleEntry { fourcc }`.
- `CmafMux` no longer silently filters `CodecConfig::Data` tracks out of segments; it returns `Error::UnmuxableDataTrack { track_id, stream_type }`. Callers wanting the old best-effort behaviour must pre-filter: `media.select_tracks_by(|t| !matches!(t.spec.config, CodecConfig::Data { .. }))`.

### Stage Trait Adoption

Eight demux/mux types now implement the `broadcast_common::Stage` trait (added in `broadcast-common` 8.7.0):

- `TsDemux`, `StreamingTsDemux`, `FlvDemux`, `StreamingFlvDemux` (demux).
- `CmafMux`, `IsoMp4Mux`, `TsMux` (mux).
- `RtpStreamDepacketiser` (reassembly).

This is a **zero-breaking-changes adoption** — `Stage` is opt-in (a separate trait impl that coexists with the existing pull/push APIs).

## Fixes

Three latent bugs fixed in this release:

1. **`TsDemux`: Audio sample timing was in wrong timescale.** Audio samples were stored in 90 kHz PES-clock ticks while the track's timescale is its sample rate (e.g. 48 kHz for AAC). This made `dts` deltas (e.g. 2089) disagree with `duration` (1024 AAC samples). Audio `dts`/`pts` and `Track::start_decode_time` are now rescaled into the track's own timescale — the same unit as `duration`.

2. **`PsDemux`: AC-3 sample duration was always 0.** Every AC-3 sample's `duration` was left at 0, making the audio timeline uninterpretable. Now carries the intrinsic 1536-sample syncframe duration (ETSI TS 102 366 §4.1) and absolute time rescaled from PES stamps.

3. **`RtpDepacketiser`: RTP timestamp discarded.** The (batch) depacketiser discarded the RTP timestamp and sync flag entirely, emitting `duration: 0` / `is_sync: true` for every sample. Now carries the unwrapped absolute RTP media clock and the real IDR-derived sync flag.

## Compatibility

MSRV: 1.86 (unchanged). `no_std` + `alloc` posture: unchanged.

This release enables the media-plane architecture (splice, DVR, pacing) — steps 3 and beyond depend on it.
