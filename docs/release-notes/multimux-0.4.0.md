# multimux 0.4.0

Additive minor release — four new ingest transports via transmux's hand-rolled parsers.

## Added

### RTMP Push

- **RTMP push ingest** (`InputSpec::Rtmp { listen, app, stream_key }`) — accepts
  a live RTMP publish (encoder / OBS). Binds a listener, accepts a publisher via
  `rtmp-runtime`, demuxes the FLV incrementally via `transmux::StreamingFlvDemux`,
  and feeds the samples into the just-in-time repackaging pipeline
  (→ LL-HLS / DASH / LL-DASH). `stream_key` optionally gates who may publish.

### Remote Pull Inputs

- **DASH-pull ingest** (`InputSpec::DashPull { url }`) — pulls a remote MPEG-DASH
  presentation: fetches + parses the MPD via `transmux::dash_parse`, resolves
  `SegmentTemplate`/`SegmentTimeline` addressing, and demuxes the fetched fMP4
  segments via `transmux::Fmp4Demux`. Supports `$Number$`/`$Time$` templates and
  both static/dynamic (live with MPD refresh) presentations. Every network step
  is bounded by `IngestTimeouts`.

- **Smooth-pull ingest** (`InputSpec::SmoothPull { url }`) — pulls a remote
  Microsoft Smooth Streaming presentation: fetches + parses the client Manifest
  via `transmux::smooth_parse`, resolves fragment-URL templates, and demuxes fMP4
  fragments. Synthesizes a bootstrapping init segment from `CodecPrivateData`
  (Annex-B SPS/PPS → `avcC`, raw ASC → `esds`). Rejects PlayReady/PIFF
  sample-encrypted sources with a typed `MultimuxError::Encrypted`. Supports both
  static and dynamic (live, manifest-refresh) presentations.

- **SRT ingest** (`InputSpec::Srt { listen, port, caller }`) — ingests MPEG-2 TS
  over SRT in either listener mode (binds once, accepts inbound Callers) or
  caller mode (dials out fresh on reconnect). Track resolution is in-band via
  PMT, exactly like TS-UDP ingest. Unencrypted only (no passphrase field).

- **HLS classic segments** — `HlsPullSource` now also routes classic MPEG-TS HLS
  (no `EXT-X-MAP`, self-contained `.ts` segments) through `transmux::TsDemux`,
  synthesizing an init segment from the first successfully demuxed segment's
  `TrackSpec`s, so the same downstream pipeline handles both fMP4/CMAF and TS
  origins transparently.

## Hardening

- All four inputs apply the same ingest standard: bounded connect + read timeouts
  (a stalled origin can't wedge the route) and full video+audio track resolution
  before the route starts (codec headers always arrive before samples).

## Dependencies

- Bumps `transmux` to 0.19 (adds `dash_parse`, `smooth_parse`,
  `StreamingFlvDemux`).
- Adds `rtmp-runtime` 0.1 (RTMP ingest server).
- Adds `srt-runtime` 0.2 (SRT listener + caller).

No breaking changes to existing inputs/outputs; `InputSpec` is `#[non_exhaustive]`.
