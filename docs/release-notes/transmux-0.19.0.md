# transmux 0.19.0

Additive minor release.

## Added

- **`StreamingFlvDemux`** — an incremental FLV → samples demuxer, the streaming
  counterpart to the one-shot `FlvDemux` (and mirroring `StreamingTsDemux`'s
  `feed` / `poll_event` / `finish` pull API). It parses FLV tags as bytes arrive
  and emits `DemuxEvent`s with **bounded memory** (only an in-progress partial
  tag is retained), for live ingest where a whole FLV buffer never exists at once
  — e.g. RTMP push (`rtmp-runtime` → `multimux`'s new RTMP input). Output matches
  the one-shot `FlvDemux` sample-for-sample on the same bytes (verified against a
  real fixture).

## Hardening

- `AVCDecoderConfigurationRecord::parse` now rejects an avcC declaring zero SPS
  instead of returning an empty set (previously a downstream `sps[0]` could panic
  on a malformed sequence header — reachable from an untrusted RTMP publisher).
- `StreamingFlvDemux` clamps the FLV header `DataOffset` so a malicious value
  can't grow the pending buffer without bound.

No breaking changes; existing `FlvDemux`/`TsDemux`/`StreamingTsDemux` APIs are
unchanged.
