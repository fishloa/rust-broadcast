# media-plane 0.1.0

Released 2026-07-28.

### Added

Initial release — the ingress/egress spine for live origins: `Dialer`/`Listener`
→ `ByteStage` → `IngestSession` → `IrTransform` → `TrunkWriter` → `Trunk`.
Bounded sample/segment/event/part rings with cursor subscribers, `ProgramId`-
keyed track sets, three egress shapes (ServedEgress, PushEgress, SegmentEgress),
tiered retention with DVR pinning. `Trunk::subscribe_from_backlog` for
consumers built after samples have landed.

### Changed (pre-1.0)

- `TapItem` is now `#[non_exhaustive]`.
