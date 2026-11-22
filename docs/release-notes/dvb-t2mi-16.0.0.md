# dvb-t2mi 16.0.0

_Released 2026-11-22._

Lockstep major (15.1 -> 16.0). Own breaking change: the `num_enum` removal, plus jiff 1.x for `T2TimestampPayload::emission_time_utc`. The rest is a test-hardening pass over the framing and payload layer; no functional change.

## Breaking changes

- `num_enum` is no longer a dependency (#3963). `PacketType`, `FrequencySource`, `S1Field`, `Bandwidth`, `SubpartVariety`, `PrbsType` and `AddressingFunctionTag` implement a hand-written `TryFrom<u8>` (`TryFrom<u16>` for `SubpartVariety`) whose error is this crate's own: `Error::InvalidPacketType { found }` for `PacketType` and `Error::InvalidValue` naming the field and the rejected value for the others. The `From<num_enum::TryFromPrimitiveError<T>> for Error` impls are removed. Migration: match on `dvb_t2mi::Error`.
- `jiff` 0.2 -> 1.0: `T2TimestampPayload::emission_time_utc` returns a jiff 1.x `Timestamp`.
- Builds against `broadcast-common` 14, `dvb-bbframe` 16 and `mpeg-ts` 0.10. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2); a consumer must resolve a single copy of each.

## Changed

- The `wire_enum!` macro now comes from `broadcast_common`, with this crate's `Error` as its error type; the generated enums and conversions are unchanged (#3989).

## Tests

The pass pins what survived mutation in the framing and payload layer: the raw-mode resync buffer bound and hunt statistics, `PacketReassembler::drop_buffered`, HEM-NPD gating in `InnerTsRecovery`, the packet header length and payload slicing, the FEF/arbitrary-cells/P2-bias/individual-addressing/timestamp payload fields and limits, the L1-post framed parse/write with AUX blocks and extension blocks, and the L1 enum wire values and labels.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3989 (`wire_enum!` one home), #4017 (Breaking/Changed split).

---

Published from tag `dvb-t2mi-v16.0.0`.
