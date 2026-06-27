# mp4-emsg v0.1.0 — 2026-06-27

## Rename from `dvb-emsg`

`mp4-emsg` 0.1.0 is the renamed successor to `dvb-emsg` 0.1.0. The code is
identical; the only change is the crate name. `mp4-emsg` is independently
versioned and was never part of the DVB lockstep.

### Migration

In `Cargo.toml`, replace:

```toml
dvb-emsg = "0.1"
```

with:

```toml
mp4-emsg = "0.1"
```

In `use` statements, replace `dvb_emsg::` with `mp4_emsg::`. All types,
functions, and module paths are identical.

The deprecated `dvb-emsg` shim (v0.1.1) re-exports `mp4-emsg` 0.1 for
backwards compatibility. It will not receive new features.

## What's in v0.1.0

ISO BMFF / DASH Event Message Box (`emsg`) parse + serialize:

- **`EmsgBox`** — the `'emsg'` ISOBMFF `FullBox` supporting both **version 0**
  (segment-relative `presentation_time_delta`, u32) and **version 1**
  (representation-relative `presentation_time`, u64). `size` and `version` are
  recomputed on serialize; no raw passthrough.
- **`PresentationTime`** — version-discriminated enum; selecting a variant
  selects the box version.
- **`EmsgVersion`** — typed `version` byte with `name()` / `Display` label.
- **`EmsgBox::is_scte35()`** — recognises the `urn:scte:scte35…` scheme URI
  prefix; `message_data` carries a SCTE 35 `splice_info_section` in that case.
- Field semantics sourced from **DASH-IF IOP Part 10 V5.0.0 §6.1 + Table 6-2**
  (transcribed in `mp4-emsg/docs/emsg.md`); normative ISOBMFF box syntax cites
  **ISO/IEC 23009-1 §5.10.3.3** (paid, not vendored — softer footing than fully-
  free crates, flagged per project policy).
- `#![no_std]` + `alloc`; optional `serde` feature (Serialize-only).
