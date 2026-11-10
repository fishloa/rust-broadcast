---
status: DRAFT
---
# rmt-flute 0.9.0

_Released 2026-11-10._

Minor release: byte-exact round trips for LCT/NORM/ExtTime plus a `Parse` impl, with breaking struct changes.

Version: 0.8.0 -> 0.9.0.

## Breaking changes and migration

- `LctHeader` gained the `reserved: u8` field and `NormCmd` the `reserved: [u8; 3]` field (struct literals need them; senders write 0).
- `ExtTime::to_content` returns `Result<Vec<u8>>` and `ExtTime::to_extension` returns `Result<HeaderExtension>`.

```rust
// before
let h = LctHeader { /* ... */ };
let e = ExtTime::to_content(&ext);
// after
let h = LctHeader { /* ... */, reserved: 0 };
let e = ExtTime::to_content(&ext)?; // may fail with Error::InvalidField
```

## Added
- `broadcast_common::Parse` implemented for `LctHeader`, `HeaderExtension` and `ExtTime` (delegating to the existing inherent methods).

## Fixed
- `LctHeader` keeps the two reserved `Res` bits (RFC 5651 §5.1) so parse then serialize is byte-exact; a value over 2 bits is rejected on serialize.
- `NormCmd` keeps the sub-type word bytes that EOT, CC, REPAIR_ADV, ACK_REQ and APPLICATION leave reserved (RFC 5740 §4.2.3), so parse then serialize is byte-exact.
- `ExtTime::to_content` and `to_extension` no longer emit SCT-Low without SCT-High, which `ExtTime::parse` rejects; they return `Error::InvalidField` instead.

---

Published from tag `rmt-flute-v0.9.0`.
