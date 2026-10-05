# broadcast-common 9.4.0

_Released 2026-10-05._

**Minor, additive.** Five new building blocks that replace logic previously duplicated across dependent crates, plus three small fixes in `time` and `ts_dup`. Nothing is removed or changed in signature; existing callers need no code change. `broadcast-common` is the fan-in crate, so this release is the one the rest of the wave requires: the notes for [mpeg-ts 0.5.0](mpeg-ts-0.5.0.md), [mpeg-pes 0.5.0](mpeg-pes-0.5.0.md), [mpeg-ps 0.5.0](mpeg-ps-0.5.0.md), [dvb-si 11.0.0](dvb-si-11.0.0.md), [scte35-splice 3.0.0](scte35-splice-3.0.0.md) and [scte104 0.5.0](scte104-0.5.0.md) all say "requires `broadcast-common` 9.4". Publish this crate first.

## Dependency change

```toml
# broadcast-common/Cargo.toml
+hex = { version = "0.4", default-features = false, features = ["alloc"] }   # backs hex::hex_encode
```

`no_std` is preserved (`alloc` only). Also, the dev-dependency `criterion` moved from 0.5 to 0.8 (benchmarks only, not visible to consumers).

## New

- **`len`**: `FieldOverflow { field, value, max }` plus `fit_bits`, `fit_u8`, `fit_u16`, `fit_u24` and `fit_u32`. Serializers use these to reject, not truncate, an oversized length or count. The comparison happens before any narrowing, so a value that `as u16` would wrap (65 540 becomes 4) is returned as an error the caller maps into its own error type (#1129). `fit_bits` panics if `bits` is outside `1..=64`.
- **`Serialize::try_to_bytes`**: allocates and serializes like `to_bytes`, but returns the serializer's error instead of panicking. Prefer it for hand-built values that can violate a wire constraint (#1143).
- **`seq::SeqSpace`**: wrap-safe modular sequence-number arithmetic over a `bits`-wide space (`add`, `diff`, `lt`, `leq`, `gt`, `geq`, `in_closed_range`). `SeqSpace::new(bits)` panics unless `1 <= bits <= MAX_SEQ_BITS` (31). It is the one algorithm behind `rist-runtime` (16-bit) and `srt-runtime` (31-bit) `arq::seq` (audit r08-RIST-O1, #1141).
- **`pusi::PusiAccumulator`** (and `DEFAULT_MAX_UNIT_SIZE`, 16 MiB): the PUSI-delimited unit accumulation rule of ISO/IEC 13818-1 §2.4.3.2. Payload before the first PUSI is ignored, a PUSI closes the in-progress unit, and a unit over the cap is discarded until the next PUSI. `mpeg-pes::PesAssembler` and `mpeg-ts::PusiReassembler` now both use it instead of two diverged copies (audit r01-W13, #1074).
- **`clock33::{add, add_signed, signed_distance}`**: modular arithmetic on the 33-bit PTS/DTS clock. `add` is `(a + b) mod 2^33` (for example SCTE-35 `pts_time` + `pts_adjustment`), `add_signed` always lands in `[0, 2^33)`, and `signed_distance` returns the shortest signed distance in `(-2^32, 2^32]`. Inputs at or above the modulus are reduced first (#1137).

## Behaviour changes and fixes

- `time::encode_mjd_bcd` returns `None` for `hour > 23`, `minute > 59` or `second > 59`, matching `decode_mjd_bcd`. It used to stop only at `to_bcd_byte`'s `<= 99` check, so it could encode a value `decode_mjd_bcd` then refused (#1073). A caller that fed it out-of-range time fields and unwrapped will now see `None`.
- `time`: the EN 300 468 Annex C MJD to calendar conversion existed as two hand copies (the dependency-free `_nogate` helpers and the `chrono`-gated functions); both now share one `mjd_to_ymd_core` / `ymd_to_mjd_core`, so they cannot drift (#1073). No output change is claimed.
- `ts_dup::is_legal_duplicate_pair`: the PCR exemption now requires `adaptation_field_length >= 7` (flags byte plus the 6-byte PCR), not merely a long-enough buffer. A packet whose adaptation field claims `PCR_flag` but declares too little length no longer has trailing payload bytes treated as the exempt PCR (#1073).
- `hex::hex_encode` now delegates to the `hex` crate; output and signature are unchanged.

---

Published from tag `broadcast-common-v9.4.0`.
