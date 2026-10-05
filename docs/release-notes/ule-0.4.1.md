# ule 0.4.1

_Released 2026-10-05._

### Added
- `ExtensionHeader::validate()`: public check that an `Optional` header's `h_len` is `1..=5` and
  its `body.len()` equals `2*h_len-2` (what `PayloadChain::serialize_into` now enforces before
  writing; see Fixed) (#1120).

### Fixed
- `ExtensionHeader::validate` (called by `PayloadChain::serialize_into`
  before any bytes are written) now rejects an `Optional` header whose
  `body.len()` does not equal exactly `2*h_len-2`, or whose `h_len` is
  outside `1..=5` (#1120). Previously a mismatched `body` could panic
  (`h_len` too small for `body`) or silently misframe the chain (`h_len` too
  large), and `h_len == 0` panicked computing `wire_len() - 2` inside
  `serialized_len()` itself.
- `Sndu::serialize_into` now rejects `D=1` with `Length=0x7FFF`: that exact
  combination is the reserved End Indicator (`0xFFFF` on the wire, §6), and
  every receiver (including this crate's own `ts::UleReceiver`) stops the
  packet there (#1120).
- `ts::UleReceiver` no longer treats a leading `0xFF` byte alone as padding —
  only the genuine 2-byte `0xFFFF` End Indicator, or a single trailing `0xFF`
  byte too short for any header, stop the packing walk. A legal `D=1` SNDU
  with `Length` in `0x7F00..=0x7FFE` also starts with `0xFF` and was
  previously discarded as if it were padding (#1120).
- `ts::UleReceiver` no longer walks non-padding bytes left over after a
  PUSI=0 continuation completes an SNDU as a new packed SNDU — RFC 4326 §6/§7
  only lets an SNDU start where a PUSI=1 packet's Payload Pointer says one
  does. Such bytes now reset the receiver to the Idle State instead (#1120).

---

Published from tag `ule-v0.4.1`.
