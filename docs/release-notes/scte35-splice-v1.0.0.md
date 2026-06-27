# scte35-splice v1.0.0 — 2026-06-27

## Rename from `dvb-scte35`

`scte35-splice` 1.0.0 is the renamed successor to `dvb-scte35` 7.9.0. The code
is identical; the only change is the crate name and the independent versioning
baseline (v1.0.0). `scte35-splice` has **left the seven-crate DVB lockstep** and
will version independently from this release forward.

### Migration

In `Cargo.toml`, replace:

```toml
dvb-scte35 = "7"
```

with:

```toml
scte35-splice = "1"
```

In `use` statements, replace `dvb_scte35::` with `scte35_splice::`. All types,
functions, and module paths are identical.

The deprecated `dvb-scte35` shim (v7.9.1) re-exports `scte35-splice` 1.0 for
backwards compatibility. It will not receive new features.

## What's in v1.0.0

All functionality from `dvb-scte35` 7.9.0, which implements **ANSI/SCTE 35
2023r1**:

- **`SpliceInfoSection`** — full §9.6 header with CRC-32 verification; clear and
  encrypted sections; `pts_adjustment_duration()` accessor.
- **6 splice commands** (`splice_null`, `splice_schedule`, `splice_insert`,
  `time_signal`, `bandwidth_reservation`, `private_command`), unified by
  `AnyCommand`.
- **5 splice descriptors** (`avail`, `DTMF`, `segmentation`, `time`, `audio`),
  unified by `AnySpliceDescriptor`; unknown tags fall through losslessly.
- **Typed enums** for `SegmentationTypeId` (48 variants), `SegmentationUpidType`
  (18 named variants), `DeviceRestrictions` — clients never re-implement spec
  lookup tables.
- **Typed UPID sub-structures**: `Mpu` (§10.3.3.3) and `MidUpid` (§10.3.3.4)
  decoded on demand.
- **90 kHz time accessors**: `SpliceTime::duration()` / `BreakDuration::duration()`
  → `core::time::Duration`.
- `#![no_std]` + `alloc`; `serde` feature (Serialize-only, on by default).
