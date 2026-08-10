# dvb-csa 0.2.0

**Release date:** 2026-08-10

MSRV bump plus an internal duplication fix. No public API change.

## What changed

- MSRV raised to **1.95.0** (issue #949), workspace-wide. No functional or API change from this alone.
- `ts::ts_payload_mut` now decodes `adaptation_field_control` via the new `mpeg-ts` dependency's `mpeg_ts::ts::TsHeader::parse` instead of hand-rolling the same bit decode a second time — a duplication-audit finding. The magic numbers `188`/`0x3f`/`0x80` are replaced with named, spec-cited constants (`mpeg_ts::ts::TS_PACKET_SIZE`/`SCRAMBLING_MASK`, plus a local `TSC_EVEN_KEY`). The payload byte-offset computation and the mutable slicing itself stay hand-rolled, because `mpeg_ts::ts::TsPacket` only exposes an immutable `payload: &[u8]` and CSA (de)scrambling needs to write back into the caller's own buffer. Identical behaviour; no public API change.

## Migration

No API changes; no action required. This crate now has a `mpeg-ts` dependency it did not have before (`0.4`, default-features disabled) — nothing for a consumer to do, but worth knowing if you audit the dependency tree.
