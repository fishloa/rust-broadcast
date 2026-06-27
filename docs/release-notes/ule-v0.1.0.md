# ule v0.1.0 — 2026-06-27

## Rename from `dvb-ule`

`ule` 0.1.0 is the renamed successor to `dvb-ule` 0.1.0. ULE (Unidirectional
Lightweight Encapsulation) is defined by IETF RFC 4326 — a generic IP-over-MPEG-TS
encapsulation, not DVB-specific. The code is identical; only the crate name
changed.

### Migration

Replace `dvb-ule = "0.1"` with `ule = "0.1"`; replace `dvb_ule::` with `ule::`.
All types and paths are identical. The deprecated `dvb-ule` shim (v0.1.1)
re-exports `ule` 0.1 and will not receive new features.

## What's in v0.1.0

All functionality from `dvb-ule` 0.1.0: RFC 4326 ULE SNDU parsing, `#![no_std]`
+ `alloc`, depends only on `dvb-common`.
