# multimux 0.5.0

**Release date:** 2026-07-28

A ground-up rebuild of multimux's ingest and egress architecture on `media-plane`. All nine input kinds (RTSP, RTP-UDP, TS-UDP, TS-HTTP, SRT, HLS-pull, DASH-pull, Smooth-pull, RTMP) now run through `media_plane::ingress` traits and publish into per-program `Trunk` instances. The old `SourceConnector`/`supervise`/`pipeline` module is deleted. Per-program serving state makes the crate MPTS-ready, and RTMP gains concurrent publisher support via `ListenDriver`.

## What's new

- **Per-program serving state**: `RouteHandle` gained a `ProgramId -> Arc<Trunk>` registry. Each program's `LlHlsOrigin` and `DashState` are created on demand, not eagerly at route construction.
- **RTMP concurrent publishers**: RTMP moved onto `media_plane::ingress::Listener`, fixing a critical defect where one stalled publisher wedged the entire route.
- **`rtsps://` restored** (#804): TLS ingest works again after the media-plane port, with real TLS handshake and SNI derivation.
- **Metrics restored** (#809): `multimux_parts_produced_total` and `multimux_segments_produced_total` counters have emitters again.
- **First-feed samples no longer dropped** (#808): `ProgramSegmenter` subscribes with `subscribe_from_backlog` so samples published alongside `NewProgram` are not missed.
- **Dispatch regression net** (#805 task 3): compile-time exhaustive coverage ensures every `InputSpec` variant dispatches to real ingest, never a stub.
- **`advance_route` facade**: a single per-iteration call replaces the error-prone `report_driver_progress` + `drive_program_segmenters` pair.

## What's changed

- Five `source` enums (`DashResourceId`, `DashAction`, `HlsFetchId`, `SmoothResourceId`, `SmoothAction`) are now `#[non_exhaustive]`.
- `SourceConnector`, `supervise`, the `pipeline` module, `RouteHandle::publish_owned_trunk`, and the `testsupport` feature are all deleted.
- `RouteHandle::set_init`/`add_part`/`add_segment` and friends now take a leading `ProgramId` parameter.

## Migration

This is a breaking release. Callers using `SourceConnector`/`supervise`/`pipeline::run_pipeline` must port onto `supervise_driver` with their own `Dialer`/`IngestSession` (see `examples/custom_scheme.rs`). `RouteHandle` no longer owns a placeholder `Trunk` — call `publish_new_program(program)` first, then write. Requires `media-plane` 0.1 and `hls-runtime` 0.2.
