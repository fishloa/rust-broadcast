# broadcast-core

**The frozen wire-format contract of the rust-broadcast workspace: the
symmetric `Parse`/`Serialize` traits, checked field-width helpers, MSB-first
bit I/O, typed reserved-bit fields, the big-endian wire field readers, media
timescales, the shared error kinds and the container-independent CENC scheme
identity.**

## What it does / what it does not

This is the low-level floor that the DVB/MPEG format crates build on. It holds
only settled pieces that do not change as new formats are added, so they can
grow without a breaking change here rippling up through every consumer. The
crate is `#![no_std]` + `alloc` on every target and depends on nothing but an
optional `serde` derive.

It carries no CRC, clock arithmetic, stage driver, XML reader or HTTP-date
codec — those stay in `broadcast-common` and its siblings. It has no shared
*format* error enum either: it owns only the cross-cutting failure kinds
(short buffer, field overflow, CRC mismatch, invalid sync, …) that each crate
wraps in its own `Error`.

## Contents

| Path | What it is |
|---|---|
| `traits` (`Parse`, `Serialize`) | The symmetric parse/serialize contract every wire type implements, re-exported at the crate root. |
| `len` | Range-checked narrowing for wire length/count fields (`fit_u8`, `fit_u16`, `fit_bits`, `fit_mask`, …). |
| `bits` | Backing MSB-first `BitReader`, `BitWriter` and growable `BitBuf`. |
| `error` | The shared failure kinds (`BufferTooShort`, `FieldOverflow`, `CrcMismatch`, …). |
| `reserved` | Typed runs of reserved bits that round-trip what was parsed. |
| `timescale` (`timescale!`) | A non-zero media timescale and wrap-safe rescaling. |
| `cenc` (`CencScheme`) | The container-independent CENC scheme identity. |
| `wire` | Big-endian field readers over one bounds-checked `take`: the checked `read_u8`/`read_u16`/`read_u32`/`read_u48`/`be_u16`/`be_u32`/`be_u64` and the `*_at` readers for an offset the caller's own guard already covers. |
| `impl_spec_display!` | The `Display`-from-`name()` macro every public spec enum uses. |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Forwards to `serde/std`; the crate itself is `no_std` + `alloc` on every target. |
| `serde` | no | `serde::Serialize` on the shared spec types. |

```toml
[dependencies]
broadcast-core = "0.1"
# embedded (no_std + alloc):
# broadcast-core = { version = "0.1", default-features = false }
```

## Stability

The API is **meant to be frozen**: it is the stable floor the wire-format
crates build on. Additions are still possible before `1.0`, but a change to a
name or signature already present is a deliberate, reviewed event — the
`core_api_pin` workspace guard fails on any un-reviewed addition or removal.

## Related crates and licence

Part of the [rust-broadcast](https://github.com/fishloa/rust-broadcast)
workspace. Licensed under either of Apache-2.0 or MIT at your option.
