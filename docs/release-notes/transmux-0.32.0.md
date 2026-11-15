# transmux 0.32.0

_Released 2026-11-15._

Minor-class release (0.31 -> 0.32). The pipeline's time surface is typed: durations are `core::time::Duration`, timescales are `Timescale`, sequence numbers are the playlist's `MediaSequence`/`PartIndex`, a PCR is the parsed `mpeg_ts::Pcr`, and a sample can no longer be built with an invented span. The IR constructors (`Sample::new`, `from_annexb`, `from_raw`, `StreamingTsDemux::feed`) are fallible where they used to fabricate a value.

Version: 0.31.0 -> 0.32.0.

## Breaking changes and migration

### `Sample` construction: use `Sample::timed`/`timed_annexb`/`timed_raw`

`Sample::new`, `from_annexb` and `from_raw` return `Result<Self>`: a PTS with no DTS is refused rather than fabricating `dts = pts`, and a zero span with no timestamps is refused rather than swallowed. A caller that has **both** timestamps uses the infallible `Sample::timed`, `Sample::timed_annexb` or `Sample::timed_raw`; `Sample::from_duration` takes a `NonZeroU32`.

```rust
// before
let sample = Sample::new(data, dts, pts, duration, is_sync);
// after
let sample = Sample::timed(data, dts, pts, duration);          // both timestamps known
let sample = Sample::new(data, Some(dts), Some(pts), duration, is_sync)?;   // fallible form
let sample = Sample::from_duration(data, NonZeroU32::new(ticks).unwrap(), is_sync)?;
```

### `SampleTiming` is an enum, read through `dts()`/`pts()`/`duration()`

A sample with a clock carries `SampleTiming::Timed { dts, pts, duration: Option<NonZeroU32> }`; a sample with only a span carries `SampleTiming::DurationOnly { duration: NonZeroU32 }`; a section-carried track stays `timing: None`. `SampleTiming::from_options` is removed (use `SampleTiming::timed`, `SampleTiming::timed_with_duration` or `SampleTiming::duration_only`), `Sample.timing` is one optional field rather than separate `dts`/`pts`/`duration` fields, and a sample's clock and span are read with the `Sample::dts()`, `Sample::pts()` and `Sample::duration()` accessors (each an `Option`, read through the `SampleTiming`).

```rust
// before
sample.dts = Some(dts);
let pts = sample.pts;
let span = sample.duration;
// after
let sample = Sample::timed(data, dts, pts, None);
let pts = sample.pts();
let span = sample.duration();
```

### A span is never invented or clamped

`SampleTiming::with_duration` returns `Self` (a positive span is exactly what it sets), `Sample::rescale_timescale` returns `Result<Self>` and refuses a span that no longer fits 32 bits instead of clamping it to one tick or `u32::MAX`, and `Sample::composition_offset` returns `Result<Option<i32>>` via `checked_sub` + `i32::try_from` instead of silently clamping an over-range offset (the new `Error::TimestampSpanOverflow` and `Error::CompositionOffsetRange { offset }` replace two `Error::InvalidInput` strings). The fragment validator gained `SegmentOrigin`: a zero span on a video track of a segment this crate wrote means the IR never stated one and is `media.sample.timing` (WARNING), while any other track's zero stays `media.sample.zero-duration` (ERROR); `validate_media_segment` and `validate_cmaf_track` take the origin.

```rust
// before
let timing = SampleTiming::from_options(Some(dts), Some(pts), Some(ticks));   // a struct
let offset = sample.composition_offset();
let issues = validate_media_segment(&bytes);
// after
let timing = Some(SampleTiming::timed_with_duration(dts, pts, ticks));      // an enum
let offset = sample.composition_offset()?;
let issues = validate_media_segment(&bytes, SegmentOrigin::External);
```

### Durations are `Duration`

`Segmenter::new`, `ll_hls::LlHlsSegmenter::with_part_target`/`with_part_target_at`, `ll_dash::LlSegmenter::new`, `repackage::Repackage::new` and `ll_dash::LlDashPackager::new` take their target/segment/chunk duration as a `Duration`; `ll_hls::PartInfo::duration`, `ll_hls::SegmentInfo::duration` and `ts_hls::TsSegment::duration` are `Duration`s; `LlHlsSegmenter::part_target_secs()`/`part_target_ms()` become one `part_target() -> Duration`; `LlDashPackager::segment_duration_secs`/`chunk_duration_secs` are `segment_duration`/`chunk_duration` and `availability_time_offset()` returns a `Duration`; `SmoothPackager::target_duration_secs: u32` is `target_duration: Duration`; and the `set_start_time_ns` methods are removed in favour of `set_start_time(Duration)`. `TimelineJumpConfig::backward`/`forward_slack`, `TsMux::with_psi_interval`/`psi_interval` (`Option<Duration>`) and `LlDashPackager::latency_target` are `Duration`s too. Wire formats are unchanged (a `Duration` renders the same decimal seconds at the encode edge).

```rust
// before
let segmenter = Segmenter::new(tracks, ts, 2.0)?;
let segmenter = LlHlsSegmenter::with_part_target(tracks, ts, 1.0, 334)?;
let target_ms = segmenter.part_target_ms();
segmenter.set_start_time_ns(ns);
// after
let segmenter = Segmenter::new(tracks, ts, Duration::from_secs(2))?;
let segmenter = LlHlsSegmenter::with_part_target(tracks, ts, Duration::from_secs(1), Duration::from_millis(334))?;
let target = segmenter.part_target();
segmenter.set_start_time(Duration::from_nanos(ns));
```

### Timescales are `Timescale`

`SegmentIndexBox::timescale`, `rtp::RtpStreamTrack::{clock_rate, new}`, `SdpTrack::clock_rate`, `TrackAligner::new` (its `(TrackId, bool, Timescale)` tuples), `DemuxEvent::ClockReference::clock_hz` and `clock_reference`, `IR_TIMESCALE`, `track_spec_from_quality_level`, `dash::TrickModeRepr::timescale`, `LlHlsSegmenter::with_part_target_at`, `TrackSpec.timescale` and `Track.timescale()` take or hold a `Timescale`. A zero `sidx.timescale` is `Error::InvalidValue`, and `CencDecryptor::demux` refuses a zero or missing `mvhd`/`mdhd` timescale (typed error) instead of falling back to 1000 Hz or the movie timescale. `VIDEO_CLOCK_TIMESCALE` and `SMOOTH_DEFAULT_TIMESCALE` are added.

```rust
// before
let spec = TrackSpec { timescale: 90_000, .. };
let rate = track.timescale();
// after
let spec = TrackSpec { timescale: Timescale::from_nonzero(NonZeroU32::new(90_000).unwrap()), .. };
let rate: Timescale = track.timescale();
let hz: u32 = rate.get();
```

### A PCR keeps its wire value

`PcrSample::pcr_27mhz: u64` is `PcrSample::pcr: mpeg_ts::Pcr` and `PcrSample::new` takes that `Pcr`: a capture's off-clock extension in `300..=511` now survives into the IR instead of being lost to a 27 MHz integer. `DemuxEvent::ClockReference` carries the parsed `pcr: Option<mpeg_ts::Pcr>` (`clock_reference(ticks, pcr, ..)` takes it), so every observation keeps its 33-bit base and 9-bit extension; the side queue that dropped later off-clock extensions is gone.

```rust
// before
let sample = PcrSample::new(pcr.as_27mhz(), pid, index, discontinuity);
let ticks = sample.pcr_27mhz;
// after
let sample = PcrSample::new(pcr, pid, index, discontinuity);
let ticks = sample.pcr_27mhz();   // None for an off-clock extension
```

### Sequence numbers are the playlist's types

`HlsPackager::media_sequence: u64` and `TsHlsPackager::media_sequence: u64` are `msn: broadcast_hls::MediaSequence`; `ll_hls::PartCutReady::msn` is a `MediaSequence` and its `part_index` a `PartIndex`, as are `PartInfo::part_index`, `PartCut::part_index` and `LlHlsSegmenter::follow_part`'s `segment_seq`/`part_index`.

```rust
// before
let packager = HlsPackager { media_sequence: 42, .. };
// after
let packager = HlsPackager { msn: MediaSequence::new(42), .. };
```

### `StreamingTsDemux::feed` returns `Result`

`StreamingTsDemux::feed(&[u8])` returns `Result<()>` instead of `()`, so a demux failure is reported at the caller rather than swallowed.

```rust
// before
demux.feed(&bytes);
// after
demux.feed(&bytes)?;
```

### The remaining public types are `#[non_exhaustive]`

`EncryptConfig`, `TimelineJumpConfig`, `ll_hls::PartInfo` and `ll_hls::SegmentInfo` are `#[non_exhaustive]`; build them with `EncryptConfig::new(scheme, kid, subsample)` + `with_*`, `TimelineJumpConfig::default()` + `with_*`, and `PartInfo::new(bytes, duration, segment_seq, part_index)`/`SegmentInfo::new(bytes, duration, segment_seq)` + `with_*`. The decoded codec records (`AvcSpsInfo`, `HevcSpsInfo`, `VvcSpsInfo`, `AudioSpecificConfig`, `Ac3SyncframeInfo`, `Ec3SyncframeInfo`, `DtsCoreFrameInfo`, `NclxColourInfo`, `AvcFieldInfo`, `DecoderSpecificInfo`) are `#[non_exhaustive]` too; `AvcFieldInfo::new(bottom, frame_num, is_reference, idr)` and `NclxColourInfo::new(colour_primaries, transfer_characteristics, matrix_coefficients, full_range_flag)` build the two a caller composes itself. `smooth_fragment::fragment_track_id` returns the IR's `TrackId` instead of a raw `u32`.

## Fixes

- `validate_media_segment` reports a timed sample with no stated span on a segment this crate wrote as `media.sample.timing`, the same finding `validate_cmaf_track` makes, without needing the init segment. A segment that carries no video-typed track at all keeps `media.sample.zero-duration`.
- `StreamingFlvDemux` reports a publish whose tag timestamps step backwards on a track as the new `FlvError::TimestampBackwards { previous, next }` (it used to emit the previous sample with a silently zero-length span) and a forward delta too large for a span as `FlvError::SampleDurationTooLarge { ticks }`, instead of an unchecked `i64` subtract that panicked a debug build.
- PSI parsing/building in `ts_demux`/`ts_mux` goes through `dvb-si`'s `PatSection`/`PmtSection` instead of hand-rolled copies, and PTS/DTS reads/writes through `mpeg-pes`'s checked codec; the demux output and the mux bytes are unchanged (every golden holds).

## Added

- `VIDEO_CLOCK_TIMESCALE`, `SMOOTH_DEFAULT_TIMESCALE`, `SegmentOrigin`, `Error::TimestampSpanOverflow`, `Error::CompositionOffsetRange`, `FlvError::TimestampBackwards`, `FlvError::SampleDurationTooLarge`.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-dash` 0.1 -> 0.2, `broadcast-hls` 0.7 -> 0.8, `broadcast-smooth` 0.1 -> 0.2, `cc-data` 0.9 -> 0.10, `container-probe` 0.4 -> 0.5.
- `dvb-si` 14 -> 15, `mp4-emsg` 0.7 -> 0.8, `mpeg-pes` 0.8 -> 0.9, `mpeg-ps` 0.8 -> 0.9, `mpeg-ts` 0.8 -> 0.9.
- `rtmp-runtime` 0.10 -> 0.11, `rtp-packet` 0.7 -> 0.8, `rtsp-runtime` 0.11 -> 0.12, `scte35-splice` 6 -> 7.

---

Published from tag `transmux-v0.32.0`.
