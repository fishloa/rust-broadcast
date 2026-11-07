
# dvb-t2mi 13.0.0

_Released 2026-11-07._

Lockstep major (12.1.0 -> 13.0.0). The breaking changes and their migration snippets are in the lockstep note [v13.0.0](v13.0.0.md); this note lists what changed in `dvb-t2mi`.

## Removed
- BREAKING: Unused public items, with no use in the workspace beyond their own unit tests: `drain_packets`, `guard_interval_set`, `inband_loop`.
## Changed
- Error enums derive `Clone`, `PartialEq` and `Eq` wherever every field allows it (c6 C6-14).
- Length pre-checks go through `broadcast_common::len::{need_in, need_out}`; the error values are unchanged (c3 S1).
- `PayloadObject` comes from `broadcast_common::declare_object_trait!`; the `serde` feature no longer names `erased-serde` itself (c2 H1).
- BREAKING: `Error::ReservedBitsViolation { field, reason }` is removed. Non-zero RFU bits return `Error::ReservedBits(ReservedBits { field, expected: 0, found })`; a serializer value that does not fit its wire field returns `Error::FieldOverflow` (including the individual-addressing body fields and length fields, and `framed_block bit_len`); enum-conversion failures (`s1_field`, `subpart_variety`, `freq_source`, `bw`), `aux_id` and `frequency_offset` return `Error::InvalidValue` carrying the value; a pre-2000 or over-range emission date returns `Error::InvalidField`; an `L1ExtBlock` whose `data` length disagrees with `data_bit_len` returns `Error::LengthMismatch`. New variants: `ReservedBits`, `InvalidValue`, `InvalidField`, `FieldOverflow`, `LengthMismatch`. The RFU policy itself (reject non-zero RFU, except individual addressing) is unchanged.
- BREAKING: error variants `BufferTooShort`, `OutputBufferTooSmall`, `CrcMismatch` now wrap `broadcast_common::error` types (tuple variants; `CrcMismatch` gains `what` = "T2-MI packet", message "<what> CRC-32 mismatch: ..."). Hand-written `From` impls removed. `ReservedBitsViolation` stays crate-specific (free-text reason).
## Added
- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type, producing the identical existing variants (c3 S1).
## Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11, `dvb-bbframe` 12 -> 13, `mpeg-ts` 0.6 -> 0.7. The `Cargo.toml` requirements name the new versions; no other manifest change.

---

Published from tag `v13.0.0`.
