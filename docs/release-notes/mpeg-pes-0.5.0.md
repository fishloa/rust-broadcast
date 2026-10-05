# mpeg-pes 0.5.0

_Released 2026-10-05._

**Minor-epoch breaking release (0.x).** The `PesExtension::pes_extension_field` field changes type, and a parse bug that rejected every conformant `PES_extension_flag_2 == 1` packet (in practice every `stream_id == 0xFD` extended-stream-id PES) is fixed. `PesAssembler` now caps buffered PES size at 16 MiB. Act if you read `pes_extension_field` or want a different reassembly cap. Read with [mpeg-ts 0.5.0](mpeg-ts-0.5.0.md) (shares the PUSI accumulator) and [mpeg-ps 0.5.0](mpeg-ps-0.5.0.md) (depends on this crate).

## Dependency changes

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
```

A new `Error::FieldOverflow` variant wraps `broadcast_common::len::FieldOverflow`.

## Breaking change: typed `pes_extension_field` (#1052)

`PesExtension::pes_extension_field` was `Option<&'a [u8]>`; it is now `Option<PesExtensionField<'a>>`, which types the `stream_id_extension_flag`(1) + 7-bit byte of ISO/IEC 13818-1 §2.4.3.7, Table 2-21.

```rust
// before: opaque slice, caller split off the flag byte
if let Some(bytes) = ext.pes_extension_field { let first = bytes[0]; /* ... */ }
// after
if let Some(f) = ext.pes_extension_field {
    let (flag, low7, rest) = (f.stream_id_extension_flag, f.low_bits, f.rest);
}
```

## Behaviour changes

- `PesAssembler` buffered a PES without bound when no further PUSI arrived (legal for `PES_packet_length == 0` video), so a hostile or broken stream could exhaust memory. Each PES is now capped at 16 MiB by default; an oversized PES is discarded and reassembly resumes at the next PUSI. `PesAssembler::with_max_unit_size(max)` sets a different cap (audit r01-W13, #1074).
- `PesAssembler` is now a thin wrapper over `broadcast_common::pusi::PusiAccumulator`, shared with `mpeg-ts`'s `PusiReassembler`, replacing two diverged copies of the PUSI-gating rule.

## Fixes

- `PES_extension_field_length` was parsed and serialized as a plain 8-bit length; Table 2-21 has `marker_bit(1) + PES_extension_field_length(7)`. Every conformant `PES_extension_flag_2 == 1` packet failed to parse with `BufferTooShort`. Verified against an `ffmpeg` `vc2` (Dirac)-in-TS fixture, one of the few codecs that produces a real `0xFD` PES (`mpeg-pes/tests/fixture_pes_extension.rs`) (#1052).
- `PesExtension::serialize_into` wrote `pack_field_length` and `PES_extension_field_length` with unchecked `as u8` casts. This is unreachable through the public `PesPacket` API today because an existing `PesHeader::optional_len` check rejects the same oversized case first; fixed as defence in depth (#1129).

---

Published from tag `mpeg-pes-v0.5.0`.
