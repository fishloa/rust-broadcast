# broadcast-common 9.2.0

**Release date:** 2026-08-02

Adds `cenc::CencScheme` (the container-independent CENC scheme identity — `Cenc`, `Cens`, `Cbc1`, `Cbcs` — previously in `transmux`) and `hex::hex_encode` (previously a private helper in `transmux`), plus a `serde` feature gate. Moving `CencScheme` here lets both `transmux` and `broadcast-hls` name it without a circular dependency.

## What's new

- `cenc::CencScheme` — ISO/IEC 23001-7 protection scheme identity enum, with `Display`/`FromStr` and optional `serde`.
- `hex::hex_encode` — write a byte slice as lowercase hex into a `&mut [u8]` buffer.
- `serde` feature gate (off by default) — enables `Serialize`/`Deserialize` on `CencScheme`.

## Migration

No breaking changes (purely additive minor).
