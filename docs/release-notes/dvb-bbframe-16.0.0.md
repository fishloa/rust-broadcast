# dvb-bbframe 16.0.0

_Released 2026-11-22._

Lockstep major (15.1 -> 16.0). Own breaking change: the `num_enum` removal, so `TsGs` and `Mode` convert through the crate's own errors. The rest is a test-hardening pass over the framing arithmetic and diagnostics; no functional change.

## Breaking changes

- `num_enum` is no longer a dependency (#3963). `TsGs` and `Mode` implement a hand-written `TryFrom<u8>` whose error is this crate's own `Error::UnsupportedTsGs { ts_gs }` / `Error::InvalidMode { mode }` instead of `num_enum::TryFromPrimitiveError<TsGs>` / `<Mode>`; the `From<num_enum::TryFromPrimitiveError<_>> for Error` impls are removed, and `Mode` gains `From<Mode> for u8`. Migration: match on `dvb_bbframe::Error`.
- Builds against `broadcast-common` 14 (the `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2); a consumer must resolve a single `broadcast-common` 14.

## Changed

- The `wire_enum!` macro now comes from `broadcast_common`, with this crate's `Error` as its error type; the generated enums and conversions are unchanged (#3989).

## Tests

The pass pins the cross-frame `CarryOverExtractor` carry (a user packet split across BBFrames, `SYNCD=0xFFFF` continuation, a mismatched `SYNCD` discarding the partial), `BbframePump` per-PLP counter aggregation and PLP carry discard, the `DFL` ceiling on parse and serialize, `nm_stride_bytes` rejecting a non-byte-aligned `UPL`, the roll-off decode and `Matype::roll_off`, field-enum labels, and `HemTsIter`/`UpIter` `size_hint`.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3989 (`wire_enum!` one home), #4017 (Breaking/Changed split).

---

Published from tag `dvb-bbframe-v16.0.0`.
