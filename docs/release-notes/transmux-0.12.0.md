# transmux 0.12.0 — 2026-07-04

Broadens the MPEG-2 TS side of the **any-to-any container hub** and adds a
progressive-MP4 demux spoke + streaming TS-HLS. `transmux` still parses only
codec *config* headers — it never en/decodes; coded samples stay opaque.
`no_std` + `alloc`. Independently versioned.

## Breaking

Two source-breaking changes (0.x minor bump per Cargo SemVer):

- `TrackSpec` gained `source_pid: Option<u16>` + `es_info_descriptors: Vec<u8>`
  (#582) — build specs via `TrackSpec::new(track_id, timescale, config)` +
  `.with_source(pid, descriptors)` instead of a struct literal.
- `CodecConfig`, `Sample`, and `TrackSpec` are now `#[non_exhaustive]` (#580) —
  construct `Sample` via `Sample::new`/`from_annexb`/`from_raw`
  (+`.with_source_timing`); external `match` on `CodecConfig` needs a `_` arm.

## Player track-picker on the IR (#582)

Every TS-demuxed track now carries its **origin PID** (`TrackSpec::source_pid`)
and the **verbatim PMT ES_info descriptor bytes** (`es_info_descriptors`) — for
every track, codec and opaque `Data` alike — so a DVB player can select/label
tracks (ISO-639 language `0x0A`, DVB subtitling `0x59`, E-AC-3 `0x7A`, …)
without running its own parallel PAT/PMT parser. `None`/empty for non-TS sources.

## New spokes

- **Progressive (non-fragmented) MP4 demux** (#561): `ProgressiveDemux` walks
  `moov` sample tables (`stts`/`ctts`/`stss`/`stsz`/`stsc`+`stco`/`co64`) into
  the IR — the dominant file-on-disk form, complementing the existing fMP4
  demux. (`sidx` v0/v1 was already complete.)
- **Streaming TS-HLS** (#571): `StreamingTsHlsSegmenter` for live/unbounded
  input alongside the batch `TsHlsPackager` — `push`→`Option<TsSegment>`
  (keyframe-cut `.ts` segments) + a rolling media playlist (sliding window,
  advancing `#EXT-X-MEDIA-SEQUENCE`). Shares the batch cut logic (one
  implementation).

## Fixed

- **avcC high-profile extension** (#563, #582): the `AVCDecoderConfigurationRecord`
  extension gate used profile_idc `144` (a pre-Amendment-3 placeholder;
  finalized as `244`, High 4:4:4 Predictive), and `TsDemux` hardcoded
  chroma/bit-depth to `None` — so High 10/4:2:2/4:4:4 lost them in the recovered
  `avcC`. Both fixed and unified onto a single `sps::is_high_profile` source of
  truth (serializer gate == demux populator, no round-trip asymmetry). New
  `tests/sps_profile_matrix.rs` pins the full H.264/HEVC profile matrix against
  ffprobe.

## Internal

- `tests/label_coverage.rs` drift-guard (#580): fails CI if a new public
  spec/field enum lacks `name()` + `Display`.

## Compatibility

Requires broadcast-common ≥ 8.4. MSRV 1.86 (edition 2024).
