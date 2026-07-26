# transmux 0.19.0

Additive minor release: hand-rolled DASH/Smooth-Streaming (MS-SSTR) parsers + FLV incremental demux for RTMP ingest.

## Added

### Parsers

- **`dash_parse` — hand-rolled MPEG-DASH MPD parser** — the structural inverse of
  the `DashPackager` writer, parsed from a remote URI into the typed
  `Mpd`/`Period`/`AdaptationSet`/`Representation` tree model (ISO/IEC 23009-1).
  Resolves `SegmentTemplate` inheritance top-down, expands `SegmentTimeline`
  `<S r=…>` runs, and substitutes variable tokens (`$Time$`, `$Number$`,
  `$RepresentationID$`, `$Bandwidth$`) with safe bounds against DoS. No external
  XML dependency; `no_std`+`alloc`.

- **`smooth_parse` — hand-rolled MS-SSTR client-Manifest parser** — parses a
  remote client Manifest into the typed `SmoothManifest`/`StreamIndex`/
  `QualityLevel` model, with live extensions (`IsLive`, `DVRWindowLength`,
  `LookAheadFragmentCount`). Resolves fragment-URL templates and synthesizes a
  bootstrapping init segment from `CodecPrivateData` (Annex-B SPS/PPS → `avcC`,
  raw ASC → `esds`). Same DoS-resistant design as `dash_parse`; `no_std`+`alloc`.

- **Shared `xml_parse` tokenizer** — extracted by both parsers: a hand-rolled,
  no_std-capable XML token stream (element/attribute/text/escape sequences) that
  neither parser allocates during streaming.

### Streaming Demux

- **`StreamingFlvDemux`** — an incremental FLV → samples demuxer, the streaming
  counterpart to the one-shot `FlvDemux` (and mirroring `StreamingTsDemux`'s
  `feed` / `poll_event` / `finish` pull API). It parses FLV tags as bytes arrive
  and emits `DemuxEvent`s with **bounded memory** (only an in-progress partial
  tag is retained), for live ingest where a whole FLV buffer never exists at once
  — e.g. RTMP push (`rtmp-runtime` → `multimux`'s new RTMP input). Output matches
  the one-shot `FlvDemux` sample-for-sample on the same bytes (verified against a
  real fixture).

## Hardening

- `dash_parse` caps `SegmentTimeline` enumeration at 100k segments (rejecting
  unbounded `<S r=…>` repeat runs) and `SegmentTemplate` format width at 20 digits
  (rejecting hostile `%9999999999d` templates); validates closing-tag names to
  prevent silent truncation.

- `smooth_parse` caps repeat runs at 100k chunks and `CodecPrivateData` decoding
  at a sane hex length before allocation.

- `AVCDecoderConfigurationRecord::parse` now rejects an avcC declaring zero SPS
  instead of returning an empty set (previously a downstream `sps[0]` could panic
  on a malformed sequence header — reachable from an untrusted RTMP publisher).

- `StreamingFlvDemux` clamps the FLV header `DataOffset` so a malicious value
  can't grow the pending buffer without bound.

No breaking changes; existing `FlvDemux`/`TsDemux`/`StreamingTsDemux` APIs are
unchanged.
