# dvb-bbframe 16.0.1

_Released 2026-11-23._

Patch release with one fix and one new constant.

## Fixed

- `CarryOverExtractor` no longer panics (debug) or indexes out of bounds (release) when a frame's user-packet layout changes while a partial packet is carried over (#4049). The carry records the mode and stride it was collected under; an NM frame whose `UPL`/ISSYI/NPD stride differs, or a HEM frame after an NM carry (or the reverse), now discards the partial packet, counts one `partial_discards`, and resyncs at `SYNCD`. Before, `stride - pos` underflowed when the new stride was not larger than the carried byte count, against the crate's never-panic-on-malformed-input contract.

## Added

- `SYNCD_NO_UP_START` (`0xFFFF`), the `SYNCD` value meaning no user packet starts in the data field. Bare numeric literals in the extractor, CRC-8, roll-off and ISSY decoders are replaced by named constants; behaviour is unchanged.

---

Published from tag `dvb-bbframe-v16.0.1`.
