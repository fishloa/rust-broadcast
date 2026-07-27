# transmux 0.20.0

**Breaking: Major IR consolidation to enable splice/DVR/pacing, track-lifecycle events for mid-stream PMT changes, an aggregate defect-fixing pass (11 defects found, 5 fixed), and two CENC confidentiality defects closed** (see `CHANGELOG.md`'s Security section for full disclosure — not reproduced here; this release ships no advisory and no 0.19.x backport, only the fix).

## Why This Release

The IR could not express absolute time — `Sample` timing was relative (`summed duration`), anchored on a per-track `start_decode_time` that many sources (FLV, WebM, MPEG-PS, RTMP, RTP) left at `0`. This prevented three critical use cases:

- **Splice/ad-stitch:** can't identify a sample's wall-clock position to inject an ad.
- **DVR/seek:** can't rebase a sample's timeline across independent TS captures.
- **Pacing/deadline:** can't express "send this sample at wall-clock T".

This release fixes the IR to carry absolute, unwrapped timestamps — once, at the demux edge — so downstream callers (splice, DVR, pacing, live origin) can work with real time. It also adds track-lifecycle events so a consumer can observe a mid-stream PMT change (a track added, updated, removed, or abandoned) instead of the demuxer silently reacting to it internally. An aggregate review of the whole step-2 range then found 11 blocking defects across the demux/segmenter family; the five worst are fixed here rather than shipped separately, since 0.20.0 was still unreleased when they were found.

## Breaking Changes

### IR Consolidated into `transmux::ir`

All IR types (`Media`, `Track`, `TrackSpec`, `Sample`, `SampleFlags`, `Provenance`, `DemuxEvent`) moved from the crate root into a new `transmux::ir` module and re-exported at the crate root for backwards compatibility **within a single crate** — but external crates must update their imports:

| Old | New |
|---|---|
| `transmux::Media` | `transmux::ir::Media` (crate-root re-export ok for local use) |
| `transmux::Sample` | `transmux::ir::Sample` (crate-root re-export ok for local use) |
| `transmux::DemuxEvent` | `transmux::ir::event::DemuxEvent` (see below) |

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
- `Sample` is now `#[non_exhaustive]`: construct with `Sample::new`/`from_annexb`/`from_raw`, not a struct literal.

### Absolute Time: No More `SourceTiming`

`SourceTiming` is deleted (it was write-only; the crate's own docs admitted "all mux paths ignore this field"). Wire-level debug info survives as `Provenance { wire_dts, wire_pts }` — optional debug-only side-fields on every sample, populated by demuxers that want to preserve the original wire stamps for round-trip verification. `Provenance` is `#[non_exhaustive]`; construct with `Provenance::new`.

**Migration:** Remove `SourceTiming` usage. If you need the original wire timestamps for debugging, check `sample.provenance`.

### Rebase: `SourceTiming` → `Provenance`; Wrap-Unwrapping Centralized

`rebase::unroll_33bit_wraps` / `rebase::MPEG_TS_WRAP` are **removed**. Wrap-unwrapping now happens once at the demux edge (33-bit MPEG-2 Systems §2.4.3.7, 32-bit RTP RFC 3550 §5.1), so re-folding and unwrapping again was an anti-pattern.

`rebase_to_zero` / `apply_offset` / `insert_discontinuity_gap` now shift every sample's absolute `dts`/`pts` in lockstep with `Track::start_decode_time` (and never fabricate a timestamp for a `None`-timed sample).

### `DemuxEvent`: Moved, Reshaped, and Track-Lifecycle-Aware

`DemuxEvent` moved to `transmux::ir::event::DemuxEvent`. **Correction to an earlier draft of this note: `TrackUpdated` was not removed — it is new in this release**, alongside two more new variants, all driven by PMT version diffing (issue #774; `StreamingTsDemux` now diffs a PMT's `version_number`/`current_next_indicator` instead of only ever inserting newly-seen PIDs):

- `TrackUpdated(TrackSpec)` — an existing track's `es_info_descriptors` or `stream_type` changed; codec-config recovery itself stays single-shot.
- `TrackRemoved { track_id, provenance }` — a PMT version change no longer lists a previously-live PID. No `Sample` for that `track_id` is ever queued afterward.
- `TrackAbandoned { track_id: Option<u32>, reason: AbandonReason, provenance }` — a track's config never resolved (`AbandonReason::ConfigUnrecoverable`) or its probe backlog exceeded its byte budget (`AbandonReason::BudgetExceeded`); `track_id` is always `None` today (abandonment happens before one would be assigned).

The rest of the enum was also reshaped in this same release:

- `TrackAdded(Track)` → `TrackAdded(TrackSpec)` — drops the always-empty `samples`/never-set `encryption` fields; every existing consumer already only read `track.spec`.
- `Discontinuity { track, provenance }` → `Discontinuity { track, kind: DiscontinuityKind, provenance }`, `#[non_exhaustive]`, with a `DemuxEvent::discontinuity(...)` constructor. `DiscontinuityKind` is `Signalled`, `TimelineReanchored`, or `BudgetExceeded { bytes }`.
- `TracksResolved` → `TracksResolved { generation: u32 }` — a monotonic counter (fixes a de-dup bug where the old count-based key could return to a previously-seen value after a remove-then-add).
- `ClockReference` is now `#[non_exhaustive]`, with a `DemuxEvent::clock_reference(...)` constructor.

**Breaking, applies across the board:** `DemuxEvent` itself and its `Sample`, `TrackRemoved`, `TrackAbandoned`, and `TracksResolved` variants are all `#[non_exhaustive]` now, as are `Provenance`, `PcrSample`, `SkippedTrack`, `TrackEncryption`, and `FragmentTrackData`. Every affected type gained a constructor (`DemuxEvent::{sample, track_removed, track_abandoned, tracks_resolved, discontinuity, clock_reference}`, `Provenance::new`, `PcrSample::new`, `SkippedTrack::new`, `TrackEncryption::new`, `FragmentTrackData::new`) so none became unconstructible from outside the crate. **Migration:** any exhaustive `match` on `DemuxEvent` needs a trailing `_ =>` arm; a struct-literal pattern match on the four non-exhaustive variants needs a trailing `..`.

### `Media`/`Track` demux leniency + mux strictness (B1–B4)

One strictness policy everywhere: **demux is lenient but loud, mux is strict but filterable**.

- `Fmp4Demux` no longer fails an entire file over one track it can't reconstruct (a QuickTime hint/chapter track, `c608`/`c708`, GoPro `gpmd`, …) — it skips that track and records it, named, in the new `Media::skipped: Vec<SkippedTrack>`, matching `ProgressiveDemux`'s existing per-track leniency.
- `CodecConfig::is_muxable_in_bmff()` (new, `pub`) now also covers `CodecConfig::Subtitle` (see below). **Migration:** pre-filter with `media.select_tracks_by(|t| t.spec.config.is_muxable_in_bmff())` before muxing a `Media` that mixes carriable and non-carriable tracks — `CmafMux`, `ProgressiveMux`, `Segmenter`, `LlSegmenter`, and `LlHlsSegmenter` all now reject a non-muxable track (via a check centralized in `build_init_segment`) rather than silently dropping it as some of the five used to.
- New `Error::UnmuxableSubtitleTrack { track_id, format }`.
- The `transmux` CLI (`cli` feature) now filters non-muxable tracks (with a stderr warning naming them) before every fMP4/CMAF-based output, so it no longer fails outright on an ordinary DVB multiplex.

### `HlsPackager` omits a timestamp-less track

A section-carried track (SCTE-35 `stream_type` `0x86`, DSM-CC, private sections) has `duration: None` on every sample; summing `unwrap_or(0)` over it used to render a literal `#EXTINF:0.000`, which RFC 8216 §4.3.2.1 defines as a real playback duration. Such a track is now left out of the media playlist (its content still reaches an output through the paths built for it — an inband `emsg`, an `EXT-X-DATERANGE`). A `Media` whose tracks are *all* timestamp-less is now `Error::InvalidInput` rather than an empty playlist.

### Two silent drops are now typed errors

- `Fmp4Demux` no longer silently skips a track whose sample entry it cannot reconstruct into a `CodecConfig`; it now returns `Error::UnsupportedSampleEntry { fourcc }`.
- `CmafMux` no longer silently filters `CodecConfig::Data` tracks out of segments; it now returns `Error::UnmuxableDataTrack { track_id, stream_type }`. Callers wanting the old best-effort behaviour must pre-filter: `media.select_tracks_by(|t| !matches!(t.spec.config, CodecConfig::Data { .. }))`.

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

`Fmp4Demux` now demuxes `stpp` and `wvtt` ISOBMFF sample entries into this variant. TS and PES paths for DVB subtitles remain on the roadmap. **There is no re-mux path yet** — `build_trak` rejects `CodecConfig::Subtitle` with `Error::UnsupportedCodec` (tracked as #753).

### Stage Trait Adoption

Eight demux/mux types now implement the `broadcast_common::Stage` trait (added in `broadcast-common` 8.7.0): `TsDemux`, `StreamingTsDemux`, `FlvDemux`, `StreamingFlvDemux` (demux); `CmafMux`, `IsoMp4Mux`, `TsMux` (mux); `RtpStreamDepacketiser` (reassembly). Zero-breaking-changes adoption — `Stage` is opt-in, coexisting with the existing pull/push APIs.

## New (non-breaking)

- **Shared segmentation primitives in `transmux::segmenter`**, now public because all four segmenters (`Segmenter`, `LlSegmenter`, `LlHlsSegmenter`, `StreamingTsHlsSegmenter`) use them and their behaviour is observable: `MediaClock`, `choose_anchor`/`is_anchor_capable`, and `MAX_PENDING_SAMPLES_PER_TRACK`. Previously each module carried its own copy and they had drifted apart.
- The `ac-4` ISOBMFF sample entry now demuxes to the existing `CodecConfig::Ac4` (the mux direction already worked) — a full mux ↔ demux round trip.

## Fixes

Three latent bugs from the original absolute-timing work:

1. **`TsDemux`: Audio sample timing was in the wrong timescale.** Audio samples were stored in 90 kHz PES-clock ticks while the track's timescale is its sample rate (e.g. 48 kHz for AAC), so `dts` deltas disagreed with `duration`. Audio `dts`/`pts` and `Track::start_decode_time` are now rescaled into the track's own timescale.
2. **`PsDemux`: AC-3 sample duration was always 0.** Now carries the intrinsic 1536-sample syncframe duration (ETSI TS 102 366 §4.1) and absolute time rescaled from PES stamps.
3. **`RtpDepacketiser`: RTP timestamp discarded.** The batch depacketiser emitted `duration: 0` / `is_sync: true` for every sample; it now carries the unwrapped absolute RTP media clock and the real IDR-derived sync flag.

Plus the five worst of 11 defects an aggregate review of the whole step-2 range found (issue refs where one exists):

4. **CRITICAL — a PMT that reclassifies a PID's codec no longer panics the process** (issue #641). PMT application used to leave stale `ConfigProbe`/`Carrier` state behind a codec reclassification (e.g. DVB's routine `stream_type` `0x06` gaining an `AC-3_descriptor`), reaching an `unreachable!()` on ordinary broadcast input. The PID is now torn down and re-registered on a codec change; every remaining panic-class site in `ts_demux` reachable from parsed input was converted the same way.
5. **CRITICAL — PSI `CRC_32` is now validated before any PAT/PMT is acted on** (ISO/IEC 13818-1 §2.4.4.1). A section failing CRC is dropped silently and disturbs nothing — previously a single bit error could destroy a live track or permanently hijack an elementary PID into PMT reassembly.
6. A PAT remapping a PMT PID, and a `current_next_indicator == 0` "next" PAT, are now handled instead of permanently freezing the PID binding.
7. A removed PID's in-flight payload is no longer replayed into a later re-added track with the same PID.
8. An elementary PID declared by two programs now survives one program's PMT dropping it (removal is refcounted).
9. The audio re-anchor threshold is now derived from real muxer behaviour (20 ms at 90 kHz) instead of one sample period, which previously made `TimelineReanchored` fire on essentially every AAC/MP2 access unit.
10. A gapped/discontinuous fMP4 now keeps its gap — `Fmp4Demux` re-seeds its cursor from each fragment's own `tfdt` instead of only the first.
11. `rescale_to_track` no longer clamps a legitimately negative audio anchor to zero.
12. **CRITICAL — a `duration` of `Some(0)`/`None` on the anchor track no longer stalls segmentation forever**, across all four segmenters — reachable on the shipped `StreamingFlvDemux` path (RTMP's first sample, and any two FLV tags sharing a timestamp).
13. **CRITICAL — a legal single-IDR/infinite-GOP stream is now bounded** instead of growing without limit, via the new `MAX_PENDING_SAMPLES_PER_TRACK` cap (`Error::InvalidInput` once exceeded; `flush`/`finish` still closes the trailing partial segment).
14. `LlSegmenter`/`LlHlsSegmenter` now anchor on any video codec, not just AVC (an HEVC-plus-AAC media used to anchor on audio and misreport `INDEPENDENT=YES`).
15. `TsHlsPackager` now places a timestamped section sample (e.g. an SCTE-35 cue) in the correct segment instead of always segment 0.
16. `ProgressiveDemux` now poisons permanently after a buffer-cap rejection instead of risking a parse with a hole in it.
17. `StreamingFlvDemux::demand()` now reports the real bytes still wanted instead of a constant 11, closing a pathological ~1.5M-call-per-tag driver behaviour.
18. `StreamingTsHlsSegmenter` now drains one ready queue instead of two that could drift apart.
19. `Segmenter::push` no longer loses the sample that triggered a failing cut.
20. `trickplay::derive_iframe_track` doc corrections (a removed field, an inaccurate timeline-coverage claim).
21. `TsMux` no longer drops a recognised codec's PMT `ES_info` descriptors on re-mux (issue #775) — language/subtitling descriptors used to survive only for *unrecognised* codecs, which was backwards. New deny-list policy (documented in the `ts_mux` module doc) explicitly drops `CA_descriptor` (scrambling signalling this muxer must not re-assert).
22. A new per-timed-track invariant test (`tests/demux_timing_invariant.rs`) checked against real fixtures across every demuxer, guarding the whole re-derive-from-a-lossy-clock class of bug.

## Security

This release also fixes two CENC (ISO/IEC 23001-7) confidentiality defects and two lower-severity CENC issues. Per the maintainers' decision, the disclosure lives in `CHANGELOG.md`'s `## [0.20.0]` → `### Security` section only — there is no separate security advisory and no patch backport to the 0.19.x line. If you used `CencEncryptor` with multi-track `cenc` content, or `IvGen::Explicit`, on transmux 0.16.0 through 0.19.0, read that section before anything else in this release.

## Compatibility

MSRV: 1.86 (unchanged). `no_std` + `alloc` posture: unchanged.

This release enables the media-plane architecture (splice, DVR, pacing) — steps 3 and beyond depend on it.
