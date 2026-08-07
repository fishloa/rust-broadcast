# media-plane 0.1.0

**Release date:** 2026-07-28

Initial release — the ingress/egress spine for live origins. Four layers: `Dialer`/`Listener` → `ByteStage` → `IngestSession` → `IrTransform` → `TrunkWriter` → `Trunk`. A `Trunk` is the per-program hub with bounded sample/segment/event/part rings, cursor subscribers (`Lagged` reported in-band), `ProgramId`-keyed track sets, and three egress shapes (`ServedEgress`, `PushEgress`, `SegmentEgress`). Tiered retention with DVR pinning and `subscribe_from_backlog` for late-joining consumers.

## What's new

- `Trunk` with bounded rings, cursor-based subscription, and `listen()` wake-ups.
- `Dialer` / `Listener` ingress traits for connection-oriented and listener-oriented sources.
- `IngestSession` / `IrTransform` / `TrunkWriter` pipeline stages.
- `ServedEgress` with `EgressResponse::Await` for LL-HLS blocking reload.
- `PushEgress` and `SegmentEgress` for push-based and segment-archive outputs.
- `TapItem` is `#[non_exhaustive]`.

## Migration

New crate — no migration needed.
