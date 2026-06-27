# cc-data v0.2.0 — 2026-06-27

## Rename from `dvb-cc`

`cc-data` 0.2.0 is the renamed successor to `dvb-cc` 0.2.0. The crate carries
`cc_data()` closed-caption triplets (CEA-608/708) whose payload is generic
(ATSC/CEA), so the DVB-prefixed name was misleading — though the carriage table
itself is cited from ETSI TS 101 154 Table B.9. The code is identical; only the
crate name changed.

### Migration

Replace `dvb-cc = "0.2"` with `cc-data = "0.2"`; replace `dvb_cc::` with
`cc_data::`. All types, paths, and the `decode` feature are identical. The
deprecated `dvb-cc` shim (v0.2.1) re-exports `cc-data` 0.2 (forwarding the
`decode` feature) and will not receive new features.

## What's in v0.2.0

All functionality from `dvb-cc` 0.2.0: `cc_data()` carriage (ETSI TS 101 154
Table B.9) — typed CEA-608/708 triplets + 608/708 split, optional `decode`
feature for the caption-decode layer, `#![no_std]` + `alloc`, depends only on
`dvb-common`.
