# smpte2038 v0.1.0 — 2026-06-27

## Rename from `dvb-smpte2038`

`smpte2038` 0.1.0 is the renamed successor to `dvb-smpte2038` 0.1.0. SMPTE ST
2038 (carriage of ancillary data packets in an MPEG-2 TS) is a SMPTE standard,
not DVB-specific. The code is identical; only the crate name changed.

### Migration

Replace `dvb-smpte2038 = "0.1"` with `smpte2038 = "0.1"`; replace `dvb_smpte2038::`
with `smpte2038::`. All types and paths are identical. The deprecated
`dvb-smpte2038` shim (v0.1.1) re-exports `smpte2038` 0.1 and will not receive new
features.

## What's in v0.1.0

All functionality from `dvb-smpte2038` 0.1.0: SMPTE ST 2038 ancillary-data
packet parsing, `#![no_std]` + `alloc`, depends only on `dvb-common`.
