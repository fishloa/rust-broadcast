# broadcast-common 9.4.0

_Released 2026-10-05._

### Added
- **`seq`** — `SeqSpace`: wrap-safe modular sequence-number arithmetic (`add`/`diff`/`lt`/`leq`/`gt`/`geq`/`in_closed_range`) over a `bits`-wide space, the one algorithm behind `rist-runtime` (16-bit) and `srt-runtime` (31-bit) `arq::seq` (audit r08-RIST-O1, #1141).
- **`pusi::PusiAccumulator`** (+ `DEFAULT_MAX_UNIT_SIZE`, 16 MiB) — the one PUSI-delimited unit-accumulation rule (ISO/IEC 13818-1 §2.4.3.2: ignore payload before the first PUSI, a PUSI closes the in-progress unit, a capped unit is discarded until the next PUSI), shared by `mpeg-pes::PesAssembler` and `mpeg-ts::PusiReassembler` instead of two diverged copies (audit r01-W13, #1074).
- **`len`** — `FieldOverflow` + the `fit_bits`/`fit_u8`/`fit_u16`/`fit_u24`/
  `fit_u32` helpers, for serializers to reject, not truncate, oversized
  length/count fields: the comparison happens before any narrowing, so a
  value an `as u16` would silently wrap (e.g. 65 540 → 4) is returned as a
  `FieldOverflow { field, value, max }` the caller maps into its own error
  type instead of emitting a corrupt frame; see #1129.
- **`Serialize::try_to_bytes`** — allocates and serializes like `to_bytes`
  but returns the serializer's error instead of panicking, truncating to the
  byte count actually written; prefer it for hand-built values that can
  violate a wire constraint (#1143).
- **`clock33::{add, add_signed, signed_distance}`** — modular arithmetic on
  the 33-bit PTS/DTS clock: `(a + b) mod 2^33` (e.g. SCTE-35 `pts_time` +
  `pts_adjustment`), a signed-delta variant always landing in `[0, 2^33)`,
  and the shortest signed distance between two samples in `(-2^32, 2^32]`;
  inputs at or above the modulus are reduced first (#1137).

### Fixed
- **`time::encode_mjd_bcd`** — now rejects `hour > 23` / `minute > 59` /
  `second > 59` (matching the ranges `decode_mjd_bcd` enforces); previously
  it only bottomed out at `to_bcd_byte`'s `<= 99` check, so it could encode
  a value `decode_mjd_bcd` then refused to decode back (#1073).
- **`time`** — the MJD↔calendar conversion algorithm (EN 300 468 Annex C)
  existed as two independent hand-copies (the dependency-free `_nogate`
  helpers and the `chrono`-gated public functions); both now share one
  `mjd_to_ymd_core`/`ymd_to_mjd_core` implementation so the two can no
  longer silently drift apart (#1073).
- **`ts_dup::is_legal_duplicate_pair`** — the PCR exemption now requires
  `adaptation_field_length >= 7` (flags byte + 6-byte PCR), not just that
  the packet buffer is long enough; a packet whose adaptation field claims
  `PCR_flag` but declares too little length no longer has trailing payload
  bytes wrongly treated as the exempt PCR field (#1073).

### Changed
- `hex::hex_encode` now delegates to the `hex` crate (`no_std` + `alloc`); output and signature unchanged. No API change.

---

Published from tag `broadcast-common-v9.4.0`.
