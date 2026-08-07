# multimux 0.5.0

**Release date:** 2026-07-28

The media-plane port — the largest single change in multimux's history. Every input scheme, the output registry, and the program lifecycle are rebuilt on the `media-plane` `Trunk` data plane. Nine input schemes ported (`RTSP`, `RTP`, `TS-UDP`, `TS-HTTP`, `SRT`, `HLS-pull`, `DASH-pull`, `Smooth-pull`, `RTMP`), with RTMP now running on the `Listener` transport trait. The external scheme plugin registry (`Custom` input/output/output-auth + `SchemeRegistry`) is rewritten so a third-party crate can add a scheme without editing this crate. The rolling-window `MediaStore` is deleted — the `Trunk` is the single copy of live data.

## What's new

- `media-plane` integration: all nine input schemes produce samples/segments into a `Trunk` instead of the old `MediaStore`.
- RTMP ingest on `Listener` transport trait (previously hard-wired TCP accept loop).
- `SchemeRegistry` — external scheme plugin registry (`Custom` input/output/output-auth) so third-party crates can register schemes without forking.
- Per-program `ProgramRegistry` with `Trunk`-backed lifecycle.
- `RouteConfig` validation at startup (fail-fast on misconfigured routes).
- Prometheus metrics: `multimux_programs_active`, `multimux_samples_written_total`, `multimux_segments_written_total`.

## What changed

- **`MediaStore` deleted** — replaced by `Trunk` rings. All egress reads directly from the `Trunk`.
- All input scheme adapters rewritten against `IngestSession`/`IngestDriver` traits.
- Output scheme adapters rewritten against `ServedEgress`/`PushEgress`/`SegmentEgress`.
- Requires `media-plane` 0.1, `hls-runtime` 0.2, `transmux` 0.21.

## Migration

Breaking: `MediaStore` is gone. Any code that reached into the store's internal ring or subscribed to its segments must use `Trunk` cursors instead. The `SchemeRegistry` API for custom schemes has changed — see the updated `examples/custom_scheme.rs`.
