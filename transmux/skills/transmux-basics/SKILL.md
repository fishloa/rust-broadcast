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
segmenters; IR timeline transforms (rebase, offset, gap, splice/concat);
CENC/CBCS decrypt and encrypt; an fMP4/CMAF conformance validator; and the
`cli`-gated `transmux` packager binary.

**It does not:** encode/decode, build ABR ladders, acquire DRM licences, or
serve HLS/DASH over HTTP (`hls-runtime`, `multimux`). HLS **playlist syntax**
lives in `broadcast-hls`; DASH MPD syntax in `broadcast-dash`.

**The one any-to-any exception:** WebM VP8 and Vorbis have no ISOBMFF sample
entry here, so they cannot enter the fMP4/CMAF/TS/DASH/HLS/Smooth path
(`Error::UnsupportedCodec`); only WebM→WebM/MKV round-trips them.

## The core loop: `Unpackage` in, `Package` out

The two `broadcast_common` traits are the whole hub: `Unpackage` (container →
IR) and `Package` (IR → container). Demux once, feed the same `Media` to every
output.

```rust
use broadcast_common::{Package, Unpackage};
use transmux::{CmafMux, DashPackager, Media, TsDemux, TsMux};

let ts = std::fs::read("../fixtures/ts/h264-aac.ts").unwrap();
let media: Media = TsDemux::new().unpackage(&ts[..]).unwrap();
assert!(media.tracks.len() >= 2, "an H.264 video and an AAC audio track");

let cmaf: Vec<u8> = CmafMux::default().package(&media).unwrap();
assert!(cmaf.len() > 1000);

let mpd: String = DashPackager::default().package(&media).unwrap();
assert!(mpd.contains("<MPD"));

// And back: TsMux is the byte-level inverse of TsDemux.
let ts_again: Vec<u8> = TsMux::default().package(&media).unwrap();
let again = TsDemux::new().unpackage(&ts_again[..]).unwrap();
assert_eq!(again.tracks.len(), media.tracks.len());
```
(`transmux/README.md`, section "Quick start" — the crate README's one
integration-tested example)

Demuxers and muxers (each one trait impl): `TsDemux`, `Fmp4Demux`,
`ProgressiveDemux`, `PsDemux`, `WebmDemux`, `FlvDemux`, `RtmpDemux` /
`CmafMux`, `ProgressiveMux` (`ProgressiveMux::new(faststart)`), `TsMux`,
`MkvMux`, `FlvMux`, `RtmpMux`, `HlsPackager`, `TsHlsPackager`, `DashPackager`,
`LlDashPackager` (`std`), `SmoothPackager`. `DashPackager`'s and `HlsPackager`'s
`Package::Output` is `String` (a manifest/playlist), not bytes.

The whole walkthrough over a real WebM (`WebmDemux` → one IR → `DashPackager`
plus `CmafMux`) is `transmux/examples/transmux_any_to_any.rs`; TS → CMAF is
`transmux/examples/transmux_hub.rs`.

## The IR

```rust
let media: Media = TsDemux::new().unpackage(&ts[..]).unwrap();
for track in &media.tracks {
    println!(
        "track {:>2}: {:>5} samples @ {} Hz",
        track.track_id(),
        track.samples.len(),
        track.timescale(),
    );
}
```
(`transmux/examples/transmux_hub.rs`, `print_track_summary`)

- `Media` — `tracks`, `movie_timescale` (`mvhd`), plus the TS-only `pcr`
  timeline and the diagnostics `skipped`, `degraded`, `degraded_at`. It is
  `#[non_exhaustive]`: construct with `Media::new(tracks, movie_timescale)`.
- `Track` — `spec`, `samples` (decode order), `start_decode_time` (first
  sample's absolute DTS anchor), `encryption` (`None` = cleartext).
- `TrackSpec` — `track_id: TrackId`, `timescale: u32`, `config: CodecConfig`,
  plus TS source fields (`source_pid`, `es_info_descriptors`, `program_number`),
  `display` (geometry/SAR/colour) and `hdr`/`dovi`. Build with
  `TrackSpec::new(TrackId::new(1), 90_000, config)`.
- `Sample` — `data: bytes::Bytes` (**length-prefixed** NAL data for AVC/HEVC,
  raw frame for audio), `dts`/`pts: Option<i64>` **absolute** in the track
  timescale, `duration: Option<u32>`, `flags.is_sync`, `display`. Build with
  `Sample::new(data, Some(dts), Some(pts), Some(duration), is_sync)`; an Annex B
  access unit goes through `Sample::from_annexb(..)` first.
- `CodecConfig` — one variant per codec (`Avc`, `Hevc`, `Vvc`, `Av1`, `Vp9`,
  `Mpeg2Video`, `Aac`, `Ac3`, `Eac3`, `Ac4`, `Dts`, `Opus`, `Flac`, `Vorbis`,
  `MpegAudio`, `MpegH`, `Vp8`, `Subtitle`, `Data`, …). It is `#[non_exhaustive]`.

Feeding hand-built samples to `build_init_segment` / `build_media_segment`
(`FragmentTrackData::new(track_id, base_media_decode_time, &samples)`) is the
samples-in path; `CmafMux` is exactly that pair over a whole `Media`.

## Streaming, not just batch

For a live or large source use the incremental demuxer:
`StreamingTsDemux::feed(bytes)` then drain `StreamingTsDemux::poll_event()` —
the event queue is unbounded and only `poll_event` empties it; call
`finish()` at end of stream. Batch `TsDemux::unpackage` is a thin wrapper over
it.

For live output `Segmenter` owns the cut state machine: `Segmenter::new(specs,
movie_timescale, target_secs)`, then `init_segment()`, `push(track_id,
sample)` in decode order, `take_ready()`, `flush()` at end of stream, and
`mark_discontinuity()` to flag the next cut (RFC 8216 §4.3.2.3). It cuts on the
anchor track's sync samples (first video track, else first anchor-capable
track). `LlHlsSegmenter` (parts), `TsHlsSegmenter` / `StreamingTsHlsSegmenter`
(TS segments, rolling playlist) and `LlSegmenter` are the siblings.

## Repackaging (select / trim / resegment)

`Repackage` composes fMP4 demux, IR transforms and resegmentation:

```rust
let out = transmux::Repackage::new(6.0)      // target segment duration, seconds
    .select_tracks(&[0])                     // keep only track index 0
    .trim(0, 90_000)                         // first second (movie timescale)
    .run(fmp4)?;
let _init = out.init_segment;
let _media = out.media_segments;             // Vec<Vec<u8>>
```
(`transmux/src/repackage.rs`, `Repackage::run` doc example)

The transforms are also usable directly on any IR: `Media::select_tracks(&[…])`,
`Media::select_tracks_by(|t| …)`, `Media::trim(start, end)` (window in the
**movie** timescale, snapped back to a keyframe),
`Repackage::run_media(&media)`, and `RepackageOutput::to_contiguous()` for one
contiguous fragmented-MP4 stream.

Timeline transforms (in `transmux::rebase`, all `&mut Media`): `rebase_to_zero`
(per-track, first sample → 0), `apply_offset(delta_ticks)` (signed, saturates at
0) and `insert_discontinuity_gap(track, at_sample_index, gap_ticks)`. They move
each sample's absolute `dts`/`pts` **and** `Track::start_decode_time` in
lockstep; 33-bit MPEG wrap is already unrolled at the demux edge.

```rust
apply_offset(&mut media, DELTA);
assert_eq!(media.tracks[0].start_decode_time, ANCHOR + DELTA as u64);
let fmp4 = CmafMux::default().package(&media).expect("package");
```
(`transmux/tests/it/rebase.rs`, test `apply_offset_bites`)

## Splice / concat (SSAI)

`splice_insert(base, ad, at_ticks)` plays `base` to the splice time, inserts
`ad`, and resumes `base` shifted by `ad`'s duration; `concat(a, b)` appends on a
monotonic timeline. Both return a `SpliceResult { media, discontinuity_points }`
— signal each `SplicePoint` downstream as `#EXT-X-DISCONTINUITY` / a DASH
`emsg`.

```rust
let result = splice_insert(&base_media, &ad_media, at_ticks)?;

let mut points_by_track: BTreeMap<TrackId, Vec<usize>> = BTreeMap::new();
for p in &result.discontinuity_points {
    points_by_track
        .entry(p.track_id)
        .or_default()
        .push(p.sample_index);
}
```
(`transmux/examples/ssai_ad_stitch/demo.rs`, step 5 "The actual splice")

Requirements: the `ad`'s first sample of every matched track must be a sync
sample, else `Error::InvalidInput`; the splice time snaps back to the base
video track's preceding sync sample (`snap_to_preceding_sync`); incoming
samples are rebased onto the join so intact assets with unrelated origins line
up. Full runnable wiring (SCTE-35 cue → splice → HLS/DASH):
`transmux/examples/ssai_ad_stitch/`.

## CENC decrypt (feature `cenc`, on by default)

```rust
use broadcast_common::Decrypt;

let mut key_map = crate::cenc_decrypt::KeyMap::new();
for spec in keys {
    let (kid, key) = parse_key(spec)?;
    key_map.insert(kid, key);
}
let decryptor = crate::cenc_decrypt::CencDecryptor::from_fmp4(input)?;
let mut media = decryptor.demux()?;
decryptor.decrypt(&mut media, &key_map)?;
```
(`transmux/src/cli.rs`, `decrypt_input` — the `--decrypt` path; drop the
`crate::` prefix outside the crate: `transmux::KeyMap`,
`transmux::CencDecryptor`)

`from_fmp4` harvests `sinf`/`tenc` + per-sample `senc` from a protected fMP4
(progressive or fragmented); `demux()` gives the IR with **still-encrypted**
samples, `decrypt(&mut media, &KeyMap)` ciphers them back in place. `cenc`
(AES-CTR) and `cbcs` (AES-CBC pattern) are supported; `cens`/`cbc1` are not
(`Error::UnsupportedCencScheme`). `CencEncryptor` is the inverse and fills
`Track::encryption`, which is what makes the packagers emit
`sinf`/`senc`/`saio`/`saiz` and DRM signalling.

## Conformance validation

`validate_init_segment(bytes)`, `validate_media_segment(bytes)` and
`validate_cmaf_track(init, &[segment, …])` return `Vec<ConformanceIssue>` —
empty means conformant. Each issue is `{ severity: Severity::Error | Warning,
code: &'static str, message: String }` with a stable dotted code such as
`media.tfdt.missing` or `track.tfdt.discontinuity`. Structural only: no media
decode, malformed input yields issues rather than panics.

## CLI (`cli` feature)

```console
$ cargo install transmux --features cli
$ transmux in.ts -o out.cmaf                                  # TS to CMAF, format inferred
$ transmux in.mp4 -o out.m3u8 -f hls --segment-duration 4     # MP4 to CMAF-HLS, 4 s segments
$ transmux in.ps -o out.mpd -f dash --ll                      # PS to low-latency DASH
$ transmux enc.mp4 -o dec.mp4 -f progressive --decrypt --key <KID>:<KEY>
```
(`transmux/README.md`, section "Command-line packager")

`-f` takes `cmaf | hls | ts-hls | dash | ts | progressive` (default: inferred
from `-o`'s extension). Input container is detected with `container-probe`
(MPEG-TS including 192-byte M2TS, MP4/CMAF, MPEG-PS, WebM/Matroska, FLV); an
inconclusive probe is `unknown input container` — the format must then be
supplied explicitly.

## Features

| Feature | Default | Effect |
|---|---|---|
| `std` | on | `std::error::Error`, and every XML path (DASH/LL-DASH MPD, Smooth, PlayReady WRMHEADER) — off → `#![no_std]` + `alloc`, no `DashPackager`/`SmoothPackager`/`LlDashPackager`/SDP |
| `cenc` | on | CENC/CBCS decrypt + encrypt; off → no `CencDecryptor`/`CencEncryptor` |
| `serde` | off | `Serialize` on box and IR types |
| `sample-aes` | off | HLS Sample-AES + whole-segment AES-128 (implies `cenc`) |
| `cli` | off | the `transmux` binary (implies `std`) |

`no_std`: `transmux = { version = "…", default-features = false, features = ["cenc"] }`.

## Pitfalls

- **It never transcodes.** A codec the output container cannot carry is
  *rejected* (`CmafMux` and every fMP4/CMAF entry point with
  `Error::UnmuxableDataTrack`/`UnmuxableSubtitleTrack`, the WebM-only VP8/Vorbis
  with `UnsupportedCodec`, FLV/RTMP/Smooth with their own codec error) or
  *dropped* (`TsMux` drops a track with no TS mapping). Filter first with
  `media.select_tracks_by(|t| t.spec.config.is_muxable_in_bmff())`; plan the
  codec set from the README input × output matrix, not by trial and error.
- **`Sample::data` for AVC/HEVC is length-prefixed, not Annex B.** Convert with
  `Sample::from_annexb(..)`; feeding Annex B straight to a muxer produces
  garbage NAL framing with no error.
- **Timestamps are absolute, and never fabricated.** `dts`/`pts` are absolute
  ticks in the *track's own* timescale (`90000` video vs `44100` audio);
  `None` only for section-carried tracks (SCTE-35, DSM-CC, private sections).
  Rescaling between timescales is `Sample::rescale_timescale(from, to)`. Do
  not re-derive timing as `start_decode_time + Σ duration` — a per-fragment
  `tfdt` reseed makes that wrong; read each sample's `dts`.
- **`Media`, `Track`, `TrackSpec`, `Sample` are `#[non_exhaustive]`.** Only
  the constructors (`Media::new`, `Track::new`/`new_at`, `TrackSpec::new`,
  `Sample::new`/`from_annexb`/`from_raw`) exist outside the crate; matching
  needs a trailing `..`.
- **`DashPackager`/`HlsPackager` return `String`, `CmafMux` returns `Vec<u8>`.**
  Read the `Package::Output` type rather than assuming bytes.
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
