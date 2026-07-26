# multimux 0.4.0

Additive minor release — a new ingest input.

## Added

- **RTMP push ingest** (`InputSpec::Rtmp { listen, app, stream_key }`) — multimux
  now accepts a live RTMP publish (encoder / OBS) as a first-class input,
  alongside RTSP / RTP / TS-UDP / TS-HTTP / HLS-pull. It binds a listener, accepts
  a publisher via `rtmp-runtime`, demuxes the FLV incrementally via
  `transmux::StreamingFlvDemux`, and feeds the samples into the same
  just-in-time repackaging pipeline (→ LL-HLS / DASH / LL-DASH). `stream_key`
  optionally gates who may publish.
- The input is hardened to the crate's ingest standard: connect + read timeouts
  (a stalled or never-publishing client can't wedge the route) and full
  video+audio track resolution before the stream starts (no dropped audio when
  codec sequence headers arrive in separate reads).

## Dependencies

- Bumps `transmux` to 0.19 (for `StreamingFlvDemux`) and adds `rtmp-runtime` 0.1.

No breaking changes to existing inputs/outputs; `InputSpec` is `#[non_exhaustive]`.
