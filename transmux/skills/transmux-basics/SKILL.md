---
name: transmux-basics
description: Use when writing Rust code against the transmux crate to demux a media container (MPEG-2 TS, fMP4/CMAF, progressive MP4, MPEG-PS, WebM/Matroska, FLV, RTMP) into its neutral Media/Track/Sample IR, mux that IR to another container (CMAF/fMP4, progressive MP4, TS, HLS, DASH, Smooth, MKV, FLV, RTMP), repackage or resegment it, decrypt CENC/CBCS, splice/rebase its timeline, or validate fMP4/CMAF structure.
---

# transmux basics

`transmux` is an any-to-any media **container** muxing hub. Every input is
demuxed once into one neutral IR (`Media` → `Track` → `Sample`), and that IR
feeds every output. Coded samples pass through **opaque**; the crate parses
codec *config headers* only (SPS/PPS/VPS, AudioSpecificConfig, BSI, …) to build
boxes and derive metadata. It never encodes or decodes media.

## Scope

**It does:** demux TS / fMP4 / progressive MP4 / MPEG-PS / WebM / FLV / RTMP
into the IR; mux the IR to CMAF, progressive MP4, TS, MKV, HLS (CMAF, TS,
low-latency), DASH (static, live, low-latency), Smooth, FLV, RTMP; streaming
segmenters; IR timeline stages (rebase, offset, gap, trick-play, splice);
CENC/CBCS protection stages; an fMP4/CMAF conformance validator; and the
`cli`-gated `transmux` packager binary.

**It does not:** encode/decode, build ABR ladders, acquire DRM licences, or
serve HLS/DASH over HTTP (`hls-runtime`, `multimux`). HLS **playlist syntax**
lives in `broadcast-hls`; DASH MPD syntax in `broadcast-dash`.

**The one any-to-any exception:** WebM VP8 and Vorbis have no ISOBMFF sample
entry here, so they cannot enter the fMP4/CMAF/TS/DASH/HLS/Smooth path
(`Error::UnsupportedCodec`); only WebM→WebM/MKV round-trips them.

## The core loop: demux to the IR, drive a mux stage

Streaming demux stages implement `broadcast_common::Stage` and emit `DemuxEvent`s. Every
muxer, segmenter and protection pass is a
`Stage<In<'a> = MuxInput, Out = MuxOutput>` over `transmux::mux_stage`:
`MuxInput::{Track, Sample, Discontinuity, Flush}` in,
`MuxOutput::{Bytes, BytesAt, Init, Segment, Text}` out. `drive_media` is the
bridge for a caller that still holds a whole `Media`.

<!-- Snippet map: this block is copied verbatim from `transmux/README.md`
     "Quick start", which `lib.rs` compiles as a doctest through
     `#[doc = include_str!("../README.md")]`. -->

```rust
use broadcast_common::Stage;
use transmux::{CmafMuxStage, DashPackager, FragmentCut, Media, TsDemux, TsMuxStage};

let ts = std::fs::read("../fixtures/ts/h264-aac.ts").unwrap();
let media: Media = TsDemux::new().unpackage(&ts[..]).unwrap();
assert!(media.tracks.len() >= 2, "an H.264 video and an AAC audio track");

let mut stage = CmafMuxStage::new(media.movie_timescale, FragmentCut::OnFlush, 1);
let outputs =
    transmux::mux_stage::drive_media(&mut stage, &media, broadcast_common::Timestamp::ZERO)
        .unwrap();
let cmaf: usize = outputs
    .iter()
    .map(|output| match output {
        transmux::mux_stage::MuxOutput::Init(b) | transmux::mux_stage::MuxOutput::Bytes(b) => b.len(),
        transmux::mux_stage::MuxOutput::Segment(s) => s.bytes.len(),
        _ => 0,
    })
    .sum();
assert!(cmaf > 1000);

let mut log = transmux::segment_log::SegmentSummaryLog::new();
for output in &outputs {
    if let transmux::mux_stage::MuxOutput::Segment(segment) = output {
        log.push(segment).unwrap();
    }
}
let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
let mpd: String = DashPackager::default().render(&specs, &log).unwrap();
assert!(mpd.contains("<MPD"));

// And back: the TS muxer stage is the byte-level inverse of TsDemux.
let mut stage = transmux::TsMuxStage::new(Some(transmux::DEFAULT_PSI_INTERVAL));
let out = transmux::mux_stage::drive_media(
    &mut stage,
    &media,
    broadcast_common::stage::Timestamp::ZERO,
)
.unwrap();
let mut ts_again = Vec::new();
for item in out {
    if let transmux::mux_stage::MuxOutput::Bytes(chunk) = item {
        ts_again.extend_from_slice(&chunk);
    }
}
let again = TsDemux::new().unpackage(&ts_again[..]).unwrap();
assert_eq!(again.tracks.len(), media.tracks.len());
```

The types each direction speaks:

- **Demux:** batch `TsDemux`, `FlvDemux`, `RtmpDemux`, `RtpDepacketiser`
  (`StreamingTsDemux`, fed bytes, `poll_event`),
  `StreamingFlvDemux`, `StreamingPsDemux`, `Mp4Demux` (a `Stage` over
  `Mp4DemuxConfig`/`Mp4Input::{At, Next, SegmentEnd, Eof}`) and
  `MatroskaDemux` (a `Stage` over
  `MatroskaDemuxConfig`/`MatroskaInput::{At, Next, Eof}`).
- **Mux stages:** `CmafMuxStage` (`FragmentCut::OnFlush` or
  `FragmentCut::AnchorKeyframe`), `ProgressiveMuxStage` (two-pass, with
  `PlanPass`), `TsMuxStage`, `MkvMuxStage`, `FlvMuxStage`, `RtmpMuxStage`,
  `RtpPacketiserStage`.
- **Segmenters:** `Segmenter` (CMAF), `StreamingTsHlsSegmenter` (classic TS-HLS
  plus its rolling `playlist()`), `LlHlsSegmenter` (LL-HLS parts),
  `LlSegmenter` (LL-DASH chunks), `SmoothSegmenter`.
- **Manifest renderers** never see a sample: they take a track-spec slice and a
  `SegmentSummaryLog` and return a `String` — `HlsPackager`,
  `DashPackager`, `LlDashPackager`, `SmoothPackager`, all via
  `.render(&specs, &log)`.

## The IR

<!-- Snippet map: `print_track_summary` from
     `transmux/examples/transmux_hub.rs`, a compiled example. -->

```rust
fn print_track_summary(media: &Media) {
    println!(
        "IR: {} track(s), movie timescale {} ticks/s",
        media.tracks.len(),
        media.movie_timescale.get()
    );
    for track in &media.tracks {
        println!(
            "  track {:>2}: {:<28} {:>5} samples @ {} Hz",
            track.track_id(),
            describe(track.config()),
            track.samples.len(),
            track.timescale().get(),
        );
    }
}
```

- `Media` — `tracks`, `movie_timescale` (`mvhd`), plus the TS-only `pcr`
  timeline and the diagnostics `skipped`, `degraded`, `degraded_at`. It is
  `#[non_exhaustive]`: construct with `Media::new(tracks, movie_timescale)`.
- `Track` — `spec`, `samples` (decode order), `start_decode_time` (the first
  sample's absolute DTS anchor). Construct with `Track::new`/`Track::new_at`.
- `TrackSpec` — `track_id: TrackId`, `timescale: Timescale`, `config:
  CodecConfig`, `protection: Option<TrackProtection>` (`None` = cleartext),
  plus the TS source fields (`source_pid`, `es_info_descriptors`,
  `program_number`), `display` (geometry/SAR/colour), `hdr`, `dovi` and
  `hdr10plus`. Build with
  `TrackSpec::new(TrackId::new(1), Timescale::try_new(90_000)?, config)`.
- `Sample` — `data: bytes::Bytes` (**length-prefixed** NAL data for AVC/HEVC,
  raw frame for audio), `timing: Option<SampleTiming>` (`Timed { dts, pts,
  duration }` absolute in the track timescale, or `DurationOnly`; `None` only
  for a section-carried sample), `flags` (`SYNC`), `display`, `provenance`,
  `protection`. Build with `Sample::new(data, dts, pts, duration, is_sync)`,
  `Sample::timed(..)`, `Sample::from_annexb(..)` or `Sample::from_raw(..)`.
- `CodecConfig` — one variant per codec (`Avc`, `Hevc`, `Vvc`, `Av1`, `Vp9`,
  `Mpeg2Video`, `Aac`, `Ac3`, `Eac3`, `Ac4`, `Dts`, `Opus`, `Flac`, `Vorbis`,
  `MpegAudio`, `MpegH`, `Vp8`, `Subtitle`, `Data`, …). It is `#[non_exhaustive]`.

Feeding hand-built samples to `build_init_segment` / `build_media_segment`
(`FragmentTrackData::new(track_id, base_media_decode_time, &samples)`) is the
samples-in path; `CmafMuxStage` emits exactly that pair, one fragment at a time.

## Streaming output

<!-- Snippet map: the doctest on `transmux::segmenter::Segmenter`. -->

```rust
let mut seg = Segmenter::new(vec![spec()], broadcast_common::timescale::Timescale::try_new(1000).expect("non-zero timescale"), core::time::Duration::from_secs(2)).unwrap();
let init = seg.init_segment().unwrap();      // ftyp + moov
assert_eq!(&init[4..8], b"ftyp");
let sample = |sync| MuxInput::Sample { track: transmux::TrackId::new(1), sample: au(sync) };
seg.feed(sample(true), Timestamp::ZERO).unwrap();      // keyframe
seg.feed(sample(false), Timestamp::ZERO).unwrap();
assert!(seg.poll().is_none());               // still buffering: no cut yet
seg.feed(MuxInput::Flush, Timestamp::ZERO).unwrap();   // trailing segment
assert!(seg.poll().is_some());
```

`Segmenter` cuts on the anchor track's sync samples (first video track, else the
first anchor-capable track). `LlHlsSegmenter` emits parts before a segment
closes; `StreamingTsHlsSegmenter` keeps a rolling playlist window.

A live demux drains its event queue: `StreamingTsDemux::feed(bytes)` then
`poll_event()` until `None` (the queue is unbounded and only `poll_event`
empties it), then `finish()` at end of stream. `Mp4Demux` is driven with
`Mp4Input` and drained with `Stage::poll`, which yields `DemuxEvent`s.

## Repackaging (select / trim / resegment)

<!-- Snippet map: the doctest on `transmux::repackage::Repackage`. -->

```rust
let out = Repackage::new(core::time::Duration::from_secs(2)) // 2-second target segments
    .select_tracks(&[0])               // keep only track 0 (video)
    .run(fmp4)?;
let _init = out.init_segment;
let _media = out.media_segments;       // Vec<Vec<u8>>
```

`Repackage::run` drives an `Mp4Demux` and a `Segmenter` in bounded memory.
`.select_tracks(&[indices])` keeps a chosen subset and `.trim(start, end)` is a
window in the **movie** timescale, snapped back to a keyframe; both are honoured
during `run`. `RepackageOutput::to_contiguous()` joins the segments into one
fragmented-MP4 stream.

## Timeline stages

`Sample` timing is absolute, so the transforms shift absolute `dts`/`pts` and
never reconstruct a running sum. 33-bit MPEG wrap is already unrolled at the
demux edge.

<!-- Snippet map: the `transmux::rebase` module doctest. -->

```rust
use transmux::{GapStage, OffsetStage, RebaseOrigin, RebaseStage, TrackId};

// Exact per-track origins, as a sample plan supplies them.
let rebase = RebaseStage::with_origin(
    RebaseOrigin::new()
        .with(TrackId::new(1), 90_000)
        .with(TrackId::new(2), 48_000),
);
// Or discover one common origin across the announced tracks, holding at
// most a two-second window and one mebibyte of output.
let discovered =
    RebaseStage::start_window(core::time::Duration::from_secs(2), 1_048_576);
let shifted = OffsetStage::new(48_000);
let gapped = GapStage::new(9_000, 42);
```

`RebaseStage` is fed `MuxInput` (a track announcement carries the timescale its
origin is converted into); `OffsetStage` and `GapStage` are fed
`(TrackId, Sample)`. `TrickPlayStage` derives an I-frame-only track and
announces it through `MuxInput::Track`.

## Splice / concat (SSAI)

A `SplicePlan` holds no samples: per-track cut indices and tick shifts, plus
the `SplicePoint`s to signal downstream as `#EXT-X-DISCONTINUITY` or a DASH
`emsg`. `SpliceStage` applies it one sample at a time.

<!-- Snippet map: the `transmux::splice` module doctest. -->

```rust
use transmux::{Media, SplicePlan, SpliceStage};

// Play `base` up to 90 000 presentation ticks (1 s of a 90 kHz track),
// insert `ad`, then resume `base` shifted by `ad`'s span.
let plan = SplicePlan::insert(base, ad, 90_000)?;
let mut stage = SpliceStage::new(plan);
// Feed (TrackId, Sample) pairs in the plan's order, then finish and poll.
let _ = &mut stage;
```

`SplicePlan::concat(a, b)` is the append case. The `ad`'s first sample of every
matched track must be a sync sample (`Error::InvalidInput` otherwise);
`SplicePlan::insert` snaps the requested time back to the base video track's
preceding sync sample (`snap_to_preceding_sync`). Built with
`SpliceStage::switching(max_gop_bytes)` instead of a plan, the stage holds at
most one GOP, waits for a sync point, and applies `splice()` on demand
(`Error::SpliceGopOverflow` past the cap). Full SCTE-35 cue → splice → HLS/DASH
wiring: `transmux/examples/ssai_ad_stitch/`.

## CENC / CBCS (feature `cenc`, on by default)

Protection is a stage pair: `CencEncryptStage` ciphers each sample and fills
`TrackSpec::protection` plus `Sample::protection`; `CencDecryptStage` takes a
`KeyMap` and clears them. The demuxer already surfaces a protected track's
`tenc` defaults and each sample's IV/subsample map, so decryption needs no
second file walk.

<!-- Snippet map: the `transmux::cenc_encrypt` module doctest. -->

```rust
use transmux::{CencEncryptStage, CencScheme, EncryptConfig, SubsamplePolicy};

let cfg = EncryptConfig::new(CencScheme::Cenc, [0x11; 16], SubsamplePolicy::Video);
// One stage per content key: its IV counter continues across every feed.
let stage = CencEncryptStage::new([0x22; 16], cfg)?;
```

`cenc` (AES-CTR) and `cbcs` (AES-CBC pattern) are supported; `cens`/`cbc1` are
not (`Error::UnsupportedCencScheme`). `CencDecryptStage::new(KeyMap)` is the
inverse. The fragment writer emits `senc`/`saiz`/`saio` from the samples
themselves; `cenc_segment::protect_init_segment` splices `sinf`/`tenc` into the
init segment. DASH `ContentProtection` is derived from `TrackSpec::protection`;
HLS `EXT-X-KEY` comes from `broadcast_hls::cenc_ext_x_key`. HLS Sample-AES and
whole-segment AES-128 are `SampleAesStage` (`sample-aes` feature).

## Conformance validation

`validate_init_segment(bytes)`, `validate_media_segment(bytes)` and
`validate_cmaf_track(init, &[segment, …])` return `Vec<ConformanceIssue>` —
empty means conformant. Each issue is `{ severity: Severity::Error | Warning,
code: &'static str, message: String }` with a stable dotted code such as
`media.tfdt.missing` or `track.tfdt.discontinuity`. Structural only: no media
decode, malformed input yields issues rather than panics.

## CLI (`cli` feature)

<!-- Snippet map: copied verbatim from `transmux/README.md` "Command-line
     packager"; the same invocations are exercised by `tests/it/cli*.rs`. -->

```console
$ cargo install transmux --features cli
$ transmux in.ts -o out.cmaf                                  # TS to CMAF, format inferred
$ transmux in.mp4 -o out.m3u8 -f hls --segment-duration 4     # MP4 to CMAF-HLS, 4 s segments
$ transmux in.ps -o out.mpd -f dash --ll                      # PS to low-latency DASH
$ transmux enc.mp4 -o dec.mp4 -f progressive --decrypt --key <KID>:<KEY>
$ cat in.ts | transmux - -o out.cmaf -f cmaf             # a forward-only stdin source streams
```

`-f` takes `cmaf | hls | ts-hls | dash | ts | progressive` (default: inferred
from `-o`'s extension). Every input is streamed through its container's stage;
Every supported input (TS, MP4/CMAF, MPEG-PS, Matroska/WebM, FLV) streams; no whole-file cap exists. An opaque
data track crosses container families only when mapped with
`--data-identity <track>=ts:<stream_type>[:<descriptors-hex>]` (or
`mp4:<handler>:<sample-entry-hex>`, `matroska:<codec_id>[:<codec-private-hex>]`):
the CLI builds the `DataIdentityMap` a `DataIdentityStage` applies, and without
it another family's muxer refuses the track with `Error::DataIdentityMismatch`.
Input detection uses `container-probe`; an inconclusive probe is
`unknown input container`, so the format must be given explicitly.

## Features

| Feature | Default | Effect |
|---|---|---|
| `std` | on | `std::error::Error`, and every XML path (DASH/LL-DASH MPD, Smooth, PlayReady WRMHEADER) — off → `#![no_std]` + `alloc`, no `DashPackager`/`SmoothPackager`/`LlDashPackager`/SDP |
| `cenc` | on | CENC/CBCS decrypt + encrypt; off → no `CencEncryptStage`/`CencDecryptStage` |
| `serde` | off | `Serialize` on box and IR types |
| `sample-aes` | off | HLS Sample-AES + whole-segment AES-128 (implies `cenc`) |
| `motion-imagery-metadata` | off | typed MISB ST 0601 values and the `payload::St0601Decoder` plug-in |
| `cli` | off | the `transmux` binary (implies `std`) |

`no_std`: `transmux = { version = "…", default-features = false, features = ["cenc"] }`.

## Pitfalls

- **It never transcodes.** A codec the output container cannot carry is
  *rejected* (`CmafMuxStage` and every fMP4/CMAF entry point with
  `Error::DataIdentityMismatch`/`UnmuxableSubtitleTrack`, the WebM-only
  VP8/Vorbis with `UnsupportedCodec`, FLV/RTMP/Smooth with their own codec
  error) or *dropped* (`TsMuxStage` drops a track with no TS mapping). Filter
  first with `.select_tracks(&[..])` on `Repackage`, or by dropping the track's
  events before the muxer;
  plan the codec set from the README input × output matrix, not by trial and
  error.
- **`Sample::data` for AVC/HEVC is length-prefixed, not Annex B.** Convert with
  `Sample::from_annexb(..)`; feeding Annex B straight to a muxer produces
  garbage NAL framing with no error.
- **Timestamps are absolute, and never fabricated.** `dts`/`pts` live on
  `Sample::timing` as absolute ticks in the *track's own* timescale (`90000`
  video vs `44100` audio); `None` only for section-carried tracks (SCTE-35,
  DSM-CC, private sections). Rescaling between timescales is
  `Sample::rescale_timescale(from, to)`. Do not re-derive timing as
  `start_decode_time + Σ duration` — a per-fragment `tfdt` reseed makes that
  wrong; read each sample's `timing.dts()`.
- **`Media`, `Track`, `TrackSpec`, `Sample`, `SampleTiming` are
  `#[non_exhaustive]`.** Only the constructors (`Media::new`,
  `Track::new`/`new_at`, `TrackSpec::new`, `Sample::new`/`timed`/`from_annexb`/
  `from_raw`) exist outside the crate; matching needs a trailing `..`.
- **A segment is `SegmentOut { bytes, summary, tracks }`.** A manifest renders
  from `SegmentSummaryLog` alone; it never sees a `Sample`. A segmenter that
  also maintains a playlist exposes it directly
  (`StreamingTsHlsSegmenter::playlist`), not through a `MuxOutput`.
- **Drain `StreamingTsDemux::poll_event` after every `feed`.** The queue is
  unbounded; it only shrinks through `poll_event` (`with_max_degraded_events`
  caps the `InputDegraded` diagnostics, not the events).
- **Damage is reported, not fatal.** A one-shot demux that skips a damaged unit
  records it in `Media::degraded` (`InputDegradation::MalformedUnit`; TS adds
  `SyncLoss`, `TruncatedPacket`, `ContinuityGap`, …) with `Media::degraded_at`
  carrying PID and packet index. Check those fields instead of assuming a clean
  input or treating `Ok` as lossless.
- **`FlvDemux` silently skips non-AVC video and non-AAC audio** (no track, no
  error); `TsDemux` never drops a stream (unknown stream types become opaque
  `Data`). Different demuxers treat unknown codecs differently.
- **VP8/Vorbis are WebM/MKV-only.** Any fMP4/CMAF/TS/DASH/HLS/Smooth mux of
  those tracks fails.
