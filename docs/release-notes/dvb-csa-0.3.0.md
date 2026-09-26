# dvb-csa 0.3.0 — 2026-09-26

Security release: `ControlWord` and its derived cipher state no longer outlive their usefulness
in memory. **Upgrade if a `ControlWord`'s lifetime in memory matters to your threat model** —
for example a process that handles multiple subscribers' control words, or one whose memory
could be inspected after a crash or core dump. This is a breaking release: `ControlWord` no
longer implements `Copy`, and two previously-public key-expansion methods are now crate-private.

## Security

| Advisory | Before this release |
|---|---|
| GHSA-f4mf-69xv-w6rp | `ControlWord` implemented `Copy`, so a value could be duplicated on the stack without the type's `Drop` zeroing every copy — a caller had no way to be sure a control word wasn't lingering in an extra stack slot after the original was dropped. `Debug` printed the raw 8 key bytes. Nothing zeroed a dropped `ControlWord`'s bytes, or the expanded key schedule/stream seed held by `BlockCipher`, `StreamCipher`, or the bitsliced batch types, so key material could remain readable in freed memory. `PartialEq` also used a derived, short-circuiting comparison, which is not appropriate for secret material. |

## Breaking changes

- **`ControlWord` no longer implements `Copy`** (`Clone` is kept). A `Copy` type can be
  duplicated on the stack without running `Drop` on each copy, so the compiler could hand out
  implicit duplicates a caller never explicitly asked for. Clone explicitly where a second owned
  value is genuinely needed. Every in-crate call site (lib, tests, examples, benches) already
  borrowed `&ControlWord` or used a single owned value, so downstream code that follows the same
  pattern needs no changes; code that relied on an implicit copy now gets a compile error at the
  point that copy happened.
- **`ControlWord::expand_block`/`expand_stream` are now `pub(crate)`.** They returned the raw key
  schedule/stream seed derived from a control word — a second, unaudited way to hand out key
  material alongside the crate's own scramble/descramble API. If nothing outside this crate
  referenced them, this is a no-op; if something did, it needs to route key material through the
  crate's public (de)scrambling functions instead.

## Behaviour changes

- **`ControlWord`'s `Debug` impl is hand-written** and prints a redacted placeholder instead of
  the control word's bytes.
- **`ControlWord` zeroes its 8 bytes on `Drop`** (a per-byte volatile write plus a compiler
  fence), so a dropped value does not linger readable in freed memory.
- **`ControlWord`'s `PartialEq` folds XOR over all 8 bytes** instead of a derived,
  short-circuiting byte-at-a-time comparison.
- **Every type holding control-word-derived cipher state** (`BlockCipher`, `StreamCipher`, and
  the bitsliced `BitslicedBlock`/`BitslicedStream`) now zeroes that state on `Drop`, the same way
  `ControlWord` does. The expanded key schedule/stream seed that `scramble`/`descramble` and the
  bitsliced batch functions hold in local variables before handing off to those types is now
  wrapped in a small zeroizing newtype too, so the local copy is cleared as well.

## Other changes

- `ts::ts_payload_mut` now decodes `adaptation_field_control` via the `mpeg-ts` dependency's
  `mpeg_ts::ts::TsHeader::parse` instead of hand-rolling the same AFC bit decode — a
  duplication-audit finding. Magic numbers `188`/`0x3f`/`0x80` are replaced with named,
  spec-cited constants (`mpeg_ts::ts::TS_PACKET_SIZE`/`SCRAMBLING_MASK`, plus a local
  `TSC_EVEN_KEY`). The payload byte-offset computation and the mutable slicing itself stay
  hand-rolled, since `mpeg_ts::ts::TsPacket` only exposes an immutable `payload: &[u8]` and CSA
  (de)scrambling needs to write back into the caller's own buffer. Identical behaviour; no public
  API change beyond the breaking changes above.

## Migration

Most call sites are unaffected. If your code copies a `ControlWord` implicitly (e.g. passing one
by value more than once, or storing it in a `Copy` struct), switch to an explicit `.clone()`. If
you called `ControlWord::expand_block`/`expand_stream` directly, use the crate's public
scramble/descramble functions instead — they are the crate's own way to reach the same
key-derived state without exposing it further.

MSRV 1.95.0.
