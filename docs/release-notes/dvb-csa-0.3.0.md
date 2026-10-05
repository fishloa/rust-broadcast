# dvb-csa 0.3.0

_Released 2026-10-05._

Breaking release (0.2 -> 0.3) with two parts. First, a key-material hygiene change: `ControlWord` and every type holding control-word-derived cipher state now zero themselves on drop, and `ControlWord` is no longer `Copy`, so a value cannot be silently duplicated on the stack past the reach of its `Drop`. Second, the TS-packet helpers become correct for even/odd key pairs: `scramble_ts_packet` and `descramble_ts_packet` now take a `KeyParity` and refuse to scramble twice or descramble with the wrong key (#1093). Upgrade if a control word's lifetime in memory matters to your threat model (for example a process that handles several subscribers' control words, or whose memory could be inspected after a crash or core dump), or if you use the `ts` helpers on streams that mix clear and scrambled packets. You must act if you call `ts::scramble_ts_packet` / `ts::descramble_ts_packet`, copy a `ControlWord` implicitly, or call `ControlWord::expand_block` / `expand_stream`.

## Breaking changes

### 1. `ts::scramble_ts_packet` / `ts::descramble_ts_packet` take a `KeyParity` (#1093)

Both gain a `parity: ts::KeyParity` parameter (`Even` or `Odd`, `#[non_exhaustive]`) naming which control word `cw` is.

```rust
// before (0.2): always stamped transport_scrambling_control = 10 (even)
ts::scramble_ts_packet(&cw, &mut packet)?;
ts::descramble_ts_packet(&cw, &mut packet)?;
// after (0.3)
use dvb_csa::ts::{self, KeyParity};
ts::scramble_ts_packet(&cw, KeyParity::Even, &mut packet)?;
ts::descramble_ts_packet(&cw, KeyParity::Even, &mut packet)?;
```

Behaviour that goes with it:
- `scramble_ts_packet` writes `10` or `11` into `transport_scrambling_control` according to `parity`, and returns `Error::AlreadyScrambled { found }` if the packet's field is not already `00`. Before, it re-scrambled blindly and always stamped `10`.
- `descramble_ts_packet` is now a no-op returning `Ok(())`, payload untouched, on a packet whose field is `00` (for example a PSI or PCR-only packet interleaved with a scrambled stream). Before, it ran the cipher over it and corrupted data that was never encrypted. A packet scrambled under the other parity returns `Error::ParityMismatch { expected, found }` instead of being descrambled with the wrong control word.
- `Error` (which is `#[non_exhaustive]`) gained `NoPayload` (adaptation-field-only packets, or an `adaptation_field_length` that consumes the whole packet), `AlreadyScrambled` and `ParityMismatch`. Cases that used to report a fabricated `BufferTooShort` for "no payload present" now report `NoPayload`; the 188-byte buffer was never short.

### 2. `ControlWord` no longer implements `Copy`

`Clone` is kept. A `Copy` type can be duplicated on the stack without running `Drop` on each copy, so an implicit duplicate could linger unzeroed. Clone explicitly where a second owned value is genuinely needed. Code that already borrowed `&ControlWord` or used a single owned value is unchanged; code that relied on an implicit copy (passing one by value twice, storing it in a `Copy` struct) now fails to compile at that point.

```rust
// before: let a = cw; let b = cw;
// after:  let a = cw.clone(); let b = cw;
```

### 3. `ControlWord::expand_block` / `expand_stream` are `pub(crate)`

They returned the raw key schedule and stream seed derived from a control word, a second unaudited way to hand out key material. If you called them, route through the public `scramble` / `descramble` functions (and the `ts` helpers) instead.

## Behaviour changes (no signature change)

- `ControlWord`'s `Debug` is hand-written and prints a redacted placeholder instead of the 8 key bytes.
- `ControlWord` zeroes its 8 bytes on `Drop` (a per-byte volatile write plus a compiler fence).
- `ControlWord`'s `PartialEq` folds XOR over all 8 bytes instead of a derived, short-circuiting byte-at-a-time comparison, so comparison time does not depend on where two control words first differ.
- `BlockCipher`, `StreamCipher` and the bitsliced `BitslicedBlock` / `BitslicedStream` zero their expanded key schedule / stream seed on `Drop`. The expanded schedule that `scramble` / `descramble` and the bitsliced batch functions hold in local variables is wrapped in a small zeroizing newtype, so that local copy is cleared too. The new code is in `src/zeroize.rs`.

## Dependencies and features (from the Cargo.toml diff against `dvb-csa-v0.2.0`)

```toml
# before (0.2.0)
broadcast-common = { path = "../broadcast-common", version = "9.3", default-features = false }
mpeg-ts          = { path = "../mpeg-ts", version = "0.4", default-features = false }
criterion = { version = "0.5", features = ["html_reports"] }   # dev-dependency
std       = ["broadcast-common/std", "thiserror/std", "mpeg-ts/std"]
# after (0.3.0)
mpeg-ts          = { path = "../mpeg-ts", version = "0.5", default-features = false }
criterion = { version = "0.8", features = ["html_reports"] }   # dev-dependency
std       = ["thiserror/std", "mpeg-ts/std"]
```

- `broadcast-common` is dropped: it was never referenced in source, so this crate no longer tracks its epoch.
- `mpeg-ts` `0.4` -> `0.5`: a new caret epoch for the TS types whose constants and `ScramblingControl` this crate uses. If you also depend on `mpeg-ts` directly, move it to `0.5` too. See `mpeg-ts-0.5.0.md`.
- MSRV is unchanged at 1.95.0 (already 1.95.0 at `dvb-csa-v0.2.0`).

The README example now shows the `KeyParity` form and is compiled as a doctest on `ts::scramble_ts_packet`.

---

Published from tag `dvb-csa-v0.3.0`.
